mod render;
mod wndproc;

use wndproc::{begin_close_mini_player, mini_player_wnd_proc};

use super::arbitration::PlaybackState;
use super::dsp::NUM_BARS;
use super::layout::{compute_mini_player_window_origin, MiniPlayerHitZone, MiniPlayerLayout, RectF};
use super::marquee::MarqueeState;
use super::smtc::{MediaCover, SourceItem};
use super::timeline::{current_filetime_100ns, MediaTimeline};
use crate::color::{build_ambient_cover_bgra, derive_background_accent, enforce_ui_accent_deadzone, Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::system::{active_taskbar_surface_style, visible_transient_overlay_rects};
use crate::ui::glass::{capture_blurred_desktop_backdrop_excluding, create_dc_render_target, create_round_cap_stroke, create_text_format, LayeredSurface, PopupNoiseCache};
use crate::ui::{compute_halo_pair, ease_out_cubic, ease_retract, popup_glass_recipe, PopupAnimPhase, Spring1D, CLOSE_ANIM_DUR_S, OPEN_SPRING_DAMPING, OPEN_SPRING_STIFFNESS, POPUP_ANIM_INTERVAL_MS, POPUP_FLOW_INTERVAL_MS, POPUP_REOPEN_DEBOUNCE_MS, SILK_H, SILK_W};
use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Layer, ID2D1StrokeStyle, D2D1_BITMAP_PROPERTIES};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_TEXT_METRICS};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, IsWindowVisible, KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, HCURSOR, HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOW, WM_APP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP};

pub const WM_APP_MINI_PLAYER_CLOSED: u32 = WM_APP + 7;
pub const WM_APP_MINI_PLAYER_ACTION: u32 = WM_APP + 8;
pub const WM_APP_MINI_PLAYER_SEEK: u32 = WM_APP + 9;
pub const WM_APP_MINI_PLAYER_WHEEL: u32 = WM_APP + 10;
pub const WM_APP_MINI_PLAYER_VOLUME: u32 = WM_APP + 12;

pub const MINI_ACTION_PREV: usize = 1;
pub const MINI_ACTION_PLAY_PAUSE: usize = 2;
pub const MINI_ACTION_NEXT: usize = 3;
pub const MINI_ACTION_ACTIVATE: usize = 4;
pub const MINI_ACTION_OPEN_MENU: usize = 5;
pub const MINI_ACTION_TOGGLE_MUTE: usize = 6;

const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const MINI_PLAYER_CLASS_NAME: PCWSTR = w!("MediaBarMiniPlayerWnd");
const TIMER_MINI_TICK_ID: usize = 1;
pub(super) const VOL_BADGE_TTL: Duration = Duration::from_millis(1200);
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
    ambient_bitmap: Option<ID2D1Bitmap>,
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

struct MiniPlayerInner {
    hwnd: HWND,
    main_hwnd: HWND,
    d2d_factory: ID2D1Factory1,
    dwrite_factory: IDWriteFactory,
    // Declare RT-owned resources and `rt` before `surface` so `rt` releases before `DeleteDC`.
    backdrop_bmp: Option<ID2D1Bitmap>,
    noise_cache: PopupNoiseCache,
    silk_bmp: Option<ID2D1Bitmap>,
    cached_art: Option<CachedArtBitmap>,
    card_layer: ID2D1Layer,
    art_layer: ID2D1Layer,
    marquee_layer: ID2D1Layer,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
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
    dragging_volume: bool,
    pending_seek_100ns: Option<i64>,
    optimistic_seek: Option<(i64, i64, Instant)>,

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
    suppress_deactivate_close: bool,

    title_marquee: MarqueeState,
    artist_marquee: MarqueeState,
    cached_formats: Option<CachedFormats>,
    cached_title: Option<CachedText>,
    cached_artist: Option<CachedText>,
}

/// Floating Liquid Glass mini-player card anchored above `MediaBar`.
pub struct MiniPlayerPopup {
    hwnd: HWND,
    inner: Rc<RefCell<MiniPlayerInner>>,
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
            let mut hwnd_guard = InitHwndGuard(Some(hwnd));

            let rt = create_dc_render_target(&d2d_factory)?;
            let card_layer = rt.CreateLayer(None)?;
            let art_layer = rt.CreateLayer(None)?;
            let marquee_layer = rt.CreateLayer(None)?;
            let surface = LayeredSurface::new()?;
            let round_stroke = create_round_cap_stroke(&d2d_factory);

