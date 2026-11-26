mod host_windows;
mod renderer;

pub use host_windows::{
    collect_tooltip_island_origins, find_visible_alttab_host_window,
    find_visible_hover_flyout_host_window, find_visible_input_switcher_host_window,
    find_visible_menu_flyout_host_windows, find_visible_osd_host_window,
    find_visible_search_host_windows, find_visible_shell_experience_host_windows,
    find_visible_shell_host_windows, find_visible_start_menu_windows,
    find_visible_tooltip_host_window,
};

use host_windows::{find_visible_overflow_window, should_hide_backdrop_for_taskbar};
use renderer::{
    render_layered_backdrop, render_shell_card_layered_backdrop, BAR_SILK_H, BAR_SILK_W,
};
use super::config::TaskbarStyle;
use super::swca::{
    backdrop_layer_plan, expand_card_rect_for_shadow, gpu_backdrop_recipe,
    overflow_backdrop_layer_plan, MAX_MENU_FLYOUT_SLOTS, MAX_SHELL_CARD_SLOTS,
    MENU_FLYOUT_SHADOW_PAD_DIP, OSD_SHADOW_PAD_DIP, TOOLTIP_SHADOW_PAD_DIP,
};
use crate::color::NEUTRAL_BACKDROP_FALLBACK;
use crate::system::{
    exclude_from_peek, is_occluded_by_same_band_topmost, overflow_backdrop_anchor,
    register_taskbar_backdrop, resolve_unified_accent, taskbar_backdrop_anchor,
    unregister_taskbar_backdrop, window_band,
};
use crate::ui::glass::{create_dc_render_target, CompositionEngine, LayeredSurface, PopupNoiseCache};
use crate::ui::{compute_halo_pair, SILK_H, SILK_W};
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Foundation::Numerics::Vector2;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionBrush, SpriteVisual, Visual};
use windows::Win32::Foundation::{BOOL, D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1BitmapBrush, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1Layer,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWCP_DONOTROUND, DWMWCP_ROUND, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
    DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_WINDOW_CORNER_PREFERENCE, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindow, GetWindowLongW, GetWindowRect,
    RegisterClassExW, SetWindowPos, ShowWindow, GWL_EXSTYLE, GW_HWNDNEXT, HWND_TOPMOST,
    HTTRANSPARENT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, WM_ERASEBKGND,
    WM_NCHITTEST, WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const GLASS_BLUR_CLASS: PCWSTR = w!("PrettyWindowsTaskbarBlurWnd");
const GLASS_OVERLAY_CLASS: PCWSTR = w!("PrettyWindowsTaskbarGlassWnd");


struct MonitorGlassBackdrop {
    blur_hwnd: HWND,
    comp_target: Option<DesktopWindowTarget>,
    comp_visual: Option<SpriteVisual>,
    last_blur_key: Option<(TaskbarStyle, bool, u32)>,
    blur_size: (i32, i32),
    has_live_blur: bool,
    glass_hwnd: HWND,
    // Declare D2D child resources and `rt` before `surface` so `rt` releases before `DeleteDC`.
    noise_brush: Option<ID2D1BitmapBrush>,
    noise_key: Option<(bool, u8)>,
    silk_bmp: Option<ID2D1Bitmap>,
    silk_buf: Vec<u8>,
    last_silk_tick: Option<Instant>,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    last_rect: RECT,
    blur_visible: bool,
    glass_visible: bool,
    blur_policy_dirty: bool,
    glass_dirty: bool,
}

impl Drop for MonitorGlassBackdrop {
    fn drop(&mut self) {
        unregister_taskbar_backdrop(self.glass_hwnd);
        unregister_taskbar_backdrop(self.blur_hwnd);
        if let Some(ref visual) = self.comp_visual {
            let _ = visual.SetBrush(None::<&CompositionBrush>);
        }
        if let Some(target) = self.comp_target.take() {
            let _ = target.SetRoot(None::<&Visual>);
            let _ = target.Close();
        }
        self.comp_visual = None;
        unsafe {
            if !self.glass_hwnd.is_invalid() {
                let _ = DestroyWindow(self.glass_hwnd);
            }
            if !self.blur_hwnd.is_invalid() {
                let _ = DestroyWindow(self.blur_hwnd);
            }
        }
    }
}

struct ShellCardGlassBackdrop {
    blur_hwnd: HWND,
    comp_target: Option<DesktopWindowTarget>,
    comp_visual: Option<SpriteVisual>,
    last_blur_key: Option<(TaskbarStyle, bool, u32)>,
    blur_size: (i32, i32),
    has_live_blur: bool,
    glass_hwnd: HWND,
    // Declare D2D child resources and `rt` before `surface` so `rt` releases before `DeleteDC`.
    noise_cache: PopupNoiseCache,
    silk_bmp: Option<ID2D1Bitmap>,
    silk_buf: Vec<u8>,
    last_silk_tick: Option<Instant>,
    shadow_geo_key: Option<(i32, i32, i32, i32, i32, u32)>,
    cached_clip_geo: Option<ID2D1Geometry>,
    cached_shadow_rings: [Option<ID2D1Geometry>; 3],
    layer: ID2D1Layer,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    last_rect: RECT,
    last_shadow_pad: i32,
    blur_visible: bool,
    glass_visible: bool,
    blur_policy_dirty: bool,
    glass_dirty: bool,
}

impl Drop for ShellCardGlassBackdrop {
    fn drop(&mut self) {
        if let Some(ref visual) = self.comp_visual {
            let _ = visual.SetBrush(None::<&CompositionBrush>);
        }
        if let Some(target) = self.comp_target.take() {
            let _ = target.SetRoot(None::<&Visual>);
            let _ = target.Close();
        }
        self.comp_visual = None;
        unsafe {
            if !self.glass_hwnd.is_invalid() {
                let _ = DestroyWindow(self.glass_hwnd);
            }
            if !self.blur_hwnd.is_invalid() {
                let _ = DestroyWindow(self.blur_hwnd);
            }
        }
    }
}

/// Manages full-width real-time GPU blur / Acrylic / Liquid Glass backdrop windows behind every
/// monitor's taskbar as well as rounded shell cards (system tray overflow flyout, Alt+Tab switcher,
/// taskbar hover flyout previews, and hardware Confirmator OSD capsules).
pub struct LiquidGlassManager {
    style: TaskbarStyle,
    light_glass: bool,
    transparency_enabled: bool,
    halo_colors: [(f32, f32, f32); 2],
    halo_ready: bool,
    flow_phase: f64,
    last_flow_tick: Instant,
    last_color_tick: Instant,
    // Declare backdrops before `comp` and `d2d_factory` so all window targets close before Compositor drops.
    backdrops: HashMap<isize, MonitorGlassBackdrop>,
    overflow: Option<ShellCardGlassBackdrop>,
    alt_tab: Option<ShellCardGlassBackdrop>,
    hover_flyout: Option<ShellCardGlassBackdrop>,
    osd: Option<ShellCardGlassBackdrop>,
    tooltip: Option<ShellCardGlassBackdrop>,
    input_switcher: Option<ShellCardGlassBackdrop>,
    shell_cards: [Option<ShellCardGlassBackdrop>; MAX_SHELL_CARD_SLOTS],
    shell_card_keys: [(isize, usize); MAX_SHELL_CARD_SLOTS],
    menu_flyouts: [Option<ShellCardGlassBackdrop>; MAX_MENU_FLYOUT_SLOTS],
    menu_flyout_host_hwnds: [isize; MAX_MENU_FLYOUT_SLOTS],
    last_comp_recreate: Option<Instant>,
    comp: Option<CompositionEngine>,
    d2d_factory: ID2D1Factory1,
}

fn hide_card_backdrop(slot: &mut Option<ShellCardGlassBackdrop>) {
    if let Some(ref mut card) = *slot {
        unsafe {
            if card.glass_visible {
                let _ = ShowWindow(card.glass_hwnd, SW_HIDE);
                card.glass_visible = false;
            }
            if card.blur_visible {
                let _ = ShowWindow(card.blur_hwnd, SW_HIDE);
                card.blur_visible = false;
            }
        }
    }
}

impl LiquidGlassManager {
    pub fn new(
        d2d_factory: ID2D1Factory1,
        light_glass: bool,
        transparency_enabled: bool,
    ) -> Self {
        Self {
            d2d_factory,
            comp: CompositionEngine::try_new(),
            style: TaskbarStyle::Default,
            light_glass,
            transparency_enabled,
            halo_colors: [(0.76, 0.76, 0.78); 2],
            halo_ready: false,
            flow_phase: 0.0,
            last_flow_tick: Instant::now(),
            last_color_tick: Instant::now(),
            backdrops: HashMap::new(),
            overflow: None,
            alt_tab: None,
            hover_flyout: None,
            osd: None,
            tooltip: None,
            input_switcher: None,
            shell_cards: std::array::from_fn(|_| None),
            shell_card_keys: [(0, 0); MAX_SHELL_CARD_SLOTS],
            menu_flyouts: std::array::from_fn(|_| None),
            menu_flyout_host_hwnds: [0; MAX_MENU_FLYOUT_SLOTS],
            last_comp_recreate: None,
        }
    }

    fn recreate_composition_engine(&mut self) {
        if self
            .last_comp_recreate
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.last_comp_recreate = Some(Instant::now());

        for entry in self.backdrops.values_mut() {
            if let Some(ref visual) = entry.comp_visual {
                let _ = visual.SetBrush(None::<&CompositionBrush>);
            }
            if let Some(target) = entry.comp_target.take() {
                let _ = target.SetRoot(None::<&Visual>);
                let _ = target.Close();
            }
            entry.comp_visual = None;
            entry.last_blur_key = None;
            entry.blur_policy_dirty = true;
            entry.blur_size = (0, 0);
            entry.has_live_blur = false;
            entry.glass_dirty = true;
        }
        for card in [
            &mut self.overflow,
            &mut self.alt_tab,
            &mut self.hover_flyout,
            &mut self.osd,
            &mut self.tooltip,
            &mut self.input_switcher,
        ]
        .into_iter()
        .chain(self.shell_cards.iter_mut())
        .chain(self.menu_flyouts.iter_mut())
        .flatten()
        {
            if let Some(ref visual) = card.comp_visual {
                let _ = visual.SetBrush(None::<&CompositionBrush>);
            }
            if let Some(target) = card.comp_target.take() {
                let _ = target.SetRoot(None::<&Visual>);
                let _ = target.Close();
            }
            card.comp_visual = None;
            card.last_blur_key = None;
            card.blur_policy_dirty = true;
            card.blur_size = (0, 0);
            card.has_live_blur = false;
            card.glass_dirty = true;
        }

        self.comp = None;
        self.comp = CompositionEngine::try_new();
        if let Some(ref engine) = self.comp {
            for entry in self.backdrops.values_mut() {
                if !entry.blur_hwnd.is_invalid() {
                    if let Some((target, visual)) = engine.create_target_and_visual(entry.blur_hwnd)
                    {
                        entry.comp_target = Some(target);
                        entry.comp_visual = Some(visual);
                    }
                }
            }
            for card in [
                &mut self.overflow,
                &mut self.alt_tab,
                &mut self.hover_flyout,
                &mut self.osd,
                &mut self.tooltip,
                &mut self.input_switcher,
            ]
            .into_iter()
            .chain(self.shell_cards.iter_mut())
            .chain(self.menu_flyouts.iter_mut())
            .flatten()
            {
                if !card.blur_hwnd.is_invalid() {
                    if let Some((target, visual)) = engine.create_target_and_visual(card.blur_hwnd)
                    {
                        card.comp_target = Some(target);
                        card.comp_visual = Some(visual);
                    }
                }
            }
        }
    }

    pub fn is_overflow_visible(&self) -> bool {
        self.overflow
            .as_ref()
            .is_some_and(|ov| ov.glass_visible || ov.blur_visible)
            || self
                .alt_tab
                .as_ref()
                .is_some_and(|at| at.glass_visible || at.blur_visible)
            || self
                .hover_flyout
                .as_ref()
                .is_some_and(|hf| hf.glass_visible || hf.blur_visible)
            || self
                .osd
                .as_ref()
                .is_some_and(|os| os.glass_visible || os.blur_visible)
            || self
                .tooltip
                .as_ref()
                .is_some_and(|tt| tt.glass_visible || tt.blur_visible)
            || self
                .input_switcher
                .as_ref()
                .is_some_and(|isw| isw.glass_visible || isw.blur_visible)
            || self
                .shell_cards
                .iter()
                .flatten()
                .any(|sc| sc.glass_visible || sc.blur_visible)
            || self
                .menu_flyouts
                .iter()
                .flatten()
                .any(|mf| mf.glass_visible || mf.blur_visible)
    }

    pub fn hide_overflow_backdrop(&mut self) {
        hide_card_backdrop(&mut self.overflow);
    }

    pub fn hide_alt_tab_backdrop(&mut self) {
        hide_card_backdrop(&mut self.alt_tab);
    }

    pub fn hide_hover_flyout_backdrop(&mut self) {
        hide_card_backdrop(&mut self.hover_flyout);
    }

    pub fn hide_osd_backdrop(&mut self) {
        hide_card_backdrop(&mut self.osd);
    }

    pub fn hide_tooltip_backdrop(&mut self) {
        hide_card_backdrop(&mut self.tooltip);
    }

    pub fn hide_input_switcher_backdrop(&mut self) {
        hide_card_backdrop(&mut self.input_switcher);
    }

    pub fn hide_shell_cards(&mut self) {
        for slot in &mut self.shell_cards {
            hide_card_backdrop(slot);
        }
        self.shell_card_keys = [(0, 0); MAX_SHELL_CARD_SLOTS];
    }

    pub fn hide_menu_flyout_backdrops(&mut self) {
        for slot in &mut self.menu_flyouts {
            hide_card_backdrop(slot);
        }
        self.menu_flyout_host_hwnds = [0; MAX_MENU_FLYOUT_SLOTS];
    }

    pub fn set_style(&mut self, style: TaskbarStyle, taskbars: &[HWND]) {
        let style_changed = self.style != style;
        self.style = style;
        let plan = backdrop_layer_plan(style, self.transparency_enabled);
        let ov_plan = overflow_backdrop_layer_plan(style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.backdrops.clear();
        }
        if !ov_plan.use_gpu_blur && !ov_plan.use_layered_surface {
            self.overflow = None;
            self.alt_tab = None;
            self.hover_flyout = None;
            self.osd = None;
            self.tooltip = None;
            self.input_switcher = None;
            self.shell_cards = std::array::from_fn(|_| None);
            self.shell_card_keys = [(0, 0); MAX_SHELL_CARD_SLOTS];
            self.menu_flyouts = std::array::from_fn(|_| None);
            self.menu_flyout_host_hwnds = [0; MAX_MENU_FLYOUT_SLOTS];
        }
        if style_changed {
            if style == TaskbarStyle::LiquidGlass {
                self.last_flow_tick = Instant::now();
                self.last_color_tick = self.last_flow_tick;
                self.halo_ready = false;
            }
            for b in self.backdrops.values_mut() {
                b.blur_policy_dirty = true;
                b.glass_dirty = true;
                if style != TaskbarStyle::LiquidGlass {
                    b.last_silk_tick = None;
                }
            }
            for card in [
                &mut self.overflow,
                &mut self.alt_tab,
                &mut self.hover_flyout,
                &mut self.osd,
                &mut self.tooltip,
                &mut self.input_switcher,
            ]
            .into_iter()
            .chain(self.shell_cards.iter_mut())
            .chain(self.menu_flyouts.iter_mut())
            .flatten()
            {
                card.blur_policy_dirty = true;
                card.glass_dirty = true;
                if style != TaskbarStyle::LiquidGlass {
                    card.last_silk_tick = None;
                }
            }
        }
        if plan.use_gpu_blur || plan.use_layered_surface {
            self.sync(taskbars);
        }
        let _ = self.sync_overflow();
    }

    pub fn set_theme(&mut self, light_glass: bool) {
        if self.light_glass != light_glass {
            self.light_glass = light_glass;
            self.halo_ready = false;
            for b in self.backdrops.values_mut() {
                b.last_silk_tick = None;
                b.noise_key = None;
                b.blur_policy_dirty = true;
                b.glass_dirty = true;
            }
            for card in [
                &mut self.overflow,
                &mut self.alt_tab,
                &mut self.hover_flyout,
                &mut self.osd,
                &mut self.tooltip,
                &mut self.input_switcher,
            ]
            .into_iter()
            .chain(self.shell_cards.iter_mut())
            .chain(self.menu_flyouts.iter_mut())
            .flatten()
            {
                card.last_silk_tick = None;
                card.blur_policy_dirty = true;
                card.glass_dirty = true;
            }
        }
    }

    pub fn set_transparency_enabled(&mut self, enabled: bool) {
        if self.transparency_enabled != enabled {
            self.transparency_enabled = enabled;
            for b in self.backdrops.values_mut() {
                b.blur_policy_dirty = true;
                b.glass_dirty = true;
                b.last_silk_tick = None;
            }
            for card in [
                &mut self.overflow,
                &mut self.alt_tab,
                &mut self.hover_flyout,
                &mut self.osd,
                &mut self.tooltip,
                &mut self.input_switcher,
            ]
            .into_iter()
            .chain(self.shell_cards.iter_mut())
            .chain(self.menu_flyouts.iter_mut())
            .flatten()
            {
                card.blur_policy_dirty = true;
                card.glass_dirty = true;
                card.last_silk_tick = None;
            }
        }
    }

    pub fn reset_windows(&mut self) {
        self.backdrops.clear();
        self.overflow = None;
        self.alt_tab = None;
        self.hover_flyout = None;
        self.osd = None;
        self.tooltip = None;
        self.input_switcher = None;
        self.shell_cards = std::array::from_fn(|_| None);
        self.shell_card_keys = [(0, 0); MAX_SHELL_CARD_SLOTS];
        self.menu_flyouts = std::array::from_fn(|_| None);
        self.menu_flyout_host_hwnds = [0; MAX_MENU_FLYOUT_SLOTS];
    }

    fn advance_liquid_glass_clock(&mut self) {
        if self.style != TaskbarStyle::LiquidGlass {
            return;
        }
        let now = Instant::now();
        if self.halo_ready && now.duration_since(self.last_flow_tick) < Duration::from_millis(8) {
            return;
        }
        let dt_flow = now
            .duration_since(self.last_flow_tick)
            .as_secs_f32()
            .clamp(0.001, 0.1);
        self.last_flow_tick = now;
        self.flow_phase += f64::from(dt_flow);

        let dt_color = now
            .duration_since(self.last_color_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_color_tick = now;

        let (_, base_dark) = resolve_unified_accent(NEUTRAL_BACKDROP_FALLBACK, self.light_glass);
        let pair = compute_halo_pair(base_dark, self.light_glass);
        let target = [pair[0].to_normalized(), pair[1].to_normalized()];
        if !self.halo_ready {
            self.halo_colors = target;
            self.halo_ready = true;
        } else {
            let k = (dt_color * 4.5).clamp(0.02, 1.0);
            for (cur, tgt) in self.halo_colors.iter_mut().zip(target.iter()) {
                cur.0 += (tgt.0 - cur.0) * k;
                cur.1 += (tgt.1 - cur.1) * k;
                cur.2 += (tgt.2 - cur.2) * k;
            }
        }
    }

    pub fn sync(&mut self, taskbars: &[HWND]) {
        let plan = backdrop_layer_plan(self.style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.backdrops.clear();
            return;
        }
        let recipe = if plan.use_gpu_blur {
            gpu_backdrop_recipe(self.style, self.light_glass)
        } else {
            None
        };

        self.advance_liquid_glass_clock();

        let valid_keys: Vec<isize> = taskbars
            .iter()
            .filter(|h| !h.is_invalid())
            .map(|h| h.0 as isize)
            .collect();
        self.backdrops.retain(|k, _| valid_keys.contains(k));

        let mut comp_needs_recreate = plan.use_gpu_blur && self.comp.is_none();

        for &tb_hwnd in taskbars {
            if tb_hwnd.is_invalid() {
                continue;
            }
            let key = tb_hwnd.0 as isize;
            if !self.backdrops.contains_key(&key) {
                if let Ok(entry) = create_backdrop(&self.d2d_factory, self.comp.as_ref()) {
                    self.backdrops.insert(key, entry);
                } else {
                    continue;
                }
            }
            let Some(entry) = self.backdrops.get_mut(&key) else {
                continue;
            };

            let mut tb_rect = RECT::default();
            let got_rect = unsafe { GetWindowRect(tb_hwnd, &mut tb_rect).is_ok() };
            let should_hide =
                !got_rect || should_hide_backdrop_for_taskbar(tb_hwnd, tb_rect);

            if should_hide {
                unsafe {
                    if entry.glass_visible {
                        let _ = ShowWindow(entry.glass_hwnd, SW_HIDE);
                        entry.glass_visible = false;
                    }
                    if entry.blur_visible {
                        let _ = ShowWindow(entry.blur_hwnd, SW_HIDE);
                        entry.blur_visible = false;
                    }
                }
                continue;
            }

            let w = (tb_rect.right - tb_rect.left).max(1);
            let h = (tb_rect.bottom - tb_rect.top).max(1);
            let dpi = unsafe { GetDpiForWindow(tb_hwnd).max(96) };
            let scale = (dpi as f32) / 96.0;
            let size_changed = entry.last_rect.left != tb_rect.left
                || entry.last_rect.top != tb_rect.top
                || entry.last_rect.right != tb_rect.right
                || entry.last_rect.bottom != tb_rect.bottom;

            let prev_live_blur = entry.has_live_blur;
            let mut live_blur_ready = false;
            if recipe.is_some() && self.comp.is_some() && entry.comp_visual.is_none() {
                comp_needs_recreate = true;
            }
            if let (Some(rec), Some(engine), Some(visual)) =
                (recipe, self.comp.as_ref(), entry.comp_visual.as_ref())
            {
                let blur_key = (self.style, self.light_glass, dpi);
                if entry.blur_policy_dirty || entry.last_blur_key != Some(blur_key) {
                    if let Some(brush) =
                        engine.build_backdrop_brush(rec.blur_sigma_dip * scale, rec.saturation)
                    {
                        if visual.SetBrush(&brush).is_ok() {
                            entry.last_blur_key = Some(blur_key);
                            entry.blur_policy_dirty = false;
                            live_blur_ready = true;
                        } else {
                            entry.last_blur_key = None;
                            comp_needs_recreate = true;
                        }
                    } else {
                        entry.last_blur_key = None;
                        comp_needs_recreate = true;
                    }
                } else {
                    live_blur_ready = true;
                }
            }
            entry.has_live_blur = live_blur_ready;
            if entry.has_live_blur != prev_live_blur {
                entry.glass_dirty = true;
            }

            let reveal_glass = plan.use_layered_surface && !entry.glass_visible;
            if plan.use_layered_surface
                && (self.style == TaskbarStyle::LiquidGlass
                    || entry.glass_dirty
                    || size_changed
                    || reveal_glass)
            {
                let mut res = render_layered_backdrop(
                    entry,
                    self.style,
                    tb_rect,
                    w,
                    h,
                    dpi,
                    self.flow_phase,
                    &self.halo_colors,
                    self.light_glass,
                );
                if let Err(ref e) = res {
                    if e.code() == D2DERR_RECREATE_TARGET {
                        comp_needs_recreate = true;
                        entry.silk_bmp = None;
                        entry.noise_brush = None;
                        entry.noise_key = None;
                        if let Ok(new_rt) = create_dc_render_target(&self.d2d_factory) {
                            entry.rt = new_rt;
                            res = render_layered_backdrop(
                                entry,
                                self.style,
                                tb_rect,
                                w,
                                h,
                                dpi,
                                self.flow_phase,
                                &self.halo_colors,
                                self.light_glass,
                            );
                        }
                    }
                }
                if res.is_ok() {
                    entry.glass_dirty = false;
                }
            }

            if plan.use_layered_surface {
                let glass_anchor = taskbar_backdrop_anchor(tb_hwnd, entry.glass_hwnd);
                unsafe {
                    sync_window_placement(
                        entry.glass_hwnd,
                        glass_anchor,
                        tb_rect,
                        w,
                        h,
                        reveal_glass,
                        size_changed,
                    );
                }
                entry.glass_visible = true;
            } else if entry.glass_visible {
                unsafe {
                    let _ = ShowWindow(entry.glass_hwnd, SW_HIDE);
                }
                entry.glass_visible = false;
            }

            if entry.has_live_blur {
                let reveal_blur = !entry.blur_visible;
                if reveal_blur || size_changed || entry.blur_size != (w, h) {
                    if let Some(ref visual) = entry.comp_visual {
                        if visual
                            .SetSize(Vector2 {
                                X: w as f32,
                                Y: h as f32,
                            })
                            .is_err()
                        {
                            comp_needs_recreate = true;
                        } else {
                            entry.blur_size = (w, h);
                        }
                    }
                }
                unsafe {
                    sync_window_placement(
                        entry.blur_hwnd,
                        entry.glass_hwnd,
                        tb_rect,
                        w,
                        h,
                        reveal_blur,
                        size_changed,
                    );
                }
                entry.blur_visible = true;
            } else if entry.blur_visible {
                unsafe {
                    let _ = ShowWindow(entry.blur_hwnd, SW_HIDE);
                }
                entry.blur_visible = false;
            }

            entry.last_rect = tb_rect;
        }

        if comp_needs_recreate {
            self.recreate_composition_engine();
        }
    }

    /// Synchronizes the rounded behind-overflow-flyout backdrop with `TopLevelWindowForOverflowXamlIsland`.
    /// Returns `true` on the transition from hidden to visible so callers can reassert TAP transparency.
    pub fn sync_overflow(&mut self) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.hide_overflow_backdrop();
            return false;
        }
        let Some((ov_hwnd, ov_rect, dpi)) = find_visible_overflow_window() else {
            self.hide_overflow_backdrop();
            return false;
        };

        self.advance_liquid_glass_clock();
        let mut comp_needs_recreate = false;
        let ok = sync_shell_card_backdrop(
            &self.d2d_factory,
            self.comp.as_ref(),
            &mut self.overflow,
            w!("Pretty Windows Overflow Blur"),
            w!("Pretty Windows Overflow Glass"),
            self.style,
            self.light_glass,
            self.transparency_enabled,
            self.flow_phase,
            &self.halo_colors,
            ov_hwnd,
            ov_rect,
            dpi,
            0.0,
            &mut comp_needs_recreate,
        );
        if comp_needs_recreate {
            self.recreate_composition_engine();
        }
        ok
    }

    /// Synchronizes the rounded behind-Alt+Tab-switcher card backdrop with `Border#BackgroundElement`.
    pub fn sync_alt_tab(&mut self, host_hwnd: HWND, card_rect: RECT, dpi: u32) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.hide_alt_tab_backdrop();
            return false;
        }

        self.advance_liquid_glass_clock();
        let mut comp_needs_recreate = false;
        let ok = sync_shell_card_backdrop(
            &self.d2d_factory,
            self.comp.as_ref(),
            &mut self.alt_tab,
            w!("Pretty Windows AltTab Blur"),
            w!("Pretty Windows AltTab Glass"),
            self.style,
            self.light_glass,
            self.transparency_enabled,
            self.flow_phase,
            &self.halo_colors,
            host_hwnd,
            card_rect,
            dpi,
            0.0,
            &mut comp_needs_recreate,
        );
        if comp_needs_recreate {
            self.recreate_composition_engine();
        }
        ok
    }

    /// Synchronizes the rounded behind-taskbar-hover-flyout card backdrop with `Border#HoverFlyoutBackground`.
    pub fn sync_hover_flyout(&mut self, host_hwnd: HWND, card_rect: RECT, dpi: u32) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.hide_hover_flyout_backdrop();
            return false;
        }

        self.advance_liquid_glass_clock();
        let mut comp_needs_recreate = false;
        let ok = sync_shell_card_backdrop(
            &self.d2d_factory,
            self.comp.as_ref(),
            &mut self.hover_flyout,
            w!("Pretty Windows HoverFlyout Blur"),
            w!("Pretty Windows HoverFlyout Glass"),
            self.style,
            self.light_glass,
            self.transparency_enabled,
            self.flow_phase,
            &self.halo_colors,
            host_hwnd,
            card_rect,
            dpi,
            0.0,
            &mut comp_needs_recreate,
        );
        if comp_needs_recreate {
            self.recreate_composition_engine();
        }
        ok
    }

    /// Synchronizes the rounded behind-OSD card backdrop with `HWConfirmatorUI`'s `Grid#ConfirmatorMainGrid`,
    /// reserving a 10 DIP outer transparent margin on the layered glass surface for 3-pass ambient shadows.
    pub fn sync_osd(&mut self, host_hwnd: HWND, card_rect: RECT, dpi: u32) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.hide_osd_backdrop();
            return false;
        }

        self.advance_liquid_glass_clock();
        let mut comp_needs_recreate = false;
        let ok = sync_shell_card_backdrop(
            &self.d2d_factory,
            self.comp.as_ref(),
            &mut self.osd,
            w!("Pretty Windows OSD Blur"),
            w!("Pretty Windows OSD Glass"),
            self.style,
            self.light_glass,
            self.transparency_enabled,
            self.flow_phase,
            &self.halo_colors,
            host_hwnd,
            card_rect,
            dpi,
            OSD_SHADOW_PAD_DIP,
            &mut comp_needs_recreate,
        );
        if comp_needs_recreate {
            self.recreate_composition_engine();
        }
        ok
    }

    /// Synchronizes the rounded behind-ToolTip card backdrop with `ToolTip > ContentPresenter#LayoutRoot`
    /// inside `Xaml_WindowedPopupClass`, reserving a 6 DIP outer transparent margin for 3-pass ambient shadows.
    pub fn sync_tooltip(&mut self, host_hwnd: HWND, card_rect: RECT, dpi: u32) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.hide_tooltip_backdrop();
            return false;
        }

        self.advance_liquid_glass_clock();
        let mut comp_needs_recreate = false;
        let ok = sync_shell_card_backdrop(
            &self.d2d_factory,
            self.comp.as_ref(),
            &mut self.tooltip,
            w!("Pretty Windows ToolTip Blur"),
            w!("Pretty Windows ToolTip Glass"),
            self.style,
            self.light_glass,
            self.transparency_enabled,
            self.flow_phase,
            &self.halo_colors,
            host_hwnd,
            card_rect,
            dpi,
            TOOLTIP_SHADOW_PAD_DIP,
            &mut comp_needs_recreate,
        );
        if comp_needs_recreate {
            self.recreate_composition_engine();
        }
        ok
    }

    /// Synchronizes the rounded behind-Input-Switcher backdrop (`键盘布局 ⊞ + 空格键`)
    /// with the outer measured card container, reserving `MENU_FLYOUT_SHADOW_PAD_DIP` for 3-pass shadows.
    pub fn sync_input_switcher(&mut self, host_hwnd: HWND, card_rect: RECT, dpi: u32) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.hide_input_switcher_backdrop();
            return false;
        }

        self.advance_liquid_glass_clock();
        let mut comp_needs_recreate = false;
        let ok = sync_shell_card_backdrop(
            &self.d2d_factory,
            self.comp.as_ref(),
            &mut self.input_switcher,
            w!("Pretty Windows InputSwitcher Blur"),
            w!("Pretty Windows InputSwitcher Glass"),
            self.style,
            self.light_glass,
            self.transparency_enabled,
            self.flow_phase,
            &self.halo_colors,
            host_hwnd,
            card_rect,
            dpi,
            MENU_FLYOUT_SHADOW_PAD_DIP,
            &mut comp_needs_recreate,
        );
        if comp_needs_recreate {
            self.recreate_composition_engine();
        }
        ok
    }

    /// Synchronizes up to `MAX_SHELL_CARD_SLOTS` concurrent rounded backdrops behind active
    /// Shell flyout cards (`ShellExperienceHost.exe` and `ShellHost.exe`: Quick Settings, Media
    /// Card, Notification Center, Calendar, Floating Toasts, JumpList, and submenus), preserving
    /// stable `(host_hwnd, ordinal)` slot affinity so multi-card hosts never flicker.
    pub fn sync_shell_cards(&mut self, active: &[(HWND, RECT, u32)]) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if (!plan.use_gpu_blur && !plan.use_layered_surface) || active.is_empty() {
            self.hide_shell_cards();
            return false;
        }

        let count = active.len().min(MAX_SHELL_CARD_SLOTS);
        let active_slice = &active[..count];
        let mut keys = [(0isize, 0usize); MAX_SHELL_CARD_SLOTS];
        for (idx, &(h, _, _)) in active_slice.iter().enumerate() {
            let raw = h.0 as isize;
            let ordinal = active_slice[..idx]
                .iter()
                .filter(|&&(ph, _, _)| (ph.0 as isize) == raw)
                .count();
            keys[idx] = (raw, ordinal);
        }

        for slot_idx in 0..MAX_SHELL_CARD_SLOTS {
            let bound = self.shell_card_keys[slot_idx];
            if bound.0 != 0 && !keys[..count].contains(&bound) {
                hide_card_backdrop(&mut self.shell_cards[slot_idx]);
                self.shell_card_keys[slot_idx] = (0, 0);
            }
        }

        self.advance_liquid_glass_clock();
        let mut comp_needs_recreate = false;
        let mut any_ok = false;
        for (i, &(host_hwnd, card_rect, dpi)) in active_slice.iter().enumerate() {
            if host_hwnd.is_invalid() {
                continue;
            }
            let key = keys[i];
            let slot_idx = self
                .shell_card_keys
                .iter()
                .position(|&k| k == key)
                .or_else(|| self.shell_card_keys.iter().position(|&k| k.0 == 0));
            let Some(idx) = slot_idx else {
                break;
            };
            self.shell_card_keys[idx] = key;
            if sync_shell_card_backdrop(
                &self.d2d_factory,
                self.comp.as_ref(),
                &mut self.shell_cards[idx],
                w!("Pretty Windows Shell Blur"),
                w!("Pretty Windows Shell Glass"),
                self.style,
                self.light_glass,
                self.transparency_enabled,
                self.flow_phase,
                &self.halo_colors,
                host_hwnd,
                card_rect,
                dpi,
                MENU_FLYOUT_SHADOW_PAD_DIP,
                &mut comp_needs_recreate,
            ) {
                any_ok = true;
            }
        }
        if comp_needs_recreate {
            self.recreate_composition_engine();
        }
        any_ok
    }

    /// Synchronizes up to `MAX_MENU_FLYOUT_SLOTS` concurrent rounded backdrops behind active
    /// `MenuFlyout` / JumpList popups (main + cascading submenus) while preserving slot affinity
    /// per `host_hwnd` to eliminate flicker.
    pub fn sync_menu_flyouts(&mut self, active: &[(HWND, RECT, u32)]) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if (!plan.use_gpu_blur && !plan.use_layered_surface) || active.is_empty() {
            self.hide_menu_flyout_backdrops();
            return false;
        }

        let count = active.len().min(MAX_MENU_FLYOUT_SLOTS);
        let active_slice = &active[..count];

        for slot_idx in 0..MAX_MENU_FLYOUT_SLOTS {
            let bound = self.menu_flyout_host_hwnds[slot_idx];
            if bound != 0
                && !active_slice
                    .iter()
                    .any(|&(h, _, _)| (h.0 as isize) == bound)
            {
                hide_card_backdrop(&mut self.menu_flyouts[slot_idx]);
                self.menu_flyout_host_hwnds[slot_idx] = 0;
            }
        }

        self.advance_liquid_glass_clock();
        let mut comp_needs_recreate = false;
        let mut any_ok = false;
        for &(host_hwnd, card_rect, dpi) in active_slice {
            if host_hwnd.is_invalid() {
                continue;
            }
            let hwnd_raw = host_hwnd.0 as isize;
            let slot_idx = self
                .menu_flyout_host_hwnds
                .iter()
                .position(|&h| h == hwnd_raw)
                .or_else(|| self.menu_flyout_host_hwnds.iter().position(|&h| h == 0));
            let Some(idx) = slot_idx else {
                break;
            };
            self.menu_flyout_host_hwnds[idx] = hwnd_raw;
            if sync_shell_card_backdrop(
                &self.d2d_factory,
                self.comp.as_ref(),
                &mut self.menu_flyouts[idx],
                w!("Pretty Windows Menu Blur"),
                w!("Pretty Windows Menu Glass"),
                self.style,
                self.light_glass,
                self.transparency_enabled,
                self.flow_phase,
                &self.halo_colors,
                host_hwnd,
                card_rect,
                dpi,
                MENU_FLYOUT_SHADOW_PAD_DIP,
                &mut comp_needs_recreate,
            ) {
                any_ok = true;
            }
        }
        if comp_needs_recreate {
            self.recreate_composition_engine();
        }
        any_ok
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_shell_card_backdrop(
    d2d_factory: &ID2D1Factory1,
    comp: Option<&CompositionEngine>,
    slot: &mut Option<ShellCardGlassBackdrop>,
    blur_title: PCWSTR,
    glass_title: PCWSTR,
    style: TaskbarStyle,
    light_glass: bool,
    transparency_enabled: bool,
    flow_phase: f64,
    halo_colors: &[(f32, f32, f32); 2],
    host_hwnd: HWND,
    card_rect: RECT,
    dpi: u32,
    shadow_pad_dip: f32,
    comp_needs_recreate: &mut bool,
) -> bool {
    let plan = overflow_backdrop_layer_plan(style, transparency_enabled);
    if !plan.use_gpu_blur && !plan.use_layered_surface {
        hide_card_backdrop(slot);
        return false;
    }
    if plan.use_gpu_blur && comp.is_none() {
        *comp_needs_recreate = true;
    }
    if slot.is_none() {
        let Ok(card) = create_shell_card_backdrop(d2d_factory, comp, blur_title, glass_title)
        else {
            return false;
        };
        *slot = Some(card);
    }
    let Some(ov) = slot.as_mut() else {
        return false;
    };

    let w = (card_rect.right - card_rect.left).max(1);
    let h = (card_rect.bottom - card_rect.top).max(1);
    let ([gl, gt, gr, gb], pad_px) = expand_card_rect_for_shadow(
        [
            card_rect.left,
            card_rect.top,
            card_rect.right,
            card_rect.bottom,
        ],
        dpi,
        shadow_pad_dip,
    );
    let glass_rect = RECT {
        left: gl,
        top: gt,
        right: gr,
        bottom: gb,
    };
    let glass_w = (glass_rect.right - glass_rect.left).max(1);
    let glass_h = (glass_rect.bottom - glass_rect.top).max(1);
    let scale = (dpi.max(96) as f32) / 96.0;
    let size_changed = ov.last_rect.left != card_rect.left
        || ov.last_rect.top != card_rect.top
        || ov.last_rect.right != card_rect.right
        || ov.last_rect.bottom != card_rect.bottom
        || ov.last_shadow_pad != pad_px;

    let recipe = if plan.use_gpu_blur {
        gpu_backdrop_recipe(style, light_glass)
    } else {
        None
    };
    let prev_live_blur = ov.has_live_blur;
    let mut live_blur_ready = false;
    if recipe.is_some() && comp.is_some() && ov.comp_visual.is_none() {
        *comp_needs_recreate = true;
    }
    if let (Some(rec), Some(engine), Some(visual)) = (recipe, comp, ov.comp_visual.as_ref()) {
        let blur_key = (style, light_glass, dpi);
        if ov.blur_policy_dirty || ov.last_blur_key != Some(blur_key) {
            if let Some(brush) =
                engine.build_backdrop_brush(rec.blur_sigma_dip * scale, rec.saturation)
            {
                if visual.SetBrush(&brush).is_ok() {
                    ov.last_blur_key = Some(blur_key);
                    ov.blur_policy_dirty = false;
                    live_blur_ready = true;
                } else {
                    ov.last_blur_key = None;
                    *comp_needs_recreate = true;
                }
            } else {
                ov.last_blur_key = None;
                *comp_needs_recreate = true;
            }
        } else {
            live_blur_ready = true;
        }
    }
    ov.has_live_blur = live_blur_ready;
    if ov.has_live_blur != prev_live_blur {
        ov.glass_dirty = true;
    }

    let reveal_glass = plan.use_layered_surface && !ov.glass_visible;
    if plan.use_layered_surface
        && (style == TaskbarStyle::LiquidGlass || ov.glass_dirty || size_changed || reveal_glass)
    {
        let mut res = render_shell_card_layered_backdrop(
            d2d_factory,
            ov,
            style,
            glass_rect,
            card_rect,
            pad_px,
            dpi,
            flow_phase,
            halo_colors,
            light_glass,
        );
        if let Err(ref e) = res {
            if e.code() == D2DERR_RECREATE_TARGET {
                *comp_needs_recreate = true;
                ov.silk_bmp = None;
                ov.noise_cache.clear();
                if let Ok(new_rt) = create_dc_render_target(d2d_factory) {
                    if let Ok(new_layer) = unsafe { new_rt.CreateLayer(None) } {
                        ov.layer = new_layer;
                        ov.rt = new_rt;
                        res = render_shell_card_layered_backdrop(
                            d2d_factory,
                            ov,
                            style,
                            glass_rect,
                            card_rect,
                            pad_px,
                            dpi,
                            flow_phase,
                            halo_colors,
                            light_glass,
                        );
                    }
                }
            }
        }
        if res.is_ok() {
            ov.glass_dirty = false;
        }
    }

    if plan.use_layered_surface {
        let anchor = overflow_backdrop_anchor(host_hwnd, ov.glass_hwnd);
        unsafe {
            sync_window_placement(
                ov.glass_hwnd,
                anchor,
                glass_rect,
                glass_w,
                glass_h,
                reveal_glass,
                size_changed,
            );
        }
        ov.glass_visible = true;
    } else if ov.glass_visible {
        unsafe {
            let _ = ShowWindow(ov.glass_hwnd, SW_HIDE);
        }
        ov.glass_visible = false;
    }

    let mut reveal_blur = false;
    if ov.has_live_blur {
        reveal_blur = !ov.blur_visible;
        if reveal_blur || size_changed || ov.blur_size != (w, h) {
            if let Some(ref visual) = ov.comp_visual {
                if visual
                    .SetSize(Vector2 {
                        X: w as f32,
                        Y: h as f32,
                    })
                    .is_err()
                {
                    *comp_needs_recreate = true;
                } else {
                    ov.blur_size = (w, h);
                }
            }
        }
        unsafe {
            sync_window_placement(
                ov.blur_hwnd,
                ov.glass_hwnd,
                card_rect,
                w,
                h,
                reveal_blur,
                size_changed,
            );
        }
        ov.blur_visible = true;
    } else if ov.blur_visible {
        unsafe {
            let _ = ShowWindow(ov.blur_hwnd, SW_HIDE);
        }
        ov.blur_visible = false;
    }

    ov.last_rect = card_rect;
    ov.last_shadow_pad = pad_px;

    reveal_glass || reveal_blur
}

