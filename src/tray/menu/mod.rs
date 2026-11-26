mod icons;
mod render;
mod wndproc;

use wndproc::tray_menu_wnd_proc;

use super::layout::{
    adjust_tray_anchor_for_work_area, animated_tray_card_rect, compute_tray_cascading_placement,
    TrayMenuHover, TrayMenuLayout, TraySubmenuKind,
};
use crate::color::{Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::deskbox::BoxViewMode;
use crate::system::{resolve_unified_accent, visible_transient_overlay_rects};
use crate::taskbar::TaskbarStyle;
use crate::ui::glass::{capture_blurred_desktop_backdrop_excluding, create_dc_render_target, create_round_cap_stroke, create_text_format, update_silk_bitmap, LayeredSurface, PopupNoiseCache};
use crate::ui::{compute_halo_pair, ease_retract, popup_glass_recipe, GlassSurfaceStyle, PopupAnimPhase, RectF, Spring1D, CLOSE_ANIM_DUR_S, OPEN_SPRING_DAMPING, OPEN_SPRING_STIFFNESS, POPUP_ANIM_INTERVAL_MS, POPUP_FLOW_INTERVAL_MS, POPUP_REOPEN_DEBOUNCE_MS, SILK_H, SILK_W, SUB_WIDTH_DAMPING, SUB_WIDTH_STIFFNESS};
use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Layer, ID2D1StrokeStyle};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, GetForegroundWindow, IsWindowVisible, KillTimer, LoadCursorW, RegisterClassExW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, HCURSOR, HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, WM_APP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP};

pub const WM_APP_TRAY_TOGGLE_MEDIABAR: u32 = WM_APP + 21;
pub const WM_APP_TRAY_TOGGLE_AUTOSTART: u32 = WM_APP + 22;
pub const WM_APP_TRAY_EXIT: u32 = WM_APP + 23;
pub const WM_APP_TRAY_MENU_CLOSED: u32 = WM_APP + 24;
pub const WM_APP_TRAY_SET_TASKBAR_STYLE: u32 = WM_APP + 26;
pub const WM_APP_TRAY_TOGGLE_MONITORBAR: u32 = WM_APP + 27;
pub const WM_APP_TRAY_DESKBOX_ACTION: u32 = WM_APP + 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DeskBoxMenuState {
    pub enabled: bool,
    pub hide_desktop_icons: bool,
    pub default_view_mode: BoxViewMode,
}

const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const TRAY_MENU_CLASS_NAME: PCWSTR = w!("PrettyWindowsTrayMenuWnd");
const TIMER_MENU_FLOW_ID: usize = 1;
const SHELL_DEACTIVATE_GRACE_MS: u64 = 80;

struct InitHwndGuard(Option<HWND>);

