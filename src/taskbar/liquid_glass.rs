use super::config::TaskbarStyle;
use super::swca::{
    backdrop_layer_plan, expand_card_rect_for_shadow, generate_acrylic_noise_bgra,
    gpu_backdrop_recipe, inset_overflow_card_rect, overflow_backdrop_layer_plan,
    OSD_SHADOW_PAD_DIP, TOOLTIP_SHADOW_PAD_DIP,
};
use crate::color::NEUTRAL_BACKDROP_FALLBACK;
use crate::system::{
    exclude_from_peek, is_occluded_by_same_band_topmost, overflow_backdrop_anchor,
    register_taskbar_backdrop, resolve_unified_accent, taskbar_backdrop_anchor,
    unregister_taskbar_backdrop,
};
use crate::ui::glass::{
    d2d_rect, draw_rounded_card_rim, identity_matrix, update_silk_bitmap, LayeredSurface,
    PopupNoiseCache,
};
use crate::ui::{
    compute_halo_pair, popup_glass_recipe, sample_liquid_silk_field, GlassSurfaceStyle, RectF,
    SILK_H, SILK_UPDATE_INTERVAL_MS, SILK_W,
};
use std::collections::HashMap;
use std::mem::{size_of, ManuallyDrop};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::core::{implement, w, Interface, Result, GUID, HSTRING, PCWSTR};
use windows::Foundation::Numerics::Vector2;
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectSource_Impl, IGraphicsEffect_Impl,
};
use windows::System::{DispatcherQueue, DispatcherQueueController};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionBrush, CompositionEffectBrush, CompositionEffectFactoryLoadStatus,
    CompositionEffectSourceParameter, Compositor, SpriteVisual, Visual,
};
use windows::Win32::Foundation::{BOOL, E_INVALIDARG, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BORDER_MODE_HARD, D2D1_COLOR_F, D2D1_GRADIENT_STOP,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation, ID2D1Bitmap, ID2D1BitmapBrush,
    ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1Layer,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_BRUSH_PROPERTIES,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_BITMAP_PROPERTIES, D2D1_COMBINE_MODE_EXCLUDE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_EXTEND_MODE_WRAP, D2D1_GAMMA_2_2, D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
    D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWCP_DONOTROUND, DWMWCP_ROUND,
    DWMWA_BORDER_COLOR, DWMWA_CLOAKED, DWMWA_COLOR_NONE, DWMWA_EXTENDED_FRAME_BOUNDS,
    DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_WINDOW_CORNER_PREFERENCE, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, MonitorFromWindow, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE,
    DQTYPE_THREAD_CURRENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, FindWindowW, GetWindow,
    GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, RegisterClassExW,
    SetWindowPos, ShowWindow, GWL_EXSTYLE, GW_HWNDNEXT, GW_OWNER, HWND_TOPMOST, HTTRANSPARENT,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, WM_ERASEBKGND, WM_NCHITTEST,
    WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const GLASS_BLUR_CLASS: PCWSTR = w!("PrettyWindowsTaskbarBlurWnd");
const GLASS_OVERLAY_CLASS: PCWSTR = w!("PrettyWindowsTaskbarGlassWnd");

/// Wide-aspect Liquid Silk bitmap dimensions matching full taskbar proportions (`192 × 16 = 3072` px)
/// to avoid horizontal aliasing stripes without increasing per-frame CPU work.
const BAR_SILK_W: u32 = 192;
const BAR_SILK_H: u32 = 16;

// ---------- WinRT Composition Direct2D Effect Interop ----------

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct GaussianBlurEffect {
    name: HSTRING,
    sigma: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for GaussianBlurEffect_Impl {}

impl IGraphicsEffect_Impl for GaussianBlurEffect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.clone())
    }

    fn SetName(&self, _name: &HSTRING) -> Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for GaussianBlurEffect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(CLSID_D2D1GaussianBlur)
    }

    fn GetNamedPropertyMapping(
        &self,
        _name: &PCWSTR,
        _index: *mut u32,
        _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        Err(E_INVALIDARG.into())
    }

    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(3)
    }

    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        match index {
            0 => PropertyValue::CreateSingle(self.sigma)?.cast(),
            1 => PropertyValue::CreateUInt32(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32)?
                .cast(),
            2 => PropertyValue::CreateUInt32(D2D1_BORDER_MODE_HARD.0 as u32)?.cast(),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(E_INVALIDARG.into())
        }
    }

    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct SaturationEffect {
    name: HSTRING,
    saturation: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for SaturationEffect_Impl {}

impl IGraphicsEffect_Impl for SaturationEffect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.clone())
    }

    fn SetName(&self, _name: &HSTRING) -> Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for SaturationEffect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(CLSID_D2D1Saturation)
    }

    fn GetNamedPropertyMapping(
        &self,
        _name: &PCWSTR,
        _index: *mut u32,
        _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        Err(E_INVALIDARG.into())
    }

    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(1)
    }

    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        if index == 0 {
            PropertyValue::CreateSingle(self.saturation)?.cast()
        } else {
            Err(E_INVALIDARG.into())
        }
    }

    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(E_INVALIDARG.into())
        }
    }

    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}

struct CompositionEngine {
    _dq_controller: Option<DispatcherQueueController>,
    compositor: Compositor,
    desktop_interop: ICompositorDesktopInterop,
}

impl CompositionEngine {
    fn try_new() -> Option<Self> {
        let dq_controller = if DispatcherQueue::GetForCurrentThread().is_ok() {
            None
        } else {
            let opts = DispatcherQueueOptions {
                dwSize: size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            };
            Some(unsafe { CreateDispatcherQueueController(opts).ok()? })
        };

        let compositor = Compositor::new().ok()?;
        let desktop_interop: ICompositorDesktopInterop = compositor.cast().ok()?;
        Some(Self {
            _dq_controller: dq_controller,
            compositor,
            desktop_interop,
        })
    }

    fn create_target_and_visual(&self, hwnd: HWND) -> Option<(DesktopWindowTarget, SpriteVisual)> {
        let target = unsafe {
            self.desktop_interop
                .CreateDesktopWindowTarget(hwnd, false)
                .ok()?
        };
        let visual = self.compositor.CreateSpriteVisual().ok()?;
        target.SetRoot(&visual).ok()?;
        Some((target, visual))
    }