unsafe fn sync_window_placement(
    hwnd: HWND,
    insert_after: HWND,
    tb_rect: RECT,
    w: i32,
    h: i32,
    reveal: bool,
    size_changed: bool,
) {
    let mut flags = SWP_NOACTIVATE;
    if !size_changed && !reveal {
        flags |= SWP_NOMOVE | SWP_NOSIZE;
    }
    if reveal {
        flags |= SWP_SHOWWINDOW;
    }
    let effective_after = if insert_after == HWND_TOPMOST
        || insert_after.is_invalid()
        || window_band(insert_after) != window_band(hwnd)
    {
        HWND_TOPMOST
    } else {
        insert_after
    };
    let need_z = if effective_after == HWND_TOPMOST {
        (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) == 0
            || is_occluded_by_same_band_topmost(hwnd)
    } else {
        GetWindow(effective_after, GW_HWNDNEXT) != Ok(hwnd)
    };
    if reveal || size_changed || need_z {
        let _ = SetWindowPos(
            hwnd,
            effective_after,
            tb_rect.left,
            tb_rect.top,
            w,
            h,
            flags,
        );
    }
}

fn ensure_window_classes() -> windows::core::Result<()> {
    static INIT: OnceLock<bool> = OnceLock::new();
    let ok = *INIT.get_or_init(|| unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else {
            return false;
        };
        let blur_wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(blur_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: GLASS_BLUR_CLASS,
            ..Default::default()
        };
        let glass_wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(glass_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: GLASS_OVERLAY_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&blur_wc);
        RegisterClassExW(&glass_wc);
        true
    });
    if ok {
        Ok(())
    } else {
        Err(windows::core::Error::from_win32())
    }
}

