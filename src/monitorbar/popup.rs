//! Liquid-fused cascading Liquid Glass context menu for MonitorBar.

use super::actions::open_shell_target;
use super::brightness::{
    format_brightness_pct, BrightnessHandle, BrightnessMode, BrightnessSnapshot,
};
use super::config::{save_config, LeftClickAction, MetricItem, MonitorConfig};
use super::wireless::{BtDeviceKind, WifiNetworkItem, WirelessHandle, WirelessSnapshot};
use super::{
    WM_APP_MONITOR_CLOSE_SELF, WM_APP_MONITOR_CONFIG_CHANGED, WM_APP_MONITOR_PICK_CUSTOM_APP,
    WM_APP_MONITOR_POPUP_CLOSED, WM_APP_MONITOR_RESET_POS, WM_APP_MONITOR_TOGGLE_LOCK,
};
use crate::color::{Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::system::{
    active_taskbar_surface_style, resolve_unified_accent, visible_transient_overlay_rects,
};
use crate::ui::glass::{
    capture_blurred_desktop_backdrop_excluding, create_round_cap_stroke, create_text_format,
    d2d_color_rgb, d2d_rect, danger_text_color, draw_divider, draw_geometry_rim,
    draw_glass_interior, draw_glass_slider, draw_pill_switch, draw_row_highlight,
    draw_settings_icon, draw_text_in_rect, draw_text_in_rect_measured, identity_matrix,
    muted_text_color, primary_text_color, update_silk_bitmap, LayeredSurface, PopupNoiseCache,
};
use crate::ui::{
    compute_halo_pair, ease_out_cubic, ease_retract, popup_glass_recipe, rgb_luma, row_stagger_dy,
    window_master_alpha, PopupAnimPhase, RectF, Spring1D, CLOSE_ANIM_DUR_S, DIVIDER_H_DIP,
    INNER_PAD_DIP, OPEN_SPRING_DAMPING, OPEN_SPRING_STIFFNESS, PANEL_RADIUS_DIP,
    POPUP_ANIM_INTERVAL_MS, POPUP_FLOAT_GAP_DIP, POPUP_FLOW_INTERVAL_MS, POPUP_REOPEN_DEBOUNCE_MS,
    ROW_GAP_DIP, ROW_H_DIP, SHADOW_PAD_DIP, SILK_H, SILK_W, SUB_WIDTH_DAMPING,
    SUB_WIDTH_STIFFNESS,
};
use std::mem::{size_of, ManuallyDrop};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{w, Interface};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1GeometrySink, ID2D1Layer,
    ID2D1PathGeometry, ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_ELLIPSE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromRect, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, IsWindowVisible, KillTimer,
    LoadCursorW, PostMessageW, RegisterClassExW, SetCursor, SetForegroundWindow, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA,
    HCURSOR, HWND_TOPMOST, HTTRANSPARENT, IDC_ARROW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_HIDE, SW_SHOW, WA_INACTIVE, WM_ACTIVATE, WM_CAPTURECHANGED, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

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
const STEP_MENISCUS_DIP: f32 = 13.5;
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

/// Manages the MonitorBar liquid-fused cascading Liquid Glass popup window lifecycle.
pub struct MonitorPopup {
    hwnd: HWND,
    inner: Box<MonitorPopupInner>,
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
    rt: ID2D1DCRenderTarget,
    layer: ID2D1Layer,
    content_layer: ID2D1Layer,
    round_stroke: Option<ID2D1StrokeStyle>,
    surface: LayeredSurface,
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
    backdrop_bmp: Option<ID2D1Bitmap>,
    noise_cache: PopupNoiseCache,
    silk_bmp: Option<ID2D1Bitmap>,
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
    dragging_brightness: Option<usize>,
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
            let round_stroke = create_round_cap_stroke(&d2d_factory);
            let surface = LayeredSurface::new()?;
            let wireless_snap = wireless.snapshot();
            let brightness_snap = brightness.snapshot();

            let mut inner = Box::new(MonitorPopupInner {
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
                rt,
                layer,
                content_layer,
                round_stroke,
                surface,
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
                backdrop_bmp: None,
                noise_cache: PopupNoiseCache::default(),
                silk_bmp: None,
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
                dragging_brightness: None,
                hover: None,
                tracking_leave: false,
                timer_interval_ms: 0,
                fmt_key: 0,
                title_fmt: None,
                badge_fmt: None,
                arrow_cursor,
            });

            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (&mut *inner as *mut MonitorPopupInner) as isize,
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
        (self.is_visible() && self.inner.anim_phase == PopupAnimPhase::Closing)
            || self
                .inner
                .close_started_at
                .is_some_and(|t| t.elapsed() < Duration::from_millis(POPUP_REOPEN_DEBOUNCE_MS))
    }

    pub fn hide_animated(&mut self) {
        self.inner.begin_close();
    }

    pub fn hide_immediately(&mut self) {
        self.inner.hide_immediately();
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
        self.inner.wireless_snap = snapshot;
        self.inner.clamp_scrolls();
        self.inner.sync_timer_rate();
        if self.is_visible() {
            let _ = self.inner.render();
        }
    }

    pub fn update_brightness(&mut self, mut snapshot: BrightnessSnapshot) {
        if let Some(drag_idx) = self.inner.dragging_brightness {
            if let Some(prev) = self.inner.brightness_snap.displays.get(drag_idx) {
                if let Some(cur) = snapshot.displays.iter_mut().find(|d| d.id == prev.id) {
                    cur.brightness = prev.brightness;
                }
            }
        }
        self.inner.brightness_snap = snapshot;
        self.inner.clamp_scrolls();
        self.inner.sync_timer_rate();
        if self.is_visible() {
            let _ = self.inner.render();
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
        self.inner.dpi = dpi.max(96);
        self.inner.light_glass = light_glass;
        self.inner.backdrop_rgb = NEUTRAL_BACKDROP_FALLBACK;
        self.inner.hover = None;
        self.inner.tracking_leave = false;
        self.inner.active_sub = None;
        self.inner.sub_anim = None;
        self.inner.wifi_scroll = 0;
        self.inner.bt_scroll = 0;
        self.inner.brightness_scroll = 0;
        self.inner.dragging_brightness = None;
        self.inner.win_h = 0;
        self.inner.wireless_snap = self.inner.wireless.snapshot();
        self.inner.wireless.request_refresh(false);
        self.inner.brightness_snap = self.inner.brightness.snapshot();
        self.inner.brightness.request_refresh();

        let s = self.inner.scale();
        let side_pad = (SHADOW_PAD_DIP * s).round() as i32;
        let float_gap = (POPUP_FLOAT_GAP_DIP * s).round() as i32;
        let primary_w = (PRIMARY_W_DIP * s).round() as i32;
        let max_sub_w = (SUB_WIRELESS_W_DIP.max(SUB_BRIGHTNESS_W_DIP) * s).round() as i32;
        let max_reach = self.inner.max_popup_reach_px();

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

        let edge_margin = (6.0 * s).round() as i32;
        let top_margin = (6.0 * s).round() as i32;
        let desired_card_bottom = anchor_rect.top - float_gap;
        let win_w = side_pad * 2 + primary_w + max_sub_w;
        let win_h = (side_pad * 2 + max_reach)
            .min((desired_card_bottom + side_pad - work.top - top_margin).max(180));

        // Center/align primary card cleanly above MonitorBar without overhanging screen edges.
        let pill_inset = (6.0 * s).round() as i32;
        let work_min = work.left + edge_margin;
        let work_max = (work.right - primary_w - edge_margin).max(work_min);
        let ideal_primary_left = if (anchor_rect.right - anchor_rect.left) >= primary_w + pill_inset * 2
        {
            let bar_min = anchor_rect.left + pill_inset;
            let bar_max = (anchor_rect.right - pill_inset - primary_w).max(bar_min);
            let min_x = bar_min.max(work_min).min(work_max);
            let max_x = bar_max.min(work_max).max(min_x);
            (click_screen_x - (24.0 * s).round() as i32).clamp(min_x, max_x)
        } else {
            let right_aligned = anchor_rect.right - primary_w;
            right_aligned.clamp(work_min, work_max)
        };

        let open_left =
            ideal_primary_left + primary_w + max_sub_w + edge_margin > work.right
                && (ideal_primary_left - work.left) > (work.right - (ideal_primary_left + primary_w));
        self.inner.open_left = open_left;

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

        self.inner.win_x = win_x;
        self.inner.win_y = win_y;
        self.inner.win_w = win_w;
        self.inner.win_h = win_h;

        let capture_rect = RECT {
            left: win_x,
            top: win_y,
            right: win_x + win_w,
            bottom: win_y + win_h,
        };
        let occluders = visible_transient_overlay_rects(capture_rect);
        let initial_recipe = popup_glass_recipe(active_taskbar_surface_style(), light_glass, 0.5);
        let (bmp, sampled_bg) = capture_blurred_desktop_backdrop_excluding(
            &self.inner.rt,
            win_x,
            win_y,
            win_w,
            win_h,
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
        self.inner.close_started_at = None;

        let _ = self.inner.render();
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
        self.inner.last_flow_tick = Instant::now();
        self.inner.last_color_tick = self.inner.last_flow_tick;
        self.inner.sync_timer_rate();
    }
}

impl Drop for MonitorPopup {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn submenu_height_dip(list_rows: usize) -> f32 {
    let rows = list_rows.max(1);
    let total_rows = 2 + rows;
    let gaps = rows.saturating_sub(1);
    INNER_PAD_DIP * 2.0
        + ROW_H_DIP * (total_rows as f32)
        + ROW_GAP_DIP * (gaps as f32)
        + DIVIDER_H_DIP * 2.0
}

fn brightness_submenu_height_dip(display_rows: usize) -> f32 {
    if display_rows == 0 {
        INNER_PAD_DIP * 2.0 + ROW_H_DIP * 2.0 + DIVIDER_H_DIP
    } else {
        let rows = display_rows.min(MAX_BRIGHTNESS_ROWS);
        let gaps = rows.saturating_sub(1);
        INNER_PAD_DIP * 2.0
            + BRIGHTNESS_ROW_H_DIP * (rows as f32)
            + ROW_GAP_DIP * (gaps as f32)
            + DIVIDER_H_DIP
            + ROW_H_DIP
    }
}

fn primary_rows_height_dip() -> f32 {
    INNER_PAD_DIP * 2.0 + ROW_H_DIP * 6.0 + ROW_GAP_DIP * 5.0
}

fn compute_submenu_vertical_span(
    row_top: f32,
    inner_pad: f32,
    natural_sh: f32,
    top_limit: f32,
    bottom_limit: f32,
) -> (f32, f32) {
    let max_sh = (bottom_limit - top_limit).max(1.0);
    let sh = natural_sh.min(max_sh);

    let down_top = row_top - inner_pad;
    if down_top >= top_limit && down_top + sh <= bottom_limit + 0.5 {
        return (down_top, (down_top + sh).min(bottom_limit));
    }

    let up_top = (bottom_limit - sh).max(top_limit);
    (up_top, (up_top + sh).min(bottom_limit))
}

fn sub_bud_start_rect(
    target_sub: RectF,
    row: RectF,
    seam_x: f32,
    open_left: bool,
    s: f32,
) -> RectF {
    let min_w = ((STEP_MENISCUS_DIP + PANEL_RADIUS_DIP + 10.0).max(40.0) * s).min(target_sub.width());
    let start_w = (target_sub.width() * 0.26).max(min_w).min(target_sub.width());
    let row_cy = ((row.top + row.bottom) * 0.5).clamp(
        target_sub.top + 20.0 * s,
        (target_sub.bottom - 20.0 * s).max(target_sub.top + 20.0 * s),
    );
    let min_h = (44.0 * s).min(target_sub.height());
    let start_h = (target_sub.height() * 0.42).max(min_h).min(target_sub.height());
    let top = (row_cy - start_h * 0.5)
        .clamp(target_sub.top, (target_sub.bottom - start_h).max(target_sub.top));
    let bottom = top + start_h;
    if open_left {
        RectF::new(seam_x - start_w, top, seam_x, bottom)
    } else {
        RectF::new(seam_x, top, seam_x + start_w, bottom)
    }
}

#[allow(clippy::too_many_arguments)]
fn clamp_sub_spring_rect(
    spring_top: f32,
    spring_bottom: f32,
    spring_width: f32,
    target_sub: RectF,
    seam_x: f32,
    bottom_limit: f32,
    win_w: f32,
    open_left: bool,
    s: f32,
) -> RectF {
    let min_safe_w = ((STEP_MENISCUS_DIP + PANEL_RADIUS_DIP + 10.0).max(40.0) * s)
        .min(target_sub.width().max(40.0 * s));
    let max_avail_w = if open_left {
        (seam_x - 5.0 * s).max(min_safe_w)
    } else {
        (win_w - 5.0 * s - seam_x).max(min_safe_w)
    };
    let w = spring_width.clamp(min_safe_w, max_avail_w);

    let min_h = (44.0 * s).min(target_sub.height().max(44.0 * s));
    let min_top = 4.0 * s;
    let max_bot = bottom_limit.max(min_top + min_h);
    let mut bot = spring_bottom.clamp(min_top + min_h, max_bot);
    let mut top = spring_top.clamp(min_top, max_bot - min_h);
    if bot - top < min_h {
        top = (bot - min_h).max(min_top);
        bot = (top + min_h).min(max_bot);
    }

    if open_left {
        RectF::new(seam_x - w, top, seam_x, bot)
    } else {
        RectF::new(seam_x, top, seam_x + w, bot)
    }
}

fn animated_primary_rect(
    target: RectF,
    phase: PopupAnimPhase,
    open_pos: f32,
    close_progress: f32,
    s: f32,
) -> RectF {
    let target_h = target.height().max(1.0);
    let min_h = (42.0 * s).min(target_h * 0.65);
    match phase {
        PopupAnimPhase::Opening => {
            let h = (min_h + (target_h - min_h) * open_pos.max(0.0)).max(min_h);
            RectF::new(
                target.left,
                (target.bottom - h).max(4.0 * s),
                target.right,
                target.bottom,
            )
        }
        PopupAnimPhase::Open => target,
        PopupAnimPhase::Closing => {
            let r = ease_retract(close_progress);
            let h = (target_h * (1.0 - 0.42 * r)).max(min_h);
            RectF::new(
                target.left,
                (target.bottom - h).max(4.0 * s),
                target.right,
                target.bottom,
            )
        }
    }
}

fn animated_closing_sub_rect(
    sub: RectF,
    anim_primary: RectF,
    seam_x: f32,
    bottom_limit: f32,
    open_left: bool,
    close_progress: f32,
    s: f32,
) -> RectF {
    let r = ease_retract(close_progress);
    let min_w = ((STEP_MENISCUS_DIP + PANEL_RADIUS_DIP + 10.0).max(40.0) * s).min(sub.width());
    let cur_w = (sub.width() * (1.0 - 0.22 * r)).max(min_w);
    let drop = 12.0 * s * r;
    let max_bot = bottom_limit.min(anim_primary.bottom);
    let cur_bot = (sub.bottom + drop).min(max_bot).max(44.0 * s);
    let min_h = (42.0 * s).min(sub.height());
    let cur_h = (sub.height() * (1.0 - 0.20 * r))
        .max(min_h)
        .min((cur_bot - 4.0 * s).max(min_h));
    let cur_top = (cur_bot - cur_h).max(4.0 * s);
    if open_left {
        RectF::new(seam_x - cur_w, cur_top, seam_x, cur_bot)
    } else {
        RectF::new(seam_x, cur_top, seam_x + cur_w, cur_bot)
    }
}

impl MonitorPopupInner {
    fn scale(&self) -> f32 {
        (self.dpi.max(96) as f32) / 96.0
    }

    fn cfg_snapshot(&self) -> MonitorConfig {
        self.cfg.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn primary_height_px(&self) -> f32 {
        primary_rows_height_dip() * self.scale()
    }

    fn sub_natural_size_px(&self, kind: SubmenuKind) -> (f32, f32) {
        let s = self.scale();
        let w = kind.width_dip() * s;
        let h_dip = match kind {
            SubmenuKind::WiFi => {
                let n = if self.wireless_snap.wifi_available && self.wireless_snap.wifi_enabled {
                    self.wireless_snap.wifi_networks.len().clamp(1, MAX_LIST_ROWS)
                } else {
                    1
                };
                submenu_height_dip(n)
            }
            SubmenuKind::Bluetooth => {
                let n = if self.wireless_snap.bt_available && self.wireless_snap.bt_enabled {
                    self.wireless_snap.bt_devices.len().clamp(1, MAX_LIST_ROWS)
                } else {
                    1
                };
                submenu_height_dip(n)
            }
            SubmenuKind::Brightness => {
                brightness_submenu_height_dip(self.brightness_snap.displays.len())
            }
            SubmenuKind::LeftClick => INNER_PAD_DIP * 2.0 + ROW_H_DIP * 5.0 + ROW_GAP_DIP * 4.0,
            SubmenuKind::Display => INNER_PAD_DIP * 2.0 + ROW_H_DIP * 5.0 + ROW_GAP_DIP * 4.0,
            SubmenuKind::Settings => {
                INNER_PAD_DIP * 2.0 + ROW_H_DIP * 3.0 + ROW_GAP_DIP + DIVIDER_H_DIP
            }
        };
        (w, h_dip * s)
    }

    fn max_popup_reach_px(&self) -> i32 {
        let s = self.scale();
        let max_sub_h = submenu_height_dip(MAX_LIST_ROWS)
            .max(brightness_submenu_height_dip(MAX_BRIGHTNESS_ROWS))
            * s;
        let pri_h = self.primary_height_px();
        pri_h.max(max_sub_h).ceil() as i32
    }

    fn primary_card_rect(&self) -> RectF {
        let s = self.scale();
        let pad = SHADOW_PAD_DIP * s;
        let pw = PRIMARY_W_DIP * s;
        let ph = self.primary_height_px();
        let bottom = ((self.win_h as f32) - pad).max(pad + 44.0 * s);
        let top = (bottom - ph).max(pad);
        let left = if self.open_left {
            (self.win_w as f32) - pad - pw
        } else {
            pad
        };
        RectF::new(left, top, left + pw, bottom)
    }

    fn primary_row_rects(&self) -> [(PrimaryItem, RectF); 6] {
        let s = self.scale();
        let card = self.primary_card_rect();
        let inner = INNER_PAD_DIP * s;
        let x0 = card.left + inner;
        let x1 = card.right - inner;
        let rh = ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let mut y = card.top + inner;

        let r0 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r1 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r2 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r3 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r4 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r5 = RectF::new(x0, y, x1, y + rh);

        [
            (PrimaryItem::WiFi, r0),
            (PrimaryItem::Bluetooth, r1),
            (PrimaryItem::Brightness, r2),
            (PrimaryItem::LeftClick, r3),
            (PrimaryItem::Display, r4),
            (PrimaryItem::Settings, r5),
        ]
    }

    fn category_row_rect(&self, kind: SubmenuKind) -> RectF {
        for (item, r) in self.primary_row_rects() {
            if item.submenu() == Some(kind) {
                return r;
            }
        }
        self.primary_card_rect()
    }

    fn sub_card_rect(&self, kind: SubmenuKind) -> RectF {
        let s = self.scale();
        let pad = SHADOW_PAD_DIP * s;
        let primary = self.primary_card_rect();
        let row = self.category_row_rect(kind);
        let (sw, natural_sh) = self.sub_natural_size_px(kind);

        let bottom_limit = primary.bottom.max(primary.top + 58.0 * s);
        let top_limit = if self.win_h > 10 {
            pad
        } else {
            primary.top - natural_sh
        };
        let (top, bottom) = compute_submenu_vertical_span(
            row.top,
            INNER_PAD_DIP * s,
            natural_sh,
            top_limit,
            bottom_limit,
        );
        if self.open_left {
            RectF::new(primary.left - sw, top, primary.left, bottom)
        } else {
            RectF::new(primary.right, top, primary.right + sw, bottom)
        }
    }

    fn current_animated_sub_rect(&self) -> Option<RectF> {
        let kind = self.active_sub?;
        let s = self.scale();
        let target_primary = self.primary_card_rect();
        let anim_primary = animated_primary_rect(
            target_primary,
            self.anim_phase,
            self.open_spring.pos,
            self.close_progress,
            s,
        );
        let target_sub = self.sub_card_rect(kind);
        let seam_x = if self.open_left {
            anim_primary.left
        } else {
            anim_primary.right
        };
        let bottom_limit = target_primary.bottom.max(target_primary.top + 58.0 * s);
        let base_sub = match self.sub_anim {
            Some(sa) => clamp_sub_spring_rect(
                sa.top.pos,
                sa.bottom.pos,
                sa.width.pos,
                target_sub,
                seam_x,
                bottom_limit,
                self.win_w as f32,
                self.open_left,
                s,
            ),
            None => target_sub,
        };

        if self.anim_phase == PopupAnimPhase::Closing {
            Some(animated_closing_sub_rect(
                base_sub,
                anim_primary,
                seam_x,
                bottom_limit,
                self.open_left,
                self.close_progress,
                s,
            ))
        } else {
            Some(base_sub)
        }
    }

    fn brightness_row_geom(&self, row_rect: RectF, s: f32) -> BrightnessRowGeom {
        let col_left = row_rect.left + 12.0 * s;
        let col_right = row_rect.right - 10.0 * s;
        let pct_w = 40.0 * s;

        let upper_top = row_rect.top + 4.0 * s;
        let upper_bottom = row_rect.top + 22.0 * s;
        let label_rect = RectF::new(
            col_left,
            upper_top,
            (col_right - pct_w).max(col_left + 30.0 * s),
            upper_bottom,
        );
        let pct_rect = RectF::new(col_right - pct_w, upper_top, col_right, upper_bottom);

        let track_h = 4.0 * s;
        let track_cy = row_rect.top + 32.5 * s;
        let track_left = col_left + 4.0 * s;
        let track_right = (col_right - 4.0 * s).max(track_left + 24.0 * s);
        let track_rect = RectF::new(
            track_left,
            track_cy - track_h * 0.5,
            track_right,
            track_cy + track_h * 0.5,
        );
        let slider_hit_rect = RectF::new(
            track_left - 8.0 * s,
            row_rect.top + 18.0 * s,
            track_right + 8.0 * s,
            row_rect.bottom,
        );

        BrightnessRowGeom {
            label_rect,
            pct_rect,
            track_rect,
            slider_hit_rect,
        }
    }

    fn build_sub_layout(&self, sr: RectF, kind: SubmenuKind) -> SubLayout {
        let s = self.scale();
        let inner = INNER_PAD_DIP * s;
        let rh = ROW_H_DIP * s;
        let brh = BRIGHTNESS_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let div_h = DIVIDER_H_DIP * s;

        let sx0 = sr.left + inner;
        let sx1 = sr.right - inner;
        let mut sy = sr.top + inner;

        let mut rows = Vec::new();
        let mut dividers = Vec::new();
        let mut list_viewport = None;
        let mut list_total_items = 0usize;
        let mut list_scroll = 0usize;
        let mut list_max_rows = MAX_LIST_ROWS;

        match kind {
            SubmenuKind::WiFi => {
                let toggle_r = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::WifiToggle),
                    rect: toggle_r,
                });
                dividers.push(toggle_r);
                sy = toggle_r.bottom + div_h;

                let list_top = sy;
                if self.wireless_snap.wifi_available && self.wireless_snap.wifi_enabled {
                    list_total_items = self.wireless_snap.wifi_networks.len();
                    list_scroll = self.wifi_scroll;
                    if list_total_items == 0 {
                        let r = RectF::new(sx0, sy, sx1, sy + rh);
                        rows.push(SubRowRect { target: None, rect: r });
                        sy = r.bottom;
                    } else {
                        let visible = (list_total_items - list_scroll).min(MAX_LIST_ROWS);
                        for i in 0..visible {
                            let r = RectF::new(sx0, sy, sx1, sy + rh);
                            rows.push(SubRowRect {
                                target: Some(SubHoverTarget::WifiNetwork(list_scroll + i)),
                                rect: r,
                            });
                            sy = r.bottom + if i + 1 < visible { gap } else { 0.0 };
                        }
                        if list_total_items > MAX_LIST_ROWS {
                            list_viewport = Some(RectF::new(sx0, list_top, sx1, sy));
                        }
                    }
                } else {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect { target: None, rect: r });
                    sy = r.bottom;
                }

                dividers.push(RectF::new(sx0, sy - rh, sx1, sy));
                sy += div_h;
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::WifiMoreSettings),
                    rect: RectF::new(sx0, sy, sx1, sy + rh),
                });
            }
            SubmenuKind::Bluetooth => {
                let toggle_r = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::BtToggle),
                    rect: toggle_r,
                });
                dividers.push(toggle_r);
                sy = toggle_r.bottom + div_h;

                let list_top = sy;
                if self.wireless_snap.bt_available && self.wireless_snap.bt_enabled {
                    list_total_items = self.wireless_snap.bt_devices.len();
                    list_scroll = self.bt_scroll;
                    if list_total_items == 0 {
                        let r = RectF::new(sx0, sy, sx1, sy + rh);
                        rows.push(SubRowRect { target: None, rect: r });
                        sy = r.bottom;
                    } else {
                        let visible = (list_total_items - list_scroll).min(MAX_LIST_ROWS);
                        for i in 0..visible {
                            let r = RectF::new(sx0, sy, sx1, sy + rh);
                            rows.push(SubRowRect {
                                target: Some(SubHoverTarget::BtDevice(list_scroll + i)),
                                rect: r,
                            });
                            sy = r.bottom + if i + 1 < visible { gap } else { 0.0 };
                        }
                        if list_total_items > MAX_LIST_ROWS {
                            list_viewport = Some(RectF::new(sx0, list_top, sx1, sy));
                        }
                    }
                } else {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect { target: None, rect: r });
                    sy = r.bottom;
                }

                dividers.push(RectF::new(sx0, sy - rh, sx1, sy));
                sy += div_h;
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::BtMoreSettings),
                    rect: RectF::new(sx0, sy, sx1, sy + rh),
                });
            }
            SubmenuKind::Brightness => {
                list_max_rows = MAX_BRIGHTNESS_ROWS;
                list_total_items = self.brightness_snap.displays.len();
                list_scroll = self.brightness_scroll;
                let list_top = sy;
                if list_total_items == 0 {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect { target: None, rect: r });
                    sy = r.bottom;
                    dividers.push(r);
                } else {
                    let visible = (list_total_items - list_scroll).min(MAX_BRIGHTNESS_ROWS);
                    for i in 0..visible {
                        let r = RectF::new(sx0, sy, sx1, sy + brh);
                        rows.push(SubRowRect {
                            target: Some(SubHoverTarget::BrightnessRow(list_scroll + i)),
                            rect: r,
                        });
                        sy = r.bottom + if i + 1 < visible { gap } else { 0.0 };
                    }
                    if list_total_items > MAX_BRIGHTNESS_ROWS {
                        list_viewport = Some(RectF::new(sx0, list_top, sx1, sy));
                    }
                    dividers.push(RectF::new(sx0, sy - brh, sx1, sy));
                }
                sy += div_h;
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::BrightnessMoreSettings),
                    rect: RectF::new(sx0, sy, sx1, sy + rh),
                });
            }
            SubmenuKind::LeftClick => {
                for action in LeftClickAction::ALL {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect {
                        target: Some(SubHoverTarget::LeftClickPreset(action)),
                        rect: r,
                    });
                    sy = r.bottom + gap;
                }
            }
            SubmenuKind::Display => {
                for item in MetricItem::ALL {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect {
                        target: Some(SubHoverTarget::DisplayMetric(item)),
                        rect: r,
                    });
                    sy = r.bottom + gap;
                }
            }
            SubmenuKind::Settings => {
                let r_lock = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::SettingLock),
                    rect: r_lock,
                });
                sy = r_lock.bottom + gap;

                let r_reset = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::SettingReset),
                    rect: r_reset,
                });
                dividers.push(r_reset);
                sy = r_reset.bottom + div_h;

                let r_close = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::SettingClose),
                    rect: r_close,
                });
            }
        }

        SubLayout {
            rows,
            dividers,
            list_viewport,
            list_total_items,
            list_scroll,
            list_max_rows,
        }
    }

    fn clamp_scrolls(&mut self) {
        let max_wifi = self.wireless_snap.wifi_networks.len().saturating_sub(MAX_LIST_ROWS);
        if self.wifi_scroll > max_wifi {
            self.wifi_scroll = max_wifi;
        }
        let max_bt = self.wireless_snap.bt_devices.len().saturating_sub(MAX_LIST_ROWS);
        if self.bt_scroll > max_bt {
            self.bt_scroll = max_bt;
        }
        let max_bright = self
            .brightness_snap
            .displays
            .len()
            .saturating_sub(MAX_BRIGHTNESS_ROWS);
        if self.brightness_scroll > max_bright {
            self.brightness_scroll = max_bright;
        }
    }

    fn ensure_formats(&mut self, s: f32) -> windows::core::Result<()> {
        let key = self.dpi.max(96);
        if self.fmt_key == key && self.title_fmt.is_some() && self.badge_fmt.is_some() {
            return Ok(());
        }
        self.title_fmt = Some(create_text_format(
            &self.dwrite_factory,
            13.0 * s,
            false,
            false,
            true,
        )?);
        self.badge_fmt = Some(create_text_format(
            &self.dwrite_factory,
            11.0 * s,
            false,
            true,
            false,
        )?);
        self.fmt_key = key;
        Ok(())
    }

    fn point_inside_cards(&self, x: f32, y: f32) -> bool {
        let s = self.scale();
        let step = STEP_MENISCUS_DIP * s;
        let primary = self.primary_card_rect();
        if primary.contains(x, y) {
            return true;
        }
        if let Some(kind) = self.active_sub {
            let sub = self.sub_card_rect(kind);
            if sub.contains(x, y) {
                return true;
            }
            let seam_x = if self.open_left {
                primary.left
            } else {
                primary.right
            };
            let top_band_min = primary.top.min(sub.top);
            let top_band_max = primary.top.max(sub.top) + step;
            if y >= top_band_min && y <= top_band_max && (x - seam_x).abs() <= step.max(14.0 * s)
            {
                return true;
            }
            let bot_band_min = primary.bottom.min(sub.bottom) - step;
            let bot_band_max = primary.bottom.max(sub.bottom);
            if y >= bot_band_min && y <= bot_band_max && (x - seam_x).abs() <= step.max(14.0 * s)
            {
                return true;
            }
        }
        false
    }

    fn hit_test(&self, x: f32, y: f32) -> Option<HoverTarget> {
        let primary = self.primary_card_rect();
        if primary.contains(x, y) {
            for (item, r) in self.primary_row_rects() {
                if r.contains(x, y) {
                    return Some(HoverTarget::Primary(item));
                }
            }
            return None;
        }
        if let Some(kind) = self.active_sub {
            let sr = self.sub_card_rect(kind);
            if sr.contains(x, y) {
                let sub_layout = self.build_sub_layout(sr, kind);
                let s = self.scale();
                for row in sub_layout.rows {
                    if row.rect.contains(x, y) {
                        if let Some(SubHoverTarget::BrightnessRow(idx)) = row.target {
                            let geom = self.brightness_row_geom(row.rect, s);
                            if geom.slider_hit_rect.contains(x, y) {
                                return Some(HoverTarget::Sub(SubHoverTarget::BrightnessSlider(idx)));
                            }
                            return Some(HoverTarget::Sub(SubHoverTarget::BrightnessRow(idx)));
                        }
                        return row.target.map(HoverTarget::Sub);
                    }
                }
            }
        }
        None
    }

    fn brightness_track_rect_for_idx(&self, idx: usize) -> Option<RectF> {
        let sr = self.sub_card_rect(SubmenuKind::Brightness);
        let sub_layout = self.build_sub_layout(sr, SubmenuKind::Brightness);
        let s = self.scale();
        for row in sub_layout.rows {
            if row.target == Some(SubHoverTarget::BrightnessRow(idx)) {
                return Some(self.brightness_row_geom(row.rect, s).track_rect);
            }
        }
        None
    }

    fn apply_brightness_drag(&mut self, idx: usize, cursor_x: f32) -> bool {
        let Some(track) = self.brightness_track_rect_for_idx(idx) else {
            return false;
        };
        let t = ((cursor_x - track.left) / track.width().max(1.0)).clamp(0.0, 1.0);
        let pct = (t * 100.0).round() as u8;
        let Some(disp) = self.brightness_snap.displays.get_mut(idx) else {
            return false;
        };
        if disp.brightness == pct {
            return false;
        }
        disp.brightness = pct;
        let id = disp.id.clone();
        self.brightness.set_brightness(&id, pct);
        true
    }

    fn step_brightness_wheel(&mut self, idx: usize, wheel_delta: i16) -> bool {
        let step = if wheel_delta > 0 { 5i16 } else { -5i16 };
        let Some(disp) = self.brightness_snap.displays.get_mut(idx) else {
            return false;
        };
        let next = (i16::from(disp.brightness) + step).clamp(0, 100) as u8;
        if next == disp.brightness {
            return false;
        }
        disp.brightness = next;
        let id = disp.id.clone();
        self.brightness.set_brightness(&id, next);
        true
    }

    fn set_active_submenu(&mut self, next: Option<SubmenuKind>) -> bool {
        if self.active_sub == next {
            return false;
        }
        let s = self.scale();
        let primary_rect = self.primary_card_rect();
        let prev_anim_sub = self.current_animated_sub_rect();
        let prev_sub = self.active_sub;
        self.active_sub = next;

        if let Some(kind) = next {
            let target_sub = self.sub_card_rect(kind);
            let seam_x = if self.open_left {
                primary_rect.left
            } else {
                primary_rect.right
            };
            self.sub_anim = match (prev_sub, self.sub_anim) {
                (Some(_), Some(mut existing)) => {
                    if let Some(prev_rect) = prev_anim_sub {
                        existing.top.pos = prev_rect.top;
                        existing.bottom.pos = prev_rect.bottom;
                        existing.width.pos = prev_rect.width();
                    }
                    existing.fade_elapsed = 0.0;
                    existing.is_budding = false;
                    Some(existing)
                }
                (Some(pkind), None) => {
                    let old_rect = prev_anim_sub.unwrap_or_else(|| self.sub_card_rect(pkind));
                    Some(SubSpringState {
                        top: Spring1D::still(old_rect.top),
                        bottom: Spring1D::still(old_rect.bottom),
                        width: Spring1D::still(old_rect.width()),
                        fade_elapsed: 0.0,
                        is_budding: false,
                    })
                }
                (None, _) => {
                    let row = self.category_row_rect(kind);
                    let bud = sub_bud_start_rect(target_sub, row, seam_x, self.open_left, s);
                    Some(SubSpringState {
                        top: Spring1D::new(bud.top, (target_sub.top - bud.top) * 2.2),
                        bottom: Spring1D::new(bud.bottom, (target_sub.bottom - bud.bottom) * 2.2),
                        width: Spring1D::new(bud.width(), target_sub.width() * 2.6),
                        fade_elapsed: 0.0,
                        is_budding: true,
                    })
                }
            };

            match kind {
                SubmenuKind::WiFi => self.wireless.request_refresh(true),
                SubmenuKind::Bluetooth => self.wireless.request_refresh(false),
                SubmenuKind::Brightness => self.brightness.request_refresh(),
                _ => {}
            }
        } else {
            self.sub_anim = None;
        }

        self.sync_timer_rate();
        true
    }

    fn sync_timer_rate(&mut self) {
        let need_fast = self.anim_phase != PopupAnimPhase::Open || self.sub_anim.is_some();
        let desired = if need_fast {
            POPUP_ANIM_INTERVAL_MS
        } else {
            POPUP_FLOW_INTERVAL_MS
        };
        if desired != self.timer_interval_ms {
            self.timer_interval_ms = desired;
            unsafe {
                let _ = SetTimer(self.hwnd, TIMER_POPUP_FLOW_ID, desired, None);
            }
        }
    }

    fn advance_animations(&mut self, dt: f32) -> bool {
        match self.anim_phase {
            PopupAnimPhase::Opening => {
                self.open_elapsed += dt;
                self.open_spring
                    .step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, dt);
                if self.open_spring.is_settled(1.0, 0.002, 0.03) && self.open_elapsed >= 0.14 {
                    self.open_spring = Spring1D::still(1.0);
                    self.anim_phase = PopupAnimPhase::Open;
                }
            }
            PopupAnimPhase::Open => {}
            PopupAnimPhase::Closing => {
                self.close_progress = (self.close_progress + dt / CLOSE_ANIM_DUR_S).clamp(0.0, 1.0);
                if self.close_progress >= 1.0 {
                    return true;
                }
            }
        }

        if let (Some(kind), Some(mut sa)) = (self.active_sub, self.sub_anim) {
            let target_sub = self.sub_card_rect(kind);
            sa.top
                .step(target_sub.top, SUB_VERT_STIFFNESS, SUB_VERT_DAMPING, dt);
            sa.bottom
                .step(target_sub.bottom, SUB_VERT_STIFFNESS, SUB_VERT_DAMPING, dt);
            sa.width.step(
                target_sub.width(),
                SUB_WIDTH_STIFFNESS,
                SUB_WIDTH_DAMPING,
                dt,
            );
            sa.fade_elapsed = (sa.fade_elapsed + dt).min(1.0);

            let settled = sa.top.is_settled(target_sub.top, 0.35, 3.0)
                && sa.bottom.is_settled(target_sub.bottom, 0.35, 3.0)
                && sa.width.is_settled(target_sub.width(), 0.35, 3.0)
                && sa.fade_elapsed >= 0.11;
            self.sub_anim = if settled { None } else { Some(sa) };
        } else {
            self.sub_anim = None;
        }

        false
    }

    fn begin_close(&mut self) {
        if self.anim_phase == PopupAnimPhase::Closing {
            return;
        }
        if self.dragging_brightness.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        self.anim_phase = PopupAnimPhase::Closing;
        self.close_progress = 0.0;
        self.close_started_at = Some(Instant::now());
        self.hover = None;
        self.last_flow_tick = Instant::now();
        self.sync_timer_rate();
        let _ = self.render();
    }

    fn hide_immediately(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.reset_transient_state();
        unsafe {
            let _ = PostMessageW(self.owner_hwnd, WM_APP_MONITOR_POPUP_CLOSED, WPARAM(0), LPARAM(0));
        }
    }

    fn finish_close(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.reset_transient_state();
        unsafe {
            let _ = PostMessageW(self.owner_hwnd, WM_APP_MONITOR_POPUP_CLOSED, WPARAM(0), LPARAM(0));
        }
    }

    fn reset_transient_state(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
        }
        if self.dragging_brightness.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        self.timer_interval_ms = 0;
        self.backdrop_bmp = None;
        self.halo_colors_ready = false;
        self.last_silk_tick = None;
        self.anim_phase = PopupAnimPhase::Open;
        self.open_spring = Spring1D::still(1.0);
        self.open_elapsed = 1.0;
        self.close_progress = 0.0;
        self.close_started_at = None;
        self.active_sub = None;
        self.sub_anim = None;
        self.hover = None;
        self.tracking_leave = false;
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn emit_top_seam_transition(
        sink: &ID2D1GeometrySink,
        seam_x: f32,
        lt: f32,
        rt: f32,
        _left_w: f32,
        _left_tl: f32,
        _right_w: f32,
        _right_tr: f32,
        r_c: f32,
        step_r: f32,
        s: f32,
    ) {
        let dy = rt - lt;

        if dy.abs() < 1.0 * s {
            sink.AddLine(D2D_POINT_2F {
                x: seam_x,
                y: (lt + rt) * 0.5,
            });
        } else if dy >= 1.0 * s && dy < r_c + step_r {
            sink.AddLine(D2D_POINT_2F {
                x: seam_x - 10.0 * s,
                y: lt,
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F {
                    x: seam_x,
                    y: lt,
                },
                point2: D2D_POINT_2F {
                    x: seam_x,
                    y: rt,
                },
                point3: D2D_POINT_2F {
                    x: seam_x + 10.0 * s,
                    y: rt,
                },
            });
        } else if dy <= -1.0 * s && (-dy) < r_c + step_r {
            sink.AddLine(D2D_POINT_2F {
                x: seam_x - 10.0 * s,
                y: lt,
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F {
                    x: seam_x,
                    y: lt,
                },
                point2: D2D_POINT_2F {
                    x: seam_x,
                    y: rt,
                },
                point3: D2D_POINT_2F {
                    x: seam_x + 10.0 * s,
                    y: rt,
                },
            });
        } else if dy >= r_c + step_r {
            sink.AddLine(D2D_POINT_2F {
                x: seam_x - r_c,
                y: lt,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: seam_x,
                    y: lt + r_c,
                },
                size: D2D_SIZE_F {
                    width: r_c,
                    height: r_c,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddLine(D2D_POINT_2F {
                x: seam_x,
                y: rt - step_r,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: seam_x + step_r,
                    y: rt,
                },
                size: D2D_SIZE_F {
                    width: step_r,
                    height: step_r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
        } else {
            sink.AddLine(D2D_POINT_2F {
                x: seam_x - step_r,
                y: lt,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: seam_x,
                    y: lt - step_r,
                },
                size: D2D_SIZE_F {
                    width: step_r,
                    height: step_r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddLine(D2D_POINT_2F {
                x: seam_x,
                y: rt + r_c,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: seam_x + r_c,
                    y: rt,
                },
                size: D2D_SIZE_F {
                    width: r_c,
                    height: r_c,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn emit_bottom_seam_transition(
        sink: &ID2D1GeometrySink,
        seam_x: f32,
        lb: f32,
        rb: f32,
        left_w: f32,
        left_bl: f32,
        right_w: f32,
        right_br: f32,
        r_c: f32,
        step_r: f32,
        s: f32,
    ) {
        let dy = lb - rb;
        if dy.abs() < 1.0 * s {
            sink.AddLine(D2D_POINT_2F {
                x: seam_x,
                y: (lb + rb) * 0.5,
            });
            return;
        }
        let max_left = (left_w - left_bl).max(2.0 * s);
        let max_right = (right_w - right_br).max(2.0 * s);
        if dy.abs() < r_c + step_r {
            let span_r = (14.0 * s).min(max_right);
            let span_l = (14.0 * s).min(max_left);
            sink.AddLine(D2D_POINT_2F {
                x: seam_x + span_r,
                y: rb,
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F { x: seam_x, y: rb },
                point2: D2D_POINT_2F { x: seam_x, y: lb },
                point3: D2D_POINT_2F {
                    x: seam_x - span_l,
                    y: lb,
                },
            });
        } else if dy > 0.0 {
            let sr = step_r.min(max_right);
            let cr = r_c.min(max_left);
            sink.AddLine(D2D_POINT_2F {
                x: seam_x + sr,
                y: rb,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: seam_x,
                    y: rb + sr,
                },
                size: D2D_SIZE_F {
                    width: sr,
                    height: sr,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddLine(D2D_POINT_2F {
                x: seam_x,
                y: lb - cr,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: seam_x - cr,
                    y: lb,
                },
                size: D2D_SIZE_F {
                    width: cr,
                    height: cr,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
        } else {
            let cr = r_c.min(max_right);
            let sr = step_r.min(max_left);
            sink.AddLine(D2D_POINT_2F {
                x: seam_x + cr,
                y: rb,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: seam_x,
                    y: rb - cr,
                },
                size: D2D_SIZE_F {
                    width: cr,
                    height: cr,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddLine(D2D_POINT_2F {
                x: seam_x,
                y: lb + sr,
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F {
                    x: seam_x - sr,
                    y: lb,
                },
                size: D2D_SIZE_F {
                    width: sr,
                    height: sr,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
        }
    }

    unsafe fn emit_unified_contour(
        sink: &ID2D1GeometrySink,
        primary: RectF,
        sub: Option<RectF>,
        open_left: bool,
        s: f32,
    ) {
        let r_c = PANEL_RADIUS_DIP * s;
        let step_r = STEP_MENISCUS_DIP * s;
        let eff_p = r_c
            .min((primary.height() * 0.45).max(4.0 * s))
            .min((primary.width() * 0.45).max(4.0 * s));

        match sub {
            None => {
                sink.BeginFigure(
                    D2D_POINT_2F {
                        x: primary.left,
                        y: primary.bottom - eff_p,
                    },
                    D2D1_FIGURE_BEGIN_FILLED,
                );
                sink.AddLine(D2D_POINT_2F {
                    x: primary.left,
                    y: primary.top + eff_p,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.left + eff_p,
                        y: primary.top,
                    },
                    size: D2D_SIZE_F {
                        width: eff_p,
                        height: eff_p,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
                sink.AddLine(D2D_POINT_2F {
                    x: primary.right - eff_p,
                    y: primary.top,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.right,
                        y: primary.top + eff_p,
                    },
                    size: D2D_SIZE_F {
                        width: eff_p,
                        height: eff_p,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
                sink.AddLine(D2D_POINT_2F {
                    x: primary.right,
                    y: primary.bottom - eff_p,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.right - eff_p,
                        y: primary.bottom,
                    },
                    size: D2D_SIZE_F {
                        width: eff_p,
                        height: eff_p,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
                sink.AddLine(D2D_POINT_2F {
                    x: primary.left + eff_p,
                    y: primary.bottom,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.left,
                        y: primary.bottom - eff_p,
                    },
                    size: D2D_SIZE_F {
                        width: eff_p,
                        height: eff_p,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
            }
            Some(sub) if !open_left => {
                let seam_x = primary.right;
                let sub_r = r_c
                    .min((sub.height() * 0.45).max(4.0 * s))
                    .min((sub.width() * 0.45).max(4.0 * s));

                sink.BeginFigure(
                    D2D_POINT_2F {
                        x: primary.left,
                        y: primary.bottom - eff_p,
                    },
                    D2D1_FIGURE_BEGIN_FILLED,
                );
                sink.AddLine(D2D_POINT_2F {
                    x: primary.left,
                    y: primary.top + eff_p,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.left + eff_p,
                        y: primary.top,
                    },
                    size: D2D_SIZE_F {
                        width: eff_p,
                        height: eff_p,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });

                Self::emit_top_seam_transition(
                    sink,
                    seam_x,
                    primary.top,
                    sub.top,
                    primary.width(),
                    eff_p,
                    sub.width(),
                    sub_r,
                    r_c,
                    step_r,
                    s,
                );

                sink.AddLine(D2D_POINT_2F {
                    x: sub.right - sub_r,
                    y: sub.top,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: sub.right,
                        y: sub.top + sub_r,
                    },
                    size: D2D_SIZE_F {
                        width: sub_r,
                        height: sub_r,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
                sink.AddLine(D2D_POINT_2F {
                    x: sub.right,
                    y: sub.bottom - sub_r,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: sub.right - sub_r,
                        y: sub.bottom,
                    },
                    size: D2D_SIZE_F {
                        width: sub_r,
                        height: sub_r,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });

                Self::emit_bottom_seam_transition(
                    sink,
                    seam_x,
                    primary.bottom,
                    sub.bottom,
                    primary.width(),
                    eff_p,
                    sub.width(),
                    sub_r,
                    r_c,
                    step_r,
                    s,
                );

                sink.AddLine(D2D_POINT_2F {
                    x: primary.left + eff_p,
                    y: primary.bottom,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.left,
                        y: primary.bottom - eff_p,
                    },
                    size: D2D_SIZE_F {
                        width: eff_p,
                        height: eff_p,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
            }
            Some(sub) => {
                let seam_x = primary.left;
                let sub_r = r_c
                    .min((sub.height() * 0.45).max(4.0 * s))
                    .min((sub.width() * 0.45).max(4.0 * s));

                sink.BeginFigure(
                    D2D_POINT_2F {
                        x: sub.left,
                        y: sub.bottom - sub_r,
                    },
                    D2D1_FIGURE_BEGIN_FILLED,
                );
                sink.AddLine(D2D_POINT_2F {
                    x: sub.left,
                    y: sub.top + sub_r,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: sub.left + sub_r,
                        y: sub.top,
                    },
                    size: D2D_SIZE_F {
                        width: sub_r,
                        height: sub_r,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });

                Self::emit_top_seam_transition(
                    sink,
                    seam_x,
                    sub.top,
                    primary.top,
                    sub.width(),
                    sub_r,
                    primary.width(),
                    eff_p,
                    r_c,
                    step_r,
                    s,
                );

                sink.AddLine(D2D_POINT_2F {
                    x: primary.right - eff_p,
                    y: primary.top,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.right,
                        y: primary.top + eff_p,
                    },
                    size: D2D_SIZE_F {
                        width: eff_p,
                        height: eff_p,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
                sink.AddLine(D2D_POINT_2F {
                    x: primary.right,
                    y: primary.bottom - eff_p,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.right - eff_p,
                        y: primary.bottom,
                    },
                    size: D2D_SIZE_F {
                        width: eff_p,
                        height: eff_p,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });

                Self::emit_bottom_seam_transition(
                    sink,
                    seam_x,
                    sub.bottom,
                    primary.bottom,
                    sub.width(),
                    sub_r,
                    primary.width(),
                    eff_p,
                    r_c,
                    step_r,
                    s,
                );

                sink.AddLine(D2D_POINT_2F {
                    x: sub.left + sub_r,
                    y: sub.bottom,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: sub.left,
                        y: sub.bottom - sub_r,
                    },
                    size: D2D_SIZE_F {
                        width: sub_r,
                        height: sub_r,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
            }
        }

        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }

    fn build_unified_menu_geometry(
        &self,
        primary: RectF,
        sub: Option<RectF>,
        s: f32,
    ) -> windows::core::Result<ID2D1PathGeometry> {
        unsafe {
            let geo = self.d2d_factory.CreatePathGeometry()?;
            let sink = geo.Open()?;
            Self::emit_unified_contour(&sink, primary, sub, self.open_left, s);
            sink.Close()?;
            Ok(geo.into())
        }
    }

    unsafe fn draw_unified_shadows(&self, primary_rect: RectF, sub_rect: Option<RectF>, s: f32) {
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
            let sh_primary = RectF::new(
                if self.open_left && sub_rect.is_some() {
                    primary_rect.left
                } else {
                    (primary_rect.left - side_exp).max(0.0)
                },
                (primary_rect.top - top_exp).max(0.0),
                if !self.open_left && sub_rect.is_some() {
                    primary_rect.right
                } else {
                    (primary_rect.right + side_exp).min(max_w)
                },
                (primary_rect.bottom + bot_exp).min(max_h),
            );
            let sh_sub = sub_rect.map(|sr| {
                RectF::new(
                    if !self.open_left {
                        sr.left
                    } else {
                        (sr.left - side_exp).max(0.0)
                    },
                    (sr.top - top_exp).max(0.0),
                    if self.open_left {
                        sr.right
                    } else {
                        (sr.right + side_exp).min(max_w)
                    },
                    (sr.bottom + bot_exp).min(max_h),
                )
            });

            if let (Ok(geo), Ok(sh_brush)) = (
                self.build_unified_menu_geometry(sh_primary, sh_sub, s),
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

    fn update_silk_and_accent(&mut self, pri_rect: RectF, s: f32, draw_silk: bool) {
        let now = Instant::now();
        let dt_color = now
            .duration_since(self.last_color_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_color_tick = now;

        let (ui_rgb, halo_base_dark) = resolve_unified_accent(self.backdrop_rgb, self.light_glass);
        let pair = compute_halo_pair(halo_base_dark, self.light_glass);
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
                pri_rect.left,
                pri_rect.bottom,
                s,
                self.flow_phase,
                &self.halo_colors,
                self.light_glass,
            );
        }
    }

    fn render(&mut self) -> windows::core::Result<()> {
        let s = self.scale();
        self.surface.ensure_size(self.win_w, self.win_h)?;
        self.ensure_formats(s)?;

        let target_primary = self.primary_card_rect();
        let recipe = popup_glass_recipe(
            active_taskbar_surface_style(),
            self.light_glass,
            rgb_luma(self.backdrop_rgb),
        );
        self.update_silk_and_accent(target_primary, s, recipe.silk_alpha > 0.002);

        let accent = Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        );
        let cfg = self.cfg_snapshot();

        let anim_primary = animated_primary_rect(
            target_primary,
            self.anim_phase,
            self.open_spring.pos,
            self.close_progress,
            s,
        );
        let primary_rows = self.primary_row_rects();
        let anim_sub_opt = self.current_animated_sub_rect();

        let (primary_alpha, close_dy) = match self.anim_phase {
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

        let left_bound = anim_sub_opt.map_or(anim_primary.left, |sr| anim_primary.left.min(sr.left));
        let right_bound =
            anim_sub_opt.map_or(anim_primary.right, |sr| anim_primary.right.max(sr.right));
        let top_bound = anim_sub_opt.map_or(anim_primary.top, |sr| anim_primary.top.min(sr.top));
        let bottom_bound =
            anim_sub_opt.map_or(anim_primary.bottom, |sr| anim_primary.bottom.max(sr.bottom));
        let union_bounds = RectF::new(
            left_bound.max(0.0),
            top_bound.max(0.0),
            right_bound.min(self.win_w as f32),
            bottom_bound.min(self.win_h as f32),
        );

        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.SetTransform(&identity_matrix());
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            // 1. Standalone ambient shadows
            self.draw_unified_shadows(anim_primary, anim_sub_opt, s);

            // 2. Closed fused silhouette clipping layer
            if let Ok(unified_geo) = self.build_unified_menu_geometry(anim_primary, anim_sub_opt, s)
            {
                let geom: ID2D1Geometry = unified_geo.cast()?;
                let geom_for_rim = geom.clone();
                let outer_params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(union_bounds),
                    geometricMask: ManuallyDrop::new(Some(geom)),
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
                if primary_alpha > 0.005 {
                    let use_layer = self.anim_phase != PopupAnimPhase::Open;
                    if use_layer {
                        let content_params = D2D1_LAYER_PARAMETERS {
                            contentBounds: d2d_rect(anim_primary),
                            geometricMask: ManuallyDrop::new(None),
                            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                            maskTransform: identity_matrix(),
                            opacity: primary_alpha,
                            opacityBrush: ManuallyDrop::new(None),
                            layerOptions: D2D1_LAYER_OPTIONS_NONE,
                        };
                        self.rt.PushLayer(&content_params, &self.content_layer);
                        let _ = ManuallyDrop::into_inner(content_params.geometricMask);
                        let _ = ManuallyDrop::into_inner(content_params.opacityBrush);
                    }

                    let row_count = primary_rows.len();
                    for (idx, (item, mut r)) in primary_rows.into_iter().enumerate() {
                        match self.anim_phase {
                            PopupAnimPhase::Opening => {
                                let dy = row_stagger_dy(
                                    idx,
                                    row_count,
                                    self.open_elapsed,
                                    self.open_spring.pos,
                                    s,
                                );
                                r.top += dy;
                                r.bottom += dy;
                            }
                            PopupAnimPhase::Open => {}
                            PopupAnimPhase::Closing => {
                                r.top += close_dy;
                                r.bottom += close_dy;
                            }
                        }

                        let hovered = self.hover == Some(HoverTarget::Primary(item));
                        let expanded =
                            self.active_sub.is_some() && item.submenu() == self.active_sub;
                        self.draw_primary_row_bg(r, hovered, expanded, accent, s);

                        let mut fg = primary_text_color(self.light_glass);
                        if !hovered && !expanded {
                            fg.a *= 0.88;
                        }

                        if let Ok(icon_brush) = self.rt.CreateSolidColorBrush(&fg, None) {
                            let icx = r.left + 19.0 * s;
                            let icy = (r.top + r.bottom) * 0.5;
                            self.draw_primary_icon(item, icx, icy, s, &icon_brush);
                            if item.submenu().is_some() {
                                let mut chev_col = muted_text_color(self.light_glass);
                                if expanded || hovered {
                                    chev_col = fg;
                                }
                                if let Ok(cb) = self.rt.CreateSolidColorBrush(&chev_col, None) {
                                    self.draw_chevron(r.right - 12.0 * s, icy, s, &cb);
                                }
                            }
                        }

                        let label_r = RectF::new(
                            r.left + 34.0 * s,
                            r.top,
                            r.right - if item.submenu().is_some() { 22.0 * s } else { 10.0 * s },
                            r.bottom,
                        );
                        if let Some(ref fmt) = self.title_fmt {
                            draw_text_in_rect(
                                &self.dwrite_factory,
                                &self.rt,
                                item.label(),
                                label_r,
                                fmt,
                                fg,
                            );
                        }
                    }

                    if use_layer {
                        self.rt.PopLayer();
                    }
                }

                // 2b. Feathered seam hairline & sliding/budding submenu content
                if let (Some(kind), Some(anim_sub)) = (self.active_sub, anim_sub_opt) {
                    let target_sub = self.sub_card_rect(kind);
                    let close_factor = if self.anim_phase == PopupAnimPhase::Closing {
                        let r = ease_retract(self.close_progress);
                        (1.0 - 0.35 * r).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    let (fade_in, dx_cur, dy_cur) = match self.sub_anim {
                        Some(sa) if sa.is_budding => {
                            let a =
                                ease_out_cubic(((sa.fade_elapsed - 0.008) / 0.078).clamp(0.0, 1.0));
                            let slide_t = ease_out_cubic((sa.fade_elapsed / 0.105).clamp(0.0, 1.0));
                            let dir = if self.open_left { 1.0 } else { -1.0 };
                            let dx = dir * 10.0 * s * (1.0 - slide_t);
                            let dy = (anim_sub.top - target_sub.top) * 0.25;
                            (a, dx, dy)
                        }
                        Some(sa) => {
                            let a =
                                ease_out_cubic(((sa.fade_elapsed - 0.010) / 0.070).clamp(0.0, 1.0));
                            let dy = (anim_sub.top - target_sub.top) * 0.22;
                            (a, 0.0, dy)
                        }
                        None => (1.0, 0.0, (anim_sub.top - target_sub.top) * 0.45),
                    };

                    let seam_fade = if self.sub_anim.is_some_and(|sa| sa.is_budding) {
                        fade_in.max(0.12)
                    } else {
                        1.0
                    };
                    self.draw_seam_hairline(anim_primary, anim_sub, seam_fade, s);

                    let cur_alpha = fade_in * close_factor;
                    if cur_alpha > 0.005 {
                        let need_layer =
                            self.sub_anim.is_some() || self.anim_phase != PopupAnimPhase::Open;
                        if need_layer {
                            let layer_params = D2D1_LAYER_PARAMETERS {
                                contentBounds: d2d_rect(anim_sub),
                                geometricMask: ManuallyDrop::new(None),
                                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                maskTransform: identity_matrix(),
                                opacity: cur_alpha,
                                opacityBrush: ManuallyDrop::new(None),
                                layerOptions: D2D1_LAYER_OPTIONS_NONE,
                            };
                            self.rt.PushLayer(&layer_params, &self.content_layer);
                            let _ = ManuallyDrop::into_inner(layer_params.geometricMask);
                            let _ = ManuallyDrop::into_inner(layer_params.opacityBrush);
                        }
                        self.rt.PushAxisAlignedClip(
                            &d2d_rect(anim_sub),
                            D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        );
                        if dx_cur.abs() > 0.05 || dy_cur.abs() > 0.05 {
                            self.rt.SetTransform(&Matrix3x2 {
                                M11: 1.0,
                                M12: 0.0,
                                M21: 0.0,
                                M22: 1.0,
                                M31: dx_cur,
                                M32: dy_cur,
                            });
                        }

                        let sub_layout = self.build_sub_layout(target_sub, kind);
                        self.draw_submenu_contents(kind, &sub_layout, &cfg, accent, s);

                        if dx_cur.abs() > 0.05 || dy_cur.abs() > 0.05 {
                            self.rt.SetTransform(&identity_matrix());
                        }
                        self.rt.PopAxisAlignedClip();
                        if need_layer {
                            self.rt.PopLayer();
                        }
                    }
                }

                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(outer_params.geometricMask);
                let _ = ManuallyDrop::into_inner(outer_params.opacityBrush);

                // 3. Restrained closed glass rim stroke harmonized with taskbar style
                draw_geometry_rim(
                    &self.rt,
                    &geom_for_rim,
                    union_bounds.left,
                    union_bounds.top,
                    union_bounds.bottom,
                    s,
                    &recipe,
                );
            }

            let res = self.rt.EndDraw(None, None);
            if res.is_ok() {
                let alpha_u8 = (window_master_alpha(
                    self.anim_phase,
                    self.open_elapsed,
                    self.close_progress,
                ) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                self.surface
                    .commit_with_alpha(self.hwnd, self.win_x, self.win_y, alpha_u8);
            }
            res
        }
    }

    unsafe fn draw_primary_row_bg(
        &self,
        rect: RectF,
        hovered: bool,
        expanded: bool,
        accent: Rgb,
        s: f32,
    ) {
        if !hovered && !expanded {
            return;
        }
        let rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(rect),
            radiusX: 8.5 * s,
            radiusY: 8.5 * s,
        };
        let fill = if self.light_glass {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: if hovered { 0.08 } else { 0.05 },
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if hovered { 0.14 } else { 0.09 },
            }
        };
        if let Ok(b) = self.rt.CreateSolidColorBrush(&fill, None) {
            self.rt.FillRoundedRectangle(&rr, &b);
        }

        if expanded {
            let bar_w = 3.0 * s;
            let bar_h = (16.0 * s).min(rect.height() - 10.0 * s);
            let bar_left = rect.left + 3.0 * s;
            let bar_top = (rect.top + rect.bottom - bar_h) * 0.5;
            let bar_rr = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: bar_left,
                    top: bar_top,
                    right: bar_left + bar_w,
                    bottom: bar_top + bar_h,
                },
                radiusX: bar_w * 0.5,
                radiusY: bar_w * 0.5,
            };
            if let Ok(ab) = self.rt.CreateSolidColorBrush(&d2d_color_rgb(accent, 0.96), None) {
                self.rt.FillRoundedRectangle(&bar_rr, &ab);
            }
        }
    }

    unsafe fn draw_seam_hairline(&self, anim_pri: RectF, sub_rect: RectF, fade: f32, s: f32) {
        let seam_x = if self.open_left {
            anim_pri.left
        } else {
            anim_pri.right
        };
        let seam_top = anim_pri.top.max(sub_rect.top) + 8.0 * s;
        let seam_bottom = anim_pri.bottom.min(sub_rect.bottom) - 8.0 * s;
        if seam_bottom <= seam_top + 4.0 * s {
            return;
        }
        let (sr, sg, sb, base_a) = if self.light_glass {
            (0.0, 0.0, 0.0, 0.075)
        } else {
            (1.0, 1.0, 1.0, 0.10)
        };
        let peak_a = base_a * fade;
        let stops = [
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
            &stops,
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
            if let Ok(brush) = self.rt.CreateLinearGradientBrush(&props, None, &sc) {
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: seam_x,
                        y: seam_top,
                    },
                    D2D_POINT_2F {
                        x: seam_x,
                        y: seam_bottom,
                    },
                    &brush,
                    1.0 * s,
                    None,
                );
            }
        }
    }

    unsafe fn draw_submenu_contents(
        &self,
        sub_kind: SubmenuKind,
        layout: &SubLayout,
        cfg: &MonitorConfig,
        accent: Rgb,
        s: f32,
    ) {
        let Some(ref fmt) = self.title_fmt else {
            return;
        };
        let Some(ref badge_fmt) = self.badge_fmt else {
            return;
        };

        let primary_fg = primary_text_color(self.light_glass);
        let muted_fg = muted_text_color(self.light_glass);

        for row in &layout.rows {
            let r = row.rect;
            let hovered = match row.target {
                Some(SubHoverTarget::BrightnessRow(idx)) => {
                    self.dragging_brightness == Some(idx)
                        || matches!(
                            self.hover,
                            Some(HoverTarget::Sub(
                                SubHoverTarget::BrightnessRow(i)
                                    | SubHoverTarget::BrightnessSlider(i)
                            )) if i == idx
                        )
                }
                Some(target) => self.hover == Some(HoverTarget::Sub(target)),
                None => false,
            };
            let selected = match row.target {
                Some(SubHoverTarget::WifiNetwork(idx)) => self
                    .wireless_snap
                    .wifi_networks
                    .get(idx)
                    .is_some_and(|w| w.is_connected),
                Some(SubHoverTarget::BtDevice(idx)) => self
                    .wireless_snap
                    .bt_devices
                    .get(idx)
                    .is_some_and(|d| d.is_connected),
                Some(SubHoverTarget::LeftClickPreset(action)) => cfg.left_click == action,
                Some(SubHoverTarget::DisplayMetric(item)) => cfg.is_visible(item),
                Some(SubHoverTarget::SettingLock) => cfg.locked,
                _ => false,
            };

            if row.target.is_some() {
                draw_row_highlight(&self.rt, r, hovered, selected, accent, s, self.light_glass);
            }

            match row.target {
                Some(SubHoverTarget::WifiToggle) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "WLAN",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 46.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                    draw_pill_switch(
                        &self.rt,
                        r.right - 10.0 * s,
                        (r.top + r.bottom) * 0.5,
                        self.wireless_snap.wifi_enabled,
                        accent,
                        s,
                        self.light_glass,
                    );
                }
                Some(SubHoverTarget::BtToggle) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "蓝牙开关",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 46.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                    draw_pill_switch(
                        &self.rt,
                        r.right - 10.0 * s,
                        (r.top + r.bottom) * 0.5,
                        self.wireless_snap.bt_enabled,
                        accent,
                        s,
                        self.light_glass,
                    );
                }
                Some(SubHoverTarget::WifiNetwork(idx)) => {
                    if let Some(net) = self.wireless_snap.wifi_networks.get(idx) {
                        let icy = (r.top + r.bottom) * 0.5;
                        self.draw_wifi_signal_icon(
                            r.left + 18.0 * s,
                            icy,
                            s,
                            net.signal_quality,
                            primary_fg,
                            muted_fg,
                        );
                        let badge = if net.is_connected {
                            "已连接"
                        } else if !net.has_profile {
                            if net.is_secured { "加密" } else { "开放" }
                        } else if net.signal_quality == 0 {
                            "已保存"
                        } else {
                            ""
                        };
                        let right_pad = if badge.is_empty() { 10.0 * s } else { 54.0 * s };
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            &net.ssid,
                            RectF::new(r.left + 32.0 * s, r.top, r.right - right_pad, r.bottom),
                            fmt,
                            primary_fg,
                        );
                        if !badge.is_empty() {
                            draw_text_in_rect(
                                &self.dwrite_factory,
                                &self.rt,
                                badge,
                                RectF::new(r.right - 56.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                                badge_fmt,
                                if net.is_connected { primary_fg } else { muted_fg },
                            );
                        }
                    }
                }
                Some(SubHoverTarget::BtDevice(idx)) => {
                    if let Some(dev) = self.wireless_snap.bt_devices.get(idx) {
                        let icy = (r.top + r.bottom) * 0.5;
                        if let Ok(br) = self.rt.CreateSolidColorBrush(&primary_fg, None) {
                            self.draw_bluetooth_icon(r.left + 17.0 * s, icy, s, &br);
                        }
                        let badge = if dev.is_connected { "已连接" } else { "已配对" };
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            &dev.name,
                            RectF::new(r.left + 30.0 * s, r.top, r.right - 56.0 * s, r.bottom),
                            fmt,
                            primary_fg,
                        );
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            badge,
                            RectF::new(r.right - 56.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                            badge_fmt,
                            if dev.is_connected { primary_fg } else { muted_fg },
                        );
                    }
                }
                Some(SubHoverTarget::WifiMoreSettings) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "更多网络设置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 12.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::BtMoreSettings) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "更多蓝牙设置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 12.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::BrightnessRow(idx)) => {
                    if let Some(disp) = self.brightness_snap.displays.get(idx) {
                        let geom = self.brightness_row_geom(r, s);
                        let is_software = disp.mode == BrightnessMode::Software;
                        let name_rect = if is_software {
                            RectF::new(
                                geom.label_rect.left,
                                geom.label_rect.top,
                                (geom.label_rect.right - 10.0 * s)
                                    .max(geom.label_rect.left + 20.0 * s),
                                geom.label_rect.bottom,
                            )
                        } else {
                            geom.label_rect
                        };
                        let measured_w = draw_text_in_rect_measured(
                            &self.dwrite_factory,
                            &self.rt,
                            &disp.name,
                            name_rect,
                            fmt,
                            primary_fg,
                        );
                        if is_software {
                            let dot_x = (name_rect.left + measured_w + 4.5 * s)
                                .min(geom.pct_rect.left - 5.0 * s);
                            let dot_y = name_rect.top + 5.0 * s;
                            let dot_r = 2.6 * s;
                            let dot_col = if self.light_glass {
                                D2D1_COLOR_F {
                                    r: 0.86,
                                    g: 0.22,
                                    b: 0.24,
                                    a: 0.96,
                                }
                            } else {
                                D2D1_COLOR_F {
                                    r: 1.0,
                                    g: 0.36,
                                    b: 0.36,
                                    a: 0.98,
                                }
                            };
                            if let Ok(db) = self.rt.CreateSolidColorBrush(&dot_col, None) {
                                self.rt.FillEllipse(
                                    &D2D1_ELLIPSE {
                                        point: D2D_POINT_2F { x: dot_x, y: dot_y },
                                        radiusX: dot_r,
                                        radiusY: dot_r,
                                    },
                                    &db,
                                );
                            }
                        }

                        let pct_str = format_brightness_pct(disp.brightness);
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            &pct_str,
                            geom.pct_rect,
                            badge_fmt,
                            muted_fg,
                        );

                        let slider_active = self.dragging_brightness == Some(idx)
                            || self.hover == Some(HoverTarget::Sub(SubHoverTarget::BrightnessSlider(idx)));
                        draw_glass_slider(
                            &self.rt,
                            geom.track_rect,
                            f32::from(disp.brightness) / 100.0,
                            false,
                            slider_active,
                            self.light_glass,
                            accent,
                            s,
                        );
                    }
                }
                Some(SubHoverTarget::BrightnessSlider(_)) => {}
                Some(SubHoverTarget::BrightnessMoreSettings) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "更多屏幕设置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 12.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::LeftClickPreset(action)) => {
                    let text = if action == LeftClickAction::CustomApp
                        && !cfg.custom_app_path.is_empty()
                    {
                        let name = Path::new(&cfg.custom_app_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("已选程序");
                        format!("自定义 ({name})")
                    } else {
                        action.label().to_string()
                    };
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        &text,
                        RectF::new(r.left + 14.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::DisplayMetric(item)) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        item.label(),
                        RectF::new(r.left + 14.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::SettingLock) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "锁定位置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::SettingReset) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "重置位置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::SettingClose) => {
                    let mut close_col = danger_text_color(self.light_glass);
                    if hovered {
                        close_col.a = 1.0;
                    }
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "关闭",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        close_col,
                    );
                }
                None => {
                    let msg = match sub_kind {
                        SubmenuKind::WiFi => {
                            if !self.wireless_snap.wifi_available {
                                "未检测到 WLAN 适配器"
                            } else if !self.wireless_snap.wifi_enabled {
                                "WLAN 已关闭"
                            } else if self.wireless_snap.wifi_busy {
                                "正在扫描..."
                            } else {
                                "未发现可用网络"
                            }
                        }
                        SubmenuKind::Bluetooth => {
                            if !self.wireless_snap.bt_available {
                                "未检测到蓝牙适配器"
                            } else if !self.wireless_snap.bt_enabled {
                                "蓝牙已关闭"
                            } else if self.wireless_snap.bt_busy {
                                "正在刷新..."
                            } else {
                                "无已配对设备"
                            }
                        }
                        SubmenuKind::Brightness => {
                            if self.brightness_snap.busy {
                                "正在检测显示器..."
                            } else {
                                "未检测到显示器"
                            }
                        }
                        _ => "",
                    };
                    if !msg.is_empty() {
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            msg,
                            RectF::new(r.left + 12.0 * s, r.top, r.right - 12.0 * s, r.bottom),
                            fmt,
                            muted_fg,
                        );
                    }
                }
            }
        }

        for div in &layout.dividers {
            draw_divider(&self.rt, *div, s, self.light_glass);
        }

        if let Some(vp) = layout.list_viewport {
            let max_visible = layout.list_max_rows.max(1);
            if layout.list_total_items > max_visible {
                let top = vp.top + 4.0 * s;
                let bottom = vp.bottom - 4.0 * s;
                let track_h = (bottom - top).max(8.0);
                let ratio = (max_visible as f32) / (layout.list_total_items as f32);
                let thumb_h = (track_h * ratio).max(16.0 * s);
                let max_scroll = (layout.list_total_items - max_visible) as f32;
                let scroll_t = if max_scroll > 0.0 {
                    (layout.list_scroll as f32) / max_scroll
                } else {
                    0.0
                };
                let thumb_top = top + (track_h - thumb_h) * scroll_t;
                let bar_rr = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: vp.right - 3.0 * s,
                        top: thumb_top,
                        right: vp.right - 0.5 * s,
                        bottom: thumb_top + thumb_h,
                    },
                    radiusX: 1.5 * s,
                    radiusY: 1.5 * s,
                };
                let mut col = muted_fg;
                col.a *= 0.65;
                if let Ok(b) = self.rt.CreateSolidColorBrush(&col, None) {
                    self.rt.FillRoundedRectangle(&bar_rr, &b);
                }
            }
        }
    }

    unsafe fn draw_chevron(&self, cx: f32, cy: f32, s: f32, brush: &ID2D1SolidColorBrush) {
        let st = self.round_stroke.as_ref();
        let dir = if self.open_left { -1.0 } else { 1.0 };
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx - 1.6 * s * dir,
                y: cy - 3.2 * s,
            },
            D2D_POINT_2F {
                x: cx + 1.8 * s * dir,
                y: cy,
            },
            brush,
            1.35 * s,
            st,
        );
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx + 1.8 * s * dir,
                y: cy,
            },
            D2D_POINT_2F {
                x: cx - 1.6 * s * dir,
                y: cy + 3.2 * s,
            },
            brush,
            1.35 * s,
            st,
        );
    }

    unsafe fn draw_wifi_signal_icon(
        &self,
        cx: f32,
        cy: f32,
        s: f32,
        quality: u8,
        active: D2D1_COLOR_F,
        dim: D2D1_COLOR_F,
    ) {
        let base_y = cy + 3.6 * s;
        let lit_bars = if quality >= 70 {
            3
        } else if quality >= 40 {
            2
        } else if quality > 0 {
            1
        } else {
            0
        };
        if let Ok(b0) = self.rt.CreateSolidColorBrush(&active, None) {
            self.rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: base_y },
                    radiusX: 1.2 * s,
                    radiusY: 1.2 * s,
                },
                &b0,
            );
        }
        let st = self.round_stroke.as_ref();
        for (idx, radius) in [(1, 3.2 * s), (2, 5.4 * s), (3, 7.6 * s)] {
            let col = if idx <= lit_bars { active } else { dim };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&col, None) {
                if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                    if let Ok(sink) = geo.Open() {
                        let dx = radius * 0.72;
                        let dy = radius * 0.68;
                        sink.BeginFigure(
                            D2D_POINT_2F {
                                x: cx - dx,
                                y: base_y - dy,
                            },
                            D2D1_FIGURE_BEGIN_HOLLOW,
                        );
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: D2D_POINT_2F {
                                x: cx + dx,
                                y: base_y - dy,
                            },
                            size: D2D_SIZE_F {
                                width: radius,
                                height: radius,
                            },
                            rotationAngle: 0.0,
                            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                            arcSize: D2D1_ARC_SIZE_SMALL,
                        });
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        let _ = sink.Close();
                        self.rt.DrawGeometry(&geo, &b, 1.25 * s, st);
                    }
                }
            }
        }
    }

    unsafe fn draw_bluetooth_icon(
        &self,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let st = self.round_stroke.as_ref();
        let pts = [
            D2D_POINT_2F {
                x: cx - 2.6 * s,
                y: cy - 2.3 * s,
            },
            D2D_POINT_2F {
                x: cx + 2.8 * s,
                y: cy + 2.4 * s,
            },
            D2D_POINT_2F {
                x: cx,
                y: cy + 4.8 * s,
            },
            D2D_POINT_2F {
                x: cx,
                y: cy - 4.8 * s,
            },
            D2D_POINT_2F {
                x: cx + 2.8 * s,
                y: cy - 2.4 * s,
            },
            D2D_POINT_2F {
                x: cx - 2.6 * s,
                y: cy + 2.3 * s,
            },
        ];
        for seg in pts.windows(2) {
            self.rt.DrawLine(seg[0], seg[1], brush, 1.25 * s, st);
        }
    }

    unsafe fn draw_primary_icon(
        &self,
        item: PrimaryItem,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let st = self.round_stroke.as_ref();
        match item {
            PrimaryItem::WiFi => {
                self.rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: cx,
                            y: cy + 3.4 * s,
                        },
                        radiusX: 1.2 * s,
                        radiusY: 1.2 * s,
                    },
                    brush,
                );
                for radius in [3.4 * s, 6.2 * s] {
                    if let Ok(geo) = self.d2d_factory.CreatePathGeometry() {
                        if let Ok(sink) = geo.Open() {
                            let dx = radius * 0.72;
                            let dy = radius * 0.68;
                            sink.BeginFigure(
                                D2D_POINT_2F {
                                    x: cx - dx,
                                    y: cy + 3.4 * s - dy,
                                },
                                D2D1_FIGURE_BEGIN_HOLLOW,
                            );
                            sink.AddArc(&D2D1_ARC_SEGMENT {
                                point: D2D_POINT_2F {
                                    x: cx + dx,
                                    y: cy + 3.4 * s - dy,
                                },
                                size: D2D_SIZE_F {
                                    width: radius,
                                    height: radius,
                                },
                                rotationAngle: 0.0,
                                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                                arcSize: D2D1_ARC_SIZE_SMALL,
                            });
                            sink.EndFigure(D2D1_FIGURE_END_OPEN);
                            let _ = sink.Close();
                            self.rt.DrawGeometry(&geo, brush, 1.3 * s, st);
                        }
                    }
                }
            }
            PrimaryItem::Bluetooth => {
                self.draw_bluetooth_icon(cx, cy, s, brush);
            }
            PrimaryItem::Brightness => {
                self.rt.DrawEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: cx, y: cy },
                        radiusX: 2.5 * s,
                        radiusY: 2.5 * s,
                    },
                    brush,
                    1.25 * s,
                    st,
                );
                let inner_r = 4.2 * s;
                let outer_r = 5.4 * s;
                for step in 0..8 {
                    let angle = (step as f32) * std::f32::consts::FRAC_PI_4;
                    let (sin_a, cos_a) = angle.sin_cos();
                    self.rt.DrawLine(
                        D2D_POINT_2F {
                            x: cx + inner_r * cos_a,
                            y: cy + inner_r * sin_a,
                        },
                        D2D_POINT_2F {
                            x: cx + outer_r * cos_a,
                            y: cy + outer_r * sin_a,
                        },
                        brush,
                        1.25 * s,
                        st,
                    );
                }
            }
            PrimaryItem::LeftClick => {
                let p0 = D2D_POINT_2F {
                    x: cx - 3.2 * s,
                    y: cy - 4.4 * s,
                };
                let p1 = D2D_POINT_2F {
                    x: cx - 3.2 * s,
                    y: cy + 4.4 * s,
                };
                let p2 = D2D_POINT_2F {
                    x: cx - 0.6 * s,
                    y: cy + 1.9 * s,
                };
                let p3 = D2D_POINT_2F {
                    x: cx + 3.2 * s,
                    y: cy + 1.5 * s,
                };
                self.rt.DrawLine(p0, p1, brush, 1.3 * s, st);
                self.rt.DrawLine(p1, p2, brush, 1.3 * s, st);
                self.rt.DrawLine(p2, p3, brush, 1.3 * s, st);
                self.rt.DrawLine(p3, p0, brush, 1.3 * s, st);
            }
            PrimaryItem::Display => {
                for dx in [-3.1 * s, 1.1 * s] {
                    for dy in [-3.1 * s, 1.1 * s] {
                        let rr = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: cx + dx,
                                top: cy + dy,
                                right: cx + dx + 3.2 * s,
                                bottom: cy + dy + 3.2 * s,
                            },
                            radiusX: 0.9 * s,
                            radiusY: 0.9 * s,
                        };
                        self.rt.DrawRoundedRectangle(&rr, brush, 1.15 * s, st);
                    }
                }
            }
            PrimaryItem::Settings => {
                draw_settings_icon(&self.rt, cx, cy, s, brush);
            }
        }
    }
}

unsafe extern "system" fn monitor_popup_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MonitorPopupInner;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *ptr;

    match msg {
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                state.begin_close();
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                state.begin_close();
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let _ = SetCursor(state.arrow_cursor);
            LRESULT(1)
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
            if state.point_inside_cards(pt.x as f32, pt.y as f32) {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        WM_TIMER => {
            if wparam.0 == TIMER_POPUP_FLOW_ID && IsWindowVisible(hwnd).as_bool() {
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
            if let Some(idx) = state.dragging_brightness {
                if state.apply_brightness_drag(idx, x) {
                    let _ = state.render();
                }
                return LRESULT(0);
            }
            let next_hover = state.hit_test(x, y);
            let sub_changed = match next_hover {
                Some(HoverTarget::Primary(item)) => state.set_active_submenu(item.submenu()),
                Some(HoverTarget::Sub(_)) | None => false,
            };
            if next_hover != state.hover || sub_changed {
                state.hover = next_hover;
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            state.tracking_leave = false;
            if state.anim_phase != PopupAnimPhase::Closing
                && state.dragging_brightness.is_none()
                && state.hover.is_some()
            {
                state.hover = None;
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if state.dragging_brightness.take().is_some() {
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if state.anim_phase == PopupAnimPhase::Closing {
                return LRESULT(0);
            }
            let delta = ((wparam.0 >> 16) as i16) as i32;
            let steps = -(delta / 120);
            if state.active_sub == Some(SubmenuKind::Brightness) && delta != 0 {
                let mut pt = POINT {
                    x: i32::from((lparam.0 & 0xFFFF) as i16),
                    y: i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
                };
                let _ = ScreenToClient(hwnd, &mut pt);
                let wheel_hover = state.hit_test(pt.x as f32, pt.y as f32);
                if let Some(HoverTarget::Sub(
                    SubHoverTarget::BrightnessRow(idx) | SubHoverTarget::BrightnessSlider(idx),
                )) = wheel_hover
                {
                    if state.step_brightness_wheel(idx, delta as i16) {
                        let _ = state.render();
                    }
                    return LRESULT(0);
                }
            }
            if steps != 0 {
                let max_wifi = state
                    .wireless_snap
                    .wifi_networks
                    .len()
                    .saturating_sub(MAX_LIST_ROWS);
                let max_bt = state
                    .wireless_snap
                    .bt_devices
                    .len()
                    .saturating_sub(MAX_LIST_ROWS);
                let max_bright = state
                    .brightness_snap
                    .displays
                    .len()
                    .saturating_sub(MAX_BRIGHTNESS_ROWS);
                let mut changed = false;
                match state.active_sub {
                    Some(SubmenuKind::WiFi) if max_wifi > 0 => {
                        let next = (state.wifi_scroll as isize + steps as isize)
                            .clamp(0, max_wifi as isize) as usize;
                        if next != state.wifi_scroll {
                            state.wifi_scroll = next;
                            changed = true;
                        }
                    }
                    Some(SubmenuKind::Bluetooth) if max_bt > 0 => {
                        let next = (state.bt_scroll as isize + steps as isize)
                            .clamp(0, max_bt as isize) as usize;
                        if next != state.bt_scroll {
                            state.bt_scroll = next;
                            changed = true;
                        }
                    }
                    Some(SubmenuKind::Brightness) if max_bright > 0 => {
                        let next = (state.brightness_scroll as isize + steps as isize)
                            .clamp(0, max_bright as isize) as usize;
                        if next != state.brightness_scroll {
                            state.brightness_scroll = next;
                            changed = true;
                        }
                    }
                    _ => {}
                }
                if changed {
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if state.anim_phase == PopupAnimPhase::Closing {
                return LRESULT(0);
            }
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            if !state.point_inside_cards(x, y) {
                state.begin_close();
                return LRESULT(0);
            }
            let next_hover = state.hit_test(x, y);
            state.hover = next_hover;
            if let Some(HoverTarget::Sub(SubHoverTarget::BrightnessSlider(idx))) = next_hover {
                state.dragging_brightness = Some(idx);
                let _ = SetCapture(hwnd);
                let _ = state.apply_brightness_drag(idx, x);
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if state.anim_phase == PopupAnimPhase::Closing {
                return LRESULT(0);
            }
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            if state.dragging_brightness.take().is_some() {
                let _ = ReleaseCapture();
                state.hover = state.hit_test(x, y);
                let _ = state.render();
                return LRESULT(0);
            }
            match state.hit_test(x, y) {
                Some(HoverTarget::Primary(item)) => {
                    if state.set_active_submenu(item.submenu()) {
                        let _ = state.render();
                    }
                }
                Some(HoverTarget::Sub(target)) => {
                    handle_sub_click(state, target);
                }
                None => {
                    if !state.point_inside_cards(x, y) {
                        state.begin_close();
                    }
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn handle_sub_click(state: &mut MonitorPopupInner, target: SubHoverTarget) {
    match target {
        SubHoverTarget::WifiToggle => {
            state.wireless.toggle_wifi_radio();
        }
        SubHoverTarget::WifiNetwork(idx) => {
            let item_opt: Option<WifiNetworkItem> =
                state.wireless_snap.wifi_networks.get(idx).cloned();
            if let Some(item) = item_opt {
                let will_open_settings = !item.has_profile;
                state.wireless.activate_wifi_network(item);
                if will_open_settings {
                    state.begin_close();
                }
            }
        }
        SubHoverTarget::WifiMoreSettings => {
            open_shell_target("ms-settings:network-wifi");
            state.begin_close();
        }
        SubHoverTarget::BtToggle => {
            state.wireless.toggle_bluetooth_radio();
        }
        SubHoverTarget::BtDevice(idx) => {
            if let Some(dev) = state.wireless_snap.bt_devices.get(idx).cloned() {
                let will_open_settings = dev.kind != BtDeviceKind::Classic || dev.address == 0;
                state.wireless.activate_bluetooth_device(dev);
                if will_open_settings {
                    state.begin_close();
                }
            }
        }
        SubHoverTarget::BtMoreSettings => {
            open_shell_target("ms-settings:bluetooth");
            state.begin_close();
        }
        SubHoverTarget::BrightnessRow(_) | SubHoverTarget::BrightnessSlider(_) => {}
        SubHoverTarget::BrightnessMoreSettings => {
            open_shell_target("ms-settings:display");
            state.begin_close();
        }
        SubHoverTarget::LeftClickPreset(action) => {
            if action == LeftClickAction::CustomApp {
                state.hide_immediately();
                unsafe {
                    let _ = PostMessageW(
                        state.owner_hwnd,
                        WM_APP_MONITOR_PICK_CUSTOM_APP,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            } else {
                if let Ok(mut guard) = state.cfg.lock() {
                    guard.left_click = action;
                    save_config(&state.config_path, &guard);
                }
                unsafe {
                    let _ = PostMessageW(
                        state.owner_hwnd,
                        WM_APP_MONITOR_CONFIG_CHANGED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
                state.begin_close();
            }
        }
        SubHoverTarget::DisplayMetric(item) => {
            let changed = if let Ok(mut guard) = state.cfg.lock() {
                if guard.toggle_metric(item) {
                    save_config(&state.config_path, &guard);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if changed {
                unsafe {
                    let _ = PostMessageW(
                        state.owner_hwnd,
                        WM_APP_MONITOR_CONFIG_CHANGED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
                let _ = state.render();
            }
        }
        SubHoverTarget::SettingLock => {
            unsafe {
                let _ = PostMessageW(
                    state.owner_hwnd,
                    WM_APP_MONITOR_TOGGLE_LOCK,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            state.begin_close();
        }
        SubHoverTarget::SettingReset => {
            unsafe {
                let _ = PostMessageW(
                    state.owner_hwnd,
                    WM_APP_MONITOR_RESET_POS,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            state.begin_close();
        }
        SubHoverTarget::SettingClose => {
            state.hide_immediately();
            unsafe {
                let _ = PostMessageW(
                    state.owner_hwnd,
                    WM_APP_MONITOR_CLOSE_SELF,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }
}