    fn build_backdrop_brush(
        &self,
        sigma_px: f32,
        saturation: f32,
    ) -> Option<CompositionEffectBrush> {
        let param_name = HSTRING::from("backdrop");
        let source_param = CompositionEffectSourceParameter::Create(&param_name).ok()?;
        let source_iface: IGraphicsEffectSource = source_param.cast().ok()?;
        let sigma = sigma_px.max(0.5);

        let blur_effect: IGraphicsEffect = GaussianBlurEffect {
            name: HSTRING::from("Blur"),
            sigma,
            source: source_iface.clone(),
        }
        .into();

        let factory = if (saturation - 1.0).abs() >= 0.01 {
            let blur_source: IGraphicsEffectSource = blur_effect.cast().ok()?;
            let sat_effect: IGraphicsEffect = SaturationEffect {
                name: HSTRING::from("Saturation"),
                saturation: saturation.max(0.0),
                source: blur_source,
            }
            .into();
            self.compile_effect_factory(&sat_effect)
                .or_else(|| self.compile_effect_factory(&blur_effect))?
        } else {
            self.compile_effect_factory(&blur_effect)?
        };

        let brush = factory.CreateBrush().ok()?;
        let host_backdrop = self.compositor.CreateHostBackdropBrush().ok()?;
        brush.SetSourceParameter(&param_name, &host_backdrop).ok()?;
        Some(brush)
    }

    fn compile_effect_factory(
        &self,
        effect: &IGraphicsEffect,
    ) -> Option<windows::UI::Composition::CompositionEffectFactory> {
        let factory = self.compositor.CreateEffectFactory(effect).ok()?;
        let status = factory.LoadStatus().ok()?;
        if matches!(
            status,
            CompositionEffectFactoryLoadStatus::EffectTooComplex
                | CompositionEffectFactoryLoadStatus::Other
        ) {
            return None;
        }
        Some(factory)
    }
}

struct MonitorGlassBackdrop {
    blur_hwnd: HWND,
    comp_target: Option<DesktopWindowTarget>,
    comp_visual: Option<SpriteVisual>,
    last_blur_key: Option<(TaskbarStyle, bool, u32)>,
    blur_size: (i32, i32),
    has_live_blur: bool,
    glass_hwnd: HWND,
    surface: LayeredSurface,
    rt: ID2D1DCRenderTarget,
    silk_bmp: Option<ID2D1Bitmap>,
    silk_buf: Vec<u8>,
    last_silk_tick: Option<Instant>,
    noise_brush: Option<ID2D1BitmapBrush>,
    noise_key: Option<(bool, u8)>,
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
    surface: LayeredSurface,
    rt: ID2D1DCRenderTarget,
    layer: ID2D1Layer,
    silk_bmp: Option<ID2D1Bitmap>,
    silk_buf: Vec<u8>,
    last_silk_tick: Option<Instant>,
    noise_cache: PopupNoiseCache,
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
    d2d_factory: ID2D1Factory1,
    comp: Option<CompositionEngine>,
    style: TaskbarStyle,
    light_glass: bool,
    transparency_enabled: bool,
    halo_colors: [(f32, f32, f32); 2],
    halo_ready: bool,
    flow_phase: f64,
    last_flow_tick: Instant,
    last_color_tick: Instant,
    backdrops: HashMap<isize, MonitorGlassBackdrop>,
    overflow: Option<ShellCardGlassBackdrop>,
    alt_tab: Option<ShellCardGlassBackdrop>,
    hover_flyout: Option<ShellCardGlassBackdrop>,
    osd: Option<ShellCardGlassBackdrop>,
    tooltip: Option<ShellCardGlassBackdrop>,
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
            ]
            .into_iter()
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
            ]
            .into_iter()
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
            ]
            .into_iter()
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
                        }
                    } else {
                        entry.last_blur_key = None;
                    }
                } else {
                    live_blur_ready = true;
                }
            }
            entry.has_live_blur = live_blur_ready;
            if entry.has_live_blur != prev_live_blur {
                entry.glass_dirty = true;
            }

            let mut reveal_glass = false;
            if plan.use_layered_surface {
                reveal_glass = !entry.glass_visible;
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
                        let _ = visual.SetSize(Vector2 {
                            X: w as f32,
                            Y: h as f32,
                        });
                        entry.blur_size = (w, h);
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

            if plan.use_layered_surface
                && (self.style == TaskbarStyle::LiquidGlass
                    || entry.glass_dirty
                    || size_changed
                    || reveal_glass)
            {
                let _ = render_layered_backdrop(
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
                entry.glass_dirty = false;
            }
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
        sync_shell_card_backdrop(
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
        )
    }

    /// Synchronizes the rounded behind-Alt+Tab-switcher card backdrop with `Border#BackgroundElement`.
    pub fn sync_alt_tab(&mut self, host_hwnd: HWND, card_rect: RECT, dpi: u32) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.hide_alt_tab_backdrop();
            return false;
        }

        self.advance_liquid_glass_clock();
        sync_shell_card_backdrop(
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
        )
    }

    /// Synchronizes the rounded behind-taskbar-hover-flyout card backdrop with `Border#HoverFlyoutBackground`.
    pub fn sync_hover_flyout(&mut self, host_hwnd: HWND, card_rect: RECT, dpi: u32) -> bool {
        let plan = overflow_backdrop_layer_plan(self.style, self.transparency_enabled);
        if !plan.use_gpu_blur && !plan.use_layered_surface {
            self.hide_hover_flyout_backdrop();
            return false;
        }

        self.advance_liquid_glass_clock();
        sync_shell_card_backdrop(
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
        )
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
        sync_shell_card_backdrop(
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
        )
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
        sync_shell_card_backdrop(
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
        )
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
) -> bool {
    let plan = overflow_backdrop_layer_plan(style, transparency_enabled);
    if !plan.use_gpu_blur && !plan.use_layered_surface {
        hide_card_backdrop(slot);
        return false;
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
                }
            } else {
                ov.last_blur_key = None;
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
                let _ = visual.SetSize(Vector2 {
                    X: w as f32,
                    Y: h as f32,
                });
                ov.blur_size = (w, h);
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

    if plan.use_layered_surface
        && (style == TaskbarStyle::LiquidGlass || ov.glass_dirty || size_changed || reveal_glass)
    {
        let _ = render_shell_card_layered_backdrop(
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
        ov.glass_dirty = false;
    }

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
    let need_z = if insert_after == HWND_TOPMOST {
        (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) == 0
            || is_occluded_by_same_band_topmost(hwnd)
    } else {
        GetWindow(insert_after, GW_HWNDNEXT) != Ok(hwnd)
    };
    if reveal || size_changed || need_z {
        let _ = SetWindowPos(
            hwnd,
            insert_after,
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
            let _ = EndPaint(hwnd, &mut ps);
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
    surface: LayeredSurface,
    rt: ID2D1DCRenderTarget,
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

        let (comp_target, comp_visual) = match comp.and_then(|c| c.create_target_and_visual(blur_hwnd)) {
            Some((target, visual)) => (Some(target), Some(visual)),
            None => (None, None),
        };

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
        let no_round = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            glass_hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&no_round as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        exclude_from_peek(glass_hwnd);

        let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: Default::default(),
        };
        let rt = d2d_factory.CreateDCRenderTarget(&rt_props)?;
        let surface = LayeredSurface::new()?;

        Ok(BackdropPairWindows {
            blur_hwnd,
            comp_target,
            comp_visual,
            glass_hwnd,
            surface,
            rt,
        })
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
        surface: pair.surface,
        rt: pair.rt,
        silk_bmp: None,
        silk_buf: vec![0u8; (BAR_SILK_W * BAR_SILK_H * 4) as usize],
        last_silk_tick: None,
        noise_brush: None,
        noise_key: None,
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
    let pair = create_backdrop_pair(
        d2d_factory,
        comp,
        blur_title,
        glass_title,
        DWMWCP_ROUND,
    )?;
    let layer = unsafe { pair.rt.CreateLayer(None)? };
    Ok(ShellCardGlassBackdrop {
        blur_hwnd: pair.blur_hwnd,
        comp_target: pair.comp_target,
        comp_visual: pair.comp_visual,
        last_blur_key: None,
        blur_size: (0, 0),
        has_live_blur: false,
        glass_hwnd: pair.glass_hwnd,
        surface: pair.surface,
        rt: pair.rt,
        layer,
        silk_bmp: None,
        silk_buf: vec![0u8; (SILK_W * SILK_H * 4) as usize],
        last_silk_tick: None,
        noise_cache: PopupNoiseCache::default(),
        last_rect: RECT::default(),
        last_shadow_pad: 0,
        blur_visible: false,
        glass_visible: false,
        blur_policy_dirty: true,
        glass_dirty: true,
    })
}

fn inspect_visible_explorer_window(
    hwnd: HWND,
    target_pid: u32,
    min_w: i32,
    min_h: i32,
) -> Option<(RECT, u32, u32)> {
    unsafe {
        if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        let mut pid = 0u32;
        let tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != target_pid {
            return None;
        }
        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return None;
        }
        let mut rect = RECT::default();
        let got_ext = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
        .is_ok();
        if !got_ext && GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w < min_w || h < min_h {
            return None;
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        Some((rect, dpi, tid))
    }
}

/// Locates the active uncloaked host window belonging to `explorer_pid` for the `Alt + Tab` switcher,
/// scanning across all ZBID bands via `FindWindowExW`, preferring `preferred_tid`, and isolating
/// taskbar / hover-flyout / OSD threads.
pub fn find_visible_alttab_host_window(
    explorer_pid: u32,
    preferred_tid: Option<u32>,
    excluded_tids: &[u32],
    taskbar_tids: &[u32],
) -> Option<(HWND, RECT, u32, u32)> {
    if explorer_pid == 0 {
        return None;
    }
    let mut found: Option<(HWND, RECT, u32, u32)> = None;
    for cls in [
        w!("XamlExplorerHostIslandWindow"),
        w!("XamlExplorerHostIslandWindow_WASDK"),
    ] {
        let mut prev = HWND::default();
        for _ in 0..256 {
            let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, cls, None) }) else {
                break;
            };
            if hwnd.is_invalid() || hwnd == prev {
                break;
            }
            prev = hwnd;
            let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, explorer_pid, 300, 200)
            else {
                continue;
            };
            if excluded_tids.contains(&tid) {
                continue;
            }
            if preferred_tid != Some(tid) && taskbar_tids.contains(&tid) {
                continue;
            }
            let matched = preferred_tid == Some(tid);
            if matched || found.is_none() {
                found = Some((hwnd, rect, dpi, tid));
            }
            if matched {
                return found;
            }
        }
    }
    found
}