unsafe extern "system" fn blur_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn glass_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCHITTEST {
        LRESULT(HTTRANSPARENT as isize)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

struct BackdropPairWindows {
    blur_hwnd: HWND,
    comp_target: Option<DesktopWindowTarget>,
    comp_visual: Option<SpriteVisual>,
    glass_hwnd: HWND,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
}

struct InitBackdropGuard {
    blur_hwnd: HWND,
    glass_hwnd: HWND,
    comp_target: Option<DesktopWindowTarget>,
    comp_visual: Option<SpriteVisual>,
}

impl Drop for InitBackdropGuard {
    fn drop(&mut self) {
        if let Some(ref visual) = self.comp_visual {
            let _ = visual.SetBrush(None::<&CompositionBrush>);
        }
        if let Some(target) = self.comp_target.take() {
            let _ = target.SetRoot(None::<&Visual>);
            let _ = target.Close();
        }
        self.comp_visual = None;
        unsafe {
            if !self.glass_hwnd.is_invalid() {
                let _ = DestroyWindow(self.glass_hwnd);
            }
            if !self.blur_hwnd.is_invalid() {
                let _ = DestroyWindow(self.blur_hwnd);
            }
        }
    }
}

fn create_backdrop_pair(
    d2d_factory: &ID2D1Factory1,
    comp: Option<&CompositionEngine>,
    blur_title: PCWSTR,
    glass_title: PCWSTR,
    blur_corner_pref: DWM_WINDOW_CORNER_PREFERENCE,
) -> windows::core::Result<BackdropPairWindows> {
    ensure_window_classes()?;
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let blur_hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP
                | WS_EX_TRANSPARENT
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE
                | WS_EX_TOPMOST,
            GLASS_BLUR_CLASS,
            blur_title,
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            hinst,
            None,
        )?;
        let mut init_guard = InitBackdropGuard {
            blur_hwnd,
            glass_hwnd: HWND::default(),
            comp_target: None,
            comp_visual: None,
        };

        let _ = DwmSetWindowAttribute(
            blur_hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&blur_corner_pref as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        if blur_corner_pref == DWMWCP_ROUND {
            let no_border: u32 = DWMWA_COLOR_NONE;
            let _ = DwmSetWindowAttribute(
                blur_hwnd,
                DWMWA_BORDER_COLOR,
                (&no_border as *const u32).cast(),
                size_of::<u32>() as u32,
            );
        }
        let enable_host_backdrop = BOOL(1);
        let _ = DwmSetWindowAttribute(
            blur_hwnd,
            DWMWA_USE_HOSTBACKDROPBRUSH,
            (&enable_host_backdrop as *const BOOL).cast(),
            size_of::<BOOL>() as u32,
        );
        exclude_from_peek(blur_hwnd);

        if let Some((target, visual)) = comp.and_then(|c| c.create_target_and_visual(blur_hwnd)) {
            init_guard.comp_target = Some(target);
            init_guard.comp_visual = Some(visual);
        }

        let glass_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            GLASS_OVERLAY_CLASS,
            glass_title,
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            hinst,
            None,
        )?;
        init_guard.glass_hwnd = glass_hwnd;

        let no_round = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            glass_hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&no_round as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        exclude_from_peek(glass_hwnd);

        let rt = create_dc_render_target(d2d_factory)?;
        let surface = LayeredSurface::new()?;

        let out = BackdropPairWindows {
            blur_hwnd: init_guard.blur_hwnd,
            comp_target: init_guard.comp_target.take(),
            comp_visual: init_guard.comp_visual.take(),
            glass_hwnd: init_guard.glass_hwnd,
            rt,
            surface,
        };
        init_guard.blur_hwnd = HWND::default();
        init_guard.glass_hwnd = HWND::default();
        Ok(out)
    }
}

