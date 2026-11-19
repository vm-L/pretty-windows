//! MonitorBar Win32 window host, taskbar synchronization, drag/click interaction, and lifecycle management.

use super::actions::{execute_left_click_non_menu, pick_custom_app_file};
use super::brightness::BrightnessHandle;
use super::config::{load_or_create, save_config, LeftClickAction, MonitorConfig};
use super::layout::MonitorLayout;
use super::metrics::MetricsHandle;
use super::popup::MonitorPopup;
use super::render::{MonitorRenderInput, MonitorRenderer};
use super::taskbar::{
    compute_clamped_offset, compute_monitor_bar_rect, ensure_above_taskbar, is_fullscreen_active,
    query_taskbar_state, TaskbarState,
};
use super::wireless::WirelessHandle;
use super::{
    WM_APP_MONITOR_BRIGHTNESS_UPDATED, WM_APP_MONITOR_CLOSE_SELF, WM_APP_MONITOR_CONFIG_CHANGED,
    WM_APP_MONITOR_METRICS_UPDATED, WM_APP_MONITOR_PICK_CUSTOM_APP, WM_APP_MONITOR_POPUP_CLOSED,
    WM_APP_MONITOR_RESET_POS, WM_APP_MONITOR_TOGGLE_LOCK, WM_APP_MONITOR_WIRELESS_UPDATED,
};
use crate::color::{Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::system::{exclude_from_peek, is_light_taskbar, resolve_unified_accent};
use crate::ui::POPUP_REOPEN_DEBOUNCE_MS;
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory1, D2D1_FACTORY_TYPE_SINGLE_THREADED,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, GetCursorPos, GetWindowLongPtrW,
    IsIconic, IsWindow, IsWindowVisible, KillTimer, LoadCursorW, PostMessageW, RegisterClassExW,
    RegisterWindowMessageW, SetCursor, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HCURSOR, HICON, HWND_TOPMOST, IDC_ARROW, MA_NOACTIVATE,
    SC_MINIMIZE, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNA, WM_CAPTURECHANGED, WM_CONTEXTMENU, WM_DESTROY,
    WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SETCURSOR, WM_SETTINGCHANGE, WM_SYSCOMMAND, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const BAR_CLASS_NAME: windows::core::PCWSTR = w!("PrettyWindows_MonitorBar");
const TIMER_SYNC_ID: usize = 1;
const TIMER_HOVER_ANIM_ID: usize = 2;
const TIMER_SYNC_MS: u32 = 300;
const TIMER_HOVER_MS: u32 = 16;
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const DRAG_THRESHOLD_PX: i32 = 4;

/// Lifecycle handle for the running MonitorBar widget owned by the tray host.
pub struct MonitorBarInstance {
    hwnd: HWND,
}

impl MonitorBarInstance {
    pub fn start(
        host_hwnd: HWND,
        close_msg: u32,
        config_path: PathBuf,
    ) -> windows::core::Result<Self> {
        unsafe {
            let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
            let arrow_cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(monitor_bar_wnd_proc),
                hInstance: hinst,
                hCursor: arrow_cursor,
                lpszClassName: BAR_CLASS_NAME,
                ..Default::default()
            };
            let _ = RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                BAR_CLASS_NAME,
                w!("PrettyWindows MonitorBar"),
                WS_POPUP,
                0,
                0,
                222,
                48,
                None,
                None,
                hinst,
                None,
            )?;
            exclude_from_peek(hwnd);

            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite_factory: IDWriteFactory =
                DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            let cfg_val = load_or_create(&config_path);
            let cfg = Arc::new(Mutex::new(cfg_val));

            let metrics = MetricsHandle::start(hwnd, WM_APP_MONITOR_METRICS_UPDATED);
            let wireless = WirelessHandle::start(hwnd, WM_APP_MONITOR_WIRELESS_UPDATED);
            let brightness = BrightnessHandle::start(hwnd, WM_APP_MONITOR_BRIGHTNESS_UPDATED);
            let renderer =
                MonitorRenderer::new_with_factories(d2d_factory.clone(), dwrite_factory.clone())?;
            let popup = MonitorPopup::new(
                hwnd,
                config_path.clone(),
                Arc::clone(&cfg),
                wireless.clone(),
                brightness.clone(),
                d2d_factory,
                dwrite_factory,
            )?;

            let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarCreated"));

            let light_taskbar = is_light_taskbar();
            let init_dpi = query_taskbar_state().map(|t| t.dpi).unwrap_or(96);
            let (drag_cursor, drag_cursor_owned) = renderer.create_grabbing_cursor(init_dpi)?;

            let mut state = Box::new(AppState {
                hwnd,
                host_hwnd,
                close_msg,
                taskbar_created_msg,
                config_path,
                cfg,
                metrics,
                wireless,
                brightness,
                renderer,
                popup,
                popup_closed_at: None,
                bar_x: 0,
                bar_y: 0,
                bar_w: 222,
                bar_h: 48,
                dpi: init_dpi,
                light_taskbar,
                accent: resolve_unified_accent(NEUTRAL_BACKDROP_FALLBACK, light_taskbar).0,
                hover_t: 0.0,
                mouse_inside: false,
                tracking_leave: false,
                hover_timer_active: false,
                lbutton_down: false,
                drag_pending: false,
                dragging: false,
                press_cursor_x: 0,
                press_offset_x: 0,
                hidden_by_guard: false,
                arrow_cursor,
                drag_cursor,
                drag_cursor_owned,
            });

            state.sync_taskbar_and_render(true);

            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                Box::into_raw(state) as isize,
            );
            crate::system::register_taskbar_overlay(hwnd);
            let _ = SetTimer(hwnd, TIMER_SYNC_ID, TIMER_SYNC_MS, None);

            Ok(Self { hwnd })
        }
    }

    pub fn shutdown(&mut self) {
        unsafe {
            if !self.hwnd.0.is_null() && IsWindow(self.hwnd).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
            self.hwnd = HWND::default();
        }
    }
}