/// Locates the active uncloaked host window belonging to `explorer_pid` for `Taskbar.FlyoutFrame`
/// (hover thumbnail previews & taskbar overflow), enumerating across all ZBID bands via `FindWindowExW`
/// (including `ZBID_IMMERSIVE_INACTIVEMOBILITY`), preferring `XamlExplorerHostIslandWindow*` and known
/// taskbar UI threads while excluding `Alt + Tab` switcher and OSD hosts.
pub fn find_visible_hover_flyout_host_window(
    explorer_pid: u32,
    preferred_tid: Option<u32>,
    excluded_tids: &[u32],
    taskbar_tids: &[u32],
) -> Option<(HWND, RECT, u32, u32)> {
    if explorer_pid == 0 {
        return None;
    }
    let mut best: Option<(u8, i64, HWND, RECT, u32, u32)> = None;
    for (cls, class_bonus, require_taskbar_or_pref) in [
        (w!("XamlExplorerHostIslandWindow"), 4u8, false),
        (w!("XamlExplorerHostIslandWindow_WASDK"), 4u8, false),
        (w!("Xaml_WindowedPopupClass"), 0u8, true),
    ] {
        let mut prev = HWND::default();
        for _ in 0..256 {
            let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, cls, None) }) else {
                break;
            };
            if hwnd.is_invalid() || hwnd == prev {
                break;
            }
            prev = hwnd;
            let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, explorer_pid, 48, 36)
            else {
                continue;
            };
            if excluded_tids.contains(&tid) {
                continue;
            }
            if require_taskbar_or_pref && preferred_tid != Some(tid) && !taskbar_tids.contains(&tid) {
                continue;
            }
            let thread_bonus = if preferred_tid == Some(tid) {
                2u8
            } else if taskbar_tids.contains(&tid) {
                1u8
            } else {
                0u8
            };
            let priority = class_bonus + thread_bonus;
            let w = rect.right - rect.left;
            let h = rect.bottom - rect.top;
            let area = i64::from(w) * i64::from(h);
            let take = match best {
                None => true,
                Some((best_prio, best_area, _, _, _, _)) => {
                    priority > best_prio || (priority == best_prio && area > best_area)
                }
            };
            if take {
                best = Some((priority, area, hwnd, rect, dpi, tid));
            }
        }
    }
    best.map(|(_, _, h, r, d, t)| (h, r, d, t))
}