fn create_backdrop(
    d2d_factory: &ID2D1Factory1,
    comp: Option<&CompositionEngine>,
) -> windows::core::Result<MonitorGlassBackdrop> {
    let pair = create_backdrop_pair(
        d2d_factory,
        comp,
        w!("Pretty Windows Taskbar Blur"),
        w!("Pretty Windows Taskbar Glass"),
        DWMWCP_DONOTROUND,
    )?;
    register_taskbar_backdrop(pair.glass_hwnd);
    register_taskbar_backdrop(pair.blur_hwnd);
    Ok(MonitorGlassBackdrop {
        blur_hwnd: pair.blur_hwnd,
        comp_target: pair.comp_target,
        comp_visual: pair.comp_visual,
        last_blur_key: None,
        blur_size: (0, 0),
        has_live_blur: false,
        glass_hwnd: pair.glass_hwnd,
        noise_brush: None,
        noise_key: None,
        silk_bmp: None,
        silk_buf: vec![0u8; (BAR_SILK_W * BAR_SILK_H * 4) as usize],
        last_silk_tick: None,
        rt: pair.rt,
        surface: pair.surface,
        last_rect: RECT::default(),
        blur_visible: false,
        glass_visible: false,
        blur_policy_dirty: true,
        glass_dirty: true,
    })
}