impl Drop for MonitorBarInstance {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct AppState {
    hwnd: HWND,
    host_hwnd: HWND,
    close_msg: u32,
    taskbar_created_msg: u32,
    config_path: PathBuf,
    cfg: Arc<Mutex<MonitorConfig>>,
    metrics: MetricsHandle,
    wireless: WirelessHandle,
    brightness: BrightnessHandle,
    renderer: MonitorRenderer,
    popup: MonitorPopup,
    popup_closed_at: Option<Instant>,
    bar_x: i32,
    bar_y: i32,
    bar_w: i32,
    bar_h: i32,
    dpi: u32,
    light_taskbar: bool,
    accent: Rgb,
    hover_t: f32,
    mouse_inside: bool,
    tracking_leave: bool,
    hover_timer_active: bool,
    lbutton_down: bool,
    drag_pending: bool,
    dragging: bool,
    press_cursor_x: i32,
    press_offset_x: i32,
    hidden_by_guard: bool,
    arrow_cursor: HCURSOR,
    drag_cursor: HCURSOR,
    drag_cursor_owned: bool,
}

impl AppState {
    fn cfg_snapshot(&self) -> MonitorConfig {
        self.cfg.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn save_current_cfg(&self, cfg: &MonitorConfig) {
        save_config(&self.config_path, cfg);
    }

    fn desired_hover_target(&self) -> f32 {
        if self.mouse_inside || self.dragging || self.popup.is_open() {
            1.0
        } else {
            0.0
        }
    }

    fn ensure_hover_timer(&mut self) {
        let target = self.desired_hover_target();
        let need_timer = (self.hover_t - target).abs() > 0.005;
        if need_timer && !self.hover_timer_active {
            unsafe {
                let _ = SetTimer(self.hwnd, TIMER_HOVER_ANIM_ID, TIMER_HOVER_MS, None);
            }
            self.hover_timer_active = true;
        } else if !need_timer && self.hover_timer_active {
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_HOVER_ANIM_ID);
            }
            self.hover_timer_active = false;
        }
    }

