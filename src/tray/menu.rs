use super::layout::{
    adjust_tray_anchor_for_work_area, animated_tray_card_rect, compute_tray_cascading_placement,
    TrayMenuHover, TrayMenuItem, TrayMenuLayout,
};
use crate::color::{Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::system::{
    is_shell_or_flyout_window, resolve_unified_accent, visible_transient_overlay_rects,
};
use crate::taskbar::TaskbarStyle;
use crate::ui::glass::{
    capture_blurred_desktop_backdrop_excluding, create_round_cap_stroke, create_text_format,
    d2d_rect, danger_text_color, draw_divider, draw_geometry_rim, draw_glass_interior,
    draw_music_note_icon, draw_row_highlight, draw_text_in_rect, identity_matrix,
    primary_text_color, update_silk_bitmap, LayeredSurface, PopupNoiseCache,
};
use crate::ui::{
    compute_halo_pair, ease_out_cubic, ease_retract, popup_glass_recipe, rgb_luma, row_stagger_dy,
    window_master_alpha, GlassSurfaceStyle, PopupAnimPhase, RectF, Spring1D, CLOSE_ANIM_DUR_S,
    OPEN_SPRING_DAMPING, OPEN_SPRING_STIFFNESS, PANEL_RADIUS_DIP, POPUP_ANIM_INTERVAL_MS,
    POPUP_FLOW_INTERVAL_MS, SILK_H, SILK_W, SUB_WIDTH_DAMPING, SUB_WIDTH_STIFFNESS,
};
use std::mem::{size_of, ManuallyDrop};
use std::time::{Duration, Instant};
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1Layer,
    ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT,
    D2D1_ARC_SIZE_LARGE, D2D1_COMBINE_MODE_UNION, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2,
    D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_SWEEP_DIRECTION_CLOCKWISE,
};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, GetWindowLongPtrW,
    IsWindowVisible, KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SetCursor,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA,
    HCURSOR, HWND_TOPMOST, HTTRANSPARENT, IDC_ARROW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SW_HIDE, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub const WM_APP_TRAY_TOGGLE_MEDIABAR: u32 = WM_APP + 21;
pub const WM_APP_TRAY_TOGGLE_AUTOSTART: u32 = WM_APP + 22;
pub const WM_APP_TRAY_EXIT: u32 = WM_APP + 23;
pub const WM_APP_TRAY_MENU_CLOSED: u32 = WM_APP + 24;
pub const WM_APP_TRAY_SET_TASKBAR_STYLE: u32 = WM_APP + 26;
pub const WM_APP_TRAY_TOGGLE_MONITORBAR: u32 = WM_APP + 27;

const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const TRAY_MENU_CLASS_NAME: PCWSTR = w!("PrettyWindowsTrayMenuWnd");
const TIMER_MENU_FLOW_ID: usize = 1;
const TRAY_TOGGLE_DEBOUNCE_MS: u64 = 180;
const SHELL_DEACTIVATE_GRACE_MS: u64 = 80;

struct TrayMenuInner {
    hwnd: HWND,
    host_hwnd: HWND,
    d2d_factory: ID2D1Factory1,
    dwrite_factory: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    layer: ID2D1Layer,
    content_layer: ID2D1Layer,
    round_stroke: Option<ID2D1StrokeStyle>,
    dpi: u32,
    light_glass: bool,
    backdrop_rgb: Rgb,
    mediabar_visible: bool,
    monitorbar_visible: bool,
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
    submenu_open: bool,
    sub_width_spring: Spring1D,
    sub_fade_elapsed: f32,
    backdrop_bmp: Option<ID2D1Bitmap>,
    noise_cache: PopupNoiseCache,
    silk_bmp: Option<ID2D1Bitmap>,
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
    inner: Box<TrayMenuInner>,
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
            let layer = rt.CreateLayer(None)?;
            let content_layer = rt.CreateLayer(None)?;
            let surface = LayeredSurface::new()?;
            let round_stroke = create_round_cap_stroke(&d2d_factory);

