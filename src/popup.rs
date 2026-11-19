use crate::arbitration::{PlaybackState, SelectionMode};
use crate::audio_devices::{
    enumerate_audio_endpoints, set_default_audio_endpoint, AudioEndpointItem,
};
use crate::color::{adjust_accent_for_theme, gaussian_blur_bgra, hsl_to_rgb, rgb_to_hsl, Rgb};
use crate::layout::RectF;
use crate::render::{
    create_round_cap_stroke, d2d_color_rgb, d2d_rect, draw_speaker_glyph, draw_volume_slider,
    identity_matrix, LayeredSurface,
};
use crate::smtc::{MediaCover, SourceItem};
use crate::taskbar::set_autostart_enabled;
use crate::volume::{
    enumerate_app_volumes, format_volume_pct, slider_ratio_from_x, step_volume_by_wheel,
    AppVolumeEntry, MasterVolumeControl,
};
use std::collections::HashMap;
use std::mem::{size_of, ManuallyDrop};
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use windows::core::{w, Interface};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1Layer,
    ID2D1PathGeometry1, ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteInlineObject, IDWriteTextFormat, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW,
    MonitorFromRect, ReleaseDC, ScreenToClient, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, IsWindowVisible,
    KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SetCursor, SetForegroundWindow,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, HCURSOR, HTCLIENT,
    HWND_TOPMOST, HTTRANSPARENT, IDC_ARROW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
    SW_SHOW, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub const WM_APP_SOURCE_SELECTED: u32 = WM_APP + 2;
pub const WM_APP_POPUP_CLOSED: u32 = WM_APP + 3;
pub const WM_APP_SETTING_ACTION: u32 = WM_APP + 4;
pub const WM_APP_AUDIO_OUTPUT_CHANGED: u32 = WM_APP + 5;

pub const SETTING_TOGGLE_LOCK: usize = 1;
pub const SETTING_RESET_POS: usize = 2;
pub const SETTING_EXIT: usize = 3;

const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const POPUP_CLASS_NAME: windows::core::PCWSTR = w!("MediaBarSourcePopupWnd");

const TIMER_POPUP_FLOW_ID: usize = 1;
const POPUP_ANIM_INTERVAL_MS: u32 = 10;
const POPUP_FLOW_INTERVAL_MS: u32 = 33;
const SILK_UPDATE_INTERVAL_MS: u64 = 32;
const VOL_RELOAD_FRESH_MS: u64 = 2500;
const CLOSE_ANIM_DUR_S: f32 = 0.130;
const OPEN_SPRING_STIFFNESS: f32 = 440.0;
const OPEN_SPRING_DAMPING: f32 = 30.5;
const SUB_WIDTH_STIFFNESS: f32 = 500.0;
const SUB_WIDTH_DAMPING: f32 = 32.0;
const SUB_VERT_STIFFNESS: f32 = 460.0;
const SUB_VERT_DAMPING: f32 = 31.5;
const SILK_W: u32 = 64;
const SILK_H: u32 = 48;

const SHADOW_PAD_DIP: f32 = 14.0;
const FOOT_MENISCUS_DIP: f32 = 10.0;
const STEP_MENISCUS_DIP: f32 = 13.5;
const INNER_PAD_DIP: f32 = 8.0;
const PANEL_RADIUS_DIP: f32 = 13.0;
const PRIMARY_WIDTH_DIP: f32 = 148.0;
const PRIMARY_ROW_H_DIP: f32 = 34.0;
const SUBMENU_GAP_DIP: f32 = 0.0;

const SOURCES_SUB_WIDTH_DIP: f32 = 280.0;
const VOLUME_SUB_WIDTH_DIP: f32 = 276.0;
const DEVICES_SUB_WIDTH_DIP: f32 = 264.0;
const SETTINGS_SUB_WIDTH_DIP: f32 = 172.0;

const AUTO_ROW_H_DIP: f32 = 34.0;
const SOURCE_ROW_H_DIP: f32 = 46.0;
const VOLUME_ROW_H_DIP: f32 = 44.0;
const DEVICE_ROW_H_DIP: f32 = 42.0;
const SETTING_ROW_H_DIP: f32 = 34.0;
const ROW_GAP_DIP: f32 = 4.0;
const DIVIDER_H_DIP: f32 = 9.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimaryCategory {
    DisplayContent,
    VolumeControl,
    PlaybackDevices,
    RecordingDevices,
    Settings,
}

