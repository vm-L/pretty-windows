use super::arbitration::PlaybackState;
use super::dsp::NUM_BARS;
use super::layout::{
    compute_mini_player_window_origin, MiniPlayerHitZone, MiniPlayerLayout, RectF,
};
use super::marquee::MarqueeState;
use super::render::{
    draw_next_icon, draw_pause_icon, draw_play_icon, draw_prev_icon, draw_speaker_glyph,
};
use super::smtc::{MediaCover, SourceItem};
use super::timeline::{current_filetime_100ns, format_media_time, MediaTimeline};
use super::volume::{format_volume_pct, slider_ratio_from_x};
use crate::color::{
    derive_background_accent, enforce_ui_accent_deadzone, Rgb, NEUTRAL_BACKDROP_FALLBACK,
};
use crate::system::{active_taskbar_surface_style, visible_transient_overlay_rects};
use crate::ui::glass::{
    capture_blurred_desktop_backdrop_excluding, create_round_cap_stroke, create_text_format,
    d2d_color_rgb, d2d_rect, draw_glass_interior, draw_glass_slider, draw_music_note_icon,
    draw_rounded_card_rim, draw_rounded_card_shadows, draw_text_in_rect, identity_matrix,
    muted_text_color, primary_text_color, update_silk_bitmap, LayeredSurface, PopupNoiseCache,
};
use crate::ui::{
    compute_halo_pair, ease_out_cubic, ease_retract, popup_glass_recipe, rgb_luma,
    window_master_alpha, PopupAnimPhase, Spring1D, CLOSE_ANIM_DUR_S, OPEN_SPRING_DAMPING,
    OPEN_SPRING_STIFFNESS, POPUP_ANIM_INTERVAL_MS, POPUP_FLOW_INTERVAL_MS,
    POPUP_REOPEN_DEBOUNCE_MS, SILK_H, SILK_W,
};
use std::mem::{size_of, ManuallyDrop};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
    D2D_POINT_2F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Geometry, ID2D1Layer,
    ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE,
    D2D1_LAYER_PARAMETERS, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_TEXT_METRICS,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromRect, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
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

pub const WM_APP_MINI_PLAYER_CLOSED: u32 = WM_APP + 7;
pub const WM_APP_MINI_PLAYER_ACTION: u32 = WM_APP + 8;
pub const WM_APP_MINI_PLAYER_SEEK: u32 = WM_APP + 9;
pub const WM_APP_MINI_PLAYER_WHEEL: u32 = WM_APP + 10;

pub const MINI_ACTION_PREV: usize = 1;
pub const MINI_ACTION_PLAY_PAUSE: usize = 2;
pub const MINI_ACTION_NEXT: usize = 3;
pub const MINI_ACTION_ACTIVATE: usize = 4;
pub const MINI_ACTION_OPEN_MENU: usize = 5;
pub const MINI_ACTION_TOGGLE_MUTE: usize = 6;

const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const MINI_PLAYER_CLASS_NAME: PCWSTR = w!("MediaBarMiniPlayerWnd");
const TIMER_MINI_TICK_ID: usize = 1;
const VOL_BADGE_TTL: Duration = Duration::from_millis(1200);
const OPTIMISTIC_SEEK_TTL: Duration = Duration::from_millis(700);

#[derive(Clone)]
struct CachedFormats {
    dpi: u32,
    badge_fmt: IDWriteTextFormat,
    title_fmt: IDWriteTextFormat,
    artist_fmt: IDWriteTextFormat,
    time_l_fmt: IDWriteTextFormat,
    time_r_fmt: IDWriteTextFormat,
}

struct CachedText {
    text: String,
    dpi: u32,
    layout: IDWriteTextLayout,
    width: f32,
}

struct CachedArtBitmap {
    hash: u64,
    bitmap: ID2D1Bitmap,
}

struct MiniPlayerInner {
    hwnd: HWND,
    main_hwnd: HWND,
    d2d_factory: ID2D1Factory1,
    dwrite_factory: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    card_layer: ID2D1Layer,
    art_layer: ID2D1Layer,
    marquee_layer: ID2D1Layer,
    round_stroke: Option<ID2D1StrokeStyle>,
    arrow_cursor: HCURSOR,

    dpi: u32,
    anchor_rect: RECT,
    light_glass: bool,
    backdrop_rgb: Rgb,
    auto_color: bool,
    source: Option<SourceItem>,
    bar_levels: [f32; NUM_BARS],
    master_vol: f32,
    master_muted: bool,
    vol_badge_until: Option<Instant>,

    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
    has_timeline: bool,

    hover_zone: MiniPlayerHitZone,
    pressed_zone: MiniPlayerHitZone,
    tracking_leave: bool,
    dragging_seek_ratio: Option<f32>,
    pending_seek_100ns: Option<i64>,
    optimistic_seek: Option<(i64, i64, Instant)>,

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
    close_started_at: Option<Instant>,
    timer_interval_ms: u32,

    title_marquee: MarqueeState,
    artist_marquee: MarqueeState,
    cached_formats: Option<CachedFormats>,
    cached_title: Option<CachedText>,
    cached_artist: Option<CachedText>,
    cached_art: Option<CachedArtBitmap>,
}

/// Floating Liquid Glass mini-player card anchored above `MediaBar`.
pub struct MiniPlayerPopup {
    hwnd: HWND,
    inner: Box<MiniPlayerInner>,
}

impl MiniPlayerPopup {
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
                lpfnWndProc: Some(mini_player_wnd_proc),
                hInstance: hinst.into(),
                hCursor: arrow_cursor,
                lpszClassName: MINI_PLAYER_CLASS_NAME,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                MINI_PLAYER_CLASS_NAME,
                w!("MediaBar Mini Player"),
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
            let card_layer = rt.CreateLayer(None)?;
            let art_layer = rt.CreateLayer(None)?;
            let marquee_layer = rt.CreateLayer(None)?;
            let surface = LayeredSurface::new()?;
            let round_stroke = create_round_cap_stroke(&d2d_factory);