            let mut inner = Box::new(TrayMenuInner {
                hwnd,
                host_hwnd,
                d2d_factory,
                dwrite_factory,
                rt,
                surface,
                layer,
                content_layer,
                round_stroke,
                dpi: 96,
                light_glass: false,
                backdrop_rgb: NEUTRAL_BACKDROP_FALLBACK,
                mediabar_visible: true,
                monitorbar_visible: true,
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
                submenu_open: false,
                sub_width_spring: Spring1D::still(0.0),
                sub_fade_elapsed: 0.0,
                backdrop_bmp: None,
                noise_cache: PopupNoiseCache::default(),
                silk_bmp: None,
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
            });

            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (&mut *inner as *mut TrayMenuInner) as isize,
            );

            Ok(Self { hwnd, inner })
        }
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn is_open(&self) -> bool {
        self.is_visible() && self.inner.anim_phase != PopupAnimPhase::Closing
    }

    pub fn is_closing_recently(&self) -> bool {
        self.inner
            .close_by_deactivate_at
            .is_some_and(|t| t.elapsed() < Duration::from_millis(TRAY_TOGGLE_DEBOUNCE_MS))
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.inner.reset_transient_state();
    }

    pub fn hide_animated(&mut self) {
        self.inner.begin_close_from_deactivate();
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
        taskbar_style: TaskbarStyle,
        autostart: bool,
    ) {
        self.inner.mediabar_visible = mediabar_visible;
        self.inner.monitorbar_visible = monitorbar_visible;
        self.inner.taskbar_style = taskbar_style;
        self.inner.autostart = autostart;
        if self.is_visible() {
            let _ = self.inner.render();
        }
    }

    pub fn set_system_theme(&mut self, light_glass: bool, _fallback_accent: Rgb) {
        let changed = self.inner.light_glass != light_glass;
        self.inner.light_glass = light_glass;
        if changed {
            self.inner.halo_colors_ready = false;
            self.inner.last_silk_tick = None;
            if self.is_visible() {
                let _ = self.inner.render();
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
        taskbar_style: TaskbarStyle,
        autostart: bool,
    ) {
        self.inner.dpi = dpi.max(96);
        self.inner.light_glass = light_glass;
        self.inner.backdrop_rgb = NEUTRAL_BACKDROP_FALLBACK;
        self.inner.mediabar_visible = mediabar_visible;
        self.inner.monitorbar_visible = monitorbar_visible;
        self.inner.taskbar_style = taskbar_style;
        self.inner.autostart = autostart;
        self.inner.hover = None;
        self.inner.submenu_open = false;
        self.inner.sub_width_spring = Spring1D::still(0.0);
        self.inner.sub_fade_elapsed = 0.0;

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
            self.inner.dpi,
        );

        let (win_x, win_y, sub_open_left, open_down) = compute_tray_cascading_placement(
            adj_x,
            adj_y,
            monitor.left,
            monitor.top,
            monitor.right,
            monitor.bottom,
            self.inner.dpi,
        );
        let layout = TrayMenuLayout::compute(self.inner.dpi, sub_open_left, open_down);

        self.inner.win_x = win_x;
        self.inner.win_y = win_y;
        self.inner.win_w = layout.win_w;
        self.inner.win_h = layout.win_h;
        self.inner.sub_open_left = sub_open_left;
        self.inner.open_down = open_down;

        let capture_rect = RECT {
            left: win_x,
            top: win_y,
            right: win_x + layout.win_w,
            bottom: win_y + layout.win_h,
        };
        // Collect visible transient shell flyouts BEFORE stealing foreground dismisses/cloaks them.
        let occluders = visible_transient_overlay_rects(capture_rect);
        unsafe {
            let _ = SetForegroundWindow(self.inner.host_hwnd);
        }

        let initial_recipe =
            popup_glass_recipe(GlassSurfaceStyle::from(taskbar_style), light_glass, 0.5);
        let (bmp, sampled_bg) = capture_blurred_desktop_backdrop_excluding(
            &self.inner.rt,
            win_x,
            win_y,
            layout.win_w,
            layout.win_h,
            initial_recipe.blur_radius_4x,
            &occluders,
        );
        self.inner.backdrop_bmp = bmp;
        if let Some(bg) = sampled_bg {
            self.inner.backdrop_rgb = bg;
        }
        self.inner.halo_colors_ready = false;
        self.inner.last_silk_tick = None;
        self.inner.last_color_tick = Instant::now();
        self.inner.anim_phase = PopupAnimPhase::Opening;
        self.inner.open_spring = Spring1D::new(0.14, 2.8);
        self.inner.open_elapsed = 0.0;
        self.inner.close_progress = 0.0;
        self.inner.close_by_deactivate_at = None;
        self.inner.shell_focus_reclaimed = false;
        self.inner.shown_at = Some(Instant::now());

        let _ = self.inner.render();

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                win_x,
                win_y,
                layout.win_w,
                layout.win_h,
                SWP_SHOWWINDOW,
            );
            if GetForegroundWindow() != self.hwnd {
                let _ = SetForegroundWindow(self.inner.host_hwnd);
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
        self.inner.last_flow_tick = Instant::now();
        self.inner.last_color_tick = self.inner.last_flow_tick;
        self.inner.sync_timer_rate();
    }
}

impl Drop for TrayMenu {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_MENU_FLOW_ID);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl TrayMenuInner {
    fn current_layout(&self) -> TrayMenuLayout {
        TrayMenuLayout::compute(self.dpi, self.sub_open_left, self.open_down)
    }

    fn reset_transient_state(&mut self) {
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
        self.submenu_open = false;
        self.sub_width_spring = Spring1D::still(0.0);
        self.sub_fade_elapsed = 0.0;
        self.hover = None;
    }

    fn submenu_animating(&self) -> bool {
        if !self.submenu_open {
            return false;
        }
        let target_w = self.current_layout().sub_rect.width();
        let s = (self.dpi.max(96) as f32) / 96.0;
        !self.sub_width_spring.is_settled(target_w, 0.35 * s, 4.0 * s)
            || self.sub_fade_elapsed < 0.14
    }

    fn sync_timer_rate(&mut self) {
        let desired = if matches!(
            self.anim_phase,
            PopupAnimPhase::Opening | PopupAnimPhase::Closing
        ) || self.submenu_animating()
        {
            POPUP_ANIM_INTERVAL_MS
        } else {
            POPUP_FLOW_INTERVAL_MS
        };
        if self.timer_interval_ms != desired {
            unsafe {
                SetTimer(self.hwnd, TIMER_MENU_FLOW_ID, desired, None);
            }
            self.timer_interval_ms = desired;
        }
    }

    fn open_submenu(&mut self) -> bool {
        if self.submenu_open || self.anim_phase == PopupAnimPhase::Closing {
            return false;
        }
        let s = (self.dpi.max(96) as f32) / 96.0;
        self.submenu_open = true;
        self.sub_width_spring = Spring1D::new(0.0, 260.0 * s);
        self.sub_fade_elapsed = 0.0;
        self.sync_timer_rate();
        true
    }

    fn close_submenu(&mut self) -> bool {
        if !self.submenu_open {
            return false;
        }
        self.submenu_open = false;
        self.sub_width_spring = Spring1D::still(0.0);
        self.sub_fade_elapsed = 0.0;
        self.sync_timer_rate();
        true
    }