impl PrimaryCategory {
    const ALL: [Self; 5] = [
        Self::DisplayContent,
        Self::VolumeControl,
        Self::PlaybackDevices,
        Self::RecordingDevices,
        Self::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::DisplayContent => "显示内容",
            Self::VolumeControl => "音量控制",
            Self::PlaybackDevices => "播放设备",
            Self::RecordingDevices => "录音设备",
            Self::Settings => "设置",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HoverTarget {
    None,
    Primary(PrimaryCategory),
    SubAuto,
    SubSource(usize),
    SubVolMasterMute,
    SubVolMasterSlider,
    SubVolMasterRow,
    SubVolAppMute(usize),
    SubVolAppSlider(usize),
    SubVolAppRow(usize),
    SubPlayback(usize),
    SubRecording(usize),
    SubSettingLock,
    SubSettingAutostart,
    SubSettingResetPos,
    SubSettingExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SliderTarget {
    Master,
    App(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopupAnimPhase {
    Opening,
    Open,
    Closing,
}

/// Critically/lightly damped 1D second-order spring preserving velocity across target changes.
#[derive(Clone, Copy, Debug)]
struct Spring1D {
    pos: f32,
    vel: f32,
}

impl Spring1D {
    fn still(pos: f32) -> Self {
        Self { pos, vel: 0.0 }
    }

    fn new(pos: f32, vel: f32) -> Self {
        Self { pos, vel }
    }

    fn step(&mut self, target: f32, stiffness: f32, damping: f32, dt: f32) {
        const SUB_DT: f32 = 0.004;
        let mut rem = dt.clamp(0.0, 0.1);
        while rem > 0.0 {
            let h = rem.min(SUB_DT);
            let acc = (target - self.pos) * stiffness - self.vel * damping;
            self.vel += acc * h;
            self.pos += self.vel * h;
            rem -= h;
        }
    }

    fn is_settled(&self, target: f32, pos_eps: f32, vel_eps: f32) -> bool {
        (target - self.pos).abs() <= pos_eps && self.vel.abs() <= vel_eps
    }
}

#[derive(Clone, Copy, Debug)]
struct SubSpringState {
    top: Spring1D,
    bottom: Spring1D,
    width: Spring1D,
    fade_elapsed: f32,
    prev_category: Option<PrimaryCategory>,
    is_budding: bool,
}

#[derive(Clone)]
struct CachedTextFormats {
    dpi: u32,
    title_fmt: IDWriteTextFormat,
    sub_fmt: IDWriteTextFormat,
    badge_fmt: IDWriteTextFormat,
    pct_fmt: IDWriteTextFormat,
}

#[derive(Clone, Copy, Debug)]
struct VolumeRowGeom {
    row_rect: RectF,
    mute_rect: RectF,
    track_rect: RectF,
    slider_hit_rect: RectF,
    label_rect: RectF,
    pct_rect: RectF,
}

struct PopupInner {
    hwnd: HWND,
    main_hwnd: HWND,
    d2d_factory: ID2D1Factory1,
    dwrite_factory: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    layer: ID2D1Layer,
    content_layer: ID2D1Layer,
    thumb_layer: ID2D1Layer,
    round_stroke: Option<ID2D1StrokeStyle>,
    dpi: u32,
    fallback_accent: Rgb,
    sources: Vec<SourceItem>,
    mode: SelectionMode,
    active_id: Option<String>,
    master_vol_ctrl: Option<MasterVolumeControl>,
    master_vol: f32,
    master_muted: bool,
    app_volumes: Vec<AppVolumeEntry>,
    dragging_slider: Option<SliderTarget>,
    playback_devices: Vec<AudioEndpointItem>,
    recording_devices: Vec<AudioEndpointItem>,
    locked: bool,
    autostart: bool,
    active_category: Option<PrimaryCategory>,
    hover_target: HoverTarget,
    open_left: bool,
    tracking_leave: bool,
    light_glass: bool,
    pill_overlap_px: i32,
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
    backdrop_bmp: Option<ID2D1Bitmap>,
    silk_bmp: Option<ID2D1Bitmap>,
    silk_buf: Vec<u8>,
    flow_phase: f64,
    halo_colors: [(f32, f32, f32); 2],
    ui_accent: (f32, f32, f32),
    halo_colors_ready: bool,
    last_flow_tick: Instant,
    last_silk_tick: Option<Instant>,
    last_color_tick: Instant,
    last_vol_reload: Option<Instant>,
    anim_phase: PopupAnimPhase,
    open_spring: Spring1D,
    open_elapsed: f32,
    close_progress: f32,
    close_started_at: Option<Instant>,
    sub_anim: Option<SubSpringState>,
    timer_interval_ms: u32,
    cached_text_formats: Option<CachedTextFormats>,
    cover_bmps: HashMap<u64, ID2D1Bitmap>,
    arrow_cursor: HCURSOR,
}

pub struct SourcePopup {
    hwnd: HWND,
    inner: Box<PopupInner>,
}

impl SourcePopup {
    pub fn new(
        main_hwnd: HWND,
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
    ) -> windows::core::Result<Self> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let arrow_cursor = LoadCursorW(None, IDC_ARROW)?;
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(popup_wnd_proc),
                hInstance: hinst.into(),
                hCursor: arrow_cursor,
                lpszClassName: POPUP_CLASS_NAME,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                POPUP_CLASS_NAME,
                w!("MediaBar Menu"),
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
            let thumb_layer = rt.CreateLayer(None)?;
            let surface = LayeredSurface::new()?;
            let round_stroke = create_round_cap_stroke(&d2d_factory);

            let mut inner = Box::new(PopupInner {
                hwnd,
                main_hwnd,
                d2d_factory,
                dwrite_factory,
                rt,
                surface,
                layer,
                content_layer,
                thumb_layer,
                round_stroke,
                dpi: 96,
                fallback_accent: Rgb::new(0, 120, 212),
                sources: Vec::new(),
                mode: SelectionMode::Auto,
                active_id: None,
                master_vol_ctrl: None,
                master_vol: 1.0,
                master_muted: false,
                app_volumes: Vec::new(),
                dragging_slider: None,
                playback_devices: Vec::new(),
                recording_devices: Vec::new(),
                locked: true,
                autostart: false,
                active_category: None,
                hover_target: HoverTarget::None,
                open_left: false,
                tracking_leave: false,
                light_glass: false,
                pill_overlap_px: 4,
                win_x: 0,
                win_y: 0,
                win_w: 1,
                win_h: 1,
                backdrop_bmp: None,
                silk_bmp: None,
                silk_buf: vec![0u8; (SILK_W * SILK_H * 4) as usize],
                flow_phase: 0.0,
                halo_colors: [(0.35, 0.58, 0.90); 2],
                ui_accent: (0.0, 120.0 / 255.0, 212.0 / 255.0),
                halo_colors_ready: false,
                last_flow_tick: Instant::now(),
                last_silk_tick: None,
                last_color_tick: Instant::now(),
                last_vol_reload: None,
                anim_phase: PopupAnimPhase::Open,
                open_spring: Spring1D::still(1.0),
                open_elapsed: 1.0,
                close_progress: 0.0,
                close_started_at: None,
                sub_anim: None,
                timer_interval_ms: 0,
                cached_text_formats: None,
                cover_bmps: HashMap::new(),
                arrow_cursor,
            });

            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (&mut *inner as *mut PopupInner) as isize,
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
                .is_some_and(|t| t.elapsed() < Duration::from_millis(240))
    }

    pub fn hide_animated(&mut self) {
        self.inner.begin_close();
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

    pub fn attached_bar_span_screen_x(&self) -> Option<(f32, f32)> {
        if !self.is_visible() {
            return None;
        }
        let primary = self.inner.primary_card_rect();
        let foot_r = FOOT_MENISCUS_DIP * self.inner.scale();
        Some((
            self.inner.win_x as f32 + primary.left - foot_r,
            self.inner.win_x as f32 + primary.right + foot_r,
        ))
    }

    pub fn source_id_at(&self, idx: usize) -> Option<String> {
        self.inner.sources.get(idx).map(|s| s.id.clone())
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.inner.reset_transient_state();
    }

    pub fn update_live_sources(
        &mut self,
        sources: Vec<SourceItem>,
        mode: SelectionMode,
        active_id: Option<String>,
    ) {
        self.inner.sources = sources;
        self.inner.mode = mode;
        self.inner.active_id = active_id;
        if self.is_visible() {
            let _ = self.inner.render();
        }
    }

    pub fn set_system_theme(&mut self, light_taskbar: bool, fallback_accent: Rgb) {
        let changed = self.inner.light_glass != light_taskbar
            || self.inner.fallback_accent != fallback_accent;
        self.inner.light_glass = light_taskbar;
        self.inner.fallback_accent = fallback_accent;
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
        anchor_rect: RECT,
        click_screen_x: i32,
        dpi: u32,
        light_taskbar: bool,
        fallback_accent: Rgb,
        sources: Vec<SourceItem>,
        mode: SelectionMode,
        active_id: Option<String>,
        locked: bool,
        autostart: bool,
        pill_overlap_px: i32,
    ) {
        self.inner.dpi = dpi.max(96);
        self.inner.light_glass = light_taskbar;
        self.inner.fallback_accent = fallback_accent;
        self.inner.sources = sources;
        self.inner.mode = mode;
        self.inner.active_id = active_id;
        self.inner.locked = locked;
        self.inner.autostart = autostart;
        self.inner.pill_overlap_px = pill_overlap_px.max(0);
        self.inner.playback_devices = enumerate_audio_endpoints(false);
        self.inner.recording_devices = enumerate_audio_endpoints(true);
        self.inner.reload_volumes();
        self.inner.dragging_slider = None;
        self.inner.active_category = None;
        self.inner.hover_target = HoverTarget::None;
        // Reset cached height before sizing so stale height from a previous open never clips items.
        self.inner.win_h = 0;

        let s = self.inner.scale();
        let side_pad = (SHADOW_PAD_DIP * s).round() as i32;
        let top_pad = side_pad;
        let primary_w = (PRIMARY_WIDTH_DIP * s).round() as i32;
        let sub_gap = (SUBMENU_GAP_DIP * s).round() as i32;
        let max_sub_w = (SOURCES_SUB_WIDTH_DIP.max(VOLUME_SUB_WIDTH_DIP) * s).round() as i32;

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

        let edge_margin = (FOOT_MENISCUS_DIP * s + 2.0 * s).round() as i32;
        let top_margin = (6.0 * s).round() as i32;
        let win_w = side_pad * 2 + primary_w + sub_gap + max_sub_w;
        let win_h =
            (top_pad + max_reach).min((anchor_rect.top - work.top - top_margin).max(180));

        // Keep primary card + bottom surface-tension feet strictly within the MediaBar pill's flat top edge.
        let pill_inset = ((7.5 + FOOT_MENISCUS_DIP + 10.0) * s).round() as i32;
        let bar_min = anchor_rect.left + pill_inset;
        let bar_max = (anchor_rect.right - pill_inset - primary_w).max(bar_min);
        let work_min = work.left + edge_margin;
        let work_max = (work.right - primary_w - edge_margin).max(work_min);
        let min_x = bar_min.max(work_min).min(work_max);
        let max_x = bar_max.min(work_max).max(min_x);
        let ideal_primary_left =
            (click_screen_x - (24.0 * s).round() as i32).clamp(min_x, max_x);

        let open_left =
            ideal_primary_left + primary_w + sub_gap + max_sub_w + edge_margin > work.right;
        self.inner.open_left = open_left;

        let win_x = if open_left {
            let right_edge = ideal_primary_left + primary_w + side_pad;
            (right_edge - win_w).clamp(
                work.left + edge_margin - side_pad,
                (work.right - edge_margin + side_pad - win_w)
                    .max(work.left + edge_margin - side_pad),
            )
        } else {
            (ideal_primary_left - side_pad).clamp(
                work.left + edge_margin - side_pad,
                (work.right - edge_margin + side_pad - win_w)
                    .max(work.left + edge_margin - side_pad),
            )
        };

        // Anchor bottom edge seamlessly onto the MediaBar pill top edge (0px gap)
        let win_y = (anchor_rect.top - win_h).max(work.top);

        self.inner.win_x = win_x;
        self.inner.win_y = win_y;
        self.inner.win_w = win_w;
        self.inner.win_h = win_h;

        self.inner.backdrop_bmp =
            capture_blurred_desktop_bitmap(&self.inner.rt, win_x, win_y, win_w, win_h);
        self.inner.halo_colors_ready = false;
        self.inner.last_silk_tick = None;
        self.inner.last_color_tick = Instant::now();
        self.inner.anim_phase = PopupAnimPhase::Opening;
        self.inner.open_spring = Spring1D::new(0.08, 1.6);
        self.inner.open_elapsed = 0.0;
        self.inner.close_progress = 0.0;
        self.inner.close_started_at = None;
        self.inner.sub_anim = None;

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
        // Stamp clock after window presentation so first WM_TIMER does not absorb SetWindowPos/DWM latency.
        self.inner.last_flow_tick = Instant::now();
        self.inner.last_color_tick = self.inner.last_flow_tick;
        self.inner.sync_timer_rate();
    }
}

impl Drop for SourcePopup {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl PopupInner {
    fn scale(&self) -> f32 {
        (self.dpi.max(96) as f32) / 96.0
    }

    fn reset_transient_state(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
        }
        self.timer_interval_ms = 0;
        self.backdrop_bmp = None;
        self.halo_colors_ready = false;
        self.last_silk_tick = None;
        self.last_vol_reload = None;
        self.anim_phase = PopupAnimPhase::Open;
        self.open_spring = Spring1D::still(1.0);
        self.open_elapsed = 1.0;
        self.close_progress = 0.0;
        self.close_started_at = None;
        self.sub_anim = None;
        self.active_category = None;
        self.hover_target = HoverTarget::None;
        self.dragging_slider = None;
        self.app_volumes.clear();
        self.master_vol_ctrl = None;
    }

    fn needs_fast_anim(&self) -> bool {
        matches!(
            self.anim_phase,
            PopupAnimPhase::Opening | PopupAnimPhase::Closing
        ) || self.sub_anim.is_some()
    }

    fn sync_timer_rate(&mut self) {
        let desired = if self.needs_fast_anim() {
            POPUP_ANIM_INTERVAL_MS
        } else {
            POPUP_FLOW_INTERVAL_MS
        };
        if self.timer_interval_ms != desired {
            unsafe {
                SetTimer(self.hwnd, TIMER_POPUP_FLOW_ID, desired, None);
            }
            self.timer_interval_ms = desired;
        }
    }

    fn begin_close(&mut self) {
        if self.anim_phase == PopupAnimPhase::Closing
            || !unsafe { IsWindowVisible(self.hwnd).as_bool() }
        {
            return;
        }
        if self.dragging_slider.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        self.hover_target = HoverTarget::None;
        self.anim_phase = PopupAnimPhase::Closing;
        self.close_progress = 0.0;
        self.close_started_at = Some(Instant::now());
        self.last_flow_tick = Instant::now();
        self.sync_timer_rate();
        let _ = self.render();
    }

    fn finish_close(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.reset_transient_state();
        self.close_started_at = Some(Instant::now());
        unsafe {
            let _ = PostMessageW(self.main_hwnd, WM_APP_POPUP_CLOSED, WPARAM(0), LPARAM(0));
        }
    }

    fn advance_animations(&mut self, dt: f32) -> bool {
        match self.anim_phase {
            PopupAnimPhase::Opening => {
                self.open_elapsed += dt;
                self.open_spring
                    .step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, dt);
                if (self.open_spring.is_settled(1.0, 0.0015, 0.015) && self.open_elapsed >= 0.22)
                    || self.open_elapsed >= 0.55
                {
                    self.open_spring = Spring1D::still(1.0);
                    self.anim_phase = PopupAnimPhase::Open;
                }
            }
            PopupAnimPhase::Open => {}
            PopupAnimPhase::Closing => {
                self.close_progress = (self.close_progress + dt / CLOSE_ANIM_DUR_S).min(1.0);
                if self.close_progress >= 1.0 {
                    return true;
                }
            }
        }

        if let Some(cat) = self.active_category {
            if self.sub_anim.is_some() {
                let target = self.sub_card_rect(cat);
                let s = self.scale();
                if let Some(ref mut sa) = self.sub_anim {
                    sa.fade_elapsed += dt;
                    sa.top
                        .step(target.top, SUB_VERT_STIFFNESS, SUB_VERT_DAMPING, dt);
                    sa.bottom
                        .step(target.bottom, SUB_VERT_STIFFNESS, SUB_VERT_DAMPING, dt);
                    sa.width
                        .step(target.width(), SUB_WIDTH_STIFFNESS, SUB_WIDTH_DAMPING, dt);
                    let pos_eps = 0.35 * s;
                    let vel_eps = 4.0 * s;
                    let converged = sa.top.is_settled(target.top, pos_eps, vel_eps)
                        && sa.bottom.is_settled(target.bottom, pos_eps, vel_eps)
                        && sa.width.is_settled(target.width(), pos_eps, vel_eps);
                    if (converged && sa.fade_elapsed >= 0.155) || sa.fade_elapsed >= 0.55 {
                        self.sub_anim = None;
                    }
                }
            }
        }

        false
    }

    fn current_animated_sub_rect(&self) -> Option<RectF> {
        let cat = self.active_category?;
        let s = self.scale();
        let target_primary = self.primary_card_rect();
        let anim_primary = animated_primary_rect(
            target_primary,
            self.anim_phase,
            self.open_spring.pos,
            self.close_progress,
            s,
        );
        let target_sub = self.sub_card_rect(cat);
        let seam_x = if self.open_left {
            anim_primary.left
        } else {
            anim_primary.right
        };
        let bottom_limit =
            (target_primary.bottom - self.primary_overlap_px()).max(target_primary.top + 58.0 * s);
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

    fn reload_volumes(&mut self) {
        self.master_vol_ctrl = MasterVolumeControl::open_default();
        if let Some((vol, muted)) = self.master_vol_ctrl.as_ref().and_then(|c| c.get_state()) {
            self.master_vol = vol;
            self.master_muted = muted;
        } else {
            self.master_vol = 1.0;
            self.master_muted = false;
        }
        self.app_volumes = enumerate_app_volumes();
        self.last_vol_reload = Some(Instant::now());
    }

    fn set_master_volume_ratio(&mut self, ratio: f32) {
        let clamped = ratio.clamp(0.0, 1.0);
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        let unmute = self.master_muted && clamped > 0.0;
        self.master_vol = clamped;
        if unmute {
            self.master_muted = false;
        }
        if let Some(ref ctrl) = self.master_vol_ctrl {
            let _ = ctrl.set_volume(clamped);
            if unmute {
                let _ = ctrl.set_mute(false);
            }
        }
    }

    fn step_master_volume_wheel(&mut self, wheel_delta: i16) {
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        let (base_vol, base_muted) = self
            .master_vol_ctrl
            .as_ref()
            .and_then(|c| c.get_state())
            .unwrap_or((self.master_vol, self.master_muted));
        let (next_vol, next_muted) = step_volume_by_wheel(base_vol, base_muted, wheel_delta);
        self.master_vol = next_vol;
        self.master_muted = next_muted;
        if let Some(ref ctrl) = self.master_vol_ctrl {
            let _ = ctrl.set_volume(next_vol);
            if base_muted != next_muted {
                let _ = ctrl.set_mute(next_muted);
            }
        }
    }

    fn toggle_master_mute(&mut self) {
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        let next_muted = !self.master_muted;
        self.master_muted = next_muted;
        if let Some(ref ctrl) = self.master_vol_ctrl {
            let _ = ctrl.set_mute(next_muted);
        }
    }

    fn primary_overlap_px(&self) -> f32 {
        let s = self.scale();
        (self.pill_overlap_px as f32).max(2.0 * s)
    }

    fn primary_height_px(&self) -> f32 {
        let s = self.scale();
        (INNER_PAD_DIP * 2.0 + PRIMARY_ROW_H_DIP * 5.0 + ROW_GAP_DIP * 4.0) * s
            + self.primary_overlap_px()
    }

    fn sub_natural_size_px(&self, cat: PrimaryCategory) -> (f32, f32) {
        let s = self.scale();
        let w_dip = match cat {
            PrimaryCategory::DisplayContent => SOURCES_SUB_WIDTH_DIP,
            PrimaryCategory::VolumeControl => VOLUME_SUB_WIDTH_DIP,
            PrimaryCategory::PlaybackDevices | PrimaryCategory::RecordingDevices => {
                DEVICES_SUB_WIDTH_DIP
            }
            PrimaryCategory::Settings => SETTINGS_SUB_WIDTH_DIP,
        };
        let h_dip = match cat {
            PrimaryCategory::DisplayContent => {
                let rows_h = if self.sources.is_empty() {
                    34.0
                } else {
                    let n = self.sources.len() as f32;
                    n * SOURCE_ROW_H_DIP + (n - 1.0).max(0.0) * ROW_GAP_DIP
                };
                INNER_PAD_DIP * 2.0 + AUTO_ROW_H_DIP + DIVIDER_H_DIP + rows_h
            }
            PrimaryCategory::VolumeControl => {
                let apps_h = if self.app_volumes.is_empty() {
                    34.0
                } else {
                    let n = self.app_volumes.len() as f32;
                    n * VOLUME_ROW_H_DIP + (n - 1.0).max(0.0) * ROW_GAP_DIP
                };
                INNER_PAD_DIP * 2.0 + VOLUME_ROW_H_DIP + DIVIDER_H_DIP + apps_h
            }
            PrimaryCategory::PlaybackDevices => {
                let rows_h = if self.playback_devices.is_empty() {
                    34.0
                } else {
                    let n = self.playback_devices.len() as f32;
                    n * DEVICE_ROW_H_DIP + (n - 1.0).max(0.0) * ROW_GAP_DIP
                };
                INNER_PAD_DIP * 2.0 + rows_h
            }
            PrimaryCategory::RecordingDevices => {
                let rows_h = if self.recording_devices.is_empty() {
                    34.0
                } else {
                    let n = self.recording_devices.len() as f32;
                    n * DEVICE_ROW_H_DIP + (n - 1.0).max(0.0) * ROW_GAP_DIP
                };
                INNER_PAD_DIP * 2.0 + rows_h
            }
            PrimaryCategory::Settings => {
                INNER_PAD_DIP * 2.0
                    + SETTING_ROW_H_DIP * 4.0
                    + ROW_GAP_DIP * 2.0
                    + DIVIDER_H_DIP
            }
        };
        (w_dip * s, h_dip * s)
    }

    fn max_popup_reach_px(&self) -> i32 {
        let mut max_h = self.primary_height_px();
        let overlap = self.primary_overlap_px();
        for &cat in &PrimaryCategory::ALL {
            let (_, sh) = self.sub_natural_size_px(cat);
            let reach = overlap + sh;
            if reach > max_h {
                max_h = reach;
            }
        }
        max_h.ceil() as i32
    }

    fn primary_card_rect(&self) -> RectF {
        let s = self.scale();
        let pad = SHADOW_PAD_DIP * s;
        let pw = PRIMARY_WIDTH_DIP * s;
        let ph = self.primary_height_px();
        let bottom = self.win_h as f32;
        let top = (bottom - ph).max(pad);
        let left = if self.open_left {
            (self.win_w as f32) - pad - pw
        } else {
            pad
        };
        RectF::new(left, top, left + pw, bottom)
    }

    fn category_row_rect(&self, cat: PrimaryCategory) -> RectF {
        for (c, r) in self.primary_row_rects() {
            if c == cat {
                return r;
            }
        }
        self.primary_card_rect()
    }

    fn sub_card_rect(&self, cat: PrimaryCategory) -> RectF {
        let s = self.scale();
        let pad = SHADOW_PAD_DIP * s;
        let gap = SUBMENU_GAP_DIP * s;
        let primary = self.primary_card_rect();
        let row = self.category_row_rect(cat);
        let (sw, natural_sh) = self.sub_natural_size_px(cat);

        let overlap = self.primary_overlap_px();
        let bottom_limit = (primary.bottom - overlap).max(primary.top + 58.0 * s);
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
            RectF::new(primary.left - gap - sw, top, primary.left - gap, bottom)
        } else {
            RectF::new(primary.right + gap, top, primary.right + gap + sw, bottom)
        }
    }

    fn primary_row_rects(&self) -> [(PrimaryCategory, RectF); 5] {
        let s = self.scale();
        let card = self.primary_card_rect();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let rh = PRIMARY_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let mut y = card.top + pad;

        let r0 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r1 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r2 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r3 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r4 = RectF::new(x0, y, x1, y + rh);

        [
            (PrimaryCategory::DisplayContent, r0),
            (PrimaryCategory::VolumeControl, r1),
            (PrimaryCategory::PlaybackDevices, r2),
            (PrimaryCategory::RecordingDevices, r3),
            (PrimaryCategory::Settings, r4),
        ]
    }

    fn display_row_rects(&self, card: RectF) -> (RectF, Vec<RectF>) {
        let s = self.scale();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let mut y = card.top + pad;

        let auto_rect = RectF::new(x0, y, x1, y + AUTO_ROW_H_DIP * s);
        y = auto_rect.bottom + DIVIDER_H_DIP * s;

        let mut source_rects = Vec::with_capacity(self.sources.len());
        let rh = SOURCE_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        for _ in &self.sources {
            if y + rh > card.bottom - pad + 2.0 {
                break;
            }
            source_rects.push(RectF::new(x0, y, x1, y + rh));
            y += rh + gap;
        }
        (auto_rect, source_rects)
    }

    fn volume_row_geom(&self, row_rect: RectF) -> VolumeRowGeom {
        let s = self.scale();
        let mute_size = 28.0 * s;
        let mute_left = row_rect.left + 8.0 * s;
        let mute_top = (row_rect.top + row_rect.bottom - mute_size) * 0.5;
        let mute_rect = RectF::new(
            mute_left,
            mute_top,
            mute_left + mute_size,
            mute_top + mute_size,
        );

        let col_left = mute_rect.right + 8.0 * s;
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
        let track_cy = row_rect.top + 31.5 * s;
        let track_left = col_left + 4.0 * s;
        let track_right = (col_right - 4.0 * s).max(track_left + 24.0 * s);
        let track_rect = RectF::new(
            track_left,
            track_cy - track_h * 0.5,
            track_right,
            track_cy + track_h * 0.5,
        );
        let slider_hit_rect = RectF::new(
            track_left - 4.0 * s,
            row_rect.top + 18.0 * s,
            track_right + 4.0 * s,
            row_rect.bottom,
        );

        VolumeRowGeom {
            row_rect,
            mute_rect,
            track_rect,
            slider_hit_rect,
            label_rect,
            pct_rect,
        }
    }

    fn volume_row_rects(&self, card: RectF) -> (VolumeRowGeom, Vec<VolumeRowGeom>) {
        let s = self.scale();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let mut y = card.top + pad;
        let rh = VOLUME_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;

        let master_row = RectF::new(x0, y, x1, y + rh);
        let master_geom = self.volume_row_geom(master_row);
        y = master_row.bottom + DIVIDER_H_DIP * s;

        let mut app_geoms = Vec::with_capacity(self.app_volumes.len());
        for _ in &self.app_volumes {
            if y + rh > card.bottom - pad + 2.0 {
                break;
            }
            app_geoms.push(self.volume_row_geom(RectF::new(x0, y, x1, y + rh)));
            y += rh + gap;
        }
        (master_geom, app_geoms)
    }

    fn device_row_rects(&self, card: RectF, count: usize) -> Vec<RectF> {
        let s = self.scale();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let mut y = card.top + pad;
        let rh = DEVICE_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let mut rects = Vec::with_capacity(count);
        for _ in 0..count {
            if y + rh > card.bottom - pad + 2.0 {
                break;
            }
            rects.push(RectF::new(x0, y, x1, y + rh));
            y += rh + gap;
        }
        rects
    }

    fn settings_row_rects(&self, card: RectF) -> [RectF; 4] {
        let s = self.scale();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let rh = SETTING_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let mut y = card.top + pad;

        let r_lock = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r_auto = RectF::new(x0, y, x1, y + rh);
        y = r_auto.bottom + DIVIDER_H_DIP * s;
        let r_reset = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r_exit = RectF::new(x0, y, x1, y + rh);
        [r_lock, r_auto, r_reset, r_exit]
    }

    fn point_inside_cards(&self, x: f32, y: f32) -> bool {
        let s = self.scale();
        let foot = FOOT_MENISCUS_DIP * s;
        let step = STEP_MENISCUS_DIP * s;
        let primary = self.primary_card_rect();
        if primary.contains(x, y)
            || (y >= primary.bottom - foot
                && y <= primary.bottom
                && x >= primary.left - foot
                && x <= primary.right + foot)
        {
            return true;
        }
        if let Some(cat) = self.active_category {
            let sub = self.sub_card_rect(cat);
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
            if y >= top_band_min && y <= top_band_max && (x - seam_x).abs() <= step.max(14.0 * s) {
                return true;
            }
            let bot_reach = if primary.bottom - sub.bottom < step + foot {
                foot + 14.0 * s
            } else {
                step
            };
            let bot_max = (sub.bottom + step).min(primary.bottom);
            let in_x = if self.open_left {
                x >= seam_x - bot_reach && x <= seam_x
            } else {
                x >= seam_x && x <= seam_x + bot_reach
            };
            if in_x && y >= sub.bottom && y <= bot_max {
                return true;
            }
        }
        false
    }

    fn update_hover_at(&mut self, x: f32, y: f32) -> bool {
        let prev_cat = self.active_category;
        let prev_hover = self.hover_target;

        let primary_rect = self.primary_card_rect();
        if self.dragging_slider.is_none() && primary_rect.contains(x, y) {
            let mut next_hover = HoverTarget::None;
            for (cat, r) in self.primary_row_rects() {
                if r.contains(x, y) {
                    if self.active_category != Some(cat) {
                        let s = self.scale();
                        let prev_anim_sub = self.current_animated_sub_rect();
                        self.active_category = Some(cat);
                        if cat == PrimaryCategory::VolumeControl
                            && self
                                .last_vol_reload
                                .is_none_or(|t| t.elapsed() >= Duration::from_millis(VOL_RELOAD_FRESH_MS))
                        {
                            self.reload_volumes();
                        }
                        let target_sub = self.sub_card_rect(cat);
                        let seam_x = if self.open_left {
                            primary_rect.left
                        } else {
                            primary_rect.right
                        };
                        self.sub_anim = match (prev_cat, self.sub_anim) {
                            (Some(pcat), Some(mut existing)) => {
                                if let Some(prev_rect) = prev_anim_sub {
                                    existing.top.pos = prev_rect.top;
                                    existing.bottom.pos = prev_rect.bottom;
                                    existing.width.pos = prev_rect.width();
                                }
                                existing.fade_elapsed = 0.0;
                                existing.prev_category = Some(pcat);
                                existing.is_budding = false;
                                Some(existing)
                            }
                            (Some(pcat), None) => {
                                let old_rect = prev_anim_sub.unwrap_or_else(|| self.sub_card_rect(pcat));
                                Some(SubSpringState {
                                    top: Spring1D::still(old_rect.top),
                                    bottom: Spring1D::still(old_rect.bottom),
                                    width: Spring1D::still(old_rect.width()),
                                    fade_elapsed: 0.0,
                                    prev_category: Some(pcat),
                                    is_budding: false,
                                })
                            }
                            (None, _) => {
                                let bud =
                                    sub_bud_start_rect(target_sub, r, seam_x, self.open_left, s);
                                Some(SubSpringState {
                                    top: Spring1D::new(
                                        bud.top,
                                        (target_sub.top - bud.top) * 1.2,
                                    ),
                                    bottom: Spring1D::new(
                                        bud.bottom,
                                        (target_sub.bottom - bud.bottom) * 1.2,
                                    ),
                                    width: Spring1D::new(bud.width(), target_sub.width() * 1.6),
                                    fade_elapsed: 0.0,
                                    prev_category: None,
                                    is_budding: true,
                                })
                            }
                        };
                        self.sync_timer_rate();
                    }
                    next_hover = HoverTarget::Primary(cat);
                    break;
                }
            }
            self.hover_target = next_hover;
        } else if let Some(cat) = self.active_category {
            let sub_rect = self.sub_card_rect(cat);
            if sub_rect.contains(x, y) {
                self.hover_target = match cat {
                    PrimaryCategory::DisplayContent => {
                        let (auto_rect, source_rects) = self.display_row_rects(sub_rect);
                        if auto_rect.contains(x, y) {
                            HoverTarget::SubAuto
                        } else {
                            source_rects
                                .iter()
                                .position(|r| r.contains(x, y))
                                .map(HoverTarget::SubSource)
                                .unwrap_or(HoverTarget::None)
                        }
                    }
                    PrimaryCategory::VolumeControl => {
                        let (master_geom, app_geoms) = self.volume_row_rects(sub_rect);
                        if master_geom.row_rect.contains(x, y) {
                            if master_geom.mute_rect.contains(x, y) {
                                HoverTarget::SubVolMasterMute
                            } else if master_geom.slider_hit_rect.contains(x, y) {
                                HoverTarget::SubVolMasterSlider
                            } else {
                                HoverTarget::SubVolMasterRow
                            }
                        } else if let Some((idx, g)) = app_geoms
                            .iter()
                            .enumerate()
                            .find(|(_, g)| g.row_rect.contains(x, y))
                        {
                            if g.mute_rect.contains(x, y) {
                                HoverTarget::SubVolAppMute(idx)
                            } else if g.slider_hit_rect.contains(x, y) {
                                HoverTarget::SubVolAppSlider(idx)
                            } else {
                                HoverTarget::SubVolAppRow(idx)
                            }
                        } else {
                            HoverTarget::None
                        }
                    }
                    PrimaryCategory::PlaybackDevices => {
                        let rects = self.device_row_rects(sub_rect, self.playback_devices.len());
                        rects
                            .iter()
                            .position(|r| r.contains(x, y))
                            .map(HoverTarget::SubPlayback)
                            .unwrap_or(HoverTarget::None)
                    }
                    PrimaryCategory::RecordingDevices => {
                        let rects = self.device_row_rects(sub_rect, self.recording_devices.len());
                        rects
                            .iter()
                            .position(|r| r.contains(x, y))
                            .map(HoverTarget::SubRecording)
                            .unwrap_or(HoverTarget::None)
                    }
                    PrimaryCategory::Settings => {
                        let [r_lock, r_auto, r_reset, r_exit] = self.settings_row_rects(sub_rect);
                        if r_lock.contains(x, y) {
                            HoverTarget::SubSettingLock
                        } else if r_auto.contains(x, y) {
                            HoverTarget::SubSettingAutostart
                        } else if r_reset.contains(x, y) {
                            HoverTarget::SubSettingResetPos
                        } else if r_exit.contains(x, y) {
                            HoverTarget::SubSettingExit
                        } else {
                            HoverTarget::None
                        }
                    }
                };
            } else {
                self.hover_target = HoverTarget::None;
            }
        } else {
            self.hover_target = HoverTarget::None;
        }

        prev_cat != self.active_category || prev_hover != self.hover_target
    }

    fn apply_slider_drag(&mut self, target: SliderTarget, cursor_x: f32) -> bool {
        if self.active_category != Some(PrimaryCategory::VolumeControl) {
            return false;
        }
        let card = self.sub_card_rect(PrimaryCategory::VolumeControl);
        let (master_geom, app_geoms) = self.volume_row_rects(card);
        match target {
            SliderTarget::Master => {
                let ratio = slider_ratio_from_x(
                    master_geom.track_rect.left,
                    master_geom.track_rect.right,
                    cursor_x,
                );
                self.set_master_volume_ratio(ratio);
                true
            }
            SliderTarget::App(idx) => {
                if let (Some(app), Some(geom)) =
                    (self.app_volumes.get_mut(idx), app_geoms.get(idx))
                {
                    let ratio =
                        slider_ratio_from_x(geom.track_rect.left, geom.track_rect.right, cursor_x);
                    app.set_group_volume(ratio);
                    true
                } else {
                    false
                }
            }
        }
    }

    fn ensure_cover_bitmaps(&mut self, cover: &MediaCover) {
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        unsafe {
            if !self.cover_bmps.contains_key(&cover.hash) {
                if let Ok(b) = self.rt.CreateBitmap(
                    D2D_SIZE_U {
                        width: cover.thumb_w,
                        height: cover.thumb_h,
                    },
                    Some(cover.thumb_pbgra.as_ptr().cast()),
                    cover.thumb_w * 4,
                    &props,
                ) {
                    if self.cover_bmps.len() > 24 {
                        self.cover_bmps.clear();
                    }
                    self.cover_bmps.insert(cover.hash, b);
                }
            }
        }
    }

    fn ensure_text_formats(&mut self) -> windows::core::Result<CachedTextFormats> {
        if let Some(ref cached) = self.cached_text_formats {
            if cached.dpi == self.dpi {
                return Ok(cached.clone());
            }
        }
        let s = self.scale();
        let title_fmt = self.create_fmt(12.5 * s, true, false)?;
        let sub_fmt = self.create_fmt(11.0 * s, false, false)?;
        let badge_fmt = self.create_fmt(10.5 * s, true, false)?;
        let pct_fmt = self.create_fmt(11.0 * s, true, true)?;
        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        let title_trim: IDWriteInlineObject =
            unsafe { self.dwrite_factory.CreateEllipsisTrimmingSign(&title_fmt)? };
        let sub_trim: IDWriteInlineObject =
            unsafe { self.dwrite_factory.CreateEllipsisTrimmingSign(&sub_fmt)? };
        unsafe {
            let _ = title_fmt.SetTrimming(&trimming, &title_trim);
            let _ = sub_fmt.SetTrimming(&trimming, &sub_trim);
        }
        let cached = CachedTextFormats {
            dpi: self.dpi,
            title_fmt,
            sub_fmt,
            badge_fmt,
            pct_fmt,
        };
        self.cached_text_formats = Some(cached.clone());
        Ok(cached)
    }

    fn update_silk_bitmap(&mut self, primary_rect: RectF, s: f32) {
        let now = Instant::now();
        let dt_color = now
            .duration_since(self.last_color_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_color_tick = now;

        let cover = self
            .active_id
            .as_ref()
            .and_then(|aid| self.sources.iter().find(|src| &src.id == aid))
            .and_then(|src| src.cover.as_deref())
            .or_else(|| self.sources.iter().find_map(|src| src.cover.as_deref()));
        let pair = compute_cover_halo_pair(cover, self.fallback_accent, self.light_glass);
        let target = [pair[0].to_normalized(), pair[1].to_normalized()];
        let ui_target = resolve_dynamic_menu_accent(
            &self.sources,
            self.active_id.as_deref(),
            self.fallback_accent,
            self.light_glass,
        )
        .to_normalized();

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

        let need_rebuild = self.silk_bmp.is_none()
            || !was_ready
            || self
                .last_silk_tick
                .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(SILK_UPDATE_INTERVAL_MS));
        if !need_rebuild {
            return;
        }
        self.last_silk_tick = Some(now);

        let expected_len = (SILK_W * SILK_H * 4) as usize;
        if self.silk_buf.len() != expected_len {
            self.silk_buf.resize(expected_len, 0);
        }

        let scale_len = (220.0 * s).max(1.0);
        let phase = (self.flow_phase % 10_000.0) as f32;
        let win_w = self.win_w.max(1) as f32;
        let win_h = self.win_h.max(1) as f32;

        for gy in 0..SILK_H {
            let py = (((gy as f32) + 0.5) / (SILK_H as f32)) * win_h;
            let y = (primary_rect.bottom - py) / scale_len;
            for gx in 0..SILK_W {
                let px = (((gx as f32) + 0.5) / (SILK_W as f32)) * win_w;
                let x = (px - primary_rect.left) / scale_len;
                let (r, g, b, a) =
                    sample_liquid_silk_field(x, y, phase, &self.halo_colors, self.light_glass);
                let idx = ((gy * SILK_W + gx) * 4) as usize;
                self.silk_buf[idx] = (b * a * 255.0).round().clamp(0.0, 255.0) as u8;
                self.silk_buf[idx + 1] = (g * a * 255.0).round().clamp(0.0, 255.0) as u8;
                self.silk_buf[idx + 2] = (r * a * 255.0).round().clamp(0.0, 255.0) as u8;
                self.silk_buf[idx + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }

        unsafe {
            if let Some(ref bmp) = self.silk_bmp {
                let _ = bmp.CopyFromMemory(None, self.silk_buf.as_ptr().cast(), SILK_W * 4);
            } else {
                let props = D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                };
                self.silk_bmp = self
                    .rt
                    .CreateBitmap(
                        D2D_SIZE_U {
                            width: SILK_W,
                            height: SILK_H,
                        },
                        Some(self.silk_buf.as_ptr().cast()),
                        SILK_W * 4,
                        &props,
                    )
                    .ok();
            }
        }
    }

    fn render(&mut self) -> windows::core::Result<()> {
        self.surface.ensure_size(self.win_w, self.win_h)?;

        let covers_to_load: Vec<_> = self
            .sources
            .iter()
            .filter_map(|s| s.cover.clone())
            .collect();
        for cov in covers_to_load {
            self.ensure_cover_bitmaps(&cov);
        }

        let s = self.scale();
        let fmts = self.ensure_text_formats()?;

        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };

        let target_primary = self.primary_card_rect();
        self.update_silk_bitmap(target_primary, s);
        let auto_accent = Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        );

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
            PopupAnimPhase::Opening => {
                let a = ease_out_cubic(((self.open_elapsed - 0.015) / 0.125).clamp(0.0, 1.0));
                (a, 0.0)
            }
            PopupAnimPhase::Open => (1.0, 0.0),
            PopupAnimPhase::Closing => {
                let r = ease_retract(self.close_progress);
                ((1.0 - 0.30 * r).clamp(0.0, 1.0), 5.0 * s * r)
            }
        };

        let foot_r = FOOT_MENISCUS_DIP * s;
        let left_bound = anim_sub_opt.map_or(anim_primary.left, |sr| anim_primary.left.min(sr.left));
        let right_bound =
            anim_sub_opt.map_or(anim_primary.right, |sr| anim_primary.right.max(sr.right));
        let top_bound = anim_sub_opt.map_or(anim_primary.top, |sr| anim_primary.top.min(sr.top));
        let union_bounds = RectF::new(
            (left_bound - foot_r).max(0.0),
            (top_bound - 1.0 * s).max(0.0),
            (right_bound + foot_r).min(self.win_w as f32),
            anim_primary.bottom,
        );

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            // 1. Unified ambient shadows (primary flat at taskbar/MediaBar bottom, submenu clamped above taskbar)
            self.draw_unified_shadows(anim_primary, anim_sub_opt, s);

            // 2. Single unified silhouette layer clipping glass interior, primary rows, divider & submenu content
            if let Ok(unified_geo) =
                self.build_unified_menu_geometry(anim_primary, anim_sub_opt, s, true)
            {
                let geom: ID2D1Geometry = unified_geo.cast().unwrap();
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

                // 2a. System-theme frosted liquid glass + restrained liquid silk ripples
                self.draw_glass_interior(union_bounds);

                // 2b. Primary row backgrounds & staggered content (skip content_layer when steady Open)
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
                    }
                    for (idx, (cat, mut rect)) in primary_rows.into_iter().enumerate() {
                        match self.anim_phase {
                            PopupAnimPhase::Opening => {
                                let dy = primary_row_stagger_dy(
                                    idx,
                                    self.open_elapsed,
                                    self.open_spring.pos,
                                    s,
                                );
                                rect.top += dy;
                                rect.bottom += dy;
                            }
                            PopupAnimPhase::Open => {}
                            PopupAnimPhase::Closing => {
                                rect.top += close_dy;
                                rect.bottom += close_dy;
                            }
                        }
                        let expanded = self.active_category == Some(cat);
                        let hovered = self.hover_target == HoverTarget::Primary(cat);
                        self.draw_primary_row_bg(rect, hovered, expanded, auto_accent, s);
                        self.draw_primary_row_content(cat, rect, expanded, &fmts.title_fmt, s);
                    }
                    if use_layer {
                        self.rt.PopLayer();
                    }
                }

                // 2c. Clean feathered vertical seam divider & animated submenu content
                if let (Some(cat), Some(anim_sub)) = (self.active_category, anim_sub_opt) {
                    let target_sub = self.sub_card_rect(cat);
                    let (fade_in, dx_cur, dy_cur) = match self.sub_anim {
                        Some(sa) if sa.is_budding => {
                            let a =
                                ease_out_cubic(((sa.fade_elapsed - 0.015) / 0.125).clamp(0.0, 1.0));
                            let slide_t = ease_out_cubic((sa.fade_elapsed / 0.165).clamp(0.0, 1.0));
                            let dir = if self.open_left { 1.0 } else { -1.0 };
                            let dx = dir * 12.0 * s * (1.0 - slide_t);
                            let dy = (anim_sub.top - target_sub.top) * 0.35;
                            (a, dx, dy)
                        }
                        Some(sa) => {
                            let a =
                                ease_out_cubic(((sa.fade_elapsed - 0.018) / 0.120).clamp(0.0, 1.0));
                            let dy = (anim_sub.top - target_sub.top) * 0.30;
                            (a, 0.0, dy)
                        }
                        None => (1.0, 0.0, (anim_sub.top - target_sub.top) * 0.45),
                    };

                    let seam_x = if self.open_left {
                        anim_primary.left
                    } else {
                        anim_primary.right
                    };
                    let seam_top = anim_primary.top.max(anim_sub.top) + 8.0 * s;
                    let seam_bottom = anim_primary.bottom.min(anim_sub.bottom) - 8.0 * s;
                    let seam_fade = if self.sub_anim.is_some_and(|sa| sa.is_budding) {
                        fade_in.max(0.12)
                    } else {
                        1.0
                    };
                    if seam_bottom > seam_top + 4.0 * s {
                        let (sr, sg, sb, base_a) = if self.light_glass {
                            (0.0, 0.0, 0.0, 0.075)
                        } else {
                            (1.0, 1.0, 1.0, 0.10)
                        };
                        let peak_a = base_a * seam_fade;
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
                            if let Ok(seam_brush) =
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
                                    &seam_brush,
                                    1.0 * s,
                                    None,
                                );
                            }
                        }
                    }