/// Locates the active uncloaked `HWConfirmatorUI` host window belonging to `explorer_pid`,
/// excluding taskbar, `Alt + Tab`, and hover-flyout threads and bounding unlearned hosts to compact OSD dimensions.
pub fn find_visible_osd_host_window(
    explorer_pid: u32,
    preferred_tid: Option<u32>,
    excluded_tids: &[u32],
    taskbar_tids: &[u32],
) -> Option<(HWND, RECT, u32, u32)> {
    if explorer_pid == 0 {
        return None;
    }
    let mut found: Option<(HWND, RECT, u32, u32)> = None;
    for cls in [
        w!("XamlExplorerHostIslandWindow"),
        w!("XamlExplorerHostIslandWindow_WASDK"),
    ] {
        let mut prev = HWND::default();
        for _ in 0..256 {
            let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, cls, None) }) else {
                break;
            };
            if hwnd.is_invalid() || hwnd == prev {
                break;
            }
            prev = hwnd;
            let Some((rect, dpi, tid)) =
                inspect_visible_explorer_window(hwnd, explorer_pid, 80, 36)
            else {
                continue;
            };
            if excluded_tids.contains(&tid) {
                continue;
            }
            if preferred_tid != Some(tid) && taskbar_tids.contains(&tid) {
                continue;
            }
            let matched = preferred_tid == Some(tid);
            if !matched {
                let scale = (dpi.max(96) as f32) / 96.0;
                let w = rect.right - rect.left;
                let h = rect.bottom - rect.top;
                let max_w = (560.0 * scale).round() as i32;
                let max_h = (220.0 * scale).round() as i32;
                if w > max_w || h > max_h {
                    continue;
                }
            }
            if matched || found.is_none() {
                found = Some((hwnd, rect, dpi, tid));
            }
            if matched {
                return found;
            }
        }
    }
    found
}

/// Locates the active uncloaked `Xaml_WindowedPopupClass` host window belonging to `explorer_pid`
/// for a system/taskbar/overflow `ToolTip`, preferring matching dimensions and known/taskbar UI threads.
pub fn find_visible_tooltip_host_window(
    explorer_pid: u32,
    preferred_tid: Option<u32>,
    taskbar_tids: &[u32],
    expected_size_dip: Option<(f32, f32)>,
) -> Option<(HWND, RECT, u32, u32)> {
    if explorer_pid == 0 {
        return None;
    }
    let mut best: Option<(u8, f32, HWND, RECT, u32, u32)> = None;
    let mut prev = HWND::default();
    for _ in 0..256 {
        let Ok(hwnd) =
            (unsafe { FindWindowExW(None, prev, w!("Xaml_WindowedPopupClass"), None) })
        else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev {
            break;
        }
        prev = hwnd;
        let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, explorer_pid, 20, 16)
        else {
            continue;
        };
        let scale = (dpi.max(96) as f32) / 96.0;
        let w_dip = ((rect.right - rect.left) as f32) / scale;
        let h_dip = ((rect.bottom - rect.top) as f32) / scale;
        if !(20.0..=680.0).contains(&w_dip) || !(16.0..=420.0).contains(&h_dip) {
            continue;
        }

        let (thread_prio, metric) = if let Some((ew, eh)) = expected_size_dip {
            if w_dip < ew - 4.0
                || h_dip < eh - 4.0
                || w_dip > ew + 64.0
                || h_dip > eh + 64.0
            {
                continue;
            }
            let prio = if preferred_tid == Some(tid) {
                2u8
            } else if taskbar_tids.contains(&tid) {
                1u8
            } else {
                0u8
            };
            (prio, (w_dip - ew).abs() + (h_dip - eh).abs())
        } else {
            if preferred_tid != Some(tid) && !taskbar_tids.contains(&tid) {
                continue;
            }
            if w_dip > 560.0 || h_dip > 240.0 {
                continue;
            }
            let prio = if preferred_tid == Some(tid) { 2u8 } else { 1u8 };
            (prio, w_dip * h_dip)
        };

        let take = match best {
            None => true,
            Some((best_prio, best_metric, _, _, _, _)) => {
                thread_prio > best_prio || (thread_prio == best_prio && metric < best_metric)
            }
        };
        if take {
            best = Some((thread_prio, metric, hwnd, rect, dpi, tid));
        }
    }
    best.map(|(_, _, h, r, d, t)| (h, r, d, t))
}

unsafe fn xaml_island_screen_origin(hwnd: HWND) -> Option<[i32; 2]> {
    if hwnd.is_invalid() {
        return None;
    }
    if let Ok(bridge) = FindWindowExW(
        hwnd,
        None,
        w!("Windows.UI.Composition.DesktopWindowContentBridge"),
        None,
    ) {
        if !bridge.is_invalid() {
            let mut rect = RECT::default();
            if GetWindowRect(bridge, &mut rect).is_ok()
                && rect.right > rect.left
                && rect.bottom > rect.top
            {
                return Some([rect.left, rect.top]);
            }
        }
    }
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_ok()
        && rect.right > rect.left
        && rect.bottom > rect.top
    {
        Some([rect.left, rect.top])
    } else {
        None
    }
}

/// Collects candidate screen origins `[x, y]` for the XAML Island hosting `host_hwnd`
/// (owner window, un-inset overflow island, and primary/secondary taskbars) so island-relative
/// `TransformToVisual(null)` coordinates map without symmetric-centering bias.
pub fn collect_tooltip_island_origins(
    host_hwnd: HWND,
    explorer_pid: u32,
    host_tid: u32,
) -> Vec<[i32; 2]> {
    if explorer_pid == 0 {
        return Vec::new();
    }
    let mut origins = Vec::new();
    let push_unique = |list: &mut Vec<[i32; 2]>, pt: [i32; 2]| {
        if !list.contains(&pt) {
            list.push(pt);
        }
    };

    unsafe {
        if !host_hwnd.is_invalid() {
            if let Ok(owner) = GetWindow(host_hwnd, GW_OWNER) {
                if !owner.is_invalid() {
                    let mut pid = 0u32;
                    let _ = GetWindowThreadProcessId(owner, Some(&mut pid));
                    if pid == explorer_pid {
                        if let Some(pt) = xaml_island_screen_origin(owner) {
                            push_unique(&mut origins, pt);
                        }
                    }
                }
            }
        }

        let mut prev_ov = HWND::default();
        for _ in 0..16 {
            let Ok(hwnd) =
                FindWindowExW(None, prev_ov, w!("TopLevelWindowForOverflowXamlIsland"), None)
            else {
                break;
            };
            if hwnd.is_invalid() || hwnd == prev_ov {
                break;
            }
            prev_ov = hwnd;
            if !IsWindowVisible(hwnd).as_bool() {
                continue;
            }
            let mut pid = 0u32;
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != explorer_pid {
                continue;
            }
            let mut cloaked: u32 = 0;
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0
            {
                continue;
            }
            if let Some(pt) = xaml_island_screen_origin(hwnd) {
                push_unique(&mut origins, pt);
            }
        }

        let mut other_taskbars = Vec::new();
        for cls in [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")] {
            let mut prev_tb = HWND::default();
            for _ in 0..32 {
                let Ok(hwnd) = FindWindowExW(None, prev_tb, cls, None) else {
                    break;
                };
                if hwnd.is_invalid() || hwnd == prev_tb {
                    break;
                }
                prev_tb = hwnd;
                if !IsWindowVisible(hwnd).as_bool() {
                    continue;
                }
                let mut pid = 0u32;
                let tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != explorer_pid {
                    continue;
                }
                if let Some(pt) = xaml_island_screen_origin(hwnd) {
                    if tid == host_tid {
                        push_unique(&mut origins, pt);
                    } else if !other_taskbars.contains(&pt) {
                        other_taskbars.push(pt);
                    }
                }
            }
        }
        for pt in other_taskbars {
            push_unique(&mut origins, pt);
        }
    }

    origins
}