    fn begin_close_from_deactivate(&mut self) {
        if self.anim_phase == PopupAnimPhase::Closing
            || !unsafe { IsWindowVisible(self.hwnd).as_bool() }
        {
            return;
        }
        self.close_by_deactivate_at = Some(Instant::now());
        self.begin_close();
    }

    fn begin_close(&mut self) {
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

    fn finish_close(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.reset_transient_state();
        unsafe {
            let _ = PostMessageW(self.host_hwnd, WM_APP_TRAY_MENU_CLOSED, WPARAM(0), LPARAM(0));
        }
    }

    fn advance_animations(&mut self, dt: f32) -> bool {
        if self.submenu_open {
            let target_w = self.current_layout().sub_rect.width();
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

    fn ensure_title_format(&mut self, s: f32) -> windows::core::Result<IDWriteTextFormat> {
        if let Some((dpi, ref fmt)) = self.cached_fmt {
            if dpi == self.dpi {
                return Ok(fmt.clone());
            }
        }
        let fmt = create_text_format(&self.dwrite_factory, 13.0 * s, false, false, true)?;
        self.cached_fmt = Some((self.dpi, fmt.clone()));
        Ok(fmt)
    }

    fn update_silk(&mut self, primary_rect: RectF, s: f32, draw_silk: bool) {
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

    fn current_animated_sub_rect(&self, layout: &TrayMenuLayout, anim_primary: RectF, s: f32) -> Option<RectF> {
        if !self.submenu_open {
            return None;
        }
        let target_sub = layout.sub_rect;
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

    fn build_fused_cards_geometry(
        &self,
        primary: RectF,
        sub_opt: Option<RectF>,
        eff_r: f32,
    ) -> windows::core::Result<ID2D1Geometry> {
        unsafe {
            let rr_pri = D2D1_ROUNDED_RECT {
                rect: d2d_rect(primary),
                radiusX: eff_r,
                radiusY: eff_r,
            };
            let g_pri: ID2D1Geometry = self
                .d2d_factory
                .CreateRoundedRectangleGeometry(&rr_pri)?
                .cast()?;

            let Some(sub) = sub_opt else {
                return Ok(g_pri);
            };

            let sub_r = eff_r.min((sub.width() * 0.45).max(3.0));
            let rr_sub = D2D1_ROUNDED_RECT {
                rect: d2d_rect(sub),
                radiusX: sub_r,
                radiusY: sub_r,
            };
            let g_sub: ID2D1Geometry = self
                .d2d_factory
                .CreateRoundedRectangleGeometry(&rr_sub)?
                .cast()?;

            let combined = self.d2d_factory.CreatePathGeometry()?;
            let sink = combined.Open()?;
            g_pri.CombineWithGeometry(&g_sub, D2D1_COMBINE_MODE_UNION, None, 0.25, &sink)?;
            sink.Close()?;
            let combined_geo: ID2D1Geometry = combined.cast()?;

            let seam_x = if self.sub_open_left {
                primary.left
            } else {
                primary.right
            };
            let overlap_top = primary.top.max(sub.top);
            let overlap_bot = primary.bottom.min(sub.bottom);
            if overlap_bot <= overlap_top + 1.0 {
                return Ok(combined_geo);
            }

            let left_lim = primary.left.min(sub.left);
            let right_lim = primary.right.max(sub.right);
            let bridge_half = eff_r.max(sub_r) + 1.0;
            let bridge_rect = RectF::new(
                (seam_x - bridge_half).max(left_lim),
                overlap_top,
                (seam_x + bridge_half).min(right_lim),
                overlap_bot,
            );
            let g_bridge: ID2D1Geometry = self
                .d2d_factory
                .CreateRectangleGeometry(&d2d_rect(bridge_rect))?
                .cast()?;
            let fused = self.d2d_factory.CreatePathGeometry()?;
            let sink2 = fused.Open()?;
            combined_geo.CombineWithGeometry(
                &g_bridge,
                D2D1_COMBINE_MODE_UNION,
                None,
                0.25,
                &sink2,
            )?;
            sink2.Close()?;
            fused.cast()
        }
    }

    unsafe fn draw_fused_shadows(
        &self,
        anim_primary: RectF,
        anim_sub_opt: Option<RectF>,
        eff_r: f32,
        s: f32,
    ) {
        let max_w = self.win_w as f32;
        let max_h = self.win_h as f32;
        let shadow_layers = if self.light_glass {
            [(7.5 * s, 0.022), (4.5 * s, 0.036), (2.0 * s, 0.058)]
        } else {
            [(7.5 * s, 0.035), (4.5 * s, 0.055), (2.0 * s, 0.09)]
        };

        for (expand, alpha) in shadow_layers {
            let side_exp = expand * 0.55;
            let top_exp = expand * 0.45;
            let bot_exp = expand * 0.65;

            let sh_pri = RectF::new(
                if self.sub_open_left && anim_sub_opt.is_some() {
                    anim_primary.left
                } else {
                    (anim_primary.left - side_exp).max(0.0)
                },
                (anim_primary.top - top_exp).max(0.0),
                if !self.sub_open_left && anim_sub_opt.is_some() {
                    anim_primary.right
                } else {
                    (anim_primary.right + side_exp).min(max_w)
                },
                (anim_primary.bottom + bot_exp).min(max_h),
            );

            let sh_sub = anim_sub_opt.map(|sr| {
                RectF::new(
                    if !self.sub_open_left {
                        sr.left
                    } else {
                        (sr.left - side_exp).max(0.0)
                    },
                    (sr.top - top_exp).max(0.0),
                    if self.sub_open_left {
                        sr.right
                    } else {
                        (sr.right + side_exp).min(max_w)
                    },
                    (sr.bottom + bot_exp).min(max_h),
                )
            });

            if let (Ok(geo), Ok(sh_brush)) = (
                self.build_fused_cards_geometry(sh_pri, sh_sub, eff_r + side_exp * 0.45),
                self.rt.CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: alpha,
                    },
                    None,
                ),
            ) {
                self.rt.FillGeometry(&geo, &sh_brush, None);
            }
        }
    }

    fn render(&mut self) -> windows::core::Result<()> {
        let layout = self.current_layout();
        let s = layout.scale;
        self.win_w = layout.win_w;
        self.win_h = layout.win_h;
        self.surface.ensure_size(self.win_w, self.win_h)?;
        let title_fmt = self.ensure_title_format(s)?;

        let recipe = popup_glass_recipe(
            GlassSurfaceStyle::from(self.taskbar_style),
            self.light_glass,
            rgb_luma(self.backdrop_rgb),
        );
        self.update_silk(layout.primary_rect, s, recipe.silk_alpha > 0.002);

        let accent = Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        );

        let anim_primary = animated_tray_card_rect(
            layout.primary_rect,
            self.win_h as f32,
            self.anim_phase,
            self.open_spring.pos,
            self.close_progress,
            self.open_down,
            s,
        );
        let anim_sub_opt = self.current_animated_sub_rect(&layout, anim_primary, s);
        let eff_r = (PANEL_RADIUS_DIP * s).min((anim_primary.height() * 0.45).max(4.0 * s));

        let union_bounds = match anim_sub_opt {
            Some(sr) => RectF::new(
                anim_primary.left.min(sr.left),
                anim_primary.top.min(sr.top),
                anim_primary.right.max(sr.right),
                anim_primary.bottom.max(sr.bottom),
            ),
            None => anim_primary,
        };

        let (content_alpha, close_dy) = match self.anim_phase {
            PopupAnimPhase::Opening => (
                ease_out_cubic(((self.open_elapsed - 0.008) / 0.080).clamp(0.0, 1.0)),
                0.0,
            ),
            PopupAnimPhase::Open => (1.0, 0.0),
            PopupAnimPhase::Closing => {
                let r = ease_retract(self.close_progress);
                ((1.0 - 0.30 * r).clamp(0.0, 1.0), 5.0 * s * r)
            }
        };

        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            // 1. 3-pass ambient shadows behind fused card silhouette
            self.draw_fused_shadows(anim_primary, anim_sub_opt, eff_r, s);

            // 2. Clipped glass card interior & menu rows
            if let Ok(clip_geo) = self.build_fused_cards_geometry(anim_primary, anim_sub_opt, eff_r) {
                let clip_geo_for_rim = clip_geo.clone();
                let outer_params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(union_bounds),
                    geometricMask: ManuallyDrop::new(Some(clip_geo)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&outer_params, &self.layer);

                draw_glass_interior(
                    &self.rt,
                    union_bounds,
                    self.win_w,
                    self.win_h,
                    self.backdrop_bmp.as_ref(),
                    self.silk_bmp.as_ref(),
                    self.light_glass,
                    &recipe,
                    &mut self.noise_cache,
                );

                // 2a. Primary card rows & divider
                if content_alpha > 0.005 {
                    let use_layer = self.anim_phase != PopupAnimPhase::Open;
                    if use_layer {
                        let content_params = D2D1_LAYER_PARAMETERS {
                            contentBounds: d2d_rect(anim_primary),
                            geometricMask: ManuallyDrop::new(None),
                            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                            maskTransform: identity_matrix(),
                            opacity: content_alpha,
                            opacityBrush: ManuallyDrop::new(None),
                            layerOptions: D2D1_LAYER_OPTIONS_NONE,
                        };
                        self.rt.PushLayer(&content_params, &self.content_layer);
                    }

                    let mut div_anchor_dy = 0.0f32;
                    for (idx, (item, mut rect)) in layout.primary_rows().into_iter().enumerate() {
                        let dy = match self.anim_phase {
                            PopupAnimPhase::Opening => {
                                if self.open_down {
                                    -row_stagger_dy(
                                        4usize.saturating_sub(idx),
                                        5,
                                        self.open_elapsed,
                                        self.open_spring.pos,
                                        s,
                                    )
                                } else {
                                    row_stagger_dy(
                                        idx,
                                        5,
                                        self.open_elapsed,
                                        self.open_spring.pos,
                                        s,
                                    )
                                }
                            }
                            PopupAnimPhase::Open => 0.0,
                            PopupAnimPhase::Closing => {
                                if self.open_down {
                                    -close_dy
                                } else {
                                    close_dy
                                }
                            }
                        };
                        if item == TrayMenuItem::TaskbarStyle {
                            div_anchor_dy = dy;
                        }
                        rect.top += dy;
                        rect.bottom += dy;

                        let hovered = self.hover == Some(TrayMenuHover::Primary(item));
                        let selected = match item {
                            TrayMenuItem::MediaBar => self.mediabar_visible,
                            TrayMenuItem::MonitorBar => self.monitorbar_visible,
                            TrayMenuItem::TaskbarStyle => self.submenu_open,
                            TrayMenuItem::Autostart => self.autostart,
                            TrayMenuItem::Exit => false,
                        };
                        draw_row_highlight(
                            &self.rt,
                            rect,
                            hovered,
                            selected && item != TrayMenuItem::TaskbarStyle,
                            accent,
                            s,
                            self.light_glass,
                        );
                        if item == TrayMenuItem::TaskbarStyle && selected && !hovered {
                            // Subtle neutral highlight while submenu is open without selecting toggle state
                            draw_row_highlight(
                                &self.rt,
                                rect,
                                true,
                                false,
                                accent,
                                s,
                                self.light_glass,
                            );
                        }

                        let mut fg = match item {
                            TrayMenuItem::Exit => danger_text_color(self.light_glass),
                            TrayMenuItem::MediaBar
                            | TrayMenuItem::MonitorBar
                            | TrayMenuItem::TaskbarStyle
                            | TrayMenuItem::Autostart => {
                                let mut c = primary_text_color(self.light_glass);
                                if item != TrayMenuItem::TaskbarStyle && !selected {
                                    c.a *= 0.90;
                                }
                                c
                            }
                        };
                        if hovered && item == TrayMenuItem::Exit {
                            fg.a = 1.0;
                        }

                        let icon_cx = rect.left + 19.0 * s;
                        let icon_cy = (rect.top + rect.bottom) * 0.5;
                        if let Ok(icon_brush) = self.rt.CreateSolidColorBrush(&fg, None) {
                            self.draw_primary_icon(item, icon_cx, icon_cy, s, &icon_brush);
                        }

                        let label = match item {
                            TrayMenuItem::MediaBar => "媒体控制",
                            TrayMenuItem::MonitorBar => "系统监控",
                            TrayMenuItem::TaskbarStyle => "整体风格",
                            TrayMenuItem::Autostart => "开机自启",
                            TrayMenuItem::Exit => "退出",
                        };
                        let right_pad = if item == TrayMenuItem::TaskbarStyle {
                            22.0 * s
                        } else {
                            10.0 * s
                        };
                        let label_rect = RectF::new(
                            rect.left + 34.0 * s,
                            rect.top,
                            rect.right - right_pad,
                            rect.bottom,
                        );
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            label,
                            label_rect,
                            &title_fmt,
                            fg,
                        );

                        if item == TrayMenuItem::TaskbarStyle {
                            let mut chev_col = fg;
                            chev_col.a *= if self.submenu_open || hovered { 0.95 } else { 0.65 };
                            if let Ok(cb) = self.rt.CreateSolidColorBrush(&chev_col, None) {
                                let mid_y = (rect.top + rect.bottom) * 0.5;
                                let tip_x = rect.right - 9.5 * s;
                                let back_x = tip_x - 3.8 * s;
                                let half_h = 3.6 * s;
                                let stroke = self.round_stroke.as_ref();
                                self.rt.DrawLine(
                                    D2D_POINT_2F {
                                        x: back_x,
                                        y: mid_y - half_h,
                                    },
                                    D2D_POINT_2F {
                                        x: tip_x,
                                        y: mid_y,
                                    },
                                    &cb,
                                    1.3 * s,
                                    stroke,
                                );
                                self.rt.DrawLine(
                                    D2D_POINT_2F {
                                        x: tip_x,
                                        y: mid_y,
                                    },
                                    D2D_POINT_2F {
                                        x: back_x,
                                        y: mid_y + half_h,
                                    },
                                    &cb,
                                    1.3 * s,
                                    stroke,
                                );
                            }
                        }
                    }

                    let mut div_upper = layout.taskbar_style_rect;
                    div_upper.top += div_anchor_dy;
                    div_upper.bottom += div_anchor_dy;
                    draw_divider(&self.rt, div_upper, s, self.light_glass);

                    if use_layer {
                        self.rt.PopLayer();
                    }
                }

                // 2b. Vertical feathered seam divider + submenu rows
                if let Some(anim_sub) = anim_sub_opt {
                    let seam_x = if self.sub_open_left {
                        anim_primary.left
                    } else {
                        anim_primary.right
                    };
                    let seam_top = anim_primary.top.max(anim_sub.top) + 8.0 * s;
                    let seam_bottom = anim_primary.bottom.min(anim_sub.bottom) - 8.0 * s;
                    let sub_alpha_in =
                        ease_out_cubic(((self.sub_fade_elapsed - 0.006) / 0.078).clamp(0.0, 1.0));
                    let sub_alpha = if self.anim_phase == PopupAnimPhase::Closing {
                        sub_alpha_in * content_alpha
                    } else {
                        sub_alpha_in
                    };
                    if seam_bottom > seam_top + 4.0 * s {
                        let (sr, sg, sb, base_a) = if self.light_glass {
                            (0.0, 0.0, 0.0, 0.075)
                        } else {
                            (1.0, 1.0, 1.0, 0.10)
                        };
                        let peak_a = base_a * sub_alpha.max(0.15);
                        let seam_stops = [
                            D2D1_GRADIENT_STOP {
                                position: 0.0,
                                color: D2D1_COLOR_F {
                                    r: sr,
                                    g: sg,
                                    b: sb,
                                    a: 0.0,
                                },
                            },
                            D2D1_GRADIENT_STOP {
                                position: 0.14,
                                color: D2D1_COLOR_F {
                                    r: sr,
                                    g: sg,
                                    b: sb,
                                    a: peak_a,
                                },
                            },
                            D2D1_GRADIENT_STOP {
                                position: 0.86,
                                color: D2D1_COLOR_F {
                                    r: sr,
                                    g: sg,
                                    b: sb,
                                    a: peak_a,
                                },
                            },
                            D2D1_GRADIENT_STOP {
                                position: 1.0,
                                color: D2D1_COLOR_F {
                                    r: sr,
                                    g: sg,
                                    b: sb,
                                    a: 0.0,
                                },
                            },
                        ];
                        if let Ok(sc) = self.rt.CreateGradientStopCollection(
                            &seam_stops,
                            D2D1_GAMMA_2_2,
                            D2D1_EXTEND_MODE_CLAMP,
                        ) {
                            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                                startPoint: D2D_POINT_2F {
                                    x: seam_x,
                                    y: seam_top,
                                },
                                endPoint: D2D_POINT_2F {
                                    x: seam_x,
                                    y: seam_bottom,
                                },
                            };
                            if let Ok(sb_brush) =
                                self.rt.CreateLinearGradientBrush(&props, None, &sc)
                            {
                                self.rt.DrawLine(
                                    D2D_POINT_2F {
                                        x: seam_x,
                                        y: seam_top,
                                    },
                                    D2D_POINT_2F {
                                        x: seam_x,
                                        y: seam_bottom,
                                    },
                                    &sb_brush,
                                    1.0 * s,
                                    None,
                                );
                            }
                        }
                    }

                    if sub_alpha > 0.005 {
                        let slide_t = ease_out_cubic((self.sub_fade_elapsed / 0.105).clamp(0.0, 1.0));
                        let slide_dx = if self.sub_open_left {
                            10.0 * s * (1.0 - slide_t)
                        } else {
                            -10.0 * s * (1.0 - slide_t)
                        };
                        let sub_layer_params = D2D1_LAYER_PARAMETERS {
                            contentBounds: d2d_rect(anim_sub),
                            geometricMask: ManuallyDrop::new(None),
                            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                            maskTransform: identity_matrix(),
                            opacity: sub_alpha,
                            opacityBrush: ManuallyDrop::new(None),
                            layerOptions: D2D1_LAYER_OPTIONS_NONE,
                        };
                        self.rt.PushLayer(&sub_layer_params, &self.content_layer);

                        let sub_dy = if self.anim_phase == PopupAnimPhase::Closing {
                            if self.open_down {
                                -close_dy
                            } else {
                                close_dy
                            }
                        } else {
                            0.0
                        };

                        for (style, mut rect) in layout.sub_rows() {
                            rect.left += slide_dx;
                            rect.right += slide_dx;
                            rect.top += sub_dy;
                            rect.bottom += sub_dy;

                            let hovered = self.hover == Some(TrayMenuHover::SubTaskbarStyle(style));
                            let selected = self.taskbar_style == style;
                            draw_row_highlight(
                                &self.rt,
                                rect,
                                hovered,
                                selected,
                                accent,
                                s,
                                self.light_glass,
                            );

                            let mut fg = primary_text_color(self.light_glass);
                            if !selected && !hovered {
                                fg.a *= 0.90;
                            }
                            let icon_cx = rect.left + 19.0 * s;
                            let icon_cy = (rect.top + rect.bottom) * 0.5;
                            if let Ok(icon_brush) = self.rt.CreateSolidColorBrush(&fg, None) {
                                self.draw_style_icon(style, icon_cx, icon_cy, s, &icon_brush);
                            }

                            let label_rect = RectF::new(
                                rect.left + 34.0 * s,
                                rect.top,
                                rect.right - 10.0 * s,
                                rect.bottom,
                            );
                            draw_text_in_rect(
                                &self.dwrite_factory,
                                &self.rt,
                                style.label(),
                                label_rect,
                                &title_fmt,
                                fg,
                            );
                        }

                        self.rt.PopLayer();
                    }
                }

                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(outer_params.geometricMask);

                // 3. Restrained specular glass rim highlight around unified contour
                draw_geometry_rim(
                    &self.rt,
                    &clip_geo_for_rim,
                    union_bounds.left,
                    union_bounds.top,
                    union_bounds.bottom,
                    s,
                    &recipe,
                );
            }

            self.rt.EndDraw(None, None)?;
        }