                    let close_factor = if self.anim_phase == PopupAnimPhase::Closing {
                        (1.0 - 0.30 * ease_retract(self.close_progress)).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };

                    if let Some(sa) = self.sub_anim {
                        if let Some(prev_cat) = sa.prev_category {
                            if sa.fade_elapsed < 0.080 {
                                let fade_out =
                                    (1.0 - ease_out_cubic((sa.fade_elapsed / 0.080).clamp(0.0, 1.0)))
                                        * close_factor;
                                if fade_out > 0.005 {
                                    let prev_target = self.sub_card_rect(prev_cat);
                                    let dy_prev = (anim_sub.top - prev_target.top) * 0.30;
                                    let layer_params = D2D1_LAYER_PARAMETERS {
                                        contentBounds: d2d_rect(anim_sub),
                                        geometricMask: ManuallyDrop::new(None),
                                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                        maskTransform: identity_matrix(),
                                        opacity: fade_out,
                                        opacityBrush: ManuallyDrop::new(None),
                                        layerOptions: D2D1_LAYER_OPTIONS_NONE,
                                    };
                                    self.rt.PushLayer(&layer_params, &self.content_layer);
                                    self.rt.PushAxisAlignedClip(
                                        &d2d_rect(anim_sub),
                                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                    );
                                    if dy_prev.abs() > 0.05 {
                                        self.rt.SetTransform(&Matrix3x2 {
                                            M11: 1.0,
                                            M12: 0.0,
                                            M21: 0.0,
                                            M22: 1.0,
                                            M31: 0.0,
                                            M32: dy_prev,
                                        });
                                    }
                                    self.render_submenu_content(
                                        prev_cat,
                                        prev_target,
                                        auto_accent,
                                        &fmts.title_fmt,
                                        &fmts.sub_fmt,
                                        &fmts.badge_fmt,
                                        &fmts.pct_fmt,
                                        s,
                                    );
                                    if dy_prev.abs() > 0.05 {
                                        self.rt.SetTransform(&identity_matrix());
                                    }
                                    self.rt.PopAxisAlignedClip();
                                    self.rt.PopLayer();
                                }
                            }
                        }
                    }

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
                        self.render_submenu_content(
                            cat,
                            target_sub,
                            auto_accent,
                            &fmts.title_fmt,
                            &fmts.sub_fmt,
                            &fmts.badge_fmt,
                            &fmts.pct_fmt,
                            s,
                        );
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
            }

            // 3. Open upper rim stroke (omits bottom edge so menu fuses seamlessly into MediaBar)
            self.draw_unified_open_rim(anim_primary, anim_sub_opt, s);

            self.rt.EndDraw(None, None)?;
        }

        let win_alpha =
            window_master_alpha(self.anim_phase, self.open_elapsed, self.close_progress);
        let alpha_u8 = (win_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        self.surface
            .commit_with_alpha(self.hwnd, self.win_x, self.win_y, alpha_u8);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn render_submenu_content(
        &self,
        cat: PrimaryCategory,
        sub_rect: RectF,
        auto_accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        badge_fmt: &IDWriteTextFormat,
        pct_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        match cat {
            PrimaryCategory::DisplayContent => {
                self.render_display_submenu(
                    sub_rect,
                    auto_accent,
                    title_fmt,
                    sub_fmt,
                    badge_fmt,
                    s,
                );
            }
            PrimaryCategory::VolumeControl => {
                self.render_volume_submenu(
                    sub_rect,
                    auto_accent,
                    title_fmt,
                    sub_fmt,
                    pct_fmt,
                    s,
                );
            }
            PrimaryCategory::PlaybackDevices => {
                self.render_devices_submenu(
                    sub_rect,
                    &self.playback_devices,
                    true,
                    auto_accent,
                    title_fmt,
                    sub_fmt,
                    s,
                );
            }
            PrimaryCategory::RecordingDevices => {
                self.render_devices_submenu(
                    sub_rect,
                    &self.recording_devices,
                    false,
                    auto_accent,
                    title_fmt,
                    sub_fmt,
                    s,
                );
            }
            PrimaryCategory::Settings => {
                self.render_settings_submenu(sub_rect, auto_accent, title_fmt, s);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn emit_top_seam_transition(
        sink: &windows::Win32::Graphics::Direct2D::ID2D1GeometrySink,
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
                    x: seam_x + 1.0 * s,
                    y: lt,
                },
                point2: D2D_POINT_2F {
                    x: seam_x + 1.0 * s,
                    y: rt,
                },
                point3: D2D_POINT_2F {
                    x: seam_x + 14.0 * s,
                    y: rt,
                },
            });
        } else if dy <= -s && -dy < r_c + step_r {
            sink.AddLine(D2D_POINT_2F {
                x: seam_x - 14.0 * s,
                y: lt,
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: D2D_POINT_2F {
                    x: seam_x - 1.0 * s,
                    y: lt,
                },
                point2: D2D_POINT_2F {
                    x: seam_x - 1.0 * s,
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

    unsafe fn emit_unified_contour(
        sink: &windows::Win32::Graphics::Direct2D::ID2D1GeometrySink,
        primary: RectF,
        sub: Option<RectF>,
        open_left: bool,
        s: f32,
        closed_fill: bool,
    ) {
        let foot_r = FOOT_MENISCUS_DIP * s;
        let r_c = PANEL_RADIUS_DIP * s;
        let step_r = STEP_MENISCUS_DIP * s;
        let bottom = primary.bottom;
        let eff_p = r_c.min((primary.height() - foot_r - 2.0 * s).max(4.0 * s));

        let fig_begin = if closed_fill {
            D2D1_FIGURE_BEGIN_FILLED
        } else {
            D2D1_FIGURE_BEGIN_HOLLOW
        };

        match sub {
            None => {
                sink.BeginFigure(
                    D2D_POINT_2F {
                        x: primary.left - foot_r,
                        y: bottom,
                    },
                    fig_begin,
                );
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.left,
                        y: bottom - foot_r,
                    },
                    size: D2D_SIZE_F {
                        width: foot_r,
                        height: foot_r,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
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
                    y: bottom - foot_r,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.right + foot_r,
                        y: bottom,
                    },
                    size: D2D_SIZE_F {
                        width: foot_r,
                        height: foot_r,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
            }
            Some(sub) if !open_left => {
                let seam_x = primary.right;
                let sub_r = r_c
                    .min((sub.height() * 0.45).max(4.0 * s))
                    .min((sub.width() * 0.4).max(4.0 * s));

                sink.BeginFigure(
                    D2D_POINT_2F {
                        x: primary.left - foot_r,
                        y: bottom,
                    },
                    fig_begin,
                );
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.left,
                        y: bottom - foot_r,
                    },
                    size: D2D_SIZE_F {
                        width: foot_r,
                        height: foot_r,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
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

                // Submenu outer right wall & rounded corners
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

                // Bottom seam capillary meniscus from sub.bottom back onto primary right wall & foot skirt
                let dy_bot = bottom - sub.bottom;
                if dy_bot >= step_r + foot_r {
                    sink.AddLine(D2D_POINT_2F {
                        x: seam_x + step_r,
                        y: sub.bottom,
                    });
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: D2D_POINT_2F {
                            x: seam_x,
                            y: sub.bottom + step_r,
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
                        y: bottom - foot_r,
                    });
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: D2D_POINT_2F {
                            x: seam_x + foot_r,
                            y: bottom,
                        },
                        size: D2D_SIZE_F {
                            width: foot_r,
                            height: foot_r,
                        },
                        rotationAngle: 0.0,
                        sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                        arcSize: D2D1_ARC_SIZE_SMALL,
                    });
                } else {
                    sink.AddLine(D2D_POINT_2F {
                        x: seam_x + foot_r + 12.0 * s,
                        y: sub.bottom,
                    });
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: D2D_POINT_2F {
                            x: seam_x + foot_r + 6.0 * s,
                            y: sub.bottom,
                        },
                        point2: D2D_POINT_2F {
                            x: seam_x + foot_r + 6.0 * s,
                            y: bottom,
                        },
                        point3: D2D_POINT_2F {
                            x: seam_x + foot_r,
                            y: bottom,
                        },
                    });
                }
            }
            Some(sub) => {
                // open_left == true: sub on LEFT, primary on RIGHT, seam_x = primary.left
                let seam_x = primary.left;
                let sub_r = r_c
                    .min((sub.height() * 0.45).max(4.0 * s))
                    .min((sub.width() * 0.4).max(4.0 * s));

                sink.BeginFigure(
                    D2D_POINT_2F {
                        x: seam_x - foot_r,
                        y: bottom,
                    },
                    fig_begin,
                );

                // Bottom-left foot skirt & seam capillary meniscus up to sub.bottom
                let dy_bot = bottom - sub.bottom;
                if dy_bot >= step_r + foot_r {
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: D2D_POINT_2F {
                            x: seam_x,
                            y: bottom - foot_r,
                        },
                        size: D2D_SIZE_F {
                            width: foot_r,
                            height: foot_r,
                        },
                        rotationAngle: 0.0,
                        sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                        arcSize: D2D1_ARC_SIZE_SMALL,
                    });
                    sink.AddLine(D2D_POINT_2F {
                        x: seam_x,
                        y: sub.bottom + step_r,
                    });
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: D2D_POINT_2F {
                            x: seam_x - step_r,
                            y: sub.bottom,
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
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: D2D_POINT_2F {
                            x: seam_x - foot_r - 6.0 * s,
                            y: bottom,
                        },
                        point2: D2D_POINT_2F {
                            x: seam_x - foot_r - 6.0 * s,
                            y: sub.bottom,
                        },
                        point3: D2D_POINT_2F {
                            x: seam_x - foot_r - 12.0 * s,
                            y: sub.bottom,
                        },
                    });
                }

                // Submenu outer left wall & rounded corners
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

                // Primary top-right & bottom-right skirt
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
                    y: bottom - foot_r,
                });
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: D2D_POINT_2F {
                        x: primary.right + foot_r,
                        y: bottom,
                    },
                    size: D2D_SIZE_F {
                        width: foot_r,
                        height: foot_r,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
            }
        }

        sink.EndFigure(if closed_fill {
            D2D1_FIGURE_END_CLOSED
        } else {
            D2D1_FIGURE_END_OPEN
        });
    }

    fn build_unified_menu_geometry(
        &self,
        primary: RectF,
        sub: Option<RectF>,
        s: f32,
        closed_fill: bool,
    ) -> windows::core::Result<ID2D1PathGeometry1> {
        unsafe {
            let geo = self.d2d_factory.CreatePathGeometry()?;
            let sink = geo.Open()?;
            Self::emit_unified_contour(&sink, primary, sub, self.open_left, s, closed_fill);
            sink.Close()?;
            Ok(geo)
        }
    }

    unsafe fn draw_unified_shadows(&self, primary_rect: RectF, sub_rect: Option<RectF>, s: f32) {
        let foot_r = FOOT_MENISCUS_DIP * s;
        let max_w = self.win_w as f32;
        let taskbar_y = primary_rect.bottom - self.primary_overlap_px();
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
                    (primary_rect.left - side_exp).max(foot_r)
                },
                (primary_rect.top - top_exp).max(0.0),
                if !self.open_left && sub_rect.is_some() {
                    primary_rect.right
                } else {
                    (primary_rect.right + side_exp).min(max_w - foot_r)
                },
                primary_rect.bottom,
            );
            let sh_sub = sub_rect.map(|sr| {
                RectF::new(
                    if !self.open_left {
                        sr.left
                    } else {
                        (sr.left - side_exp).max(foot_r)
                    },
                    (sr.top - top_exp).max(0.0),
                    if self.open_left {
                        sr.right
                    } else {
                        (sr.right + side_exp).min(max_w - foot_r)
                    },
                    (sr.bottom + bot_exp + 1.5 * s)
                        .min(taskbar_y)
                        .max(sr.bottom),
                )
            });

            if let (Ok(geo), Ok(sh_brush)) = (
                self.build_unified_menu_geometry(sh_primary, sh_sub, s, true),
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

    unsafe fn draw_glass_interior(&self, union_bounds: RectF) {
        let full_win = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: self.win_w as f32,
            bottom: self.win_h as f32,
        };

        // 1. Soft frosted desktop refraction base (or fallback solid surface if capture failed)
        if let Some(ref bg_bmp) = self.backdrop_bmp {
            self.rt.DrawBitmap(
                bg_bmp,
                Some(&full_win),
                0.88,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        } else {
            let fallback_bg = if self.light_glass {
                D2D1_COLOR_F {
                    r: 246.0 / 255.0,
                    g: 246.0 / 255.0,
                    b: 248.0 / 255.0,
                    a: 0.90,
                }
            } else {
                D2D1_COLOR_F {
                    r: 26.0 / 255.0,
                    g: 26.0 / 255.0,
                    b: 28.0 / 255.0,
                    a: 0.88,
                }
            };
            if let Ok(fb_brush) = self.rt.CreateSolidColorBrush(&fallback_bg, None) {
                self.rt.FillRectangle(&d2d_rect(union_bounds), &fb_brush);
            }
        }

        // 2. System-theme neutral dark/light liquid glass base (preserves backdrop tone without cold blue cast)
        let glass_base = if self.light_glass {
            D2D1_COLOR_F {
                r: 246.0 / 255.0,
                g: 246.0 / 255.0,
                b: 248.0 / 255.0,
                a: 0.74,
            }
        } else {
            D2D1_COLOR_F {
                r: 24.0 / 255.0,
                g: 24.0 / 255.0,
                b: 26.0 / 255.0,
                a: 0.72,
            }
        };
        if let Ok(base_brush) = self.rt.CreateSolidColorBrush(&glass_base, None) {
            self.rt.FillRectangle(&d2d_rect(union_bounds), &base_brush);
        }

        // 3. Restrained cover-theme liquid silk wisps (smooth organic ribbons, mostly clear glass)
        if let Some(ref silk) = self.silk_bmp {
            self.rt.DrawBitmap(
                silk,
                Some(&full_win),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }

        // 4. Diagonal liquid glass specular sheen
        let sheen_stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: if self.light_glass { 0.26 } else { 0.14 },
                },
            },
            D2D1_GRADIENT_STOP {
                position: 0.45,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: if self.light_glass { 0.03 } else { 0.02 },
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: if self.light_glass { 0.10 } else { 0.06 },
                },
            },
        ];
        if let Ok(sc) =
            self.rt
                .CreateGradientStopCollection(&sheen_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
        {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: union_bounds.left,
                    y: union_bounds.top,
                },
                endPoint: D2D_POINT_2F {
                    x: union_bounds.right,
                    y: union_bounds.bottom,
                },
            };
            if let Ok(sheen_brush) = self.rt.CreateLinearGradientBrush(&props, None, &sc) {
                self.rt
                    .FillRectangle(&d2d_rect(union_bounds), &sheen_brush);
            }
        }
    }

    unsafe fn draw_unified_open_rim(
        &self,
        primary_rect: RectF,
        sub_rect: Option<RectF>,
        s: f32,
    ) {
        let mut rim_primary = primary_rect;
        rim_primary.bottom -= 0.5 * s;
        let Ok(rim_geo) = self.build_unified_menu_geometry(rim_primary, sub_rect, s, false) else {
            return;
        };
        let top_min = sub_rect.map_or(rim_primary.top, |r| rim_primary.top.min(r.top));

        if self.light_glass {
            let def_col = D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.09,
            };
            if let Ok(def_brush) = self.rt.CreateSolidColorBrush(&def_col, None) {
                self.rt.DrawGeometry(&rim_geo, &def_brush, 1.3 * s, None);
            }
        }

        let rim_stops = if self.light_glass {
            [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.82,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.55,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.42,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.22,
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
                        a: 0.48,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.55,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.18,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.13,
                    },
                },
            ]
        };
        let stroke_w = if self.light_glass { 1.0 * s } else { 1.15 * s };
        if let Ok(sc) =
            self.rt
                .CreateGradientStopCollection(&rim_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
        {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: rim_primary.left,
                    y: top_min,
                },
                endPoint: D2D_POINT_2F {
                    x: rim_primary.left,
                    y: rim_primary.bottom,
                },
            };
            if let Ok(rim_brush) = self.rt.CreateLinearGradientBrush(&props, None, &sc) {
                self.rt.DrawGeometry(&rim_geo, &rim_brush, stroke_w, None);
            }
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
            radiusX: 8.0 * s,
            radiusY: 8.0 * s,
        };
        let fill = if expanded {
            if self.light_glass {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: if hovered { 0.10 } else { 0.07 },
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: if hovered { 0.17 } else { 0.12 },
                }
            }
        } else if self.light_glass {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.06,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.10,
            }
        };
        if let Ok(b) = self.rt.CreateSolidColorBrush(&fill, None) {
            self.rt.FillRoundedRectangle(&rr, &b);
        }
        if expanded {
            let bar_w = 3.0 * s;
            let bar_h = 14.0 * s;
            let bar_left = if self.open_left {
                rect.right - 3.0 * s - bar_w
            } else {
                rect.left + 3.0 * s
            };
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
            if let Ok(ab) = self
                .rt
                .CreateSolidColorBrush(&d2d_color_rgb(accent, 0.95), None)
            {
                self.rt.FillRoundedRectangle(&bar_rr, &ab);
            }
        }
    }

    unsafe fn draw_primary_row_content(
        &self,
        cat: PrimaryCategory,
        rect: RectF,
        expanded: bool,
        title_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let mut fg = self.primary_text_color();
        if !expanded {
            fg.a *= 0.90;
        }
        let icon_cx = rect.left + 19.0 * s;
        let icon_cy = (rect.top + rect.bottom) * 0.5;
        let arrow_cx = rect.right - 13.0 * s;

        if let Ok(brush) = self.rt.CreateSolidColorBrush(&fg, None) {
            self.draw_category_icon(cat, icon_cx, icon_cy, s, &brush);
            let mut arrow_col = self.muted_text_color();
            if expanded {
                arrow_col.a = 0.92;
            }
            if let Ok(ab) = self.rt.CreateSolidColorBrush(&arrow_col, None) {
                self.draw_chevron(arrow_cx, icon_cy, self.open_left, s, &ab);
            }
        }

        let label_rect = RectF::new(
            rect.left + 34.0 * s,
            rect.top,
            rect.right - 22.0 * s,
            rect.bottom,
        );
        self.draw_text_in_rect(cat.label(), label_rect, title_fmt, fg);
    }

    unsafe fn draw_category_icon(
        &self,
        cat: PrimaryCategory,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        match cat {
            PrimaryCategory::DisplayContent => {
                // Beamed eighth notes
                self.rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: cx - 2.6 * s,
                            y: cy + 3.0 * s,
                        },
                        radiusX: 1.9 * s,
                        radiusY: 1.5 * s,
                    },
                    brush,
                );
                self.rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: cx + 2.8 * s,
                            y: cy + 2.1 * s,
                        },
                        radiusX: 1.9 * s,
                        radiusY: 1.5 * s,
                    },
                    brush,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 1.1 * s,
                        y: cy + 2.8 * s,
                    },
                    D2D_POINT_2F {
                        x: cx - 1.1 * s,
                        y: cy - 4.2 * s,
                    },
                    brush,
                    1.3 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx + 4.3 * s,
                        y: cy + 1.9 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 4.3 * s,
                        y: cy - 5.2 * s,
                    },
                    brush,
                    1.3 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 1.5 * s,
                        y: cy - 4.2 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 4.7 * s,
                        y: cy - 5.2 * s,
                    },
                    brush,
                    1.8 * s,
                    None,
                );
            }
            PrimaryCategory::VolumeControl => {
                // Speaker cone with double sound wave arcs
                draw_speaker_glyph(
                    &self.d2d_factory,
                    &self.rt,
                    cx,
                    cy,
                    0.92 * s,
                    0.75,
                    false,
                    brush,
                    brush,
                    brush,
                    self.round_stroke.as_ref(),
                );
            }
            PrimaryCategory::PlaybackDevices => {
                // Over-ear headphones (top headband arc + left/right earcups)
                if let Ok(band) = self.d2d_factory.CreatePathGeometry() {
                    if let Ok(sink) = band.Open() {
                        sink.BeginFigure(
                            D2D_POINT_2F {
                                x: cx - 4.3 * s,
                                y: cy + 0.4 * s,
                            },
                            D2D1_FIGURE_BEGIN_HOLLOW,
                        );
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: D2D_POINT_2F {
                                x: cx + 4.3 * s,
                                y: cy + 0.4 * s,
                            },
                            size: D2D_SIZE_F {
                                width: 4.3 * s,
                                height: 4.9 * s,
                            },
                            rotationAngle: 0.0,
                            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                            arcSize: D2D1_ARC_SIZE_SMALL,
                        });
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        let _ = sink.Close();
                        self.rt
                            .DrawGeometry(&band, brush, 1.45 * s, self.round_stroke.as_ref());
                    }
                }
                let left_cup = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.5 * s,
                        top: cy - 0.3 * s,
                        right: cx - 2.9 * s,
                        bottom: cy + 4.6 * s,
                    },
                    radiusX: 1.3 * s,
                    radiusY: 1.3 * s,
                };
                let right_cup = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx + 2.9 * s,
                        top: cy - 0.3 * s,
                        right: cx + 5.5 * s,
                        bottom: cy + 4.6 * s,
                    },
                    radiusX: 1.3 * s,
                    radiusY: 1.3 * s,
                };
                self.rt.FillRoundedRectangle(&left_cup, brush);
                self.rt.FillRoundedRectangle(&right_cup, brush);
            }
            PrimaryCategory::RecordingDevices => {
                // Microphone capsule + stand
                let mic_rr = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 2.1 * s,
                        top: cy - 5.5 * s,
                        right: cx + 2.1 * s,
                        bottom: cy + 1.2 * s,
                    },
                    radiusX: 2.1 * s,
                    radiusY: 2.1 * s,
                };
                self.rt.FillRoundedRectangle(&mic_rr, brush);
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 3.8 * s,
                        y: cy - 0.5 * s,
                    },
                    D2D_POINT_2F {
                        x: cx - 3.8 * s,
                        y: cy + 2.4 * s,
                    },
                    brush,
                    1.2 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx + 3.8 * s,
                        y: cy - 0.5 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 3.8 * s,
                        y: cy + 2.4 * s,
                    },
                    brush,
                    1.2 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 3.8 * s,
                        y: cy + 2.4 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 3.8 * s,
                        y: cy + 2.4 * s,
                    },
                    brush,
                    1.2 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx,
                        y: cy + 2.4 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy + 5.2 * s,
                    },
                    brush,
                    1.3 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 2.6 * s,
                        y: cy + 5.2 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 2.6 * s,
                        y: cy + 5.2 * s,
                    },
                    brush,
                    1.3 * s,
                    None,
                );
            }
            PrimaryCategory::Settings => {
                // Two clean horizontal slider bars with thumb circles
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 5.2 * s,
                        y: cy - 2.6 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 5.2 * s,
                        y: cy - 2.6 * s,
                    },
                    brush,
                    1.3 * s,
                    None,
                );
                self.rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: cx - 1.6 * s,
                            y: cy - 2.6 * s,
                        },
                        radiusX: 2.0 * s,
                        radiusY: 2.0 * s,
                    },
                    brush,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 5.2 * s,
                        y: cy + 2.6 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 5.2 * s,
                        y: cy + 2.6 * s,
                    },
                    brush,
                    1.3 * s,
                    None,
                );
                self.rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: cx + 1.8 * s,
                            y: cy + 2.6 * s,
                        },
                        radiusX: 2.0 * s,
                        radiusY: 2.0 * s,
                    },
                    brush,
                );
            }
        }
    }

    unsafe fn draw_chevron(
        &self,
        cx: f32,
        cy: f32,
        point_left: bool,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let dir = if point_left { -1.0 } else { 1.0 };
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx - 1.8 * dir * s,
                y: cy - 3.4 * s,
            },
            D2D_POINT_2F {
                x: cx + 1.8 * dir * s,
                y: cy,
            },
            brush,
            1.35 * s,
            None,
        );
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx + 1.8 * dir * s,
                y: cy,
            },
            D2D_POINT_2F {
                x: cx - 1.8 * dir * s,
                y: cy + 3.4 * s,
            },
            brush,
            1.35 * s,
            None,
        );
    }

    unsafe fn render_display_submenu(
        &self,
        card: RectF,
        auto_accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        badge_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let (auto_rect, source_rects) = self.display_row_rects(card);
        let is_auto = matches!(self.mode, SelectionMode::Auto);
        self.draw_row_highlight(
            auto_rect,
            self.hover_target == HoverTarget::SubAuto,
            is_auto,
            auto_accent,
            s,
        );
        let mut auto_col = self.primary_text_color();
        if !is_auto {
            auto_col.a *= 0.85;
        }
        let auto_label_r = RectF::new(
            auto_rect.left + 12.0 * s,
            auto_rect.top,
            auto_rect.right - 12.0 * s,
            auto_rect.bottom,
        );
        self.draw_text_in_rect("自动", auto_label_r, title_fmt, auto_col);

        self.draw_divider(auto_rect, s);

        if self.sources.is_empty() {
            let empty_y = auto_rect.bottom + DIVIDER_H_DIP * s;
            let empty_rect = RectF::new(
                auto_rect.left + 4.0 * s,
                empty_y,
                auto_rect.right - 4.0 * s,
                empty_y + 34.0 * s,
            );
            self.draw_text_in_rect(
                "暂无其他活跃媒体会话",
                empty_rect,
                sub_fmt,
                self.muted_text_color(),
            );
            return;
        }

        for (idx, (src, row_rect)) in self.sources.iter().zip(source_rects.iter()).enumerate() {
            let hovered = self.hover_target == HoverTarget::SubSource(idx);
            let selected = matches!(self.mode, SelectionMode::Manual(ref id) if id == &src.id);
            let row_accent = src
                .cover
                .as_ref()
                .map(|c| {
                    if self.light_glass {
                        c.accent_light
                    } else {
                        c.accent_dark
                    }
                })
                .unwrap_or(auto_accent);
            self.draw_row_highlight(*row_rect, hovered, selected, row_accent, s);
            self.draw_source_row(*row_rect, src, selected, title_fmt, sub_fmt, badge_fmt, s);
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn render_volume_submenu(
        &self,
        card: RectF,
        accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        pct_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let (master_geom, app_geoms) = self.volume_row_rects(card);

        let master_hovered = matches!(
            self.hover_target,
            HoverTarget::SubVolMasterMute
                | HoverTarget::SubVolMasterSlider
                | HoverTarget::SubVolMasterRow
        );
        self.draw_row_highlight(master_geom.row_rect, master_hovered, false, accent, s);
        self.draw_volume_row_content(
            master_geom,
            "总音量",
            self.master_vol,
            self.master_muted,
            true,
            self.hover_target == HoverTarget::SubVolMasterMute,
            self.dragging_slider == Some(SliderTarget::Master)
                || self.hover_target == HoverTarget::SubVolMasterSlider,
            accent,
            title_fmt,
            pct_fmt,
            s,
        );

        self.draw_divider(master_geom.row_rect, s);

        if self.app_volumes.is_empty() {
            let empty_y = master_geom.row_rect.bottom + DIVIDER_H_DIP * s;
            let empty_rect = RectF::new(
                master_geom.row_rect.left + 6.0 * s,
                empty_y,
                master_geom.row_rect.right - 6.0 * s,
                empty_y + 34.0 * s,
            );
            self.draw_text_in_rect(
                "暂无正在使用音频的应用",
                empty_rect,
                sub_fmt,
                self.muted_text_color(),
            );
            return;
        }

        for (idx, (app, geom)) in self.app_volumes.iter().zip(app_geoms.iter()).enumerate() {
            let hovered = matches!(
                self.hover_target,
                HoverTarget::SubVolAppMute(i)
                    | HoverTarget::SubVolAppSlider(i)
                    | HoverTarget::SubVolAppRow(i)
                    if i == idx
            );
            self.draw_row_highlight(geom.row_rect, hovered, false, accent, s);
            self.draw_volume_row_content(
                *geom,
                &app.name,
                app.volume,
                app.muted,
                app.is_active,
                self.hover_target == HoverTarget::SubVolAppMute(idx),
                self.dragging_slider == Some(SliderTarget::App(idx))
                    || self.hover_target == HoverTarget::SubVolAppSlider(idx),
                accent,
                title_fmt,
                pct_fmt,
                s,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_volume_row_content(
        &self,
        geom: VolumeRowGeom,
        label: &str,
        volume: f32,
        muted: bool,
        is_active: bool,
        mute_hovered: bool,
        slider_active: bool,
        accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        pct_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        if mute_hovered {
            let bg = if self.light_glass {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.08,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.13,
                }
            };
            if let Ok(bb) = self.rt.CreateSolidColorBrush(&bg, None) {
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(geom.mute_rect),
                    radiusX: 6.0 * s,
                    radiusY: 6.0 * s,
                };
                self.rt.FillRoundedRectangle(&rr, &bb);
            }
        }

        let mut label_col = self.primary_text_color();
        if !is_active {
            label_col.a *= 0.78;
        }
        let muted_icon_col = D2D1_COLOR_F {
            r: label_col.r,
            g: label_col.g,
            b: label_col.b,
            a: label_col.a * 0.52,
        };
        let slash_col = if self.light_glass {
            D2D1_COLOR_F {
                r: 0.80,
                g: 0.18,
                b: 0.20,
                a: 0.92,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 0.46,
                b: 0.46,
                a: 0.95,
            }
        };

        if let (Ok(fg_b), Ok(mu_b), Ok(sl_b)) = (
            self.rt.CreateSolidColorBrush(&label_col, None),
            self.rt.CreateSolidColorBrush(&muted_icon_col, None),
            self.rt.CreateSolidColorBrush(&slash_col, None),
        ) {
            let cx = (geom.mute_rect.left + geom.mute_rect.right) * 0.5;
            let cy = (geom.mute_rect.top + geom.mute_rect.bottom) * 0.5;
            let u = geom.mute_rect.width() / 22.0;
            draw_speaker_glyph(
                &self.d2d_factory,
                &self.rt,
                cx,
                cy,
                u,
                volume,
                muted,
                &fg_b,
                &mu_b,
                &sl_b,
                self.round_stroke.as_ref(),
            );
        }

        self.draw_text_in_rect(label, geom.label_rect, title_fmt, label_col);

        let mut pct_col = self.muted_text_color();
        if muted {
            pct_col.a *= 0.68;
        }
        let pct_str = format_volume_pct(volume);
        self.draw_text_in_rect(&pct_str, geom.pct_rect, pct_fmt, pct_col);

        draw_volume_slider(
            &self.rt,
            geom.track_rect,
            volume,
            muted,
            slider_active,
            self.light_glass,
            accent,
            s,
        );
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn render_devices_submenu(
        &self,
        card: RectF,
        devices: &[AudioEndpointItem],
        is_playback: bool,
        accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let pad = INNER_PAD_DIP * s;
        if devices.is_empty() {
            let empty_r = RectF::new(
                card.left + pad + 4.0 * s,
                card.top + pad,
                card.right - pad - 4.0 * s,
                card.top + pad + 34.0 * s,
            );
            let msg = if is_playback {
                "未检测到可用播放设备"
            } else {
                "未检测到可用录音设备"
            };
            self.draw_text_in_rect(msg, empty_r, sub_fmt, self.muted_text_color());
            return;
        }

        let rects = self.device_row_rects(card, devices.len());
        for (idx, (dev, row_rect)) in devices.iter().zip(rects.iter()).enumerate() {
            let hovered = if is_playback {
                self.hover_target == HoverTarget::SubPlayback(idx)
            } else {
                self.hover_target == HoverTarget::SubRecording(idx)
            };
            self.draw_row_highlight(*row_rect, hovered, dev.is_default, accent, s);

            let text_left = row_rect.left + 12.0 * s;
            let text_right = row_rect.right - 10.0 * s;
            let mid_y = (row_rect.top + row_rect.bottom) * 0.5;
            if dev.sub_name.is_empty() {
                let single_r = RectF::new(text_left, row_rect.top, text_right, row_rect.bottom);
                self.draw_text_in_rect(
                    &dev.main_name,
                    single_r,
                    title_fmt,
                    self.primary_text_color(),
                );
            } else {
                let top_r = RectF::new(text_left, mid_y - 16.0 * s, text_right, mid_y + 1.0 * s);
                let bot_r = RectF::new(text_left, mid_y + 0.5 * s, text_right, mid_y + 15.5 * s);
                self.draw_text_in_rect(&dev.main_name, top_r, title_fmt, self.primary_text_color());
                self.draw_text_in_rect(&dev.sub_name, bot_r, sub_fmt, self.muted_text_color());
            }
        }
    }

    unsafe fn render_settings_submenu(
        &self,
        card: RectF,
        accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let [r_lock, r_auto, r_reset, r_exit] = self.settings_row_rects(card);

        self.draw_row_highlight(
            r_lock,
            self.hover_target == HoverTarget::SubSettingLock,
            self.locked,
            accent,
            s,
        );
        self.draw_setting_label(r_lock, "锁定位置", self.primary_text_color(), title_fmt, s);

        self.draw_row_highlight(
            r_auto,
            self.hover_target == HoverTarget::SubSettingAutostart,
            self.autostart,
            accent,
            s,
        );
        self.draw_setting_label(r_auto, "开机自启", self.primary_text_color(), title_fmt, s);

        self.draw_divider(r_auto, s);

        self.draw_row_highlight(
            r_reset,
            self.hover_target == HoverTarget::SubSettingResetPos,
            false,
            accent,
            s,
        );
        self.draw_setting_label(r_reset, "重置位置", self.primary_text_color(), title_fmt, s);

        self.draw_row_highlight(
            r_exit,
            self.hover_target == HoverTarget::SubSettingExit,
            false,
            accent,
            s,
        );
        let exit_col = if self.light_glass {
            D2D1_COLOR_F {
                r: 0.78,
                g: 0.16,
                b: 0.18,
                a: 0.94,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 0.48,
                b: 0.48,
                a: 0.95,
            }
        };
        self.draw_setting_label(r_exit, "退出", exit_col, title_fmt, s);
    }

    unsafe fn draw_setting_label(
        &self,
        rect: RectF,
        label: &str,
        color: D2D1_COLOR_F,
        fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let label_r = RectF::new(
            rect.left + 12.0 * s,
            rect.top,
            rect.right - 10.0 * s,
            rect.bottom,
        );
        self.draw_text_in_rect(label, label_r, fmt, color);
    }

    unsafe fn draw_divider(&self, upper_row: RectF, s: f32) {
        let div_y = upper_row.bottom + (DIVIDER_H_DIP * s) * 0.5;
        let div_col = if self.light_glass {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.10,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.14,
            }
        };
        if let Ok(div_brush) = self.rt.CreateSolidColorBrush(&div_col, None) {
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: upper_row.left + 6.0 * s,
                    y: div_y,
                },
                D2D_POINT_2F {
                    x: upper_row.right - 6.0 * s,
                    y: div_y,
                },
                &div_brush,
                1.0 * s,
                None,
            );
        }
    }

    fn create_fmt(
        &self,
        size: f32,
        semibold: bool,
        trailing: bool,
    ) -> windows::core::Result<IDWriteTextFormat> {
        unsafe {
            let fmt = self.dwrite_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                if semibold {
                    DWRITE_FONT_WEIGHT_SEMI_BOLD
                } else {
                    DWRITE_FONT_WEIGHT_NORMAL
                },
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("zh-CN"),
            )?;
            let _ = fmt.SetTextAlignment(if trailing {
                DWRITE_TEXT_ALIGNMENT_TRAILING
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            });
            let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let _ = fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            Ok(fmt)
        }
    }

    fn primary_text_color(&self) -> D2D1_COLOR_F {
        if self.light_glass {
            D2D1_COLOR_F {
                r: 0.07,
                g: 0.085,
                b: 0.12,
                a: 0.92,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.95,
            }
        }
    }

    fn muted_text_color(&self) -> D2D1_COLOR_F {
        if self.light_glass {
            D2D1_COLOR_F {
                r: 0.20,
                g: 0.21,
                b: 0.23,
                a: 0.68,
            }
        } else {
            D2D1_COLOR_F {
                r: 0.94,
                g: 0.94,
                b: 0.95,
                a: 0.72,
            }
        }
    }

    unsafe fn draw_row_highlight(
        &self,
        rect: RectF,
        hovered: bool,
        selected: bool,
        accent: Rgb,
        s: f32,
    ) {
        if !hovered && !selected {
            return;
        }
        let rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(rect),
            radiusX: 8.5 * s,
            radiusY: 8.5 * s,
        };

        if selected {
            let fill_alpha = if self.light_glass {
                if hovered {
                    0.26
                } else {
                    0.20
                }
            } else if hovered {
                0.34
            } else {
                0.26
            };
            let fill = d2d_color_rgb(accent, fill_alpha);
            if let Ok(b) = self.rt.CreateSolidColorBrush(&fill, None) {
                self.rt.FillRoundedRectangle(&rr, &b);
            }
            let border_alpha = if self.light_glass { 0.48 } else { 0.58 };
            let border = d2d_color_rgb(accent, border_alpha);
            if let Ok(bb) = self.rt.CreateSolidColorBrush(&border, None) {
                self.rt.DrawRoundedRectangle(&rr, &bb, 1.1 * s, None);
            }

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
            let bar_col = d2d_color_rgb(accent, 0.98);
            if let Ok(ab) = self.rt.CreateSolidColorBrush(&bar_col, None) {
                self.rt.FillRoundedRectangle(&bar_rr, &ab);
            }
        } else {
            let fill = if self.light_glass {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.06,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.10,
                }
            };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&fill, None) {
                self.rt.FillRoundedRectangle(&rr, &b);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_source_row(
        &self,
        rect: RectF,
        src: &SourceItem,
        selected: bool,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        badge_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let thumb_size = 30.0 * s;
        let thumb_left = rect.left + 10.0 * s;
        let thumb_top = (rect.top + rect.bottom - thumb_size) * 0.5;
        let thumb_rect = RectF::new(
            thumb_left,
            thumb_top,
            thumb_left + thumb_size,
            thumb_top + thumb_size,
        );
        let thumb_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(thumb_rect),
            radiusX: 6.0 * s,
            radiusY: 6.0 * s,
        };

        let bmp = src
            .cover
            .as_ref()
            .and_then(|c| self.cover_bmps.get(&c.hash));
        if let Some(cover_bmp) = bmp {
            if let Ok(geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&thumb_rr) {
                let geom: ID2D1Geometry = geo.cast().unwrap();
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(thumb_rect),
                    geometricMask: ManuallyDrop::new(Some(geom)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, &self.thumb_layer);
                self.rt.DrawBitmap(
                    cover_bmp,
                    Some(&d2d_rect(thumb_rect)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(params.geometricMask);
            }
        } else {
            let tile_col = if self.light_glass {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.08,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.12,
                }
            };
            if let Ok(tb) = self.rt.CreateSolidColorBrush(&tile_col, None) {
                self.rt.FillRoundedRectangle(&thumb_rr, &tb);
            }
            if let Ok(nb) = self.rt.CreateSolidColorBrush(&self.primary_text_color(), None) {
                self.rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: (thumb_rect.left + thumb_rect.right) * 0.5,
                            y: (thumb_rect.top + thumb_rect.bottom) * 0.5,
                        },
                        radiusX: 3.8 * s,
                        radiusY: 3.8 * s,
                    },
                    &nb,
                );
            }
        }

        let text_left = thumb_rect.right + 10.0 * s;
        let status_w = 48.0 * s;
        let text_right = (rect.right - status_w).max(text_left + 40.0 * s);
        let mid_y = (rect.top + rect.bottom) * 0.5;
        let title_r = RectF::new(text_left, mid_y - 16.0 * s, text_right, mid_y + 1.0 * s);
        let sub_r = RectF::new(text_left, mid_y + 0.5 * s, text_right, mid_y + 15.5 * s);

        self.draw_text_in_rect(&src.title, title_r, title_fmt, self.primary_text_color());
        let subtitle = if src.artist.is_empty() {
            src.app_name.clone()
        } else {
            format!("{} · {}", src.artist, src.app_name)
        };
        self.draw_text_in_rect(&subtitle, sub_r, sub_fmt, self.muted_text_color());

        let is_playing = src.status == PlaybackState::Playing;
        let accent = src
            .cover
            .as_ref()
            .map(|c| {
                if self.light_glass {
                    c.accent_light
                } else {
                    c.accent_dark
                }
            })
            .unwrap_or_else(|| adjust_accent_for_theme(self.fallback_accent, self.light_glass));
        let badge_col = if is_playing {
            d2d_color_rgb(accent, 0.98)
        } else if selected {
            self.primary_text_color()
        } else {
            self.muted_text_color()
        };
        let badge_text = if is_playing { "播放中" } else { "已暂停" };
        let badge_r = RectF::new(rect.right - status_w, rect.top, rect.right - 8.0 * s, rect.bottom);
        self.draw_text_in_rect(badge_text, badge_r, badge_fmt, badge_col);
    }

    unsafe fn draw_text_in_rect(
        &self,
        text: &str,
        rect: RectF,
        fmt: &IDWriteTextFormat,
        color: D2D1_COLOR_F,
    ) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let Ok(layout) = self.dwrite_factory.CreateTextLayout(
            &wide,
            fmt,
            rect.width().max(1.0),
            rect.height().max(12.0),
        ) else {
            return;
        };
        let Ok(brush) = self.rt.CreateSolidColorBrush(&color, None) else {
            return;
        };
        self.rt.DrawTextLayout(
            D2D_POINT_2F {
                x: rect.left,
                y: rect.top,
            },
            &layout,
            &brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
        );
    }
}