    fn render_now(&mut self) {
        if self.hidden_by_guard {
            return;
        }
        let cfg = self.cfg_snapshot();
        let layout = MonitorLayout::compute(self.bar_h, self.dpi, &cfg);
        let snap = self.metrics.snapshot();
        let popup_open_span_x = self
            .popup
            .attached_bar_span_screen_x()
            .map(|(l, r)| (l - self.bar_x as f32, r - self.bar_x as f32));
        let input = MonitorRenderInput {
            layout: &layout,
            snapshot: snap,
            dpi: self.dpi,
            light_taskbar: self.light_taskbar,
            accent: self.accent,
            hover_t: self.hover_t,
            dragging: self.dragging,
            popup_open_span_x,
        };
        if self.renderer.render(&input).is_ok() {
            self.renderer
                .commit_to_window(self.hwnd, self.bar_x, self.bar_y);
        }
    }

    fn sync_taskbar_and_render(&mut self, force_render: bool) {
        let state_opt: Option<TaskbarState> = query_taskbar_state();
        let should_hide = match state_opt {
            None => true,
            Some(ref tb) => tb.auto_hide_hidden || is_fullscreen_active(tb.taskbar_hwnd),
        };

        if should_hide {
            if !self.hidden_by_guard {
                self.hidden_by_guard = true;
                self.popup.hide_immediately();
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
            }
            return;
        }

        let tb = state_opt.expect("checked Some above");
        let cfg = self.cfg_snapshot();
        let width_dip = MonitorLayout::desired_width_dip(&cfg);
        let (x, y, w, h) = compute_monitor_bar_rect(&tb, width_dip, cfg.offset_x);

        let light_now = is_light_taskbar();
        let accent_now = resolve_unified_accent(NEUTRAL_BACKDROP_FALLBACK, light_now).0;
        let changed_geom = self.bar_x != x
            || self.bar_y != y
            || self.bar_w != w
            || self.bar_h != h
            || self.dpi != tb.dpi;
        let changed_theme = self.light_taskbar != light_now || self.accent != accent_now;
        let was_hidden = self.hidden_by_guard;

        self.bar_x = x;
        self.bar_y = y;
        self.bar_w = w;
        self.bar_h = h;
        self.dpi = tb.dpi;
        self.light_taskbar = light_now;
        self.accent = accent_now;
        self.hidden_by_guard = false;

        let os_hidden = unsafe {
            !IsWindowVisible(self.hwnd).as_bool() || IsIconic(self.hwnd).as_bool()
        };

        unsafe {
            if changed_geom || was_hidden || os_hidden {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    x,
                    y,
                    w,
                    h,
                    SWP_NOACTIVATE,
                );
            } else {
                ensure_above_taskbar(self.hwnd, tb.taskbar_hwnd);
            }
            if was_hidden || os_hidden {
                let _ = ShowWindow(self.hwnd, SW_SHOWNA);
            }
        }