fn inspect_overflow_candidate(hwnd: HWND, is_xaml_island: bool) -> Option<(HWND, RECT, u32)> {
    unsafe {
        if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return None;
        }
        let mut rect = RECT::default();
        let got_ext = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
        .is_ok();
        if !got_ext && GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w <= 0 || h <= 0 || w > 960 || h > 720 {
            return None;
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        let [l, t, r, b] =
            inset_overflow_card_rect([rect.left, rect.top, rect.right, rect.bottom], dpi, is_xaml_island);
        rect = RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        };
        Some((hwnd, rect, dpi))
    }
}

fn find_visible_overflow_window() -> Option<(HWND, RECT, u32)> {
    unsafe {
        for (cls, is_xaml_island) in [
            (w!("TopLevelWindowForOverflowXamlIsland"), true),
            (w!("NotifyIconOverflowWindow"), false),
        ] {
            if let Ok(hwnd) = FindWindowW(cls, None) {
                if let Some(found) = inspect_overflow_candidate(hwnd, is_xaml_island) {
                    return Some(found);
                }
            }
        }
        None
    }
}

fn should_hide_backdrop_for_taskbar(tb_hwnd: HWND, tb_rect: RECT) -> bool {
    unsafe {
        if !IsWindowVisible(tb_hwnd).as_bool() {
            return true;
        }
        if tb_rect.right <= tb_rect.left || tb_rect.bottom <= tb_rect.top {
            return true;
        }

        let mon_tb = MonitorFromWindow(tb_hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(mon_tb, &mut mi).as_bool() {
            let mut abd = APPBARDATA {
                cbSize: size_of::<APPBARDATA>() as u32,
                hWnd: tb_hwnd,
                ..Default::default()
            };
            let is_autohide = (SHAppBarMessage(ABM_GETSTATE, &mut abd) as u32 & ABS_AUTOHIDE) != 0;
            let threshold = if is_autohide { 8 } else { 2 };
            if tb_rect.top >= mi.rcMonitor.bottom - threshold
                || tb_rect.bottom <= mi.rcMonitor.top + threshold
            {
                return true;
            }
        }
    }
    crate::system::is_fullscreen_occluding_taskbar(tb_hwnd)
}

#[allow(clippy::too_many_arguments)]
fn update_wide_silk_bitmap(
    rt: &ID2D1DCRenderTarget,
    silk_bmp: &mut Option<ID2D1Bitmap>,
    silk_buf: &mut Vec<u8>,
    last_silk_tick: &mut Option<Instant>,
    win_w: i32,
    win_h: i32,
    s: f32,
    flow_phase: f64,
    halo_colors: &[(f32, f32, f32); 2],
    light_glass: bool,
) {
    let now = Instant::now();
    let need_rebuild = silk_bmp.is_none()
        || last_silk_tick
            .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(SILK_UPDATE_INTERVAL_MS));
    if !need_rebuild {
        return;
    }
    *last_silk_tick = Some(now);

    let expected_len = (BAR_SILK_W * BAR_SILK_H * 4) as usize;
    if silk_buf.len() != expected_len {
        silk_buf.resize(expected_len, 0);
    }

    let scale_len = (420.0 * s).max(1.0);
    let phase = (flow_phase % 10_000.0) as f32;
    let wf = win_w.max(1) as f32;
    let hf = win_h.max(1) as f32;

    for gy in 0..BAR_SILK_H {
        let py = (((gy as f32) + 0.5) / (BAR_SILK_H as f32)) * hf;
        let y = (hf - py) / scale_len;
        for gx in 0..BAR_SILK_W {
            let px = (((gx as f32) + 0.5) / (BAR_SILK_W as f32)) * wf;
            let x = px / scale_len;
            let (r, g, b, a) = sample_liquid_silk_field(x, y, phase, halo_colors, light_glass);
            let idx = ((gy * BAR_SILK_W + gx) * 4) as usize;
            silk_buf[idx] = (b * a * 255.0).round().clamp(0.0, 255.0) as u8;
            silk_buf[idx + 1] = (g * a * 255.0).round().clamp(0.0, 255.0) as u8;
            silk_buf[idx + 2] = (r * a * 255.0).round().clamp(0.0, 255.0) as u8;
            silk_buf[idx + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    unsafe {
        if let Some(ref bmp) = *silk_bmp {
            let _ = bmp.CopyFromMemory(None, silk_buf.as_ptr().cast(), BAR_SILK_W * 4);
        } else {
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            *silk_bmp = rt
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: BAR_SILK_W,
                        height: BAR_SILK_H,
                    },
                    Some(silk_buf.as_ptr().cast()),
                    BAR_SILK_W * 4,
                    &props,
                )
                .ok();
        }
    }
}

unsafe fn ensure_acrylic_noise_brush<'a>(
    rt: &ID2D1DCRenderTarget,
    noise_brush: &'a mut Option<ID2D1BitmapBrush>,
    noise_key: &mut Option<(bool, u8)>,
    light_glass: bool,
    max_alpha: u8,
) -> Option<&'a ID2D1BitmapBrush> {
    if noise_brush.is_none() || *noise_key != Some((light_glass, max_alpha)) {
        const NOISE_SIZE: u32 = 128;
        let noise_bytes =
            generate_acrylic_noise_bgra(NOISE_SIZE as usize, 0x51A7_C301, max_alpha);
        let bmp_props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let bmp = rt
            .CreateBitmap(
                D2D_SIZE_U {
                    width: NOISE_SIZE,
                    height: NOISE_SIZE,
                },
                Some(noise_bytes.as_ptr().cast()),
                NOISE_SIZE * 4,
                &bmp_props,
            )
            .ok()?;
        let brush_props = D2D1_BITMAP_BRUSH_PROPERTIES {
            extendModeX: D2D1_EXTEND_MODE_WRAP,
            extendModeY: D2D1_EXTEND_MODE_WRAP,
            interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
        };
        let brush = rt.CreateBitmapBrush(&bmp, Some(&brush_props), None).ok()?;
        *noise_brush = Some(brush);
        *noise_key = Some((light_glass, max_alpha));
    }
    noise_brush.as_ref()
}

unsafe fn fill_linear_gradient(
    rt: &ID2D1DCRenderTarget,
    rect: D2D_RECT_F,
    start: D2D_POINT_2F,
    end: D2D_POINT_2F,
    stops: &[D2D1_GRADIENT_STOP],
) {
    if let Ok(sc) = rt.CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: start,
            endPoint: end,
        };
        if let Ok(brush) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.FillRectangle(&rect, &brush);
        }
    }
}