/// Cubic ease-out curve in `[0, 1]`.
fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Smooth ease-in-quad retract curve (`t^2 * (1.6 - 0.6*t)`) in `[0, 1]` for brisk collapse into the taskbar.
fn ease_retract(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (1.6 - 0.6 * t)
}

/// Staggered bottom-to-top vertical micro-glide offset (`idx = 4` bottom row settles first, `0` top last).
fn primary_row_stagger_dy(idx: usize, open_elapsed: f32, open_pos: f32, s: f32) -> f32 {
    let order = 4usize.saturating_sub(idx) as f32;
    let delay = order * 0.011;
    let row_t = ((open_elapsed - delay) / 0.165).clamp(0.0, 1.0);
    let settle_glide = (1.0 - ease_out_cubic(row_t)) * (7.0 + order * 1.4) * s;
    let crest_breath = (1.0 - open_pos) * 0.35 * 14.0 * s;
    settle_glide + crest_breath
}

/// Computes the primary card's animated envelope, keeping left, right, and bottom anchored on MediaBar.
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

/// Computes the initial side-budding submenu droplet anchored at `seam_x` with safe minimum width.
fn sub_bud_start_rect(
    target_sub: RectF,
    row: RectF,
    seam_x: f32,
    open_left: bool,
    s: f32,
) -> RectF {
    let min_w = ((FOOT_MENISCUS_DIP + 14.0 + PANEL_RADIUS_DIP).max(40.0) * s)
        .min(target_sub.width());
    let start_w = (target_sub.width() * 0.20).max(min_w).min(target_sub.width());
    let row_cy = ((row.top + row.bottom) * 0.5).clamp(
        target_sub.top + 20.0 * s,
        (target_sub.bottom - 20.0 * s).max(target_sub.top + 20.0 * s),
    );
    let min_h = (44.0 * s).min(target_sub.height());
    let start_h = (target_sub.height() * 0.36).max(min_h).min(target_sub.height());
    let top = (row_cy - start_h * 0.5)
        .clamp(target_sub.top, (target_sub.bottom - start_h).max(target_sub.top));
    let bottom = top + start_h;
    if open_left {
        RectF::new(seam_x - start_w, top, seam_x, bottom)
    } else {
        RectF::new(seam_x, top, seam_x + start_w, bottom)
    }
}