            let mut inner = Box::new(MiniPlayerInner {
                hwnd,
                main_hwnd,
                d2d_factory,
                dwrite_factory,
                rt,
                surface,
                card_layer,
                art_layer,
                marquee_layer,
                round_stroke,
                arrow_cursor,
                dpi: 96,
                anchor_rect: RECT::default(),
                light_glass: false,
                backdrop_rgb: NEUTRAL_BACKDROP_FALLBACK,
                auto_color: true,
                source: None,
                bar_levels: [0.0; NUM_BARS],
                master_vol: 1.0,
                master_muted: false,
                vol_badge_until: None,
                win_x: 0,
                win_y: 0,
                win_w: 1,
                win_h: 1,
                has_timeline: false,
                hover_zone: MiniPlayerHitZone::None,
                pressed_zone: MiniPlayerHitZone::None,
                tracking_leave: false,
                dragging_seek_ratio: None,
                pending_seek_100ns: None,
                optimistic_seek: None,
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
                close_started_at: None,
                timer_interval_ms: 0,
                title_marquee: MarqueeState::default(),
                artist_marquee: MarqueeState::default(),
                cached_formats: None,
                cached_title: None,
                cached_artist: None,
                cached_art: None,
            });

            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (&mut *inner as *mut MiniPlayerInner) as isize,
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

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.inner.reset_transient_state();
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

    pub fn take_pending_seek(&mut self) -> Option<i64> {
        self.inner.pending_seek_100ns.take()
    }

    pub fn set_system_theme(&mut self, light_glass: bool, backdrop_rgb: Rgb) {
        let changed =
            self.inner.light_glass != light_glass || self.inner.backdrop_rgb != backdrop_rgb;
        self.inner.light_glass = light_glass;
        self.inner.backdrop_rgb = backdrop_rgb;
        if changed {
            self.inner.halo_colors_ready = false;
            self.inner.last_silk_tick = None;
            if self.is_visible() {
                let _ = self.inner.render();
            }
        }
    }

    pub fn set_auto_color(&mut self, auto_color: bool) {
        if self.inner.auto_color != auto_color {
            self.inner.auto_color = auto_color;
            self.inner.halo_colors_ready = false;
            self.inner.last_silk_tick = None;
            if self.is_visible() {
                let _ = self.inner.render();
            }
        }
    }

    pub fn update_volume(&mut self, volume: f32, muted: bool, show_badge: bool) {
        let changed = (self.inner.master_vol - volume).abs() > 0.002
            || self.inner.master_muted != muted
            || show_badge;
        self.inner.master_vol = volume.clamp(0.0, 1.0);
        self.inner.master_muted = muted;
        if show_badge {
            self.inner.vol_badge_until = Some(Instant::now() + VOL_BADGE_TTL);
        }
        if changed && self.is_visible() {
            let _ = self.inner.render();
        }
    }

    pub fn update_bars(&mut self, bars: &[f32; NUM_BARS]) {
        self.inner.bar_levels = *bars;
    }

    pub fn update_live_source(&mut self, source: Option<SourceItem>) {
        let prev_has_tl = self.inner.has_timeline;
        let next_has_tl = source
            .as_ref()
            .and_then(|s| s.timeline)
            .is_some_and(|tl| tl.is_valid());
        let prev_track = self
            .inner
            .source
            .as_ref()
            .map(|s| (s.id.as_str(), s.title.as_str(), s.artist.as_str()));
        let next_track = source
            .as_ref()
            .map(|s| (s.id.as_str(), s.title.as_str(), s.artist.as_str()));
        if prev_track != next_track {
            self.inner.dragging_seek_ratio = None;
            self.inner.optimistic_seek = None;
        }
        self.inner.source = source;
        self.inner.has_timeline = next_has_tl;

        if self.is_visible() {
            if prev_has_tl != next_has_tl {
                self.inner.reposition_and_recapture();
            }
            let _ = self.inner.render();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        anchor_rect: RECT,
        dpi: u32,
        light_glass: bool,
        fallback_backdrop_rgb: Rgb,
        auto_color: bool,
        source: Option<SourceItem>,
        bars: &[f32; NUM_BARS],
        master_vol: f32,
        master_muted: bool,
    ) {
        self.inner.dpi = dpi.max(96);
        self.inner.anchor_rect = anchor_rect;
        self.inner.light_glass = light_glass;
        self.inner.backdrop_rgb = fallback_backdrop_rgb;
        self.inner.auto_color = auto_color;
        self.inner.source = source;
        self.inner.bar_levels = *bars;
        self.inner.master_vol = master_vol.clamp(0.0, 1.0);
        self.inner.master_muted = master_muted;
        self.inner.vol_badge_until = None;
        self.inner.hover_zone = MiniPlayerHitZone::None;
        self.inner.pressed_zone = MiniPlayerHitZone::None;
        self.inner.dragging_seek_ratio = None;
        self.inner.pending_seek_100ns = None;
        self.inner.optimistic_seek = None;
        self.inner.has_timeline = self
            .inner
            .source
            .as_ref()
            .and_then(|s| s.timeline)
            .is_some_and(|tl| tl.is_valid());

        self.inner.reposition_and_recapture();

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
                self.inner.win_x,
                self.inner.win_y,
                self.inner.win_w,
                self.inner.win_h,
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

impl Drop for MiniPlayerPopup {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_MINI_TICK_ID);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl MiniPlayerInner {
    fn current_layout(&self) -> MiniPlayerLayout {
        MiniPlayerLayout::compute(self.dpi, self.has_timeline, &self.bar_levels)
    }

    fn is_playing(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|s| s.status == PlaybackState::Playing)
    }

    fn effective_timeline(&self) -> Option<MediaTimeline> {
        let mut tl = self.source.as_ref()?.timeline?;
        if !tl.is_valid() {
            return None;
        }
        if let Some((target_pos, seek_ts, dl)) = self.optimistic_seek {
            if Instant::now() < dl {
                let player_caught_up = (tl.position_100ns - target_pos).abs() <= 20_000_000
                    && tl.last_updated_100ns >= seek_ts;
                if !player_caught_up {
                    tl.position_100ns = target_pos.clamp(tl.start_100ns, tl.end_100ns);
                    tl.last_updated_100ns = seek_ts;
                }
            }
        }
        Some(tl)
    }

    fn can_seek(&self) -> bool {
        self.effective_timeline().is_some_and(|tl| tl.can_seek)
    }

    fn target_accent_rgb(&self) -> Rgb {
        let bg_fallback = enforce_ui_accent_deadzone(
            derive_background_accent(self.backdrop_rgb, self.light_glass),
            self.light_glass,
        );
        if self.auto_color {
            self.source
                .as_ref()
                .and_then(|s| s.cover.as_ref())
                .map(|c| {
                    let raw = if self.light_glass {
                        c.accent_light
                    } else {
                        c.accent_dark
                    };
                    enforce_ui_accent_deadzone(raw, self.light_glass)
                })
                .unwrap_or(bg_fallback)
        } else {
            bg_fallback
        }
    }

    fn current_ui_accent_rgb(&self) -> Rgb {
        Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    }

    fn reposition_and_recapture(&mut self) {
        let layout = self.current_layout();
        let mon = unsafe { MonitorFromRect(&self.anchor_rect, MONITOR_DEFAULTTONEAREST) };
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

        let (win_x, win_y) = compute_mini_player_window_origin(
            self.anchor_rect.left,
            self.anchor_rect.top,
            self.anchor_rect.right,
            layout.win_w,
            layout.win_h,
            self.dpi,
            work.left,
            work.top,
            work.right,
            work.bottom,
        );
        self.win_x = win_x;
        self.win_y = win_y;
        self.win_w = layout.win_w;
        self.win_h = layout.win_h;

        let capture_rect = RECT {
            left: win_x,
            top: win_y,
            right: win_x + layout.win_w,
            bottom: win_y + layout.win_h,
        };
        let occluders = visible_transient_overlay_rects(capture_rect);
        let initial_recipe =
            popup_glass_recipe(active_taskbar_surface_style(), self.light_glass, 0.5);
        let (bmp, sampled_bg) = capture_blurred_desktop_backdrop_excluding(
            &self.rt,
            win_x,
            win_y,
            layout.win_w,
            layout.win_h,
            initial_recipe.blur_radius_4x,
            &occluders,
        );
        self.backdrop_bmp = bmp;
        if let Some(bg) = sampled_bg {
            self.backdrop_rgb = bg;
        }
        if unsafe { IsWindowVisible(self.hwnd).as_bool() } {
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    self.win_x,
                    self.win_y,
                    self.win_w,
                    self.win_h,
                    SWP_NOACTIVATE,
                );
            }
        }
    }