        if force_render || was_hidden || os_hidden || changed_geom || changed_theme {
            self.render_now();
        }
        self.popup.ensure_topmost();
    }

    fn toggle_popup(&mut self, click_screen_x: i32) {
        if self.popup.is_open() {
            self.popup.hide_animated();
            return;
        }
        if self.popup.is_closing_recently() {
            return;
        }
        if let Some(closed_at) = self.popup_closed_at.take() {
            if closed_at.elapsed() < Duration::from_millis(POPUP_REOPEN_DEBOUNCE_MS) {
                return;
            }
        }
        self.hover_t = 1.0;
        let cfg = self.cfg_snapshot();
        let layout = MonitorLayout::compute(self.bar_h, self.dpi, &cfg);
        let pill_overlap_px = (layout.bg_rect.top + 0.5 * layout.scale).round() as i32;
        let anchor_rect = RECT {
            left: self.bar_x + layout.bg_rect.left.round() as i32,
            top: self.bar_y + pill_overlap_px,
            right: self.bar_x + layout.bg_rect.right.round() as i32,
            bottom: self.bar_y + self.bar_h,
        };
        self.popup.show(
            anchor_rect,
            click_screen_x,
            self.dpi,
            self.light_taskbar,
            self.accent,
            pill_overlap_px,
        );
        self.render_now();
        self.popup.ensure_topmost();
    }

    fn handle_left_click(&mut self, click_screen_x: i32) {
        let cfg = self.cfg_snapshot();
        match cfg.left_click {
            LeftClickAction::Menu => self.toggle_popup(click_screen_x),
            LeftClickAction::CustomApp => {
                if cfg.custom_app_path.trim().is_empty() {
                    unsafe {
                        let _ = PostMessageW(
                            self.hwnd,
                            WM_APP_MONITOR_PICK_CUSTOM_APP,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                } else {
                    let _ = execute_left_click_non_menu(cfg.left_click, &cfg.custom_app_path);
                }
            }
            other => {
                let _ = execute_left_click_non_menu(other, &cfg.custom_app_path);
            }
        }
    }
}

unsafe extern "system" fn monitor_bar_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *ptr;

    if state.taskbar_created_msg != 0 && msg == state.taskbar_created_msg {
        crate::system::refresh_taskbar_overlay_hooks();
        state.sync_taskbar_and_render(true);
        return LRESULT(0);
    }

    match msg {
        WM_SYSCOMMAND => {
            if (wparam.0 & 0xFFF0) as u32 == SC_MINIMIZE {
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_TIMER => {
            if wparam.0 == TIMER_SYNC_ID {
                state.sync_taskbar_and_render(false);
            } else if wparam.0 == TIMER_HOVER_ANIM_ID {
                let target = state.desired_hover_target();
                let step: f32 = if target > state.hover_t { 0.18 } else { -0.14 };
                state.hover_t = if (target - state.hover_t).abs() <= step.abs() {
                    target
                } else {
                    (state.hover_t + step).clamp(0.0, 1.0)
                };
                state.ensure_hover_timer();
                state.render_now();
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let cfg = state.cfg_snapshot();
            let movable = state.dragging || !cfg.locked;
            let _ = SetCursor(if movable {
                state.drag_cursor
            } else {
                state.arrow_cursor
            });
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
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
            if !state.mouse_inside {
                state.mouse_inside = true;
                state.ensure_hover_timer();
            }

            if state.drag_pending || state.dragging {
                let mut pt = POINT::default();
                if GetCursorPos(&mut pt).is_ok() {
                    let dx = pt.x - state.press_cursor_x;
                    if state.drag_pending && dx.abs() >= DRAG_THRESHOLD_PX {
                        state.drag_pending = false;
                        state.dragging = true;
                        let _ = SetCursor(state.drag_cursor);
                    }
                    if state.dragging {
                        if let Some(tb) = query_taskbar_state() {
                            let width_dip = {
                                let cfg = state.cfg_snapshot();
                                MonitorLayout::desired_width_dip(&cfg)
                            };
                            let clamped =
                                compute_clamped_offset(&tb, width_dip, state.press_offset_x + dx);
                            if let Ok(mut guard) = state.cfg.lock() {
                                if guard.offset_x != clamped {
                                    guard.offset_x = clamped;
                                }
                            }
                            state.sync_taskbar_and_render(true);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            state.tracking_leave = false;
            state.mouse_inside = false;
            state.ensure_hover_timer();
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let cfg = state.cfg_snapshot();
            let can_drag = !cfg.locked;
            state.lbutton_down = true;
            state.drag_pending = can_drag;
            state.dragging = false;
            state.press_cursor_x = pt.x;
            state.press_offset_x = cfg.offset_x;
            let _ = SetCapture(hwnd);
            if can_drag {
                let _ = SetCursor(state.drag_cursor);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let was_pressed = state.lbutton_down;
            let was_dragging = state.dragging;
            // Clear flags before ReleaseCapture so synchronous WM_CAPTURECHANGED cannot clobber was_pressed.
            state.lbutton_down = false;
            state.drag_pending = false;
            state.dragging = false;
            let _ = ReleaseCapture();

            let cfg = state.cfg_snapshot();
            if was_dragging {
                state.save_current_cfg(&cfg);
                state.ensure_hover_timer();
                state.render_now();
            } else if was_pressed && cfg.locked {
                let mut pt = POINT::default();
                let inside = GetCursorPos(&mut pt).is_ok()
                    && pt.x >= state.bar_x
                    && pt.x <= state.bar_x + state.bar_w
                    && pt.y >= state.bar_y
                    && pt.y <= state.bar_y + state.bar_h;
                if inside {
                    state.handle_left_click(pt.x);
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN | WM_CONTEXTMENU => LRESULT(0),
        WM_RBUTTONUP => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            state.toggle_popup(pt.x);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            state.lbutton_down = false;
            state.drag_pending = false;
            state.dragging = false;
            state.ensure_hover_timer();
            LRESULT(0)
        }
        WM_DPICHANGED | WM_SETTINGCHANGE => {
            state.sync_taskbar_and_render(true);
            if msg == WM_DPICHANGED {
                if let Ok((new_drag, new_owned)) = state.renderer.create_grabbing_cursor(state.dpi)
                {
                    if state.drag_cursor_owned && !state.drag_cursor.is_invalid() {
                        let _ = DestroyIcon(HICON(state.drag_cursor.0));
                    }
                    state.drag_cursor = new_drag;
                    state.drag_cursor_owned = new_owned;
                }
            }
            LRESULT(0)
        }
        WM_APP_MONITOR_METRICS_UPDATED => {
            state.render_now();
            LRESULT(0)
        }
        WM_APP_MONITOR_WIRELESS_UPDATED => {
            state.popup.update_wireless(state.wireless.snapshot());
            LRESULT(0)
        }
        WM_APP_MONITOR_BRIGHTNESS_UPDATED => {
            state.popup.update_brightness(state.brightness.snapshot());
            LRESULT(0)
        }
        WM_APP_MONITOR_POPUP_CLOSED => {
            state.popup_closed_at = Some(Instant::now());
            let mut pt = POINT::default();
            state.mouse_inside = GetCursorPos(&mut pt).is_ok()
                && pt.x >= state.bar_x
                && pt.x <= state.bar_x + state.bar_w
                && pt.y >= state.bar_y
                && pt.y <= state.bar_y + state.bar_h;
            state.ensure_hover_timer();
            state.render_now();
            LRESULT(0)
        }
        WM_APP_MONITOR_CONFIG_CHANGED => {
            state.sync_taskbar_and_render(true);
            LRESULT(0)
        }
        WM_APP_MONITOR_TOGGLE_LOCK => {
            let updated = if let Ok(mut guard) = state.cfg.lock() {
                guard.locked = !guard.locked;
                Some(guard.clone())
            } else {
                None
            };
            if let Some(cfg) = updated {
                state.save_current_cfg(&cfg);
            }
            state.render_now();
            LRESULT(0)
        }
        WM_APP_MONITOR_RESET_POS => {
            let updated = if let Ok(mut guard) = state.cfg.lock() {
                guard.offset_x = 0;
                Some(guard.clone())
            } else {
                None
            };
            if let Some(cfg) = updated {
                state.save_current_cfg(&cfg);
            }
            state.sync_taskbar_and_render(true);
            LRESULT(0)
        }
        WM_APP_MONITOR_PICK_CUSTOM_APP => {
            state.popup.hide_immediately();
            let current = state.cfg_snapshot().custom_app_path;
            if let Some(chosen) = pick_custom_app_file(hwnd, &current) {
                let updated = if let Ok(mut guard) = state.cfg.lock() {
                    guard.custom_app_path = chosen;
                    guard.left_click = LeftClickAction::CustomApp;
                    Some(guard.clone())
                } else {
                    None
                };
                if let Some(cfg) = updated {
                    state.save_current_cfg(&cfg);
                }
            }
            state.ensure_hover_timer();
            state.render_now();
            LRESULT(0)
        }
        WM_APP_MONITOR_CLOSE_SELF => {
            state.popup.hide_immediately();
            let _ = PostMessageW(state.host_hwnd, state.close_msg, WPARAM(0), LPARAM(0));
            LRESULT(0)
        }
        WM_DESTROY => {
            crate::system::unregister_taskbar_overlay(hwnd);
            let _ = KillTimer(hwnd, TIMER_SYNC_ID);
            let _ = KillTimer(hwnd, TIMER_HOVER_ANIM_ID);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let mut owned = Box::from_raw(ptr);
            owned.popup.hide_immediately();
            owned.metrics.stop();
            owned.wireless.stop();
            owned.brightness.stop();
            if owned.drag_cursor_owned && !owned.drag_cursor.is_invalid() {
                let _ = DestroyIcon(HICON(owned.drag_cursor.0));
            }
            drop(owned);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