/// Clamps spring-driven submenu edges into geometry-safe bounds anchored at `seam_x`.
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
    let min_safe_w = ((FOOT_MENISCUS_DIP + 14.0 + PANEL_RADIUS_DIP).max(40.0) * s)
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

/// Computes the submenu's closing collapse rectangle anchored to `seam_x` and clamped above `bottom_limit`.
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
    let min_w = ((FOOT_MENISCUS_DIP + 14.0 + PANEL_RADIUS_DIP).max(40.0) * s).min(sub.width());
    let cur_w = (sub.width() * (1.0 - 0.22 * r)).max(min_w);
    let drop = 12.0 * s * r;
    let max_bot = bottom_limit.min(anim_primary.bottom - 2.0 * s);
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

fn window_master_alpha(phase: PopupAnimPhase, open_elapsed: f32, close_progress: f32) -> f32 {
    match phase {
        PopupAnimPhase::Opening => {
            (0.18 + 0.82 * ease_out_cubic((open_elapsed / 0.075).clamp(0.0, 1.0))).clamp(0.0, 1.0)
        }
        PopupAnimPhase::Open => 1.0,
        PopupAnimPhase::Closing => (1.0 - close_progress.clamp(0.0, 1.0)).powf(1.35),
    }
}

/// Computes vertical `(top, bottom)` for a cascading submenu:
/// 1. Prefers aligning first submenu item's top with `row_top` (`row_top - inner_pad`) downward.
/// 2. When downward space above `bottom_limit` is insufficient, aligns the last submenu item's
///    bottom with the primary menu's last item (`bottom_limit`) and expands upward.
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