fn create_shell_card_backdrop(
    d2d_factory: &ID2D1Factory1,
    comp: Option<&CompositionEngine>,
    blur_title: PCWSTR,
    glass_title: PCWSTR,
) -> windows::core::Result<ShellCardGlassBackdrop> {
    let mut pair = create_backdrop_pair(
        d2d_factory,
        comp,
        blur_title,
        glass_title,
        DWMWCP_ROUND,
    )?;
    let layer = match unsafe { pair.rt.CreateLayer(None) } {
        Ok(l) => l,
        Err(e) => {
            if let Some(ref visual) = pair.comp_visual {
                let _ = visual.SetBrush(None::<&CompositionBrush>);
            }
            if let Some(target) = pair.comp_target.take() {
                let _ = target.SetRoot(None::<&Visual>);
                let _ = target.Close();
            }
            pair.comp_visual = None;
            unsafe {
                if !pair.glass_hwnd.is_invalid() {
                    let _ = DestroyWindow(pair.glass_hwnd);
                }
                if !pair.blur_hwnd.is_invalid() {
                    let _ = DestroyWindow(pair.blur_hwnd);
                }
            }
            return Err(e);
        }
    };
    Ok(ShellCardGlassBackdrop {
        blur_hwnd: pair.blur_hwnd,
        comp_target: pair.comp_target,
        comp_visual: pair.comp_visual,
        last_blur_key: None,
        blur_size: (0, 0),
        has_live_blur: false,
        glass_hwnd: pair.glass_hwnd,
        noise_cache: PopupNoiseCache::default(),
        silk_bmp: None,
        silk_buf: vec![0u8; (SILK_W * SILK_H * 4) as usize],
        last_silk_tick: None,
        shadow_geo_key: None,
        cached_clip_geo: None,
        cached_shadow_rings: [None, None, None],
        layer,
        rt: pair.rt,
        surface: pair.surface,
        last_rect: RECT::default(),
        last_shadow_pad: 0,
        blur_visible: false,
        glass_visible: false,
        blur_policy_dirty: true,
        glass_dirty: true,
    })
}