        let win_alpha =
            window_master_alpha(self.anim_phase, self.open_elapsed, self.close_progress);
        let alpha_u8 = (win_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        self.surface
            .commit_with_alpha(self.hwnd, self.win_x, self.win_y, alpha_u8);
        Ok(())
    }

    unsafe fn draw_primary_icon(
        &self,
        item: TrayMenuItem,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let stroke = self.round_stroke.as_ref();
        match item {
            TrayMenuItem::MediaBar => {
                draw_music_note_icon(&self.rt, cx, cy, s, brush);
            }
            TrayMenuItem::MonitorBar => {
                let pts = [
                    D2D_POINT_2F {
                        x: cx - 5.2 * s,
                        y: cy + 0.6 * s,
                    },
                    D2D_POINT_2F {
                        x: cx - 2.4 * s,
                        y: cy + 0.6 * s,
                    },
                    D2D_POINT_2F {
                        x: cx - 0.8 * s,
                        y: cy - 4.0 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 1.2 * s,
                        y: cy + 3.8 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 2.8 * s,
                        y: cy - 0.8 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 5.2 * s,
                        y: cy - 0.8 * s,
                    },
                ];
                for seg in pts.windows(2) {
                    self.rt.DrawLine(seg[0], seg[1], brush, 1.35 * s, stroke);
                }
            }
            TrayMenuItem::TaskbarStyle => {
                // Minimal window monitor silhouette + accented bottom taskbar strip
                let outer = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.2 * s,
                        top: cy - 4.4 * s,
                        right: cx + 5.2 * s,
                        bottom: cy + 4.4 * s,
                    },
                    radiusX: 1.8 * s,
                    radiusY: 1.8 * s,
                };
                self.rt.DrawRoundedRectangle(&outer, brush, 1.25 * s, stroke);
                let strip = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 3.6 * s,
                        top: cy + 1.4 * s,
                        right: cx + 3.6 * s,
                        bottom: cy + 3.1 * s,
                    },
                    radiusX: 0.85 * s,
                    radiusY: 0.85 * s,
                };
                self.rt.FillRoundedRectangle(&strip, brush);
            }
            TrayMenuItem::Autostart => {
                // Upward launch arrow above a minimal tray bracket
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx,
                        y: cy + 2.2 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 4.7 * s,
                    },
                    brush,
                    1.35 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 3.0 * s,
                        y: cy - 1.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 4.8 * s,
                    },
                    brush,
                    1.35 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx + 3.0 * s,
                        y: cy - 1.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 4.8 * s,
                    },
                    brush,
                    1.35 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 4.4 * s,
                        y: cy + 2.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx - 4.4 * s,
                        y: cy + 4.7 * s,
                    },
                    brush,
                    1.3 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 4.4 * s,
                        y: cy + 4.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 4.4 * s,
                        y: cy + 4.7 * s,
                    },
                    brush,
                    1.3 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx + 4.4 * s,
                        y: cy + 4.7 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 4.4 * s,
                        y: cy + 2.7 * s,
                    },
                    brush,
                    1.3 * s,
                    stroke,
                );
            }
            TrayMenuItem::Exit => {
                // IEC power symbol: open top arc + vertical stem
                if let Ok(arc) = self.d2d_factory.CreatePathGeometry() {
                    if let Ok(sink) = arc.Open() {
                        sink.BeginFigure(
                            D2D_POINT_2F {
                                x: cx + 3.1 * s,
                                y: cy - 3.2 * s,
                            },
                            D2D1_FIGURE_BEGIN_HOLLOW,
                        );
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: D2D_POINT_2F {
                                x: cx - 3.1 * s,
                                y: cy - 3.2 * s,
                            },
                            size: D2D_SIZE_F {
                                width: 4.5 * s,
                                height: 4.5 * s,
                            },
                            rotationAngle: 0.0,
                            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                            arcSize: D2D1_ARC_SIZE_LARGE,
                        });
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        let _ = sink.Close();
                        self.rt.DrawGeometry(&arc, brush, 1.35 * s, stroke);
                    }
                }
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 5.2 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy - 0.3 * s,
                    },
                    brush,
                    1.35 * s,
                    stroke,
                );
            }
        }
    }

    unsafe fn draw_style_icon(
        &self,
        style: TaskbarStyle,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let stroke = self.round_stroke.as_ref();
        match style {
            TaskbarStyle::Default => {
                // Classic opaque taskbar bar inside rounded frame
                let frame = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.0 * s,
                        top: cy - 4.2 * s,
                        right: cx + 5.0 * s,
                        bottom: cy + 4.2 * s,
                    },
                    radiusX: 1.8 * s,
                    radiusY: 1.8 * s,
                };
                self.rt.DrawRoundedRectangle(&frame, brush, 1.25 * s, stroke);
                let fill = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 3.4 * s,
                        top: cy + 0.9 * s,
                        right: cx + 3.4 * s,
                        bottom: cy + 2.8 * s,
                    },
                    radiusX: 0.8 * s,
                    radiusY: 0.8 * s,
                };
                self.rt.FillRoundedRectangle(&fill, brush);
            }
            TaskbarStyle::Transparent => {
                // Crystal-clear hollow frame with light diagonal refraction glint
                let frame = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.0 * s,
                        top: cy - 4.2 * s,
                        right: cx + 5.0 * s,
                        bottom: cy + 4.2 * s,
                    },
                    radiusX: 2.0 * s,
                    radiusY: 2.0 * s,
                };
                self.rt.DrawRoundedRectangle(&frame, brush, 1.25 * s, stroke);
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 2.4 * s,
                        y: cy - 1.6 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 0.3 * s,
                        y: cy + 1.1 * s,
                    },
                    brush,
                    1.1 * s,
                    stroke,
                );
            }
            TaskbarStyle::Blur => {
                // Three softly staggered horizontal blur bands
                for (dy, half_w) in [(-3.0 * s, 4.4 * s), (0.0, 5.2 * s), (3.0 * s, 3.6 * s)] {
                    self.rt.DrawLine(
                        D2D_POINT_2F {
                            x: cx - half_w,
                            y: cy + dy,
                        },
                        D2D_POINT_2F {
                            x: cx + half_w,
                            y: cy + dy,
                        },
                        brush,
                        1.4 * s,
                        stroke,
                    );
                }
            }
            TaskbarStyle::Acrylic => {
                // Frosted rounded tile with micro 2x2 noise grain dots
                let frame = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 4.8 * s,
                        top: cy - 4.4 * s,
                        right: cx + 4.8 * s,
                        bottom: cy + 4.4 * s,
                    },
                    radiusX: 2.0 * s,
                    radiusY: 2.0 * s,
                };
                self.rt.DrawRoundedRectangle(&frame, brush, 1.2 * s, stroke);
                for (dx, dy) in [(-1.8, -1.6), (1.8, -1.6), (-1.8, 1.6), (1.8, 1.6)] {
                    self.rt.FillEllipse(
                        &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                            point: D2D_POINT_2F {
                                x: cx + dx * s,
                                y: cy + dy * s,
                            },
                            radiusX: 0.9 * s,
                            radiusY: 0.9 * s,
                        },
                        brush,
                    );
                }
            }
            TaskbarStyle::LiquidGlass => {
                // Signature 4-point Liquid Glass sparkle star + secondary droplet glint
                if let Ok(star_geo) = self.d2d_factory.CreatePathGeometry() {
                    if let Ok(sink) = star_geo.Open() {
                        let scx = cx - 0.6 * s;
                        let scy = cy + 0.4 * s;
                        let outer = 4.6 * s;
                        let inner = 1.45 * s;
                        sink.BeginFigure(
                            D2D_POINT_2F {
                                x: scx,
                                y: scy - outer,
                            },
                            D2D1_FIGURE_BEGIN_FILLED,
                        );
                        sink.AddLine(D2D_POINT_2F {
                            x: scx + inner,
                            y: scy - inner,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx + outer,
                            y: scy,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx + inner,
                            y: scy + inner,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx,
                            y: scy + outer,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx - inner,
                            y: scy + inner,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx - outer,
                            y: scy,
                        });
                        sink.AddLine(D2D_POINT_2F {
                            x: scx - inner,
                            y: scy - inner,
                        });
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        let _ = sink.Close();
                        self.rt.FillGeometry(&star_geo, brush, None);
                    }
                }
                self.rt.FillEllipse(
                    &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: cx + 4.1 * s,
                            y: cy - 3.6 * s,
                        },
                        radiusX: 1.05 * s,
                        radiusY: 1.05 * s,
                    },
                    brush,
                );
            }
        }
    }
}