impl InitHwndGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for InitHwndGuard {
    fn drop(&mut self) {
        if let Some(hwnd) = self.0.take() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

struct TrayMenuInner {
    hwnd: HWND,
    host_hwnd: HWND,
    d2d_factory: ID2D1Factory1,
    dwrite_factory: IDWriteFactory,
    // Declare RT-owned resources and `rt` before `surface` so `rt` releases before `DeleteDC`.
    backdrop_bmp: Option<ID2D1Bitmap>,
    noise_cache: PopupNoiseCache,
    silk_bmp: Option<ID2D1Bitmap>,
    layer: ID2D1Layer,
    content_layer: ID2D1Layer,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    round_stroke: Option<ID2D1StrokeStyle>,
    dpi: u32,
    light_glass: bool,
    backdrop_rgb: Rgb,
    mediabar_visible: bool,
    monitorbar_visible: bool,
    deskbox_state: DeskBoxMenuState,
    taskbar_style: TaskbarStyle,
    autostart: bool,
    hover: Option<TrayMenuHover>,
    tracking_leave: bool,
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
    sub_open_left: bool,
    open_down: bool,
    active_submenu: Option<TraySubmenuKind>,
    sub_width_spring: Spring1D,
    sub_fade_elapsed: f32,
    silk_buf: Vec<u8>,
    flow_phase: f64,
    halo_colors: [(f32, f32, f32); 2],
    ui_accent: (f32, f32, f32),
    halo_colors_ready: bool,
    last_flow_tick: Instant,
    last_silk_tick: Option<Instant>,
    last_color_tick: Instant,
    anim_phase: PopupAnimPhase,
    open_spring: Spring1D,
    open_elapsed: f32,
    close_progress: f32,
    shown_at: Option<Instant>,
    shell_focus_reclaimed: bool,
    close_by_deactivate_at: Option<Instant>,
    timer_interval_ms: u32,
    cached_fmt: Option<(u32, IDWriteTextFormat)>,
    arrow_cursor: HCURSOR,
}

pub struct TrayMenu {
    hwnd: HWND,
    inner: Rc<RefCell<TrayMenuInner>>,
}

impl TrayMenu {
    pub fn new(
        host_hwnd: HWND,
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
    ) -> windows::core::Result<Self> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let arrow_cursor = LoadCursorW(None, IDC_ARROW)?;
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(tray_menu_wnd_proc),
                hInstance: hinst.into(),
                hCursor: arrow_cursor,
                lpszClassName: TRAY_MENU_CLASS_NAME,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                TRAY_MENU_CLASS_NAME,
                w!("Pretty Windows Menu"),
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
            let mut hwnd_guard = InitHwndGuard(Some(hwnd));

            let rt = create_dc_render_target(&d2d_factory)?;
            let layer = rt.CreateLayer(None)?;
            let content_layer = rt.CreateLayer(None)?;
            let surface = LayeredSurface::new()?;
            let round_stroke = create_round_cap_stroke(&d2d_factory);

            let inner = Rc::new(RefCell::new(TrayMenuInner {
                hwnd,
                host_hwnd,
                d2d_factory,
                dwrite_factory,
                backdrop_bmp: None,
                noise_cache: PopupNoiseCache::default(),
                silk_bmp: None,
                layer,
                content_layer,
                rt,
                surface,
                round_stroke,
                dpi: 96,
                light_glass: false,
                backdrop_rgb: NEUTRAL_BACKDROP_FALLBACK,
                mediabar_visible: true,
                monitorbar_visible: true,
                deskbox_state: DeskBoxMenuState::default(),
                taskbar_style: TaskbarStyle::default(),
                autostart: false,
                hover: None,
                tracking_leave: false,
                win_x: 0,
                win_y: 0,
                win_w: 1,
                win_h: 1,
                sub_open_left: false,
                open_down: false,
                active_submenu: None,
                sub_width_spring: Spring1D::still(0.0),
                sub_fade_elapsed: 0.0,
                silk_buf: vec![0u8; (SILK_W * SILK_H * 4) as usize],
                flow_phase: 0.0,
                halo_colors: [(0.76, 0.76, 0.78); 2],
                ui_accent: (0.76, 0.76, 0.78),
                halo_colors_ready: false,
                last_flow_tick: Instant::now(),
                last_silk_tick: None,
                last_color_tick: Instant::now(),
                anim_phase: PopupAnimPhase::Open,
                open_spring: Spring1D::still(1.0),
                open_elapsed: 1.0,
                close_progress: 0.0,
                shown_at: None,
                shell_focus_reclaimed: false,
                close_by_deactivate_at: None,
                timer_interval_ms: 0,
                cached_fmt: None,
                arrow_cursor,
            }));

            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                Rc::into_raw(Rc::clone(&inner)) as isize,
            );
            hwnd_guard.disarm();

            Ok(Self { hwnd, inner })
        }
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn is_open(&self) -> bool {
        self.is_visible()
            && self
                .inner
                .try_borrow()
                .is_ok_and(|s| s.anim_phase != PopupAnimPhase::Closing)
    }