/// Resolves the menu's dynamic UI accent color from the active source cover (matching spectrum bars),
/// falling back to any available source cover or the system accent color.
fn resolve_dynamic_menu_accent(
    sources: &[SourceItem],
    active_id: Option<&str>,
    fallback: Rgb,
    light_theme: bool,
) -> Rgb {
    let cover = active_id
        .and_then(|aid| sources.iter().find(|src| src.id == aid))
        .and_then(|src| src.cover.as_deref())
        .or_else(|| sources.iter().find_map(|src| src.cover.as_deref()));
    cover
        .map(|cov| {
            if light_theme {
                cov.accent_light
            } else {
                cov.accent_dark
            }
        })
        .unwrap_or_else(|| adjust_accent_for_theme(fallback, light_theme))
}

/// Computes two soft, restrained silk-tint colors derived from the active cover's purified accent color.
fn compute_cover_halo_pair(
    cover: Option<&MediaCover>,
    fallback: Rgb,
    light_theme: bool,
) -> [Rgb; 2] {
    let base_rgb = cover.map_or_else(
        || adjust_accent_for_theme(fallback, false),
        |cov| cov.accent_dark,
    );
    let (h, s, l) = rgb_to_hsl(base_rgb);
    if s < 0.12 {
        let s0 = s.min(0.10);
        let (l0, l1) = if light_theme { (0.66, 0.60) } else { (0.76, 0.82) };
        let c0 = hsl_to_rgb(h, s0, l0);
        let c1 = hsl_to_rgb(h, s0, l1);
        return [c0, c1];
    }

    let s0 = (s * 0.92).clamp(0.12, 0.72);
    let l0 = if light_theme {
        l.clamp(0.60, 0.72)
    } else {
        l.clamp(0.55, 0.68)
    };
    let c0 = hsl_to_rgb(h, s0, l0);

    let shift = 24.0 * ((s - 0.12) / 0.35).clamp(0.25, 1.0);
    let h1 = (h + shift).rem_euclid(360.0);
    let s1 = (s0 * 0.90).clamp(0.10, 0.66);
    let l1 = (l0 + 0.04).clamp(0.57, 0.75);
    let c1 = hsl_to_rgb(h1, s1, l1);
    [c0, c1]
}