unsafe extern "system" fn tray_menu_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayMenuInner;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *ptr;

    match msg {
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                let other_hwnd = HWND(lparam.0 as *mut core::ffi::c_void);
                let just_shown = state
                    .shown_at
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(SHELL_DEACTIVATE_GRACE_MS));
                if just_shown
                    && !state.shell_focus_reclaimed
                    && (other_hwnd.is_invalid() || is_shell_or_flyout_window(other_hwnd))
                {
                    state.shell_focus_reclaimed = true;
                    let _ = SetForegroundWindow(hwnd);
                } else {
                    state.begin_close_from_deactivate();
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                state.begin_close();
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_MENU_FLOW_ID && IsWindowVisible(hwnd).as_bool() {
                let now = Instant::now();
                let raw_dt = now.duration_since(state.last_flow_tick).as_secs_f32();
                state.last_flow_tick = now;
                let dt_flow = raw_dt.clamp(0.001, 0.1);
                let dt_anim = raw_dt.clamp(0.001, 0.028);
                state.flow_phase += f64::from(dt_flow);
                if state.advance_animations(dt_anim) {
                    state.finish_close();
                    return LRESULT(0);
                }
                state.sync_timer_rate();
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if state.anim_phase == PopupAnimPhase::Closing {
                return LRESULT(0);
            }
            if !state.tracking_leave {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                if TrackMouseEvent(&mut tme).is_ok() {
                    state.tracking_leave = true;
                }
            }
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let layout = state.current_layout();
            let next_hover = layout.hit_test(x, y, state.submenu_open);
            let sub_changed = match next_hover {
                Some(TrayMenuHover::Primary(TrayMenuItem::TaskbarStyle)) => state.open_submenu(),
                Some(TrayMenuHover::Primary(_)) => state.close_submenu(),
                Some(TrayMenuHover::SubTaskbarStyle(_)) | None => false,
            };
            if next_hover != state.hover || sub_changed {
                state.hover = next_hover;
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            state.tracking_leave = false;
            if state.anim_phase != PopupAnimPhase::Closing && state.hover.is_some() {
                state.hover = None;
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if state.anim_phase == PopupAnimPhase::Closing {
                return LRESULT(0);
            }
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let layout = state.current_layout();
            if !layout.visible_contains(x, y, state.submenu_open) {
                state.begin_close();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if state.anim_phase == PopupAnimPhase::Closing {
                return LRESULT(0);
            }
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let layout = state.current_layout();
            match layout.hit_test(x, y, state.submenu_open) {
                Some(TrayMenuHover::Primary(TrayMenuItem::MediaBar)) => {
                    state.mediabar_visible = !state.mediabar_visible;
                    let _ = PostMessageW(
                        state.host_hwnd,
                        WM_APP_TRAY_TOGGLE_MEDIABAR,
                        WPARAM(0),
                        LPARAM(0),
                    );
                    state.begin_close();
                }
                Some(TrayMenuHover::Primary(TrayMenuItem::MonitorBar)) => {
                    state.monitorbar_visible = !state.monitorbar_visible;
                    let _ = PostMessageW(
                        state.host_hwnd,
                        WM_APP_TRAY_TOGGLE_MONITORBAR,
                        WPARAM(0),
                        LPARAM(0),
                    );
                    state.begin_close();
                }
                Some(TrayMenuHover::Primary(TrayMenuItem::TaskbarStyle)) => {
                    if state.open_submenu() {
                        let _ = state.render();
                    }
                }
                Some(TrayMenuHover::SubTaskbarStyle(style)) => {
                    state.taskbar_style = style;
                    let _ = PostMessageW(
                        state.host_hwnd,
                        WM_APP_TRAY_SET_TASKBAR_STYLE,
                        WPARAM(style.to_u8() as usize),
                        LPARAM(0),
                    );
                    state.begin_close();
                }
                Some(TrayMenuHover::Primary(TrayMenuItem::Autostart)) => {
                    state.autostart = !state.autostart;
                    let _ = PostMessageW(
                        state.host_hwnd,
                        WM_APP_TRAY_TOGGLE_AUTOSTART,
                        WPARAM(0),
                        LPARAM(0),
                    );
                    state.begin_close();
                }
                Some(TrayMenuHover::Primary(TrayMenuItem::Exit)) => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    state.reset_transient_state();
                    let _ = PostMessageW(state.host_hwnd, WM_APP_TRAY_EXIT, WPARAM(0), LPARAM(0));
                }
                None => {
                    if !layout.visible_contains(x, y, state.submenu_open) {
                        state.begin_close();
                    }
                }
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == windows::Win32::UI::WindowsAndMessaging::HTCLIENT {
                SetCursor(state.arrow_cursor);
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCHITTEST => {
            if state.anim_phase == PopupAnimPhase::Closing {
                return LRESULT(HTTRANSPARENT as isize);
            }
            let mut pt = POINT {
                x: i32::from((lparam.0 & 0xFFFF) as i16),
                y: i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let layout = state.current_layout();
            if layout.visible_contains(pt.x as f32, pt.y as f32, state.submenu_open) {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