    pub fn is_closing_recently(&self) -> bool {
        self.inner.try_borrow().is_ok_and(|s| {
            s.close_by_deactivate_at
                .is_some_and(|t| t.elapsed() < Duration::from_millis(POPUP_REOPEN_DEBOUNCE_MS))
        })
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.reset_transient_state();
        }
    }

    pub fn hide_animated(&mut self) {
        if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.begin_close_from_deactivate();
        }
    }

    pub fn ensure_topmost(&self) {
        if self.is_visible() {
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }

    pub fn set_state(
        &mut self,
        mediabar_visible: bool,
        monitorbar_visible: bool,
        deskbox_state: DeskBoxMenuState,
        taskbar_style: TaskbarStyle,
        autostart: bool,
    ) {
        let visible = self.is_visible();
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.mediabar_visible = mediabar_visible;
            s.monitorbar_visible = monitorbar_visible;
            s.deskbox_state = deskbox_state;
            s.taskbar_style = taskbar_style;
            s.autostart = autostart;
            if visible {
                s.sync_timer_rate();
            }
            visible
        } else {
            false
        };
        if should_render {
            if let Ok(mut s) = self.inner.try_borrow_mut() {
                let _ = s.render();
            }
        }
    }

    pub fn set_system_theme(&mut self, light_glass: bool, _fallback_accent: Rgb) {
        let visible = self.is_visible();
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            let changed = s.light_glass != light_glass;
            s.light_glass = light_glass;
            if changed {
                s.halo_colors_ready = false;
                s.last_silk_tick = None;
                if visible {
                    s.sync_timer_rate();
                }
                visible
            } else {
                false
            }
        } else {
            false
        };
        if should_render {
            if let Ok(mut s) = self.inner.try_borrow_mut() {
                let _ = s.render();
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        anchor_pt: POINT,
        dpi: u32,
        light_glass: bool,
        _fallback_accent: Rgb,
        mediabar_visible: bool,
        monitorbar_visible: bool,
        deskbox_state: DeskBoxMenuState,
        taskbar_style: TaskbarStyle,
        autostart: bool,
    ) {
        let Some((host_hwnd, capture_rect, win_x, win_y, win_w, win_h)) = (|| {
            let mut s = self.inner.try_borrow_mut().ok()?;
            s.dpi = dpi.max(96);
            s.light_glass = light_glass;
            s.backdrop_rgb = NEUTRAL_BACKDROP_FALLBACK;
            s.mediabar_visible = mediabar_visible;
            s.monitorbar_visible = monitorbar_visible;
            s.deskbox_state = deskbox_state;
            s.taskbar_style = taskbar_style;
            s.autostart = autostart;
            s.hover = None;
            s.active_submenu = None;
            s.sub_width_spring = Spring1D::still(0.0);
            s.sub_fade_elapsed = 0.0;

            let mon = unsafe { MonitorFromPoint(anchor_pt, MONITOR_DEFAULTTONEAREST) };
            let mut mi = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let (monitor, work) = unsafe {
                if GetMonitorInfoW(mon, &mut mi).as_bool() {
                    (mi.rcMonitor, mi.rcWork)
                } else {
                    (
                        RECT {
                            left: 0,
                            top: 0,
                            right: 1920,
                            bottom: 1080,
                        },
                        RECT {
                            left: 0,
                            top: 0,
                            right: 1920,
                            bottom: 1032,
                        },
                    )
                }
            };

            let (adj_x, adj_y) = adjust_tray_anchor_for_work_area(
                anchor_pt.x,
                anchor_pt.y,
                work.left,
                work.top,
                work.right,
                work.bottom,
                monitor.left,
                monitor.top,
                monitor.right,
                monitor.bottom,
                s.dpi,
            );

            let (win_x, win_y, sub_open_left, open_down) = compute_tray_cascading_placement(
                adj_x,
                adj_y,
                monitor.left,
                monitor.top,
                monitor.right,
                monitor.bottom,
                s.dpi,
            );
            let layout = TrayMenuLayout::compute(s.dpi, sub_open_left, open_down);

            s.win_x = win_x;
            s.win_y = win_y;
            s.win_w = layout.win_w;
            s.win_h = layout.win_h;
            s.sub_open_left = sub_open_left;
            s.open_down = open_down;

            let capture_rect = RECT {
                left: win_x,
                top: win_y,
                right: win_x + layout.win_w,
                bottom: win_y + layout.win_h,
            };
            Some((
                s.host_hwnd,
                capture_rect,
                win_x,
                win_y,
                layout.win_w,
                layout.win_h,
            ))
        })() else {
            return;
        };

        // Hide any lingering visible frame before capturing desktop backdrop so self-blur ghosting cannot occur.
        if self.is_visible() {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }

        // Collect visible transient shell flyouts BEFORE stealing foreground dismisses/cloaks them.
        let occluders = visible_transient_overlay_rects(capture_rect);
        unsafe {
            let _ = SetForegroundWindow(host_hwnd);
        }

        if let Ok(mut s) = self.inner.try_borrow_mut() {
            let initial_recipe =
                popup_glass_recipe(GlassSurfaceStyle::from(taskbar_style), light_glass, 0.5);
            let (bmp, sampled_bg) = capture_blurred_desktop_backdrop_excluding(
                &s.rt,
                win_x,
                win_y,
                win_w,
                win_h,
                initial_recipe.blur_radius_4x,
                &occluders,
            );
            s.backdrop_bmp = bmp;
            if let Some(bg) = sampled_bg {
                s.backdrop_rgb = bg;
            }
            s.halo_colors_ready = false;
            s.last_silk_tick = None;
            s.last_color_tick = Instant::now();
            s.anim_phase = PopupAnimPhase::Opening;
            s.open_spring = Spring1D::new(0.14, 2.8);
            s.open_elapsed = 0.0;
            s.close_progress = 0.0;
            s.close_by_deactivate_at = None;
            s.shell_focus_reclaimed = false;
            s.shown_at = Some(Instant::now());

            let _ = s.render();
        }

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                win_x,
                win_y,
                win_w,
                win_h,
                SWP_SHOWWINDOW,
            );
            if GetForegroundWindow() != self.hwnd {
                let _ = SetForegroundWindow(host_hwnd);
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
        if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.last_flow_tick = Instant::now();
            s.last_color_tick = s.last_flow_tick;
            s.sync_timer_rate();
        }
    }
}