unsafe fn draw_horizontal_gradient_line(
    rt: &ID2D1DCRenderTarget,
    wf: f32,
    y: f32,
    stroke_w: f32,
    stops: &[D2D1_GRADIENT_STOP],
) {
    if let Ok(sc) = rt.CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: D2D_POINT_2F { x: 0.0, y },
            endPoint: D2D_POINT_2F { x: wf, y },
        };
        if let Ok(eb) = rt.CreateLinearGradientBrush(&props, None, &sc) {
            rt.DrawLine(
                D2D_POINT_2F { x: 0.0, y },
                D2D_POINT_2F { x: wf, y },
                &eb,
                stroke_w,
                None,
            );
        }
    }
}

unsafe fn draw_top_rim_line(
    rt: &ID2D1DCRenderTarget,
    wf: f32,
    s: f32,
    light_glass: bool,
    dark_sep_alpha: Option<f32>,
    edge_alpha: f32,
    mid_alpha: f32,
) {
    let edge_y = 0.6 * s;
    if let Some(sep_a) = dark_sep_alpha {
        let def_col = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: sep_a,
        };
        if let Ok(db) = rt.CreateSolidColorBrush(&def_col, None) {
            rt.DrawLine(
                D2D_POINT_2F { x: 0.0, y: edge_y },
                D2D_POINT_2F { x: wf, y: edge_y },
                &db,
                1.0 * s,
                None,
            );
        }
    }
    let edge_stops = [
        D2D1_GRADIENT_STOP {
            position: 0.0,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: edge_alpha,
            },
        },
        D2D1_GRADIENT_STOP {
            position: 0.5,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: mid_alpha,
            },
        },
        D2D1_GRADIENT_STOP {
            position: 1.0,
            color: D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: edge_alpha,
            },
        },
    ];
    let stroke_w = if light_glass { 1.0 * s } else { 1.15 * s };
    draw_horizontal_gradient_line(rt, wf, edge_y, stroke_w, &edge_stops);
}