/// Evaluates two restrained, domain-warped liquid silk light ribbons with wide clear-glass negative space.
fn sample_liquid_silk_field(
    x: f32,
    y: f32,
    phase: f32,
    colors: &[(f32, f32, f32); 2],
    light_glass: bool,
) -> (f32, f32, f32, f32) {
    let wx = x
        + 0.28 * (2.1 * y + 0.29 * phase).sin()
        + 0.14 * (3.3 * y - 0.19 * phase + 1.3).cos();
    let wy = y
        + 0.26 * (1.9 * x - 0.23 * phase + 0.7).cos()
        + 0.13 * (3.1 * x + 0.31 * phase + 2.4).sin();

    let ang1 = 0.68 + 0.26 * (0.14 * phase).sin();
    let (s1, c1_ang) = ang1.sin_cos();
    let u1 = wx * c1_ang + wy * s1 - 0.19 * phase;
    let v1 = -wx * s1 + wy * c1_ang;
    let crest1 = (0.5 + 0.5 * (u1 * 5.0).sin()).powi(5);
    let along1 = 0.45 + 0.55 * (0.5 + 0.5 * (v1 * 2.4 + 0.34 * phase).sin());
    let breathe1 = 0.82 + 0.18 * (0.37 * phase + 0.5).sin();
    let w1 = crest1 * along1 * breathe1;

    let ang2 = -0.82 + 0.22 * (0.11 * phase + 1.8).cos();
    let (s2, c2_ang) = ang2.sin_cos();
    let u2 = wx * c2_ang + wy * s2 + 0.15 * phase + 1.9;
    let v2 = -wx * s2 + wy * c2_ang;
    let crest2 = (0.5 + 0.5 * (u2 * 4.4).sin()).powi(6);
    let along2 = 0.40 + 0.60 * (0.5 + 0.5 * (v2 * 2.1 - 0.27 * phase + 2.2).cos());
    let breathe2 = 0.80 + 0.20 * (0.29 * phase + 2.1).cos();
    let w2 = crest2 * along2 * breathe2 * 0.68;

    let w_sum = w1 + w2;
    if w_sum <= 1e-4 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let (c0, c1) = (colors[0], colors[1]);
    let r = ((c0.0 * w1 + c1.0 * w2) / w_sum).clamp(0.0, 1.0);
    let g = ((c0.1 * w1 + c1.1 * w2) / w_sum).clamp(0.0, 1.0);
    let b = ((c0.2 * w1 + c1.2 * w2) / w_sum).clamp(0.0, 1.0);

    let peak_a = if light_glass { 0.095 } else { 0.125 };
    let raw_t = w_sum.min(1.2);
    let alpha = (peak_a * (raw_t / (1.0 + 0.30 * raw_t))).clamp(0.0, peak_a);
    (r, g, b, alpha)
}

/// Captures the desktop region directly behind the popup and applies soft frosted blur.
fn capture_blurred_desktop_bitmap(
    rt: &ID2D1DCRenderTarget,
    screen_x: i32,
    screen_y: i32,
    w: i32,
    h: i32,
) -> Option<ID2D1Bitmap> {
    if w <= 0 || h <= 0 {
        return None;
    }

    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return None;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_invalid() {
            let _ = ReleaseDC(None, screen_dc);
            return None;
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits_ptr = null_mut();
        let Ok(bmp) = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0) else {
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
            return None;
        };
        let old = SelectObject(mem_dc, bmp);
        let _ = BitBlt(mem_dc, 0, 0, w, h, screen_dc, screen_x, screen_y, SRCCOPY);
        let _ = ReleaseDC(None, screen_dc);

        let result = if !bits_ptr.is_null() {
            let raw = std::slice::from_raw_parts(bits_ptr.cast::<u8>(), (w as usize) * (h as usize) * 4);
            let dw = (w as usize / 4).max(1);
            let dh = (h as usize / 4).max(1);
            let mut down = Vec::with_capacity(dw * dh * 4);
            for by in 0..dh {
                let y0 = (by * 4).min((h as usize) - 1);
                let y1 = ((by + 1) * 4).min(h as usize).max(y0 + 1);
                for bx in 0..dw {
                    let x0 = (bx * 4).min((w as usize) - 1);
                    let x1 = ((bx + 1) * 4).min(w as usize).max(x0 + 1);
                    let mut sum_b = 0u32;
                    let mut sum_g = 0u32;
                    let mut sum_r = 0u32;
                    let mut count = 0u32;
                    for sy in y0..y1 {
                        for sx in x0..x1 {
                            let idx = (sy * (w as usize) + sx) * 4;
                            sum_b += u32::from(raw[idx]);
                            sum_g += u32::from(raw[idx + 1]);
                            sum_r += u32::from(raw[idx + 2]);
                            count += 1;
                        }
                    }
                    let count = count.max(1);
                    down.push((sum_b / count) as u8);
                    down.push((sum_g / count) as u8);
                    down.push((sum_r / count) as u8);
                    down.push(255);
                }
            }
            let blurred = gaussian_blur_bgra(&down, dw, dh, 6);
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            rt.CreateBitmap(
                D2D_SIZE_U {
                    width: dw as u32,
                    height: dh as u32,
                },
                Some(blurred.as_ptr().cast()),
                (dw * 4) as u32,
                &props,
            )
            .ok()
        } else {
            None
        };

        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem_dc);
        result
    }
}