impl Drop for TrayMenu {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_MENU_FLOW_ID);
            let raw =
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0) as *const RefCell<TrayMenuInner>;
            if !raw.is_null() {
                drop(Rc::from_raw(raw));
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl TrayMenuInner {
    pub(super) fn current_layout(&self) -> TrayMenuLayout {
        TrayMenuLayout::compute(self.dpi, self.sub_open_left, self.open_down)
    }

    pub(super) fn reset_transient_state(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_MENU_FLOW_ID);
        }
        self.timer_interval_ms = 0;
        self.backdrop_bmp = None;
        self.halo_colors_ready = false;
        self.last_silk_tick = None;
        self.anim_phase = PopupAnimPhase::Open;
        self.open_spring = Spring1D::still(1.0);
        self.open_elapsed = 1.0;
        self.close_progress = 0.0;
        self.shown_at = None;
        self.shell_focus_reclaimed = false;
        self.active_submenu = None;
        self.sub_width_spring = Spring1D::still(0.0);
        self.sub_fade_elapsed = 0.0;
        self.hover = None;
    }

    fn submenu_animating(&self) -> bool {
        let Some(kind) = self.active_submenu else {
            return false;
        };
        let target_w = self.current_layout().sub_rect_for(kind).width();
        let s = (self.dpi.max(96) as f32) / 96.0;
        !self.sub_width_spring.is_settled(target_w, 0.35 * s, 4.0 * s)
            || self.sub_fade_elapsed < 0.14
    }

    pub(super) fn sync_timer_rate(&mut self) {
        let desired = if matches!(
            self.anim_phase,
            PopupAnimPhase::Opening | PopupAnimPhase::Closing
        ) || self.submenu_animating()
        {
            POPUP_ANIM_INTERVAL_MS
        } else {
            let recipe = popup_glass_recipe(
                GlassSurfaceStyle::from(self.taskbar_style),
                self.light_glass,
                0.5,
            );
            if recipe.silk_alpha > 0.002 {
                POPUP_FLOW_INTERVAL_MS
            } else {
                0
            }
        };
        if desired == POPUP_ANIM_INTERVAL_MS {
            crate::ui::notify_interactive_anim_frame();
        }
        if self.timer_interval_ms != desired {
            unsafe {
                if desired == 0 {
                    let _ = KillTimer(self.hwnd, TIMER_MENU_FLOW_ID);
                } else {
                    if self.timer_interval_ms == 0 {
                        self.last_flow_tick = Instant::now();
                    }
                    SetTimer(self.hwnd, TIMER_MENU_FLOW_ID, desired, None);
                }
            }
            self.timer_interval_ms = desired;
        }
    }

    pub(super) fn open_submenu(&mut self, kind: TraySubmenuKind) -> bool {
        if self.active_submenu == Some(kind) || self.anim_phase == PopupAnimPhase::Closing {
            return false;
        }
        let s = (self.dpi.max(96) as f32) / 96.0;
        if self.active_submenu.is_none() {
            self.sub_width_spring = Spring1D::new(0.0, 260.0 * s);
        }
        self.active_submenu = Some(kind);
        self.sub_fade_elapsed = 0.0;
        self.sync_timer_rate();
        true
    }

    pub(super) fn close_submenu(&mut self) -> bool {
        if self.active_submenu.is_none() {
            return false;
        }
        self.active_submenu = None;
        self.sub_width_spring = Spring1D::still(0.0);
        self.sub_fade_elapsed = 0.0;
        self.sync_timer_rate();
        true
    }

    pub(super) fn begin_close_from_deactivate(&mut self) {
        if self.anim_phase == PopupAnimPhase::Closing
            || !unsafe { IsWindowVisible(self.hwnd).as_bool() }
        {
            return;
        }
        self.close_by_deactivate_at = Some(Instant::now());
        self.begin_close();
    }

    pub(super) fn begin_close(&mut self) {
        if self.anim_phase == PopupAnimPhase::Closing
            || !unsafe { IsWindowVisible(self.hwnd).as_bool() }
        {
            return;
        }
        self.hover = None;
        self.anim_phase = PopupAnimPhase::Closing;
        self.close_progress = 0.0;
        self.last_flow_tick = Instant::now();
        self.sync_timer_rate();
        let _ = self.render();
    }

    pub(super) fn advance_animations(&mut self, dt: f32) -> bool {
        if let Some(kind) = self.active_submenu {
            let target_w = self.current_layout().sub_rect_for(kind).width();
            self.sub_fade_elapsed += dt;
            self.sub_width_spring
                .step(target_w, SUB_WIDTH_STIFFNESS, SUB_WIDTH_DAMPING, dt);
        }

        match self.anim_phase {
            PopupAnimPhase::Opening => {
                self.open_elapsed += dt;
                self.open_spring
                    .step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, dt);
                if (self.open_spring.is_settled(1.0, 0.0015, 0.015) && self.open_elapsed >= 0.145)
                    || self.open_elapsed >= 0.38
                {
                    self.open_spring = Spring1D::still(1.0);
                    self.anim_phase = PopupAnimPhase::Open;
                }
                false
            }
            PopupAnimPhase::Open => false,
            PopupAnimPhase::Closing => {
                self.close_progress = (self.close_progress + dt / CLOSE_ANIM_DUR_S).min(1.0);
                self.close_progress >= 1.0
            }
        }
    }

    pub(super) fn ensure_title_format(&mut self, s: f32) -> windows::core::Result<IDWriteTextFormat> {
        if let Some((dpi, ref fmt)) = self.cached_fmt {
            if dpi == self.dpi {
                return Ok(fmt.clone());
            }
        }
        let fmt = create_text_format(&self.dwrite_factory, 13.0 * s, false, false, true)?;
        self.cached_fmt = Some((self.dpi, fmt.clone()));
        Ok(fmt)
    }

    pub(super) fn update_silk(&mut self, primary_rect: RectF, s: f32, draw_silk: bool) {
        let now = Instant::now();
        let dt_color = now
            .duration_since(self.last_color_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_color_tick = now;

        let (ui_rgb, base_dark) = resolve_unified_accent(self.backdrop_rgb, self.light_glass);
        let pair = compute_halo_pair(base_dark, self.light_glass);
        let target = [pair[0].to_normalized(), pair[1].to_normalized()];
        let ui_target = ui_rgb.to_normalized();

        let was_ready = self.halo_colors_ready;
        if !was_ready {
            self.halo_colors = target;
            self.ui_accent = ui_target;
            self.halo_colors_ready = true;
        } else {
            let k = (dt_color * 4.5).clamp(0.02, 1.0);
            for (cur, tgt) in self.halo_colors.iter_mut().zip(target.iter()) {
                cur.0 += (tgt.0 - cur.0) * k;
                cur.1 += (tgt.1 - cur.1) * k;
                cur.2 += (tgt.2 - cur.2) * k;
            }
            let k_ui = (dt_color * 6.0).clamp(0.02, 1.0);
            self.ui_accent.0 += (ui_target.0 - self.ui_accent.0) * k_ui;
            self.ui_accent.1 += (ui_target.1 - self.ui_accent.1) * k_ui;
            self.ui_accent.2 += (ui_target.2 - self.ui_accent.2) * k_ui;
        }

        if draw_silk {
            update_silk_bitmap(
                &self.rt,
                &mut self.silk_bmp,
                &mut self.silk_buf,
                &mut self.last_silk_tick,
                !was_ready,
                self.win_w,
                self.win_h,
                primary_rect.left,
                primary_rect.bottom,
                s,
                self.flow_phase,
                &self.halo_colors,
                self.light_glass,
            );
        }
    }

    pub(super) fn current_animated_sub_rect(&self, layout: &TrayMenuLayout, anim_primary: RectF, s: f32) -> Option<RectF> {
        let kind = self.active_submenu?;
        let target_sub = layout.sub_rect_for(kind);
        let v_sub = animated_tray_card_rect(
            target_sub,
            self.win_h as f32,
            self.anim_phase,
            self.open_spring.pos,
            self.close_progress,
            self.open_down,
            s,
        );
        let retract = if self.anim_phase == PopupAnimPhase::Closing {
            1.0 - 0.16 * ease_retract(self.close_progress)
        } else {
            1.0
        };
        let cur_w = (self.sub_width_spring.pos * retract)
            .clamp(24.0 * s, target_sub.width() + 8.0 * s);
        let seam_x = if self.sub_open_left {
            anim_primary.left
        } else {
            anim_primary.right
        };
        let (left, right) = if self.sub_open_left {
            ((seam_x - cur_w).max(4.0 * s), seam_x)
        } else {
            (seam_x, (seam_x + cur_w).min((self.win_w as f32) - 4.0 * s))
        };
        Some(RectF::new(left, v_sub.top, right, v_sub.bottom))
    }

}