#[allow(clippy::too_many_arguments)]
fn render_layered_backdrop(
    entry: &mut MonitorGlassBackdrop,
    style: TaskbarStyle,
    tb_rect: RECT,
    w: i32,
    h: i32,
    dpi: u32,
    flow_phase: f64,
    halo_colors: &[(f32, f32, f32); 2],
    light_glass: bool,
) -> windows::core::Result<()> {
    entry.surface.ensure_size(w, h)?;
    let s = (dpi.max(96) as f32) / 96.0;
    let wf = w as f32;
    let hf = h as f32;
    let has_live_blur = entry.has_live_blur;

    if style == TaskbarStyle::LiquidGlass {
        update_wide_silk_bitmap(
            &entry.rt,
            &mut entry.silk_bmp,
            &mut entry.silk_buf,
            &mut entry.last_silk_tick,
            w,
            h,
            s,
            flow_phase,
            halo_colors,
            light_glass,
        );
    }

    let bind_rect = RECT {
        left: 0,
        top: 0,
        right: entry.surface.width(),
        bottom: entry.surface.height(),
    };
    let full = d2d_rect(RectF::new(0.0, 0.0, wf, hf));

    unsafe {
        entry.rt.BindDC(entry.surface.dc(), &bind_rect)?;
        entry.rt.BeginDraw();
        entry.rt.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        match style {
            TaskbarStyle::Blur => {
                // Tier 1: Frosted Glass (毛玻璃) — creamy uniform frost tint + soft white
                // volumetric scattering veil + fine acid-etched micro-grain + gentle hairline.
                let base_alpha = match (light_glass, has_live_blur) {
                    (false, true) => 0.24,
                    (false, false) => 0.36,
                    (true, true) => 0.34,
                    (true, false) => 0.44,
                };
                let base_col = if light_glass {
                    D2D1_COLOR_F {
                        r: 248.0 / 255.0,
                        g: 250.0 / 255.0,
                        b: 254.0 / 255.0,
                        a: base_alpha,
                    }
                } else {
                    D2D1_COLOR_F {
                        r: 26.0 / 255.0,
                        g: 28.0 / 255.0,
                        b: 34.0 / 255.0,
                        a: base_alpha,
                    }
                };
                if let Ok(base_brush) = entry.rt.CreateSolidColorBrush(&base_col, None) {
                    entry.rt.FillRectangle(&full, &base_brush);
                }

                let frost_veil_stops = if light_glass {
                    [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.20,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.55,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.08,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.03,
                            },
                        },
                    ]
                } else {
                    [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.11,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.55,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.04,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.015,
                            },
                        },
                    ]
                };
                fill_linear_gradient(
                    &entry.rt,
                    full,
                    D2D_POINT_2F { x: 0.0, y: 0.0 },
                    D2D_POINT_2F { x: 0.0, y: hf },
                    &frost_veil_stops,
                );

                let grain_max_alpha = if light_glass { 5 } else { 6 };
                if let Some(noise_brush) = ensure_acrylic_noise_brush(
                    &entry.rt,
                    &mut entry.noise_brush,
                    &mut entry.noise_key,
                    light_glass,
                    grain_max_alpha,
                ) {
                    entry.rt.FillRectangle(&full, noise_brush);
                }

                let edge_y = 0.5 * s;
                let sep_col = if light_glass {
                    D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.07,
                    }
                } else {
                    D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.16,
                    }
                };
                if let Ok(sb) = entry.rt.CreateSolidColorBrush(&sep_col, None) {
                    entry.rt.DrawLine(
                        D2D_POINT_2F { x: 0.0, y: edge_y },
                        D2D_POINT_2F { x: wf, y: edge_y },
                        &sb,
                        1.0 * s,
                        None,
                    );
                }
            }
            TaskbarStyle::Acrylic => {
                // Tier 2: Classic Win11 dense frosted acrylic with steady tint, vertical finish & 128x128 grain.
                let base_alpha = match (light_glass, has_live_blur) {
                    (false, true) => 0.42,
                    (false, false) => 0.50,
                    (true, true) => 0.50,
                    (true, false) => 0.58,
                };
                let base_col = if light_glass {
                    D2D1_COLOR_F {
                        r: 246.0 / 255.0,
                        g: 247.0 / 255.0,
                        b: 250.0 / 255.0,
                        a: base_alpha,
                    }
                } else {
                    D2D1_COLOR_F {
                        r: 22.0 / 255.0,
                        g: 23.0 / 255.0,
                        b: 27.0 / 255.0,
                        a: base_alpha,
                    }
                };
                if let Ok(base_brush) = entry.rt.CreateSolidColorBrush(&base_col, None) {
                    entry.rt.FillRectangle(&full, &base_brush);
                }

                let frost_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: if light_glass { 0.12 } else { 0.05 },
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.0,
                        },
                    },
                ];
                fill_linear_gradient(
                    &entry.rt,
                    full,
                    D2D_POINT_2F { x: 0.0, y: 0.0 },
                    D2D_POINT_2F { x: 0.0, y: hf },
                    &frost_stops,
                );

                let acrylic_grain_alpha = if light_glass { 9 } else { 11 };
                if let Some(noise_brush) = ensure_acrylic_noise_brush(
                    &entry.rt,
                    &mut entry.noise_brush,
                    &mut entry.noise_key,
                    light_glass,
                    acrylic_grain_alpha,
                ) {
                    entry.rt.FillRectangle(&full, noise_brush);
                }

                if light_glass {
                    draw_top_rim_line(&entry.rt, wf, s, true, Some(0.08), 0.48, 0.48);
                } else {
                    draw_top_rim_line(&entry.rt, wf, s, false, None, 0.22, 0.22);
                }
            }
            TaskbarStyle::LiquidGlass => {
                // Tier 3: Optical Liquid Glass (无高光棱边 + 强柔焦) — crystal body +
                // soft bottom legibility gradient + wide-aspect liquid silk ribbons + gentle diffuse glow.
                let base_stops = match (light_glass, has_live_blur) {
                    (false, true) => [
                        (0.0, 0.03),
                        (0.52, 0.07),
                        (1.0, 0.20),
                    ],
                    (false, false) => [
                        (0.0, 0.10),
                        (0.52, 0.15),
                        (1.0, 0.28),
                    ],
                    (true, true) => [
                        (0.0, 0.10),
                        (0.55, 0.15),
                        (1.0, 0.24),
                    ],
                    (true, false) => [
                        (0.0, 0.18),
                        (0.55, 0.24),
                        (1.0, 0.34),
                    ],
                };
                let (br, bg, bb) = if light_glass {
                    (248.0 / 255.0, 250.0 / 255.0, 1.0)
                } else {
                    (14.0 / 255.0, 16.0 / 255.0, 22.0 / 255.0)
                };
                let body_grad = [
                    D2D1_GRADIENT_STOP {
                        position: base_stops[0].0,
                        color: D2D1_COLOR_F {
                            r: br,
                            g: bg,
                            b: bb,
                            a: base_stops[0].1,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: base_stops[1].0,
                        color: D2D1_COLOR_F {
                            r: br,
                            g: bg,
                            b: bb,
                            a: base_stops[1].1,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: base_stops[2].0,
                        color: D2D1_COLOR_F {
                            r: br,
                            g: bg,
                            b: bb,
                            a: base_stops[2].1,
                        },
                    },
                ];
                fill_linear_gradient(
                    &entry.rt,
                    full,
                    D2D_POINT_2F { x: 0.0, y: 0.0 },
                    D2D_POINT_2F { x: 0.0, y: hf },
                    &body_grad,
                );

                // 2. Wide-aspect domain-warped 3-ribbon Liquid Silk wisps (adapted to unified theme accent).
                if let Some(ref silk) = entry.silk_bmp {
                    let silk_alpha = if light_glass { 0.96 } else { 1.0 };
                    entry.rt.DrawBitmap(
                        silk,
                        Some(&full),
                        silk_alpha,
                        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                        None,
                    );
                }

                // 3. Soft diagonal optical diffuse glow without bright glass edge rims.
                let sheen_stops = if light_glass {
                    [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.20,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.42,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.03,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.75,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.06,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.11,
                            },
                        },
                    ]
                } else {
                    [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.12,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.42,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.015,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.75,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.035,
                            },
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: D2D1_COLOR_F {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 0.07,
                            },
                        },
                    ]
                };
                fill_linear_gradient(
                    &entry.rt,
                    full,
                    D2D_POINT_2F { x: 0.0, y: 0.0 },
                    D2D_POINT_2F { x: wf, y: hf },
                    &sheen_stops,
                );
            }
            TaskbarStyle::Default | TaskbarStyle::Transparent => {}
        }

        entry.rt.EndDraw(None, None)?;
    }

    entry
        .surface
        .commit(entry.glass_hwnd, tb_rect.left, tb_rect.top);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_shell_card_layered_backdrop(
    d2d_factory: &ID2D1Factory1,
    ov: &mut ShellCardGlassBackdrop,
    style: TaskbarStyle,
    glass_rect: RECT,
    card_rect: RECT,
    pad_px: i32,
    dpi: u32,
    flow_phase: f64,
    halo_colors: &[(f32, f32, f32); 2],
    light_glass: bool,
) -> windows::core::Result<()> {
    let glass_w = (glass_rect.right - glass_rect.left).max(1);
    let glass_h = (glass_rect.bottom - glass_rect.top).max(1);
    let card_w = (card_rect.right - card_rect.left).max(1);
    let card_h = (card_rect.bottom - card_rect.top).max(1);
    ov.surface.ensure_size(glass_w, glass_h)?;

    let s = (dpi.max(96) as f32) / 96.0;
    let pad_f = pad_px.max(0) as f32;
    let wf = card_w as f32;
    let hf = card_h as f32;
    let radius = (8.0 * s).min((wf.min(hf) * 0.45).max(2.0));
    let card_local = RectF::new(pad_f, pad_f, pad_f + wf, pad_f + hf);
    let card_d2d = d2d_rect(card_local);
    let has_live_blur = ov.has_live_blur;
    let recipe = popup_glass_recipe(GlassSurfaceStyle::from(style), light_glass, 0.5);

    if style == TaskbarStyle::LiquidGlass {
        let force_silk = ov.silk_bmp.is_none();
        update_silk_bitmap(
            &ov.rt,
            &mut ov.silk_bmp,
            &mut ov.silk_buf,
            &mut ov.last_silk_tick,
            force_silk,
            card_w,
            card_h,
            0.0,
            hf,
            s,
            flow_phase,
            halo_colors,
            light_glass,
        );
    }

    let bind_rect = RECT {
        left: 0,
        top: 0,
        right: ov.surface.width(),
        bottom: ov.surface.height(),
    };

    unsafe {
        ov.rt.BindDC(ov.surface.dc(), &bind_rect)?;
        ov.rt.BeginDraw();
        ov.rt.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        let rr = D2D1_ROUNDED_RECT {
            rect: card_d2d,
            radiusX: radius,
            radiusY: radius,
        };
        if let Ok(clip_raw) = d2d_factory.CreateRoundedRectangleGeometry(&rr) {
            let clip_geo: ID2D1Geometry = clip_raw.cast()?;

            if pad_px > 0 {
                let shadow_scale = (pad_f / (10.0 * s)).clamp(0.55, 1.0);
                let shadow_layers: [(f32, f32); 3] = if light_glass {
                    [(7.5, 0.022), (4.5, 0.036), (2.0, 0.058)]
                } else {
                    [(7.5, 0.035), (4.5, 0.055), (2.0, 0.090)]
                };
                let mult = if style == TaskbarStyle::Transparent {
                    0.65
                } else {
                    1.0
                };
                let gf_w = glass_w as f32;
                let gf_h = glass_h as f32;
                for (expand_dip, base_alpha) in shadow_layers {
                    let expand = expand_dip * shadow_scale * s;
                    let side_exp = expand * 0.55;
                    let top_exp = expand * 0.45;
                    let bot_exp = expand * 0.65;
                    let sh_rect = RectF::new(
                        (card_local.left - side_exp).max(0.0),
                        (card_local.top - top_exp).max(0.0),
                        (card_local.right + side_exp).min(gf_w),
                        (card_local.bottom + bot_exp).min(gf_h),
                    );
                    let sh_rr = D2D1_ROUNDED_RECT {
                        rect: d2d_rect(sh_rect),
                        radiusX: radius + side_exp * 0.45,
                        radiusY: radius + side_exp * 0.45,
                    };
                    if let Ok(outer_raw) = d2d_factory.CreateRoundedRectangleGeometry(&sh_rr) {
                        let outer_geo: ID2D1Geometry = outer_raw.cast()?;
                        if let Ok(ring) = d2d_factory.CreatePathGeometry() {
                            if let Ok(sink) = ring.Open() {
                                if outer_geo
                                    .CombineWithGeometry(
                                        &clip_geo,
                                        D2D1_COMBINE_MODE_EXCLUDE,
                                        None,
                                        0.25,
                                        &sink,
                                    )
                                    .is_ok()
                                    && sink.Close().is_ok()
                                {
                                    let ring_geo: ID2D1Geometry = ring.cast()?;
                                    let sh_col = D2D1_COLOR_F {
                                        r: 0.0,
                                        g: 0.0,
                                        b: 0.0,
                                        a: base_alpha * mult,
                                    };
                                    if let Ok(sh_brush) = ov.rt.CreateSolidColorBrush(&sh_col, None)
                                    {
                                        ov.rt.FillGeometry(&ring_geo, &sh_brush, None);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let outer_params = D2D1_LAYER_PARAMETERS {
                contentBounds: card_d2d,
                geometricMask: ManuallyDrop::new(Some(clip_geo)),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: identity_matrix(),
                opacity: 1.0,
                opacityBrush: ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            ov.rt.PushLayer(&outer_params, &ov.layer);

            let (tr, tg, tb) = recipe.tint_rgb;
            let base_a = match (style, light_glass, has_live_blur) {
                (TaskbarStyle::Transparent, false, _) => 0.22,
                (TaskbarStyle::Transparent, true, _) => 0.26,
                (TaskbarStyle::Blur, false, true) => 0.26,
                (TaskbarStyle::Blur, false, false) => 0.38,
                (TaskbarStyle::Blur, true, true) => 0.34,
                (TaskbarStyle::Blur, true, false) => 0.48,
                (TaskbarStyle::Acrylic, false, true) => 0.42,
                (TaskbarStyle::Acrylic, false, false) => 0.52,
                (TaskbarStyle::Acrylic, true, true) => 0.50,
                (TaskbarStyle::Acrylic, true, false) => 0.62,
                (TaskbarStyle::LiquidGlass, false, true) => 0.14,
                (TaskbarStyle::LiquidGlass, false, false) => 0.28,
                (TaskbarStyle::LiquidGlass, true, true) => 0.20,
                (TaskbarStyle::LiquidGlass, true, false) => 0.34,
                (TaskbarStyle::Default, _, _) => 0.0,
            };
            let base_col = D2D1_COLOR_F {
                r: tr,
                g: tg,
                b: tb,
                a: base_a,
            };
            if let Ok(base_brush) = ov.rt.CreateSolidColorBrush(&base_col, None) {
                ov.rt.FillRectangle(&card_d2d, &base_brush);
            }

            if recipe.frost_veil_top_alpha > 0.002 {
                let veil_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: recipe.frost_veil_top_alpha,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.55,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: recipe.frost_veil_mid_alpha,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: recipe.frost_veil_bot_alpha,
                        },
                    },
                ];
                fill_linear_gradient(
                    &ov.rt,
                    card_d2d,
                    D2D_POINT_2F {
                        x: card_local.left,
                        y: card_local.top,
                    },
                    D2D_POINT_2F {
                        x: card_local.left,
                        y: card_local.bottom,
                    },
                    &veil_stops,
                );
            }

            if recipe.grain_max_alpha > 0 {
                if let Some(noise_brush) = ov.noise_cache.ensure_brush(&ov.rt, recipe.grain_max_alpha)
                {
                    ov.rt.FillRectangle(&card_d2d, noise_brush);
                }
            }

            if style == TaskbarStyle::LiquidGlass && recipe.silk_alpha > 0.002 {
                if let Some(ref silk) = ov.silk_bmp {
                    ov.rt.DrawBitmap(
                        silk,
                        Some(&card_d2d),
                        recipe.silk_alpha,
                        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                        None,
                    );
                }
            }

            if recipe.sheen_scale > 0.01 {
                let ss = recipe.sheen_scale;
                let sheen_stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: (if light_glass { 0.24 } else { 0.14 }) * ss,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.45,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: (if light_glass { 0.03 } else { 0.02 }) * ss,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.0,
                        color: D2D1_COLOR_F {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: (if light_glass { 0.09 } else { 0.06 }) * ss,
                        },
                    },
                ];
                fill_linear_gradient(
                    &ov.rt,
                    card_d2d,
                    D2D_POINT_2F {
                        x: card_local.left,
                        y: card_local.top,
                    },
                    D2D_POINT_2F {
                        x: card_local.right,
                        y: card_local.bottom,
                    },
                    &sheen_stops,
                );
            }

            ov.rt.PopLayer();
            let _ = ManuallyDrop::into_inner(outer_params.geometricMask);

            let inset = 0.5 * s;
            let rim_rect = RectF::new(
                card_local.left + inset,
                card_local.top + inset,
                (card_local.right - inset).max(card_local.left + inset + 1.0),
                (card_local.bottom - inset).max(card_local.top + inset + 1.0),
            );
            let rim_r = (radius - inset).max(1.0);
            draw_rounded_card_rim(&ov.rt, rim_rect, rim_r, s, &recipe);
        }

        ov.rt.EndDraw(None, None)?;
    }

    ov.surface.commit(ov.glass_hwnd, glass_rect.left, glass_rect.top);
    Ok(())
}
