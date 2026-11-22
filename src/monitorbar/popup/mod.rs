//! Liquid-fused cascading Liquid Glass context menu for MonitorBar.

mod icons;
mod layout;
mod render;
mod wndproc;

use wndproc::{
    begin_close_monitor_popup, hide_immediately_monitor_popup, monitor_popup_wnd_proc,
};

use super::brightness::{BrightnessHandle, BrightnessSnapshot};
use super::config::{LeftClickAction, MetricItem, MonitorConfig};
use super::wireless::{WirelessHandle, WirelessSnapshot};
use crate::color::{Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::system::{active_taskbar_surface_style, visible_transient_overlay_rects};
use crate::ui::glass::{capture_blurred_desktop_backdrop_excluding, create_dc_render_target, create_round_cap_stroke, LayeredSurface, PopupNoiseCache};
use crate::ui::{popup_glass_recipe, PopupAnimPhase, RectF, Spring1D, POPUP_FLOAT_GAP_DIP, POPUP_REOPEN_DEBOUNCE_MS, SHADOW_PAD_DIP, SILK_H, SILK_W};
use std::cell::RefCell;
use std::mem::size_of;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Layer, ID2D1StrokeStyle};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, IsWindowVisible, KillTimer, LoadCursorW, RegisterClassExW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HCURSOR, HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOW, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP};

const POPUP_CLASS_NAME: windows::core::PCWSTR = w!("PrettyWindows_MonitorPopup");
const TIMER_POPUP_FLOW_ID: usize = 1;
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;

const PRIMARY_W_DIP: f32 = 154.0;
const SUB_WIRELESS_W_DIP: f32 = 244.0;
const SUB_BRIGHTNESS_W_DIP: f32 = 256.0;
const SUB_CLICK_W_DIP: f32 = 206.0;
const SUB_DISPLAY_W_DIP: f32 = 152.0;
const SUB_SETTINGS_W_DIP: f32 = 172.0;
const MAX_LIST_ROWS: usize = 6;
const BRIGHTNESS_ROW_H_DIP: f32 = 46.0;
const MAX_BRIGHTNESS_ROWS: usize = 5;
const SUB_VERT_STIFFNESS: f32 = 860.0;
const SUB_VERT_DAMPING: f32 = 44.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimaryItem {
    WiFi,
    Bluetooth,
    Brightness,
    LeftClick,
    Display,
    Settings,
}

impl PrimaryItem {
    fn submenu(self) -> Option<SubmenuKind> {
        match self {
            PrimaryItem::WiFi => Some(SubmenuKind::WiFi),
            PrimaryItem::Bluetooth => Some(SubmenuKind::Bluetooth),
            PrimaryItem::Brightness => Some(SubmenuKind::Brightness),
            PrimaryItem::LeftClick => Some(SubmenuKind::LeftClick),
            PrimaryItem::Display => Some(SubmenuKind::Display),
            PrimaryItem::Settings => Some(SubmenuKind::Settings),
        }
    }