    fn reset_transient_state(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_MINI_TICK_ID);
        }
        self.timer_interval_ms = 0;
        self.hover_zone = MiniPlayerHitZone::None;
        self.pressed_zone = MiniPlayerHitZone::None;
        self.tracking_leave = false;
        self.dragging_seek_ratio = None;
        self.vol_badge_until = None;
        self.anim_phase = PopupAnimPhase::Open;
        self.open_spring = Spring1D::still(1.0);
        self.open_elapsed = 1.0;
        self.close_progress = 0.0;
        self.backdrop_bmp = None;
    }

    fn begin_close(&mut self) {
        if !unsafe { IsWindowVisible(self.hwnd).as_bool() } {
            return;
        }
        if self.anim_phase == PopupAnimPhase::Closing {
            return;
        }
        if self.dragging_seek_ratio.is_some() {
            self.dragging_seek_ratio = None;
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        self.anim_phase = PopupAnimPhase::Closing;
        self.close_progress = 0.0;
        self.close_started_at = Some(Instant::now());
        self.last_flow_tick = Instant::now();
        self.sync_timer_rate();
    }

    fn finish_close(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            let _ = PostMessageW(
                self.main_hwnd,
                WM_APP_MINI_PLAYER_CLOSED,
                WPARAM(0),
                LPARAM(0),
            );
        }
        self.reset_transient_state();
    }

    fn desired_timer_interval_ms(&self) -> u32 {
        let is_animating = matches!(
            self.anim_phase,
            PopupAnimPhase::Opening | PopupAnimPhase::Closing
        );
        let is_playing = self.is_playing();
        let marquee_active = self.title_marquee.is_active() || self.artist_marquee.is_active();
        let bars_active = self.bar_levels.iter().any(|&v| v > 0.01);
        if is_animating || is_playing || marquee_active || bars_active {
            POPUP_ANIM_INTERVAL_MS
        } else {
            POPUP_FLOW_INTERVAL_MS
        }
    }

    fn sync_timer_rate(&mut self) {
        let want = self.desired_timer_interval_ms();
        if self.timer_interval_ms != want {
            unsafe {
                SetTimer(self.hwnd, TIMER_MINI_TICK_ID, want, None);
            }
            self.timer_interval_ms = want;
        }
    }

    fn update_halo_colors(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_color_tick).as_secs_f32().min(0.1);
        self.last_color_tick = now;

        let target_rgb = self.target_accent_rgb();
        let pair = compute_halo_pair(target_rgb, self.light_glass);
        let target_pair = [pair[0].to_normalized(), pair[1].to_normalized()];
        let target_ui = target_rgb.to_normalized();

        if !self.halo_colors_ready {
            self.halo_colors = target_pair;
            self.ui_accent = target_ui;
            self.halo_colors_ready = true;
            return;
        }

        let k = (dt * 6.5).min(1.0);
        for i in 0..2 {
            self.halo_colors[i].0 += (target_pair[i].0 - self.halo_colors[i].0) * k;
            self.halo_colors[i].1 += (target_pair[i].1 - self.halo_colors[i].1) * k;
            self.halo_colors[i].2 += (target_pair[i].2 - self.halo_colors[i].2) * k;
        }
        self.ui_accent.0 += (target_ui.0 - self.ui_accent.0) * k;
        self.ui_accent.1 += (target_ui.1 - self.ui_accent.1) * k;
        self.ui_accent.2 += (target_ui.2 - self.ui_accent.2) * k;
    }

    fn ensure_formats(&mut self) -> windows::core::Result<CachedFormats> {
        if let Some(ref f) = self.cached_formats {
            if f.dpi == self.dpi {
                return Ok(f.clone());
            }
        }
        let s = (self.dpi.max(96) as f32) / 96.0;
        let badge_fmt = create_text_format(&self.dwrite_factory, 10.5 * s, true, false, true)?;
        let title_fmt = create_text_format(&self.dwrite_factory, 14.0 * s, true, false, false)?;
        let artist_fmt = create_text_format(&self.dwrite_factory, 11.8 * s, false, false, false)?;
        let time_l_fmt = create_text_format(&self.dwrite_factory, 10.2 * s, false, false, false)?;
        let time_r_fmt = create_text_format(&self.dwrite_factory, 10.2 * s, false, true, false)?;
        let cached = CachedFormats {
            dpi: self.dpi,
            badge_fmt,
            title_fmt,
            artist_fmt,
            time_l_fmt,
            time_r_fmt,
        };
        self.cached_formats = Some(cached.clone());
        self.cached_title = None;
        self.cached_artist = None;
        Ok(cached)
    }

    fn measure_marquee_text(
        dwrite: &IDWriteFactory,
        fmt: &IDWriteTextFormat,
        cache: &mut Option<CachedText>,
        text: &str,
        dpi: u32,
        height_px: f32,
    ) -> Option<(IDWriteTextLayout, f32)> {
        if let Some(ref c) = cache {
            if c.dpi == dpi && c.text == text {
                return Some((c.layout.clone(), c.width));
            }
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let layout = dwrite
                .CreateTextLayout(&wide, fmt, 4096.0, height_px.max(12.0))
                .ok()?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics).ok()?;
            let width = metrics.widthIncludingTrailingWhitespace;
            *cache = Some(CachedText {
                text: text.to_string(),
                dpi,
                layout: layout.clone(),
                width,
            });
            Some((layout, width))
        }
    }

    fn ensure_art_bitmap(&mut self, cover: &MediaCover) -> Option<ID2D1Bitmap> {
        if let Some(ref c) = self.cached_art {
            if c.hash == cover.hash {
                return Some(c.bitmap.clone());
            }
        }
        unsafe {
            let props = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            let bmp = self
                .rt
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: cover.art_w,
                        height: cover.art_h,
                    },
                    Some(cover.art_pbgra.as_ptr().cast()),
                    cover.art_w * 4,
                    &props,
                )
                .ok()?;
            self.cached_art = Some(CachedArtBitmap {
                hash: cover.hash,
                bitmap: bmp.clone(),
            });
            Some(bmp)
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_flow_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_flow_tick = now;
        self.flow_phase += f64::from(dt);

        if let Some((_, _, dl)) = self.optimistic_seek {
            if now >= dl {
                self.optimistic_seek = None;
            }
        }
        if let Some(until) = self.vol_badge_until {
            if now >= until {
                self.vol_badge_until = None;
            }
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
            }
            PopupAnimPhase::Closing => {
                self.close_progress = (self.close_progress + dt / CLOSE_ANIM_DUR_S).min(1.0);
                if self.close_progress >= 1.0 {
                    self.finish_close();
                    return;
                }
            }
            PopupAnimPhase::Open => {}
        }

        let _ = self.title_marquee.tick(dt);
        let _ = self.artist_marquee.tick(dt);

        self.sync_timer_rate();
        let _ = self.render();
    }

    fn animated_card_rect(&self, target: RectF, s: f32) -> (RectF, f32) {
        match self.anim_phase {
            PopupAnimPhase::Open => (target, 1.0),
            PopupAnimPhase::Opening => {
                let p = self.open_spring.pos.clamp(0.0, 1.06);
                let dy = (1.0 - p) * 14.0 * s;
                let h = (target.height() * (0.86 + 0.14 * p.min(1.0))).max(16.0);
                let bottom = target.bottom + dy;
                let rect = RectF::new(target.left, bottom - h, target.right, bottom);
                let content_a = ease_out_cubic((self.open_elapsed / 0.16).clamp(0.0, 1.0));
                (rect, content_a)
            }
            PopupAnimPhase::Closing => {
                let r = ease_retract(self.close_progress);
                let dy = r * 12.0 * s;
                let h = (target.height() * (1.0 - 0.14 * r)).max(16.0);
                let bottom = target.bottom + dy;
                let rect = RectF::new(target.left, bottom - h, target.right, bottom);
                let content_a = (1.0 - self.close_progress * 1.45).clamp(0.0, 1.0);
                (rect, content_a)
            }
        }
    }

    fn render(&mut self) -> windows::core::Result<()> {
        let layout = self.current_layout();
        self.surface.ensure_size(layout.win_w, layout.win_h)?;
        let formats = self.ensure_formats()?;
        self.update_halo_colors();

        let s = layout.scale;
        let light = self.light_glass;
        let accent = self.current_ui_accent_rgb();
        let is_playing = self.is_playing();
        let (card_rect, content_alpha) = self.animated_card_rect(layout.card_rect, s);
        let master_alpha = window_master_alpha(self.anim_phase, self.open_elapsed, self.close_progress);

        let bg_luma = rgb_luma(self.backdrop_rgb);
        let recipe = popup_glass_recipe(active_taskbar_surface_style(), light, bg_luma);

        if recipe.silk_alpha > 0.002 {
            update_silk_bitmap(
                &self.rt,
                &mut self.silk_bmp,
                &mut self.silk_buf,
                &mut self.last_silk_tick,
                false,
                layout.win_w,
                layout.win_h,
                card_rect.left,
                card_rect.bottom,
                s,
                self.flow_phase,
                &self.halo_colors,
                light,
            );
        }

        let cover_arc: Option<Arc<MediaCover>> =
            self.source.as_ref().and_then(|src| src.cover.clone());
        let art_bmp = cover_arc.as_ref().and_then(|c| self.ensure_art_bitmap(c));

        let title_str = self
            .source
            .as_ref()
            .map(|src| src.title.as_str())
            .filter(|t| !t.is_empty())
            .unwrap_or("未在播放");
        let artist_str = self
            .source
            .as_ref()
            .map(|src| src.artist.as_str())
            .unwrap_or("");
        let badge_str = self
            .source
            .as_ref()
            .map(|src| src.app_name.as_str())
            .filter(|a| !a.is_empty())
            .unwrap_or("媒体控制");

        let title_layout = Self::measure_marquee_text(
            &self.dwrite_factory,
            &formats.title_fmt,
            &mut self.cached_title,
            title_str,
            self.dpi,
            layout.title_rect.height(),
        );
        if let Some((_, w)) = &title_layout {
            self.title_marquee.set_text(title_str, *w);
            self.title_marquee
                .set_widths(*w, layout.title_rect.width());
        } else {
            self.title_marquee.set_text("", 0.0);
        }

        let artist_layout = if !artist_str.is_empty() {
            Self::measure_marquee_text(
                &self.dwrite_factory,
                &formats.artist_fmt,
                &mut self.cached_artist,
                artist_str,
                self.dpi,
                layout.artist_rect.height(),
            )
        } else {
            None
        };
        if let Some((_, w)) = &artist_layout {
            self.artist_marquee.set_text(artist_str, *w);
            self.artist_marquee
                .set_widths(*w, layout.artist_rect.width());
        } else {
            self.artist_marquee.set_text("", 0.0);
        }

        let timeline = self.effective_timeline();
        let now_ft = current_filetime_100ns();
        let (slider_ratio, elapsed_secs, total_secs, can_seek) = if let Some(tl) = timeline {
            if let Some(drag_r) = self.dragging_seek_ratio {
                let (el, tot) = tl.preview_secs_for_ratio(drag_r).unwrap_or((0.0, 0.0));
                (drag_r, el, tot, tl.can_seek)
            } else {
                let r = tl.progress_ratio(now_ft, is_playing).unwrap_or(0.0);
                let (el, tot) = tl
                    .elapsed_and_total_secs(now_ft, is_playing)
                    .unwrap_or((0.0, 0.0));
                (r, el, tot, tl.can_seek)
            }
        } else {
            (0.0, 0.0, 0.0, false)
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

            // 1. 3-pass ambient shadows
            draw_rounded_card_shadows(
                &self.rt,
                card_rect,
                layout.card_radius,
                layout.win_w as f32,
                layout.win_h as f32,
                s,
                light,
            );

            // 2. Clipped glass interior + subtle album artwork radial atmosphere glow
            let card_rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(card_rect),
                radiusX: layout.card_radius,
                radiusY: layout.card_radius,
            };
            if let Ok(geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&card_rr) {
                let geom: ID2D1Geometry = geo.cast().unwrap();
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(card_rect),
                    geometricMask: ManuallyDrop::new(Some(geom)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, &self.card_layer);

                draw_glass_interior(
                    &self.rt,
                    card_rect,
                    layout.win_w,
                    layout.win_h,
                    self.backdrop_bmp.as_ref(),
                    self.silk_bmp.as_ref(),
                    light,
                    &recipe,
                    &mut self.noise_cache,
                );

                // Soft album-tinted radial glow emanating from behind the artwork
                if self.source.is_some() {
                    let glow_cx = (layout.cover_rect.left + layout.cover_rect.right) * 0.5;
                    let glow_cy = (layout.cover_rect.top + layout.cover_rect.bottom) * 0.5
                        + (card_rect.top - layout.card_rect.top);
                    let glow_stops = [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: d2d_color_rgb(accent, if light { 0.14 } else { 0.22 }),
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.55,
                            color: d2d_color_rgb(accent, if light { 0.05 } else { 0.08 }),
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: d2d_color_rgb(accent, 0.0),
                        },
                    ];
                    if let Ok(sc) = self.rt.CreateGradientStopCollection(
                        &glow_stops,
                        D2D1_GAMMA_2_2,
                        D2D1_EXTEND_MODE_CLAMP,
                    ) {
                        let rad_props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                            center: D2D_POINT_2F {
                                x: glow_cx,
                                y: glow_cy,
                            },
                            gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                            radiusX: card_rect.width() * 0.68,
                            radiusY: card_rect.height() * 0.85,
                        };
                        if let Ok(glow_brush) =
                            self.rt.CreateRadialGradientBrush(&rad_props, None, &sc)
                        {
                            self.rt.FillRectangle(&d2d_rect(card_rect), &glow_brush);
                        }
                    }
                }

                // 3. Foreground content (fades smoothly with spring animation)
                if content_alpha > 0.01 {
                    let dy = card_rect.top - layout.card_rect.top;
                    self.draw_content(
                        &layout,
                        &formats,
                        dy,
                        content_alpha,
                        accent,
                        is_playing,
                        art_bmp.as_ref(),
                        badge_str,
                        title_layout.as_ref().map(|(tl, _)| tl),
                        artist_layout.as_ref().map(|(al, _)| al),
                        slider_ratio,
                        elapsed_secs,
                        total_secs,
                        can_seek,
                    );
                }

                self.rt.PopLayer();
            }

            // 4. Glass rim highlight
            draw_rounded_card_rim(&self.rt, card_rect, layout.card_radius, s, &recipe);

            self.rt.EndDraw(None, None)?;
        }

        let alpha_u8 = (master_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        self.surface
            .commit_with_alpha(self.hwnd, self.win_x, self.win_y, alpha_u8);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_content(
        &self,
        layout: &MiniPlayerLayout,
        formats: &CachedFormats,
        dy: f32,
        alpha: f32,
        accent: Rgb,
        is_playing: bool,
        art_bmp: Option<&ID2D1Bitmap>,
        badge_str: &str,
        title_layout: Option<&IDWriteTextLayout>,
        artist_layout: Option<&IDWriteTextLayout>,
        slider_ratio: f32,
        elapsed_secs: f64,
        total_secs: f64,
        can_seek: bool,
    ) {
        let s = layout.scale;
        let light = self.light_glass;
        let shift = |r: RectF| RectF::new(r.left, r.top + dy, r.right, r.bottom + dy);

        // Hover backdrop behind artwork + title when user hovers to jump to player
        if self.hover_zone == MiniPlayerHitZone::CoverOrInfo && self.source.is_some() {
            let hr = shift(layout.info_hit_rect);
            let pad = 4.0 * s;
            let pill = RectF::new(hr.left - pad, hr.top - pad, hr.right + pad, hr.bottom + pad);
            let bg = if light {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.045 * alpha,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.055 * alpha,
                }
            };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&bg, None) {
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(pill),
                    radiusX: 11.0 * s,
                    radiusY: 11.0 * s,
                };
                self.rt.FillRoundedRectangle(&rr, &b);
            }
        }

        // 1. Large crisp album artwork (or default music note card)
        let cover_rect = shift(layout.cover_rect);
        let cover_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(cover_rect),
            radiusX: layout.cover_radius,
            radiusY: layout.cover_radius,
        };
        // Subtle elevation shadow under cover tile
        let art_shadow = RectF::new(
            cover_rect.left,
            cover_rect.top + 2.5 * s,
            cover_rect.right,
            cover_rect.bottom + 3.5 * s,
        );
        if let Ok(sb) = self.rt.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: (if light { 0.14 } else { 0.30 }) * alpha,
            },
            None,
        ) {
            let sh_rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(art_shadow),
                radiusX: layout.cover_radius,
                radiusY: layout.cover_radius,
            };
            self.rt.FillRoundedRectangle(&sh_rr, &sb);
        }

        if let Some(bmp) = art_bmp {
            if let Ok(geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&cover_rr) {
                let geom: ID2D1Geometry = geo.cast().unwrap();
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(cover_rect),
                    geometricMask: ManuallyDrop::new(Some(geom)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: alpha,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, &self.art_layer);
                self.rt.DrawBitmap(
                    bmp,
                    Some(&d2d_rect(cover_rect)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
                self.rt.PopLayer();
            }
        } else {
            let tile_bg = if light {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.07 * alpha,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.10 * alpha,
                }
            };
            if let Ok(tb) = self.rt.CreateSolidColorBrush(&tile_bg, None) {
                self.rt.FillRoundedRectangle(&cover_rr, &tb);
            }
            let note_col = d2d_color_rgb(accent, 0.85 * alpha);
            if let Ok(nb) = self.rt.CreateSolidColorBrush(&note_col, None) {
                let cx = (cover_rect.left + cover_rect.right) * 0.5;
                let cy = (cover_rect.top + cover_rect.bottom) * 0.5;
                draw_music_note_icon(&self.rt, cx, cy, 2.2 * s, &nb);
            }
        }

        let cover_rim = if light {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.12 * alpha,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.20 * alpha,
            }
        };
        if let Ok(rb) = self.rt.CreateSolidColorBrush(&cover_rim, None) {
            self.rt.DrawRoundedRectangle(&cover_rr, &rb, 1.0 * s, None);
        }

        // 2. Source badge (status dot + app name)
        let badge_rect = shift(layout.source_badge_rect);
        let dot_r = 2.8 * s;
        let dot_cx = badge_rect.left + dot_r + 0.5 * s;
        let dot_cy = (badge_rect.top + badge_rect.bottom) * 0.5;
        let dot_col = if is_playing {
            d2d_color_rgb(accent, 0.95 * alpha)
        } else {
            let mut m = muted_text_color(light);
            m.a *= 0.72 * alpha;
            m
        };
        if let Ok(db) = self.rt.CreateSolidColorBrush(&dot_col, None) {
            self.rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: dot_cx,
                        y: dot_cy,
                    },
                    radiusX: dot_r,
                    radiusY: dot_r,
                },
                &db,
            );
        }
        let mut badge_text_col = muted_text_color(light);
        badge_text_col.a *= 0.92 * alpha;
        let badge_text_rect = RectF::new(
            dot_cx + dot_r + 5.0 * s,
            badge_rect.top,
            badge_rect.right,
            badge_rect.bottom,
        );
        draw_text_in_rect(
            &self.dwrite_factory,
            &self.rt,
            badge_str,
            badge_text_rect,
            &formats.badge_fmt,
            badge_text_col,
        );

        // 3. Compact live mini-spectrum in header
        let spec_alpha = (if self.source.is_some() { 0.90 } else { 0.30 }) * alpha;
        let spec_col = d2d_color_rgb(accent, spec_alpha);
        if let Ok(sb) = self.rt.CreateSolidColorBrush(&spec_col, None) {
            for r in &layout.mini_spectrum_rects {
                let sr = shift(*r);
                let rad = sr.width() * 0.5;
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(sr),
                    radiusX: rad,
                    radiusY: rad,
                };
                self.rt.FillRoundedRectangle(&rr, &sb);
            }
        }

        // 4. Title & Artist (marquee-aware)
        let mut title_col = primary_text_color(light);
        title_col.a *= if self.source.is_some() {
            alpha
        } else {
            0.60 * alpha
        };
        if let Some(tl) = title_layout {
            let t_rect = if artist_layout.is_none() {
                let mid = (layout.title_rect.top + layout.artist_rect.bottom) * 0.5;
                let h = layout.title_rect.height();
                shift(RectF::new(
                    layout.title_rect.left,
                    mid - h * 0.5,
                    layout.title_rect.right,
                    mid + h * 0.5,
                ))
            } else {
                shift(layout.title_rect)
            };
            self.draw_marquee_text(tl, t_rect, title_col, &self.title_marquee, s);
        }
        if let Some(al) = artist_layout {
            let mut artist_col = muted_text_color(light);
            artist_col.a *= alpha;
            self.draw_marquee_text(
                al,
                shift(layout.artist_rect),
                artist_col,
                &self.artist_marquee,
                s,
            );
        }

        // 6. Interactive timeline slider + elapsed / total timestamps
        if layout.has_timeline {
            let track = shift(layout.progress_track_rect);
            let active = self.dragging_seek_ratio.is_some()
                || (can_seek
                    && (self.hover_zone == MiniPlayerHitZone::ProgressSlider
                        || self.pressed_zone == MiniPlayerHitZone::ProgressSlider));
            draw_glass_slider(
                &self.rt,
                track,
                slider_ratio,
                !is_playing && self.dragging_seek_ratio.is_none(),
                active,
                light,
                accent,
                s,
            );

            let mut time_col = muted_text_color(light);
            time_col.a *= if self.dragging_seek_ratio.is_some() {
                0.96 * alpha
            } else {
                0.82 * alpha
            };
            let el_str = format_media_time(elapsed_secs);
            let tot_str = format_media_time(total_secs);
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                &el_str,
                shift(layout.elapsed_rect),
                &formats.time_l_fmt,
                time_col,
            );
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                &tot_str,
                shift(layout.duration_rect),
                &formats.time_r_fmt,
                time_col,
            );
        }

        // 6. Bottom control bar: Menu | Prev | Play/Pause (accent circle) | Next | Mute
        let more_rect = shift(layout.more_btn_rect);
        let more_hovered = self.hover_zone == MiniPlayerHitZone::MoreMenuButton;
        let more_pressed = self.pressed_zone == MiniPlayerHitZone::MoreMenuButton;
        self.draw_icon_button_bg(more_rect, 7.0 * s, more_hovered, more_pressed, alpha);
        let mut util_col = if more_hovered {
            primary_text_color(light)
        } else {
            muted_text_color(light)
        };
        util_col.a *= (if more_pressed { 0.72 } else { 1.0 }) * alpha;
        if let Ok(ub) = self.rt.CreateSolidColorBrush(&util_col, None) {
            self.draw_menu_icon(more_rect, s * if more_pressed { 0.92 } else { 1.0 }, &ub);
        }

        // Prev / Next buttons
        for (zone, base_rect) in [
            (MiniPlayerHitZone::PrevButton, layout.prev_btn_rect),
            (MiniPlayerHitZone::NextButton, layout.next_btn_rect),
        ] {
            let rect = shift(base_rect);
            let hovered = self.hover_zone == zone;
            let pressed = self.pressed_zone == zone;
            self.draw_icon_button_bg(rect, 8.0 * s, hovered, pressed, alpha);
            let mut fg = primary_text_color(light);
            fg.a *= (if pressed { 0.72 } else { 0.94 }) * alpha;
            if let Ok(b) = self.rt.CreateSolidColorBrush(&fg, None) {
                let cx = (rect.left + rect.right) * 0.5;
                let cy = (rect.top + rect.bottom) * 0.5;
                let u = (rect.width() / 24.0) * if pressed { 0.90 } else { 1.0 };
                match zone {
                    MiniPlayerHitZone::PrevButton => {
                        draw_prev_icon(&self.d2d_factory, &self.rt, cx, cy, u, &b)
                    }
                    MiniPlayerHitZone::NextButton => {
                        draw_next_icon(&self.d2d_factory, &self.rt, cx, cy, u, &b)
                    }
                    _ => {}
                }
            }
        }

        // Primary circular Play/Pause button tinted with theme-calibrated accent
        let play_rect = shift(layout.play_btn_rect);
        let play_hovered = self.hover_zone == MiniPlayerHitZone::PlayPauseButton;
        let play_pressed = self.pressed_zone == MiniPlayerHitZone::PlayPauseButton;
        let play_cx = (play_rect.left + play_rect.right) * 0.5;
        let play_cy = (play_rect.top + play_rect.bottom) * 0.5;
        let play_r = (play_rect.width() * 0.5) * if play_pressed { 0.94 } else { 1.0 };
        let accent_c = d2d_color_rgb(accent, (if play_hovered { 0.98 } else { 0.90 }) * alpha);
        let fill_col = if play_hovered {
            D2D1_COLOR_F {
                r: (accent_c.r * 1.08 + 0.04).min(1.0),
                g: (accent_c.g * 1.08 + 0.04).min(1.0),
                b: (accent_c.b * 1.08 + 0.04).min(1.0),
                a: accent_c.a,
            }
        } else {
            accent_c
        };
        if let Ok(pb) = self.rt.CreateSolidColorBrush(&fill_col, None) {
            self.rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: play_cx,
                        y: play_cy,
                    },
                    radiusX: play_r,
                    radiusY: play_r,
                },
                &pb,
            );
        }
        // Subtle specular top rim on the circular accent button
        let btn_rim = D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: (if light { 0.34 } else { 0.28 }) * alpha,
        };
        if let Ok(rb) = self.rt.CreateSolidColorBrush(&btn_rim, None) {
            self.rt.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: play_cx,
                        y: play_cy,
                    },
                    radiusX: play_r - 0.5 * s,
                    radiusY: play_r - 0.5 * s,
                },
                &rb,
                1.0 * s,
                None,
            );
        }
        // Choose crisp dark or white glyph depending on accent luminance for high contrast
        let accent_luma = rgb_luma(accent);
        let glyph_col = if accent_luma > 0.62 {
            D2D1_COLOR_F {
                r: 0.06,
                g: 0.07,
                b: 0.10,
                a: 0.96 * alpha,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.98 * alpha,
            }
        };
        if let Ok(gb) = self.rt.CreateSolidColorBrush(&glyph_col, None) {
            let u = (play_rect.width() / 28.0) * if play_pressed { 0.92 } else { 1.0 };
            if is_playing {
                draw_pause_icon(&self.rt, play_cx, play_cy, u, &gb);
            } else {
                draw_play_icon(&self.d2d_factory, &self.rt, play_cx + 0.8 * s, play_cy, u, &gb);
            }
        }

        // Mute button + ephemeral volume % feedback when scrolling wheel over the popup
        let mute_rect = shift(layout.mute_btn_rect);
        let mute_hovered = self.hover_zone == MiniPlayerHitZone::MuteButton;
        let mute_pressed = self.pressed_zone == MiniPlayerHitZone::MuteButton;
        self.draw_icon_button_bg(mute_rect, 7.0 * s, mute_hovered, mute_pressed, alpha);
        let mut spk_fg = if mute_hovered {
            primary_text_color(light)
        } else {
            muted_text_color(light)
        };
        spk_fg.a *= alpha;
        let spk_muted = D2D1_COLOR_F {
            a: spk_fg.a * 0.58,
            ..spk_fg
        };
        let spk_slash = if light {
            D2D1_COLOR_F {
                r: 0.80,
                g: 0.18,
                b: 0.20,
                a: 0.92 * alpha,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 0.46,
                b: 0.46,
                a: 0.95 * alpha,
            }
        };
        if let (Ok(fg_b), Ok(mu_b), Ok(sl_b)) = (
            self.rt.CreateSolidColorBrush(&spk_fg, None),
            self.rt.CreateSolidColorBrush(&spk_muted, None),
            self.rt.CreateSolidColorBrush(&spk_slash, None),
        ) {
            let cx = (mute_rect.left + mute_rect.right) * 0.5;
            let cy = (mute_rect.top + mute_rect.bottom) * 0.5;
            let u = (mute_rect.width() / 24.0) * if mute_pressed { 0.90 } else { 1.0 };
            draw_speaker_glyph(
                &self.d2d_factory,
                &self.rt,
                cx,
                cy,
                u,
                self.master_vol,
                self.master_muted,
                &fg_b,
                &mu_b,
                &sl_b,
                self.round_stroke.as_ref(),
            );
        }

        if self.vol_badge_until.is_some_and(|t| Instant::now() < t) {
            let pct_str = format_volume_pct(self.master_vol);
            let badge_w = 34.0 * s;
            let badge_h = 16.0 * s;
            let badge_cy = (mute_rect.top + mute_rect.bottom) * 0.5;
            let pct_rect = RectF::new(
                mute_rect.left - badge_w - 2.0 * s,
                badge_cy - badge_h * 0.5,
                mute_rect.left - 2.0 * s,
                badge_cy + badge_h * 0.5,
            );
            let mut pct_col = primary_text_color(light);
            pct_col.a *= 0.88 * alpha;
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                &pct_str,
                pct_rect,
                &formats.time_r_fmt,
                pct_col,
            );
        }
    }

    unsafe fn draw_icon_button_bg(
        &self,
        rect: RectF,
        radius: f32,
        hovered: bool,
        pressed: bool,
        alpha: f32,
    ) {
        if !hovered && !pressed {
            return;
        }
        let col = if self.light_glass {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: (if pressed { 0.13 } else { 0.07 }) * alpha,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: (if pressed { 0.17 } else { 0.10 }) * alpha,
            }
        };
        if let Ok(b) = self.rt.CreateSolidColorBrush(&col, None) {
            let rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(rect),
                radiusX: radius,
                radiusY: radius,
            };
            self.rt.FillRoundedRectangle(&rr, &b);
        }
    }

    unsafe fn draw_menu_icon(&self, rect: RectF, s: f32, brush: &ID2D1SolidColorBrush) {
        let cx = (rect.left + rect.right) * 0.5;
        let cy = (rect.top + rect.bottom) * 0.5;
        let half_w = 5.6 * s;
        let gap_y = 3.8 * s;
        let stroke = 1.45 * s;
        let rs = self.round_stroke.as_ref();

        for dy in [-gap_y, 0.0, gap_y] {
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
                stroke,
                rs,
            );
        }
    }

    unsafe fn draw_marquee_text(
        &self,
        text_layout: &IDWriteTextLayout,
        rect: RectF,
        color: D2D1_COLOR_F,
        marquee: &MarqueeState,
        scale: f32,
    ) {
        let Ok(text_brush) = self.rt.CreateSolidColorBrush(&color, None) else {
            return;
        };

        if !marquee.is_overflowing() {
            self.rt
                .PushAxisAlignedClip(&d2d_rect(rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: rect.left,
                    y: rect.top,
                },
                text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
            self.rt.PopAxisAlignedClip();
            return;
        }

        let (x1, x2_opt) = marquee.render_offsets();
        let w = rect.width().max(1.0);
        let fade_frac = (12.0 * scale / w).clamp(0.05, 0.22);
        let left_alpha = if x1 < -0.5 { 0.0 } else { 1.0 };

        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: left_alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: fade_frac,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0 - fade_frac,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
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

        let mut pushed_layer = false;
        let opacity_brush_keepalive;
        if let Ok(sc) =
            self.rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
        {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: rect.left,
                    y: rect.top,
                },
                endPoint: D2D_POINT_2F {
                    x: rect.right,
                    y: rect.top,
                },
            };
            if let Ok(grad) = self.rt.CreateLinearGradientBrush(&props, None, &sc) {
                let brush_iface: ID2D1Brush = grad.cast().unwrap();
                opacity_brush_keepalive = brush_iface.clone();
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(rect),
                    geometricMask: ManuallyDrop::new(None),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(Some(opacity_brush_keepalive)),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, &self.marquee_layer);
                pushed_layer = true;
            }
        }

        if !pushed_layer {
            self.rt
                .PushAxisAlignedClip(&d2d_rect(rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        self.rt.DrawTextLayout(
            D2D_POINT_2F {
                x: rect.left + x1,
                y: rect.top,
            },
            text_layout,
            &text_brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );
        if let Some(x2) = x2_opt {
            self.rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: rect.left + x2,
                    y: rect.top,
                },
                text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        if pushed_layer {
            self.rt.PopLayer();
        } else {
            self.rt.PopAxisAlignedClip();
        }
    }

    fn post_action(&self, action: usize) {
        unsafe {
            let _ = PostMessageW(
                self.main_hwnd,
                WM_APP_MINI_PLAYER_ACTION,
                WPARAM(action),
                LPARAM(0),
            );
        }
    }

    fn commit_seek_ratio(&mut self, ratio: f32) {
        if let Some(tl) = self.effective_timeline() {
            if let Some(target_100ns) = tl.seek_target_100ns(ratio) {
                self.pending_seek_100ns = Some(target_100ns);
                self.optimistic_seek = Some((
                    target_100ns,
                    current_filetime_100ns(),
                    Instant::now() + OPTIMISTIC_SEEK_TTL,
                ));
                unsafe {
                    let _ = PostMessageW(
                        self.main_hwnd,
                        WM_APP_MINI_PLAYER_SEEK,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        }
    }
}

unsafe extern "system" fn mini_player_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MiniPlayerInner;
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
            if wparam.0 == VK_ESCAPE.0 as usize {
                state.begin_close();
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCHITTEST => {
            let sx = ((lparam.0 & 0xFFFF) as i16) as i32;
            let sy = (((lparam.0 >> 16) & 0xFFFF) as i16) as i32;
            let mut pt = POINT { x: sx, y: sy };
            let _ = ScreenToClient(hwnd, &mut pt);
            let layout = state.current_layout();
            if layout.card_rect.contains(pt.x as f32, pt.y as f32) {
                LRESULT(HTCLIENT as isize)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                SetCursor(state.arrow_cursor);
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_MINI_TICK_ID {
                state.tick();
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let wheel_delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
            if wheel_delta != 0 {
                let _ = PostMessageW(
                    state.main_hwnd,
                    WM_APP_MINI_PLAYER_WHEEL,
                    WPARAM(wheel_delta as isize as usize),
                    LPARAM(0),
                );
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if state.dragging_seek_ratio.is_some() || state.pressed_zone != MiniPlayerHitZone::None {
                state.dragging_seek_ratio = None;
                state.pressed_zone = MiniPlayerHitZone::None;
                let _ = state.render();
            }
            LRESULT(0)
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
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let layout = state.current_layout();

            if state.dragging_seek_ratio.is_some() {
                let r = slider_ratio_from_x(
                    layout.progress_track_rect.left,
                    layout.progress_track_rect.right,
                    x,
                );
                state.dragging_seek_ratio = Some(r);
                let _ = state.render();
                return LRESULT(0);
            }

            let next_zone = layout.hit_test(x, y, state.can_seek());
            if state.hover_zone != next_zone {
                state.hover_zone = next_zone;
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            state.tracking_leave = false;
            if state.dragging_seek_ratio.is_none() && state.hover_zone != MiniPlayerHitZone::None {
                state.hover_zone = MiniPlayerHitZone::None;
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let layout = state.current_layout();
            let zone = layout.hit_test(x, y, state.can_seek());
            state.pressed_zone = zone;
            if zone == MiniPlayerHitZone::ProgressSlider {
                let r = slider_ratio_from_x(
                    layout.progress_track_rect.left,
                    layout.progress_track_rect.right,
                    x,
                );
                state.dragging_seek_ratio = Some(r);
                let _ = SetCapture(hwnd);
            } else if zone != MiniPlayerHitZone::None {
                let _ = SetCapture(hwnd);
            }
            let _ = state.render();
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let layout = state.current_layout();

            if state.dragging_seek_ratio.is_some() {
                let final_r = slider_ratio_from_x(
                    layout.progress_track_rect.left,
                    layout.progress_track_rect.right,
                    x,
                );
                state.dragging_seek_ratio = None;
                state.pressed_zone = MiniPlayerHitZone::None;
                state.hover_zone = layout.hit_test(x, y, state.can_seek());
                let _ = ReleaseCapture();
                state.commit_seek_ratio(final_r);
                let _ = state.render();
                return LRESULT(0);
            }

            let pressed = state.pressed_zone;
            state.pressed_zone = MiniPlayerHitZone::None;
            let _ = ReleaseCapture();
            let zone = layout.hit_test(x, y, state.can_seek());
            state.hover_zone = zone;

            if zone == pressed {
                match zone {
                    MiniPlayerHitZone::PrevButton => state.post_action(MINI_ACTION_PREV),
                    MiniPlayerHitZone::PlayPauseButton => state.post_action(MINI_ACTION_PLAY_PAUSE),
                    MiniPlayerHitZone::NextButton => state.post_action(MINI_ACTION_NEXT),
                    MiniPlayerHitZone::MuteButton => state.post_action(MINI_ACTION_TOGGLE_MUTE),
                    MiniPlayerHitZone::CoverOrInfo => {
                        state.post_action(MINI_ACTION_ACTIVATE);
                        state.begin_close();
                    }
                    MiniPlayerHitZone::MoreMenuButton => {
                        state.post_action(MINI_ACTION_OPEN_MENU);
                    }
                    MiniPlayerHitZone::ProgressSlider | MiniPlayerHitZone::None => {}
                }
            }
            let _ = state.render();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