            let inner = Rc::new(RefCell::new(MiniPlayerInner {
                hwnd,
                main_hwnd,
                d2d_factory,
                dwrite_factory,
                backdrop_bmp: None,
                noise_cache: PopupNoiseCache::default(),
                silk_bmp: None,
                cached_art: None,
                card_layer,
                art_layer,
                marquee_layer,
                rt,
                surface,
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
                dragging_volume: false,
                pending_seek_100ns: None,
                optimistic_seek: None,
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
                suppress_deactivate_close: false,
                title_marquee: MarqueeState::default(),
                artist_marquee: MarqueeState::default(),
                cached_formats: None,
                cached_title: None,
                cached_artist: None,
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
            begin_close_mini_player(self.hwnd, &self.inner);
        }
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.reset_transient_state();
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

    pub fn take_pending_seek(&mut self) -> Option<i64> {
        self.inner
            .try_borrow_mut()
            .ok()
            .and_then(|mut s| s.pending_seek_100ns.take())
    }

    pub fn set_system_theme(&mut self, light_glass: bool, backdrop_rgb: Rgb) {
        let visible = self.is_visible();
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            let changed = s.light_glass != light_glass || s.backdrop_rgb != backdrop_rgb;
            s.light_glass = light_glass;
            s.backdrop_rgb = backdrop_rgb;
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

    pub fn set_auto_color(&mut self, auto_color: bool) {
        let visible = self.is_visible();
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            if s.auto_color != auto_color {
                s.auto_color = auto_color;
                s.halo_colors_ready = false;
                s.last_silk_tick = None;
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

    pub fn update_volume(&mut self, volume: f32, muted: bool, show_badge: bool) {
        let visible = self.is_visible();
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            if s.dragging_volume {
                false
            } else {
                let changed =
                    (s.master_vol - volume).abs() > 0.002 || s.master_muted != muted || show_badge;
                s.master_vol = volume.clamp(0.0, 1.0);
                s.master_muted = muted;
                if show_badge {
                    s.vol_badge_until = Some(Instant::now() + VOL_BADGE_TTL);
                }
                if visible && (changed || show_badge) {
                    s.sync_timer_rate();
                }
                changed && visible
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

    pub fn update_bars(&mut self, bars: &[f32; NUM_BARS]) {
        let visible = self.is_visible();
        if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.bar_levels = *bars;
            if visible && s.timer_interval_ms == 0 && bars.iter().any(|&v| v > 0.002) {
                s.sync_timer_rate();
            }
        }
    }

    pub fn update_live_source(&mut self, source: Option<SourceItem>) {
        let visible = self.is_visible();
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            let next_has_tl = source
                .as_ref()
                .and_then(|src| src.timeline)
                .is_some_and(|tl| tl.is_valid());
            let prev_track = s
                .source
                .as_ref()
                .map(|src| (src.id.as_str(), src.title.as_str(), src.artist.as_str()));
            let next_track = source
                .as_ref()
                .map(|src| (src.id.as_str(), src.title.as_str(), src.artist.as_str()));
            if prev_track != next_track {
                s.dragging_seek_ratio = None;
                s.optimistic_seek = None;
            }
            s.source = source;
            s.has_timeline = next_has_tl;
            visible
        } else {
            false
        };

        if should_render && self.is_visible() {
            if let Ok(mut s) = self.inner.try_borrow_mut() {
                let _ = s.render();
                s.sync_timer_rate();
            }
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
        if self.is_visible() {
            if let Ok(mut s) = self.inner.try_borrow_mut() {
                s.suppress_deactivate_close = true;
            }
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            if let Ok(mut s) = self.inner.try_borrow_mut() {
                s.suppress_deactivate_close = false;
            }
        }
        let Some((win_x, win_y, win_w, win_h)) = (|| {
            let mut s = self.inner.try_borrow_mut().ok()?;
            s.dpi = dpi.max(96);
            s.anchor_rect = anchor_rect;
            s.light_glass = light_glass;
            s.backdrop_rgb = fallback_backdrop_rgb;
            s.auto_color = auto_color;
            s.source = source;
            s.bar_levels = *bars;
            s.master_vol = master_vol.clamp(0.0, 1.0);
            s.master_muted = master_muted;
            s.vol_badge_until = None;
            s.hover_zone = MiniPlayerHitZone::None;
            s.pressed_zone = MiniPlayerHitZone::None;
            s.dragging_seek_ratio = None;
            s.dragging_volume = false;
            s.pending_seek_100ns = None;
            s.optimistic_seek = None;
            s.has_timeline = s
                .source
                .as_ref()
                .and_then(|src| src.timeline)
                .is_some_and(|tl| tl.is_valid());

            let coords = s.reposition_and_recapture();

            s.halo_colors_ready = false;
            s.last_silk_tick = None;
            s.last_color_tick = Instant::now();
            s.anim_phase = PopupAnimPhase::Opening;
            s.open_spring = Spring1D::new(0.14, 2.8);
            s.open_elapsed = 0.0;
            s.close_progress = 0.0;
            s.close_started_at = None;

            let _ = s.render();
            Some(coords)
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

impl Drop for MiniPlayerPopup {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_MINI_TICK_ID);
            let raw =
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0) as *const RefCell<MiniPlayerInner>;
            if !raw.is_null() {
                drop(Rc::from_raw(raw));
            }
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
                let player_caught_up = tl.position_100ns.abs_diff(target_pos) <= 20_000_000
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

    fn reposition_and_recapture(&mut self) -> (i32, i32, i32, i32) {
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
        (win_x, win_y, layout.win_w, layout.win_h)
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
        self.dragging_volume = false;
        self.vol_badge_until = None;
        self.anim_phase = PopupAnimPhase::Open;
        self.open_spring = Spring1D::still(1.0);
        self.open_elapsed = 1.0;
        self.close_progress = 0.0;
        self.backdrop_bmp = None;
        self.suppress_deactivate_close = false;
    }

    fn desired_timer_interval_ms(&self) -> u32 {
        let is_animating = matches!(
            self.anim_phase,
            PopupAnimPhase::Opening | PopupAnimPhase::Closing
        );
        let is_playing = self.is_playing();
        let marquee_active = self.title_marquee.is_active() || self.artist_marquee.is_active();
        let bars_active = self.bar_levels.iter().any(|&v| v > 0.002);
        let badge_active = self.vol_badge_until.is_some_and(|t| Instant::now() < t);
        let seek_active = self.optimistic_seek.is_some();
        if is_animating
            || is_playing
            || marquee_active
            || bars_active
            || badge_active
            || seek_active
            || self.dragging_volume
        {
            POPUP_ANIM_INTERVAL_MS
        } else {
            let recipe =
                popup_glass_recipe(active_taskbar_surface_style(), self.light_glass, 0.5);
            if recipe.silk_alpha > 0.002 {
                POPUP_FLOW_INTERVAL_MS
            } else {
                0
            }
        }
    }

    fn sync_timer_rate(&mut self) {
        let want = self.desired_timer_interval_ms();
        if self.timer_interval_ms != want {
            unsafe {
                if want == 0 {
                    let _ = KillTimer(self.hwnd, TIMER_MINI_TICK_ID);
                } else {
                    if self.timer_interval_ms == 0 {
                        let now = Instant::now();
                        self.last_flow_tick = now;
                        self.last_color_tick = now;
                    }
                    SetTimer(self.hwnd, TIMER_MINI_TICK_ID, want, None);
                }
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
        for (cur, target) in self.halo_colors.iter_mut().zip(target_pair.iter()) {
            cur.0 += (target.0 - cur.0) * k;
            cur.1 += (target.1 - cur.1) * k;
            cur.2 += (target.2 - cur.2) * k;
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
        let badge_fmt = create_text_format(&self.dwrite_factory, 11.0 * s, true, false, true)?;
        let title_fmt = create_text_format(&self.dwrite_factory, 16.0 * s, true, false, false)?;
        let artist_fmt = create_text_format(&self.dwrite_factory, 12.5 * s, false, false, false)?;
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

    fn ensure_art_bitmap(&mut self, cover: &MediaCover) -> Option<(ID2D1Bitmap, Option<ID2D1Bitmap>)> {
        if let Some(ref c) = self.cached_art {
            if c.hash == cover.hash {
                return Some((c.bitmap.clone(), c.ambient_bitmap.clone()));
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

            let ambient_bgra = build_ambient_cover_bgra(
                &cover.thumb_pbgra,
                cover.thumb_w as usize,
                cover.thumb_h as usize,
            );
            let ambient_bitmap = self
                .rt
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: cover.thumb_w,
                        height: cover.thumb_h,
                    },
                    Some(ambient_bgra.as_ptr().cast()),
                    cover.thumb_w * 4,
                    &props,
                )
                .ok();

            self.cached_art = Some(CachedArtBitmap {
                hash: cover.hash,
                bitmap: bmp.clone(),
                ambient_bitmap: ambient_bitmap.clone(),
            });
            Some((bmp, ambient_bitmap))
        }
    }

    fn tick(&mut self) -> bool {
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
                    return true;
                }
            }
            PopupAnimPhase::Open => {}
        }

        let _ = self.title_marquee.tick(dt);
        let _ = self.artist_marquee.tick(dt);

        self.sync_timer_rate();
        let _ = self.render();
        false
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

    fn post_volume(&self, ratio: f32) {
        let scaled = (ratio.clamp(0.0, 1.0) * 10000.0).round() as usize;
        unsafe {
            let _ = PostMessageW(
                self.main_hwnd,
                WM_APP_MINI_PLAYER_VOLUME,
                WPARAM(scaled),
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
                self.sync_timer_rate();
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