unsafe extern "system" fn popup_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PopupInner;
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
        WM_CAPTURECHANGED => {
            if state.dragging_slider.take().is_some()
                && IsWindowVisible(hwnd).as_bool()
                && state.anim_phase != PopupAnimPhase::Closing
            {
                let _ = state.render();
            }
            LRESULT(0)
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
            if let Some(target) = state.dragging_slider {
                if state.apply_slider_drag(target, x) {
                    let _ = state.render();
                }
                return LRESULT(0);
            }
            if state.update_hover_at(x, y) {
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            state.tracking_leave = false;
            if state.anim_phase != PopupAnimPhase::Closing
                && state.dragging_slider.is_none()
                && state.hover_target != HoverTarget::None
            {
                state.hover_target = HoverTarget::None;
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
            if !state.point_inside_cards(x, y) {
                state.begin_close();
                return LRESULT(0);
            }
            let _ = state.update_hover_at(x, y);
            match state.hover_target {
                HoverTarget::SubVolMasterSlider => {
                    state.dragging_slider = Some(SliderTarget::Master);
                    let _ = SetCapture(hwnd);
                    let _ = state.apply_slider_drag(SliderTarget::Master, x);
                    let _ = state.render();
                }
                HoverTarget::SubVolAppSlider(idx) => {
                    state.dragging_slider = Some(SliderTarget::App(idx));
                    let _ = SetCapture(hwnd);
                    let _ = state.apply_slider_drag(SliderTarget::App(idx), x);
                    let _ = state.render();
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if state.anim_phase == PopupAnimPhase::Closing {
                return LRESULT(0);
            }
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            if state.dragging_slider.take().is_some() {
                let _ = ReleaseCapture();
                let _ = state.update_hover_at(x, y);
                let _ = state.render();
                return LRESULT(0);
            }

            let _ = state.update_hover_at(x, y);

            match state.hover_target {
                HoverTarget::SubAuto => {
                    state.mode = SelectionMode::Auto;
                    let _ = PostMessageW(
                        state.main_hwnd,
                        WM_APP_SOURCE_SELECTED,
                        WPARAM(usize::MAX),
                        LPARAM(0),
                    );
                    let _ = state.render();
                }
                HoverTarget::SubSource(idx) => {
                    if let Some(src) = state.sources.get(idx) {
                        let src_id = src.id.clone();
                        state.mode = SelectionMode::Manual(src_id.clone());
                        state.active_id = Some(src_id);
                        let _ = PostMessageW(
                            state.main_hwnd,
                            WM_APP_SOURCE_SELECTED,
                            WPARAM(idx),
                            LPARAM(0),
                        );
                        let _ = state.render();
                    }
                }
                HoverTarget::SubVolMasterMute => {
                    state.toggle_master_mute();
                    let _ = state.render();
                }
                HoverTarget::SubVolAppMute(idx) => {
                    if let Some(app) = state.app_volumes.get_mut(idx) {
                        app.toggle_group_mute();
                        let _ = state.render();
                    }
                }
                HoverTarget::SubPlayback(idx) => {
                    if let Some(dev_id) = state.playback_devices.get(idx).map(|d| d.id.clone()) {
                        if set_default_audio_endpoint(&dev_id) {
                            state.playback_devices = enumerate_audio_endpoints(false);
                            state.reload_volumes();
                            let _ = PostMessageW(
                                state.main_hwnd,
                                WM_APP_AUDIO_OUTPUT_CHANGED,
                                WPARAM(0),
                                LPARAM(0),
                            );
                            let _ = state.render();
                        }
                    }
                }
                HoverTarget::SubRecording(idx) => {
                    if let Some(dev_id) = state.recording_devices.get(idx).map(|d| d.id.clone()) {
                        if set_default_audio_endpoint(&dev_id) {
                            state.recording_devices = enumerate_audio_endpoints(true);
                            let _ = state.render();
                        }
                    }
                }
                HoverTarget::SubSettingLock => {
                    state.locked = !state.locked;
                    let _ = PostMessageW(
                        state.main_hwnd,
                        WM_APP_SETTING_ACTION,
                        WPARAM(SETTING_TOGGLE_LOCK),
                        LPARAM(0),
                    );
                    let _ = state.render();
                }
                HoverTarget::SubSettingAutostart => {
                    let next = !state.autostart;
                    if set_autostart_enabled(next) {
                        state.autostart = next;
                        let _ = state.render();
                    }
                }
                HoverTarget::SubSettingResetPos => {
                    let _ = PostMessageW(
                        state.main_hwnd,
                        WM_APP_SETTING_ACTION,
                        WPARAM(SETTING_RESET_POS),
                        LPARAM(0),
                    );
                    let _ = state.render();
                }
                HoverTarget::SubSettingExit => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    state.reset_transient_state();
                    let _ = PostMessageW(
                        state.main_hwnd,
                        WM_APP_SETTING_ACTION,
                        WPARAM(SETTING_EXIT),
                        LPARAM(0),
                    );
                }
                HoverTarget::Primary(_)
                | HoverTarget::SubVolMasterSlider
                | HoverTarget::SubVolMasterRow
                | HoverTarget::SubVolAppSlider(_)
                | HoverTarget::SubVolAppRow(_)
                | HoverTarget::None => {}
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if state.anim_phase == PopupAnimPhase::Closing {
                return LRESULT(0);
            }
            let wheel_delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
            if wheel_delta == 0 {
                return LRESULT(0);
            }
            let mut pt = POINT {
                x: i32::from((lparam.0 & 0xFFFF) as i16),
                y: i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let _ = state.update_hover_at(pt.x as f32, pt.y as f32);

            let handled = match state.hover_target {
                HoverTarget::SubVolMasterMute
                | HoverTarget::SubVolMasterSlider
                | HoverTarget::SubVolMasterRow => {
                    state.step_master_volume_wheel(wheel_delta);
                    true
                }
                HoverTarget::SubVolAppMute(idx)
                | HoverTarget::SubVolAppSlider(idx)
                | HoverTarget::SubVolAppRow(idx) => {
                    if let Some(app) = state.app_volumes.get_mut(idx) {
                        app.step_group_wheel(wheel_delta);
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };
            if handled {
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                SetCursor(state.arrow_cursor);
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCHITTEST => {
            if state.anim_phase == PopupAnimPhase::Closing {
                LRESULT(HTTRANSPARENT as isize)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_halo_pair_preserves_clean_accent_for_dark_and_light_themes() {
        let teal_cover = MediaCover {
            hash: 1,
            thumb_w: 2,
            thumb_h: 2,
            thumb_pbgra: vec![255; 16],
            accent_dark: Rgb::new(72, 195, 220),
            accent_light: Rgb::new(40, 128, 172),
        };
        let warm_cover = MediaCover {
            hash: 2,
            thumb_w: 2,
            thumb_h: 2,
            thumb_pbgra: vec![255; 16],
            accent_dark: Rgb::new(235, 192, 86),
            accent_light: Rgb::new(155, 118, 30),
        };
        let gray_cover = MediaCover {
            hash: 3,
            thumb_w: 2,
            thumb_h: 2,
            thumb_pbgra: vec![255; 16],
            accent_dark: Rgb::new(128, 128, 128),
            accent_light: Rgb::new(128, 128, 128),
        };

        for light_theme in [false, true] {
            for cov in [Some(&teal_cover), Some(&warm_cover), None] {
                let pair = compute_cover_halo_pair(cov, Rgb::new(0, 120, 212), light_theme);
                for c in pair {
                    let (_, s, l) = rgb_to_hsl(c);
                    assert!(
                        (0.40..=0.74).contains(&s),
                        "expected restrained halo saturation in [0.40, 0.74], got {s} ({c:?})"
                    );
                    assert!(
                        (0.52..=0.78).contains(&l),
                        "expected controlled halo luminance in [0.52, 0.78], got {l} ({c:?})"
                    );
                }
            }

            let gray_pair =
                compute_cover_halo_pair(Some(&gray_cover), Rgb::new(0, 120, 212), light_theme);
            for c in gray_pair {
                let (_, s, l) = rgb_to_hsl(c);
                assert!(
                    s <= 0.12,
                    "grayscale cover halo must stay neutral pearl gray, got s={s} ({c:?})"
                );
                assert!(
                    (0.55..=0.85).contains(&l),
                    "expected soft grayscale halo luminance in [0.55, 0.85], got {l} ({c:?})"
                );
            }
        }
    }

    #[test]
    fn liquid_silk_field_stays_sparse_smooth_and_restrained() {
        let colors = [(0.25, 0.65, 0.80), (0.55, 0.45, 0.85)];
        for light_glass in [false, true] {
            let cap = if light_glass { 0.096 } else { 0.126 };
            let mut max_alpha = 0.0f32;
            let mut quiet_samples = 0usize;
            let mut total_samples = 0usize;
            let mut max_step = 0.0f32;
            let nx = 64usize;
            let ny = 48usize;
            for iy in 0..ny {
                let y = (iy as f32) / (ny as f32) * 1.4;
                let mut prev_a = 0.0f32;
                for ix in 0..nx {
                    let x = (ix as f32) / (nx as f32) * 2.0;
                    let (r, g, b, a) = sample_liquid_silk_field(x, y, 1.25, &colors, light_glass);
                    assert!((0.0..=1.0).contains(&r));
                    assert!((0.0..=1.0).contains(&g));
                    assert!((0.0..=1.0).contains(&b));
                    assert!(a >= 0.0 && a <= cap, "alpha exceeded cap {cap}: {a}");
                    max_alpha = max_alpha.max(a);
                    if a < 0.025 {
                        quiet_samples += 1;
                    }
                    total_samples += 1;
                    if ix > 0 {
                        max_step = max_step.max((a - prev_a).abs());
                    }
                    prev_a = a;
                }
            }
            let quiet_ratio = (quiet_samples as f32) / (total_samples as f32);
            assert!(
                quiet_ratio >= 0.45,
                "expected >= 45% clear negative space (alpha < 0.025), got {quiet_ratio}"
            );
            assert!(
                max_alpha >= 0.04,
                "expected visible silk crest >= 0.04, got {max_alpha}"
            );
            assert!(max_step < 0.035, "silk field step too steep: {max_step}");
        }
    }

    #[test]
    fn dynamic_menu_accent_matches_active_cover_and_falls_back_cleanly() {
        use std::sync::Arc;

        let cover_a = Arc::new(MediaCover {
            hash: 10,
            thumb_w: 2,
            thumb_h: 2,
            thumb_pbgra: vec![255; 16],
            accent_dark: Rgb::new(228, 186, 78),
            accent_light: Rgb::new(152, 114, 24),
        });
        let cover_b = Arc::new(MediaCover {
            hash: 20,
            thumb_w: 2,
            thumb_h: 2,
            thumb_pbgra: vec![255; 16],
            accent_dark: Rgb::new(82, 205, 198),
            accent_light: Rgb::new(28, 138, 132),
        });
        let mk = |id: &str, cover: Option<Arc<MediaCover>>| SourceItem {
            id: id.to_string(),
            app_name: id.to_string(),
            title: id.to_string(),
            artist: String::new(),
            status: PlaybackState::Playing,
            pid: None,
            cover,
        };

        let sources = vec![
            mk("a", Some(cover_a.clone())),
            mk("b", Some(cover_b.clone())),
            mk("no_cover", None),
        ];
        let fallback = Rgb::new(0, 120, 212);

        // Matches active_id cover per theme
        assert_eq!(
            resolve_dynamic_menu_accent(&sources, Some("b"), fallback, false),
            cover_b.accent_dark
        );
        assert_eq!(
            resolve_dynamic_menu_accent(&sources, Some("a"), fallback, true),
            cover_a.accent_light
        );

        // Active source without cover falls back to first source cover
        assert_eq!(
            resolve_dynamic_menu_accent(&sources, Some("no_cover"), fallback, false),
            cover_a.accent_dark
        );

        // No covers anywhere falls back to calibrated system accent
        let plain_sources = vec![mk("no_cover", None)];
        assert_eq!(
            resolve_dynamic_menu_accent(&plain_sources, Some("no_cover"), fallback, false),
            adjust_accent_for_theme(fallback, false)
        );
        assert_eq!(
            resolve_dynamic_menu_accent(&plain_sources, Some("no_cover"), fallback, true),
            adjust_accent_for_theme(fallback, true)
        );
    }

    #[test]
    fn submenu_vertical_span_prefers_downward_then_aligns_last_item_upward() {
        let pad = 8.0f32;
        let top_limit = 14.0f32;
        let bottom_limit = 316.0f32; // primary last row (`设置`, top=274, bottom=308) + pad

        // 1. Ample downward space: top aligns with parent row (`row_top - pad`)
        let (t1, b1) = compute_submenu_vertical_span(122.0, pad, 110.0, top_limit, bottom_limit);
        assert!((t1 - 114.0).abs() < 0.01);
        assert!((b1 - 224.0).abs() < 0.01);

        // 2. Middle row without enough downward space: last submenu item aligns with primary last item (`bottom_limit`)
        let (t2, b2) = compute_submenu_vertical_span(236.0, pad, 140.0, top_limit, bottom_limit);
        assert!((b2 - bottom_limit).abs() < 0.01);
        assert!((t2 - 176.0).abs() < 0.01);
        assert!(((b2 - pad) - 308.0).abs() < 0.01);

        // 3. Bottom-most Settings row (`退出` 34px aligns flush with `设置` 34px at bottom_limit)
        let (t3, b3) = compute_submenu_vertical_span(274.0, pad, 169.0, top_limit, bottom_limit);
        assert!((b3 - bottom_limit).abs() < 0.01);
        assert!((t3 - 147.0).abs() < 0.01);
        let exit_bottom = b3 - pad;
        let exit_top = exit_bottom - 34.0;
        assert!((exit_bottom - 308.0).abs() < 0.01);
        assert!((exit_top - 274.0).abs() < 0.01);

        // 4. Extremely tall submenu exceeding upward headroom clamps cleanly within [top_limit, bottom_limit]
        let (t4, b4) = compute_submenu_vertical_span(160.0, pad, 340.0, top_limit, bottom_limit);
        assert!((t4 - top_limit).abs() < 0.01);
        assert!((b4 - bottom_limit).abs() < 0.01);
    }

    #[test]
    fn popup_anim_curves_and_morph_stay_bounded_and_safe() {
        // 1. Easing curves & window alpha boundaries
        assert!((ease_out_cubic(0.0) - 0.0).abs() < 1e-5);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < 1e-5);
        assert!((ease_retract(0.0) - 0.0).abs() < 1e-5);
        assert!((ease_retract(1.0) - 1.0).abs() < 1e-5);
        assert!((window_master_alpha(PopupAnimPhase::Opening, 0.0, 0.0) - 0.18).abs() < 1e-5);
        assert!((window_master_alpha(PopupAnimPhase::Opening, 0.075, 0.0) - 1.0).abs() < 1e-5);
        assert!((window_master_alpha(PopupAnimPhase::Open, 1.0, 0.0) - 1.0).abs() < 1e-5);
        assert!((window_master_alpha(PopupAnimPhase::Closing, 1.0, 1.0) - 0.0).abs() < 1e-5);

        // 2. Open spring convergence, liquid surface-tension swell [1.02..=1.06], & frame-rate independence
        let mut s_10ms = Spring1D::new(0.08, 1.6);
        let mut s_25ms = Spring1D::new(0.08, 1.6);
        let mut peak_pos = s_10ms.pos;
        let mut max_dt_diff = 0.0f32;
        for step in 1..=30 {
            let _ = step;
            for _ in 0..5 {
                s_10ms.step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, 0.010);
                peak_pos = peak_pos.max(s_10ms.pos);
            }
            for _ in 0..2 {
                s_25ms.step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, 0.025);
            }
            max_dt_diff = max_dt_diff.max((s_10ms.pos - s_25ms.pos).abs());
        }
        assert!(
            (1.02..=1.06).contains(&peak_pos),
            "expected liquid spring crest in [1.02, 1.06], got {peak_pos}"
        );
        assert!(
            max_dt_diff < 0.01,
            "expected sub-stepped spring frame-rate invariance (<0.01), got {max_dt_diff}"
        );
        assert!(
            s_10ms.is_settled(1.0, 0.002, 0.02),
            "open spring failed to settle: {s_10ms:?}"
        );

        // 3. Staggered bottom-to-top row micro-glide & clean zeroing at rest
        let dy_top = primary_row_stagger_dy(0, 0.04, 0.55, 1.0);
        let dy_bot = primary_row_stagger_dy(4, 0.04, 0.55, 1.0);
        assert!(dy_top > dy_bot, "top row should trail bottom row during rise");
        let dy_rest = primary_row_stagger_dy(0, 0.25, 1.0, 1.0);
        assert!(dy_rest.abs() < 1e-4, "row stagger did not settle to zero: {dy_rest}");

        // 4. Primary envelope stays strictly anchored to MediaBar bottom/sides with safe top >= 4 DIP & min height
        let target_primary = RectF::new(14.0, 14.0, 162.0, 220.0);
        for step in 0..=20 {
            let p = (step as f32) / 20.0;
            let op = animated_primary_rect(
                target_primary,
                PopupAnimPhase::Opening,
                0.08 + 0.98 * p,
                0.0,
                1.0,
            );
            assert!((op.left - target_primary.left).abs() < 1e-4);
            assert!((op.right - target_primary.right).abs() < 1e-4);
            assert!((op.bottom - target_primary.bottom).abs() < 1e-4);
            assert!(op.top >= 4.0);
            assert!(op.height() >= 42.0);

            let cl = animated_primary_rect(target_primary, PopupAnimPhase::Closing, 1.0, p, 1.0);
            assert!((cl.left - target_primary.left).abs() < 1e-4);
            assert!((cl.right - target_primary.right).abs() < 1e-4);
            assert!((cl.bottom - target_primary.bottom).abs() < 1e-4);
            assert!(cl.height() >= 42.0);
        }

        // 5. Submenu spring continuity on rapid target retargeting, lateral budding & closing bounds
        let mut morph_w = Spring1D::still(172.0);
        for _ in 0..5 {
            morph_w.step(280.0, SUB_WIDTH_STIFFNESS, SUB_WIDTH_DAMPING, 0.010);
        }
        let vel_before_retarget = morph_w.vel;
        assert!(vel_before_retarget > 100.0);
        morph_w.step(264.0, SUB_WIDTH_STIFFNESS, SUB_WIDTH_DAMPING, 0.010);
        assert!(
            morph_w.vel > 0.0,
            "spring velocity should remain continuous across retargeting"
        );

        let sub = RectF::new(162.0, 106.0, 438.0, 316.0);
        let row = RectF::new(22.0, 122.0, 154.0, 156.0);
        for open_left in [false, true] {
            let seam_x = if open_left { 300.0 } else { 162.0 };
            let target_s = if open_left {
                RectF::new(24.0, 106.0, 300.0, 316.0)
            } else {
                sub
            };
            let bud = sub_bud_start_rect(target_s, row, seam_x, open_left, 1.0);
            assert!(bud.width() >= 40.0, "bud width too narrow: {}", bud.width());
            assert!(bud.height() >= 44.0);
            if open_left {
                assert!((bud.right - seam_x).abs() < 1e-4);
            } else {
                assert!((bud.left - seam_x).abs() < 1e-4);
            }

            let clamped = clamp_sub_spring_rect(
                -10.0,
                400.0,
                295.0,
                target_s,
                seam_x,
                316.0,
                460.0,
                open_left,
                1.0,
            );
            assert!(clamped.width() >= 40.0);
            assert!(clamped.top >= 4.0);
            assert!(clamped.bottom <= 316.0);

            let close_sub = animated_closing_sub_rect(
                target_s,
                RectF::new(162.0, 190.0, 310.0, 320.0),
                seam_x,
                316.0,
                open_left,
                0.7,
                1.0,
            );
            assert!(close_sub.width() >= 40.0);
            assert!(close_sub.height() >= 42.0);
            assert!(close_sub.bottom <= 316.0);
        }
    }
}