    fn label(self) -> &'static str {
        match self {
            PrimaryItem::WiFi => "WiFi",
            PrimaryItem::Bluetooth => "蓝牙",
            PrimaryItem::Brightness => "亮度",
            PrimaryItem::LeftClick => "左键",
            PrimaryItem::Display => "数据",
            PrimaryItem::Settings => "设置",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubmenuKind {
    WiFi,
    Bluetooth,
    Brightness,
    LeftClick,
    Display,
    Settings,
}

impl SubmenuKind {
    fn width_dip(self) -> f32 {
        match self {
            SubmenuKind::WiFi | SubmenuKind::Bluetooth => SUB_WIRELESS_W_DIP,
            SubmenuKind::Brightness => SUB_BRIGHTNESS_W_DIP,
            SubmenuKind::LeftClick => SUB_CLICK_W_DIP,
            SubmenuKind::Display => SUB_DISPLAY_W_DIP,
            SubmenuKind::Settings => SUB_SETTINGS_W_DIP,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubHoverTarget {
    WifiToggle,
    WifiNetwork(usize),
    WifiMoreSettings,
    BtToggle,
    BtDevice(usize),
    BtMoreSettings,
    BrightnessRow(usize),
    BrightnessSlider(usize),
    BrightnessMoreSettings,
    LeftClickPreset(LeftClickAction),
    DisplayMetric(MetricItem),
    SettingLock,
    SettingReset,
    SettingClose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HoverTarget {
    Primary(PrimaryItem),
    Sub(SubHoverTarget),
}

#[derive(Clone, Copy, Debug)]
struct BrightnessRowGeom {
    label_rect: RectF,
    pct_rect: RectF,
    track_rect: RectF,
    slider_hit_rect: RectF,
}

#[derive(Clone, Copy, Debug)]
struct SubSpringState {
    top: Spring1D,
    bottom: Spring1D,
    width: Spring1D,
    fade_elapsed: f32,
    is_budding: bool,
}

struct SubRowRect {
    target: Option<SubHoverTarget>,
    rect: RectF,
}

struct SubLayout {
    rows: Vec<SubRowRect>,
    dividers: Vec<RectF>,
    list_viewport: Option<RectF>,
    list_total_items: usize,
    list_scroll: usize,
    list_max_rows: usize,
}

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

/// Manages the MonitorBar liquid-fused cascading Liquid Glass popup window lifecycle.
pub struct MonitorPopup {
    hwnd: HWND,
    inner: Rc<RefCell<MonitorPopupInner>>,
}

struct MonitorPopupInner {
    hwnd: HWND,
    owner_hwnd: HWND,
    config_path: PathBuf,
    cfg: Arc<Mutex<MonitorConfig>>,
    wireless: WirelessHandle,
    wireless_snap: WirelessSnapshot,
    brightness: BrightnessHandle,
    brightness_snap: BrightnessSnapshot,
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
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
    open_left: bool,
    light_glass: bool,
    backdrop_rgb: Rgb,
    ui_accent: (f32, f32, f32),
    halo_colors: [(f32, f32, f32); 2],
    halo_colors_ready: bool,
    silk_buf: Vec<u8>,
    last_silk_tick: Option<Instant>,
    last_color_tick: Instant,
    last_flow_tick: Instant,
    flow_phase: f64,
    anim_phase: PopupAnimPhase,
    open_spring: Spring1D,
    open_elapsed: f32,
    close_progress: f32,
    close_started_at: Option<Instant>,
    active_sub: Option<SubmenuKind>,
    sub_anim: Option<SubSpringState>,
    wifi_scroll: usize,
    bt_scroll: usize,
    brightness_scroll: usize,
    wheel_accum: i32,
    dragging_brightness: Option<String>,
    hover: Option<HoverTarget>,
    tracking_leave: bool,
    timer_interval_ms: u32,
    fmt_key: u32,
    title_fmt: Option<IDWriteTextFormat>,
    badge_fmt: Option<IDWriteTextFormat>,
    arrow_cursor: HCURSOR,
}

impl MonitorPopup {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner_hwnd: HWND,
        config_path: PathBuf,
        cfg: Arc<Mutex<MonitorConfig>>,
        wireless: WirelessHandle,
        brightness: BrightnessHandle,
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
    ) -> windows::core::Result<Self> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let arrow_cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(monitor_popup_wnd_proc),
                hInstance: hinst.into(),
                hCursor: arrow_cursor,
                lpszClassName: POPUP_CLASS_NAME,
                ..Default::default()
            };
            let _ = RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                POPUP_CLASS_NAME,
                w!("MonitorBar Menu"),
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
            let round_stroke = create_round_cap_stroke(&d2d_factory);
            let surface = LayeredSurface::new()?;
            let wireless_snap = wireless.snapshot();
            let brightness_snap = brightness.snapshot();

            let inner = Rc::new(RefCell::new(MonitorPopupInner {
                hwnd,
                owner_hwnd,
                config_path,
                cfg,
                wireless,
                wireless_snap,
                brightness,
                brightness_snap,
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
                win_x: 0,
                win_y: 0,
                win_w: 1,
                win_h: 1,
                open_left: true,
                light_glass: false,
                backdrop_rgb: NEUTRAL_BACKDROP_FALLBACK,
                ui_accent: (0.76, 0.76, 0.78),
                halo_colors: [(0.76, 0.76, 0.78); 2],
                halo_colors_ready: false,
                silk_buf: vec![0u8; (SILK_W * SILK_H * 4) as usize],
                last_silk_tick: None,
                last_color_tick: Instant::now(),
                last_flow_tick: Instant::now(),
                flow_phase: 0.0,
                anim_phase: PopupAnimPhase::Open,
                open_spring: Spring1D::still(1.0),
                open_elapsed: 1.0,
                close_progress: 0.0,
                close_started_at: None,
                active_sub: None,
                sub_anim: None,
                wifi_scroll: 0,
                bt_scroll: 0,
                brightness_scroll: 0,
                wheel_accum: 0,
                dragging_brightness: None,
                hover: None,
                tracking_leave: false,
                timer_interval_ms: 0,
                fmt_key: 0,
                title_fmt: None,
                badge_fmt: None,
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
            (self.is_visible() && s.anim_phase == PopupAnimPhase::Closing)
                || s.close_started_at.is_some_and(|t| {
                    t.elapsed() < Duration::from_millis(POPUP_REOPEN_DEBOUNCE_MS)
                })
        })
    }

    pub fn hide_animated(&mut self) {
        unsafe {
            begin_close_monitor_popup(self.hwnd, &self.inner);
        }
    }

    pub fn hide_immediately(&mut self) {
        unsafe {
            let _ = hide_immediately_monitor_popup(self.hwnd, &self.inner);
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

    pub fn update_wireless(&mut self, snapshot: WirelessSnapshot) {
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.wireless_snap = snapshot;
            s.clamp_scrolls();
            s.sync_timer_rate();
            self.is_visible()
        } else {
            false
        };
        if should_render {
            if let Ok(mut s) = self.inner.try_borrow_mut() {
                let _ = s.render();
            }
        }
    }

    pub fn update_brightness(&mut self, mut snapshot: BrightnessSnapshot) {
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            if let Some(ref drag_id) = s.dragging_brightness {
                if let Some(prev) = s.brightness_snap.displays.iter().find(|d| d.id == *drag_id) {
                    if let Some(cur) = snapshot.displays.iter_mut().find(|d| d.id == *drag_id) {
                        cur.brightness = prev.brightness;
                    }
                }
            }
            s.brightness_snap = snapshot;
            s.clamp_scrolls();
            s.sync_timer_rate();
            self.is_visible()
        } else {
            false
        };
        if should_render {
            if let Ok(mut s) = self.inner.try_borrow_mut() {
                let _ = s.render();
            }
        }
    }

    pub fn show(
        &mut self,
        anchor_rect: RECT,
        click_screen_x: i32,
        dpi: u32,
        light_glass: bool,
        _accent: Rgb,
    ) {
        let Some((win_x, win_y, win_w, win_h)) = (|| {
            let mut s = self.inner.try_borrow_mut().ok()?;
            s.dpi = dpi.max(96);
            s.light_glass = light_glass;
            s.backdrop_rgb = NEUTRAL_BACKDROP_FALLBACK;
            s.hover = None;
            s.tracking_leave = false;
            s.active_sub = None;
            s.sub_anim = None;
            s.wifi_scroll = 0;
            s.bt_scroll = 0;
            s.brightness_scroll = 0;
            s.wheel_accum = 0;
            s.dragging_brightness = None;
            s.win_h = 0;
            s.wireless_snap = s.wireless.snapshot();
            s.wireless.request_refresh(false);
            s.brightness_snap = s.brightness.snapshot();
            s.brightness.request_refresh();

            let sc = s.scale();
            let side_pad = (SHADOW_PAD_DIP * sc).round() as i32;
            let float_gap = (POPUP_FLOAT_GAP_DIP * sc).round() as i32;
            let primary_w = (PRIMARY_W_DIP * sc).round() as i32;
            let max_sub_w = (SUB_WIRELESS_W_DIP.max(SUB_BRIGHTNESS_W_DIP) * sc).round() as i32;
            let max_reach = s.max_popup_reach_px();

            let mon = unsafe { MonitorFromRect(&anchor_rect, MONITOR_DEFAULTTONEAREST) };
            let mut mi = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let work = unsafe {
                if GetMonitorInfoW(mon, &mut mi).as_bool() {
                    mi.rcWork
                } else {
                    RECT {
                        left: 0,
                        top: 0,
                        right: 1920,
                        bottom: 1040,
                    }
                }
            };

            let edge_margin = (6.0 * sc).round() as i32;
            let top_margin = (6.0 * sc).round() as i32;
            let desired_card_bottom = anchor_rect.top - float_gap;
            let win_w = side_pad * 2 + primary_w + max_sub_w;
            let win_h = (side_pad * 2 + max_reach)
                .min((desired_card_bottom + side_pad - work.top - top_margin).max(180));

            // Center/align primary card cleanly above MonitorBar without overhanging screen edges.
            let pill_inset = (6.0 * sc).round() as i32;
            let work_min = work.left + edge_margin;
            let work_max = (work.right - primary_w - edge_margin).max(work_min);
            let ideal_primary_left =
                if (anchor_rect.right - anchor_rect.left) >= primary_w + pill_inset * 2 {
                    let bar_min = anchor_rect.left + pill_inset;
                    let bar_max = (anchor_rect.right - pill_inset - primary_w).max(bar_min);
                    let min_x = bar_min.max(work_min).min(work_max);
                    let max_x = bar_max.min(work_max).max(min_x);
                    (click_screen_x - (24.0 * sc).round() as i32).clamp(min_x, max_x)
                } else {
                    let right_aligned = anchor_rect.right - primary_w;
                    right_aligned.clamp(work_min, work_max)
                };

            let open_left =
                ideal_primary_left + primary_w + max_sub_w + edge_margin > work.right
                    && (ideal_primary_left - work.left)
                        > (work.right - (ideal_primary_left + primary_w));
            s.open_left = open_left;

            let min_win_x = work.left + edge_margin - side_pad;
            let max_win_x = (work.right - edge_margin + side_pad - win_w).max(min_win_x);
            let win_x = if open_left {
                let right_edge = ideal_primary_left + primary_w + side_pad;
                (right_edge - win_w).clamp(min_win_x, max_win_x)
            } else {
                (ideal_primary_left - side_pad).clamp(min_win_x, max_win_x)
            };

            // Float the closed menu card `POPUP_FLOAT_GAP_DIP` cleanly above the taskbar top edge.
            let win_y = (desired_card_bottom + side_pad - win_h).max(work.top);

            s.win_x = win_x;
            s.win_y = win_y;
            s.win_w = win_w;
            s.win_h = win_h;

            let capture_rect = RECT {
                left: win_x,
                top: win_y,
                right: win_x + win_w,
                bottom: win_y + win_h,
            };
            let occluders = visible_transient_overlay_rects(capture_rect);
            let initial_recipe =
                popup_glass_recipe(active_taskbar_surface_style(), light_glass, 0.5);
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
            s.close_started_at = None;

            let _ = s.render();
            Some((win_x, win_y, win_w, win_h))
        })() else {
            return;
        };

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                win_x,
                win_y,
                win_w,
                win_h,
                SWP_NOACTIVATE,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
        if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.last_flow_tick = Instant::now();
            s.last_color_tick = s.last_flow_tick;
            s.sync_timer_rate();
        }
    }
}

impl Drop for MonitorPopup {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
            let raw =
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0) as *const RefCell<MonitorPopupInner>;
            if !raw.is_null() {
                drop(Rc::from_raw(raw));
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

