use super::arbitration::{PlaybackState, SessionArbitrator, SessionSummary};
use super::audio::{start_audio_worker, AudioHandle};
use super::config::{portable_config_path, Config};
use super::dsp::NUM_BARS;
use super::layout::{hit_test_mode, BarLayout, HitZone, BAR_WIDTH_DIP};
use super::marquee::MarqueeState;
use super::mini_player::{
    MiniPlayerPopup, MINI_ACTION_ACTIVATE, MINI_ACTION_NEXT, MINI_ACTION_OPEN_MENU,
    MINI_ACTION_PLAY_PAUSE, MINI_ACTION_PREV, MINI_ACTION_TOGGLE_MUTE, WM_APP_MINI_PLAYER_ACTION,
    WM_APP_MINI_PLAYER_CLOSED, WM_APP_MINI_PLAYER_SEEK, WM_APP_MINI_PLAYER_WHEEL,
};
use super::popup::{
    SourcePopup, SETTING_CLOSE, SETTING_RESET_POS, SETTING_TOGGLE_AUTO_COLOR, SETTING_TOGGLE_LOCK,
    WM_APP_AUDIO_OUTPUT_CHANGED, WM_APP_POPUP_CLOSED, WM_APP_SETTING_ACTION,
    WM_APP_SOURCE_SELECTED,
};
use super::process_util::activate_player_window;
use super::render::{BarRenderer, RenderFrame};
use super::smtc::{
    start_smtc_worker, MediaCover, SmtcCommand, SmtcHandle, SmtcSnapshot, SourceItem,
    WM_APP_SMTC_UPDATED,
};
use super::taskbar::{
    compute_bar_rect, default_bar_x, ensure_above_taskbar, is_fullscreen_active, query_taskbar_state,
    TaskbarState,
};
use super::timeline::current_filetime_100ns;
use super::volume::{
    slider_ratio_from_x, MasterVolumeControl, VolumeWatcher, VOL_EVENT_DEVICE, VOL_EVENT_MASTER,
    VOL_EVENT_SESSIONS, WM_APP_VOLUME_UPDATED,
};
use crate::color::{
    derive_background_accent, enforce_ui_accent_deadzone, Rgb, NEUTRAL_BACKDROP_FALLBACK,
};
use crate::system::{
    exclude_from_peek, is_light_taskbar, notify_peer_taskbar_bar_resync, peer_taskbar_bar_span,
    register_taskbar_bar, set_mediabar_shared_accent, unregister_taskbar_bar,
    update_taskbar_bar_bounds, BarCollisionMode, TaskbarBarSlot,
};
use crate::ui::glass::sample_screen_region_rgb;
use crate::ui::POPUP_REOPEN_DEBOUNCE_MS;
use std::cell::RefCell;
use std::mem::size_of;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1Factory1;
use windows::Win32::Graphics::DirectWrite::IDWriteFactory;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, GetCursorPos,
    GetWindowLongPtrW, IsIconic, IsWindow, IsWindowVisible, KillTimer, LoadCursorW, PostMessageW,
    RegisterClassExW, RegisterWindowMessageW, SetCursor, SetTimer, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWLP_USERDATA, HCURSOR, HICON, HTCLIENT, HWND_TOPMOST, IDC_ARROW,
    MA_NOACTIVATE, SC_MINIMIZE, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, WM_APP,
    WM_CAPTURECHANGED, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP, WM_SETCURSOR,
    WM_SETTINGCHANGE, WM_SYSCOMMAND, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const MAIN_CLASS_NAME: PCWSTR = w!("MediaBarMainWnd");
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const WM_APP_MEDIABAR_RESYNC_POS: u32 = WM_APP + 6;

const TIMER_GUARD_ID: usize = 1;
const TIMER_ANIM_ID: usize = 2;
const GUARD_INTERVAL_MS: u32 = 150;
const ANIM_INTERVAL_MS: u32 = 16;
const VOL_HUD_TIMEOUT: Duration = Duration::from_millis(1500);

static REGISTER_MAIN_CLASS: Once = Once::new();

struct AppState {
    hwnd: HWND,
    host_hwnd: HWND,
    close_request_msg: u32,
    taskbar_created_msg: u32,
    config_path: PathBuf,
    config: Config,
    light_taskbar: bool,
    backdrop_rgb: Rgb,
    taskbar: Option<TaskbarState>,
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
    dpi: u32,
    visible: bool,
    hidden_by_guard: bool,

    smtc: SmtcHandle,
    audio: Arc<AudioHandle>,
    snapshot: SmtcSnapshot,
    arbitrator: SessionArbitrator,
    active_id: Option<String>,

    renderer: BarRenderer,
    popup: SourcePopup,
    popup_closed_at: Option<Instant>,
    mini_player: MiniPlayerPopup,
    mini_player_closed_at: Option<Instant>,

    hover_t: f32,
    target_hover_t: f32,
    hover_zone: HitZone,
    pressed_zone: HitZone,
    tracking_leave: bool,

    is_mouse_down: bool,
    is_dragging: bool,
    drag_start_screen_x: i32,
    drag_start_offset_x: i32,

    volume_watcher: Option<VolumeWatcher>,
    master_vol_ctrl: Option<MasterVolumeControl>,
    master_vol: f32,
    master_muted: bool,
    vol_hud_active: bool,
    vol_hud_dragging: bool,
    vol_hud_deadline: Option<Instant>,

    last_bars: [f32; NUM_BARS],
    title_marquee: MarqueeState,
    artist_marquee: MarqueeState,
    anim_timer_running: bool,
    last_anim_tick: Instant,
    last_progress_draw: Instant,
    dirty: bool,
    arrow_cursor: HCURSOR,
    grab_cursor: HCURSOR,
    grab_cursor_owned: bool,
}

impl Drop for AppState {
    fn drop(&mut self) {
        if self.grab_cursor_owned && !self.grab_cursor.is_invalid() {
            unsafe {
                let _ = DestroyIcon(HICON(self.grab_cursor.0));
            }
        }
    }
}

impl AppState {
    fn new(
        hwnd: HWND,
        host_hwnd: HWND,
        close_request_msg: u32,
        taskbar_created_msg: u32,
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
    ) -> windows::core::Result<Self> {
        let config_path = portable_config_path();
        let mut config = Config::load_from_path(&config_path);
        config.enabled = true;
        let light_taskbar = is_light_taskbar();
        let taskbar = query_taskbar_state();
        let backdrop_rgb = taskbar
            .and_then(|tb| {
                let sample_h = (tb.taskbar_rect.bottom - tb.taskbar_rect.top).clamp(8, 32);
                let sample_w = 64.min((tb.taskbar_rect.right - tb.taskbar_rect.left).max(8));
                let sample_x = tb.taskbar_rect.left + ((tb.taskbar_rect.right - tb.taskbar_rect.left - sample_w) / 2).max(0);
                sample_screen_region_rgb(sample_x, tb.taskbar_rect.top, sample_w, sample_h)
            })
            .unwrap_or(NEUTRAL_BACKDROP_FALLBACK);

        let renderer = BarRenderer::new_with_factories(d2d_factory, dwrite_factory)?;
        let mut popup = SourcePopup::new(
            hwnd,
            renderer.d2d_factory.clone(),
            renderer.dwrite_factory.clone(),
        )?;
        let mini_player = MiniPlayerPopup::new(
            hwnd,
            renderer.d2d_factory.clone(),
            renderer.dwrite_factory.clone(),
        )?;

        let volume_watcher = VolumeWatcher::start(hwnd);
        if let Some(ref watcher) = volume_watcher {
            popup.set_volume_shared(watcher.shared_state());
        }
        let (init_master_vol, init_master_muted) = volume_watcher
            .as_ref()
            .and_then(|w| w.current_master_state())
            .unwrap_or((1.0, false));

        let smtc = start_smtc_worker(hwnd, NEUTRAL_BACKDROP_FALLBACK);
        let audio = start_audio_worker();
        let init_dpi = taskbar.map(|t| t.dpi).unwrap_or(96);
        let arrow_cursor = unsafe { LoadCursorW(None, IDC_ARROW)? };
        let (grab_cursor, grab_cursor_owned) = renderer.create_grabbing_cursor(init_dpi)?;

        let mut app = Self {
            hwnd,
            host_hwnd,
            close_request_msg,
            taskbar_created_msg,
            config_path,
            config,
            light_taskbar,
            backdrop_rgb,
            taskbar,
            win_x: 0,
            win_y: 0,
            win_w: BAR_WIDTH_DIP as i32,
            win_h: 48,
            dpi: init_dpi,
            visible: false,
            hidden_by_guard: false,
            smtc,
            audio,
            snapshot: SmtcSnapshot::default(),
            arbitrator: SessionArbitrator::new(),
            active_id: None,
            renderer,
            popup,
            popup_closed_at: None,
            mini_player,
            mini_player_closed_at: None,
            hover_t: 0.0,
            target_hover_t: 0.0,
            hover_zone: HitZone::None,
            pressed_zone: HitZone::None,
            tracking_leave: false,
            is_mouse_down: false,
            is_dragging: false,
            drag_start_screen_x: 0,
            drag_start_offset_x: 0,
            volume_watcher,
            master_vol_ctrl: None,
            master_vol: init_master_vol,
            master_muted: init_master_muted,
            vol_hud_active: false,
            vol_hud_dragging: false,
            vol_hud_deadline: None,
            last_bars: [0.0; NUM_BARS],
            title_marquee: MarqueeState::default(),
            artist_marquee: MarqueeState::default(),
            anim_timer_running: false,
            last_anim_tick: Instant::now(),
            last_progress_draw: Instant::now(),
            dirty: true,
            arrow_cursor,
            grab_cursor,
            grab_cursor_owned,
        };

        app.sync_shared_accent();
        app.sync_taskbar_geometry(true);
        Ok(app)
    }

    fn sync_shared_accent(&self) {
        let shared = if self.config.auto_color {
            self.active_source()
                .and_then(|s| s.cover.as_ref())
                .or_else(|| self.snapshot.sources.iter().find_map(|s| s.cover.as_ref()))
                .map(|c| (c.accent_light, c.accent_dark))
        } else {
            None
        };
        set_mediabar_shared_accent(shared);
    }

    fn refresh_backdrop_sample(&mut self) {
        let Some(tb) = self.taskbar else {
            return;
        };
        // Sample taskbar strip just above or to the left of MediaBar so we measure the taskbar/wallpaper itself.
        let sample_w = 48i32;
        let sample_h = 20i32.min((tb.taskbar_rect.bottom - tb.taskbar_rect.top).max(8));
        let sample_x = (self.win_x - sample_w - 8).max(tb.taskbar_rect.left + 8);
        let sample_y = tb.taskbar_rect.top + 4;
        if let Some(bg) = sample_screen_region_rgb(sample_x, sample_y, sample_w, sample_h) {
            if bg != self.backdrop_rgb {
                self.backdrop_rgb = bg;
                self.dirty = true;
            }
        }
    }

    fn active_source(&self) -> Option<&SourceItem> {
        self.active_id
            .as_ref()
            .and_then(|id| self.snapshot.sources.iter().find(|s| &s.id == id))
    }

    fn is_playing(&self) -> bool {
        self.active_source()
            .is_some_and(|s| s.status == PlaybackState::Playing)
    }

    fn active_progress(&self) -> Option<f32> {
        let src = self.active_source()?;
        let tl = src.timeline?;
        tl.progress_ratio(current_filetime_100ns(), src.status == PlaybackState::Playing)
    }

    fn any_popup_visible(&self) -> bool {
        self.popup.is_visible() || self.mini_player.is_visible()
    }

    fn any_popup_open(&self) -> bool {
        self.popup.is_open() || self.mini_player.is_open()
    }

    fn sync_taskbar_geometry(&mut self, force_render: bool) {
        self.sync_taskbar_geometry_with_mode(force_render, None, BarCollisionMode::Passive);
    }

    fn sync_taskbar_geometry_with_mode(
        &mut self,
        force_render: bool,
        drag_offset_override: Option<i32>,
        mode: BarCollisionMode,
    ) {
        let tb_opt = query_taskbar_state();
        self.taskbar = tb_opt;

        let Some(tb) = tb_opt else {
            update_taskbar_bar_bounds(TaskbarBarSlot::MediaBar, None);
            self.set_bar_visible(false);
            return;
        };

        let occluded = tb.auto_hide_collapsed || is_fullscreen_active(tb.taskbar_hwnd);
        if occluded {
            update_taskbar_bar_bounds(TaskbarBarSlot::MediaBar, None);
            self.hidden_by_guard = true;
            if self.popup.is_visible() {
                self.popup.hide();
            }
            if self.mini_player.is_visible() {
                self.mini_player.hide();
            }
            self.clear_vol_hud();
            self.set_bar_visible(false);
            return;
        }
        self.hidden_by_guard = false;

        let scale = (tb.dpi as f32) / 96.0;
        let width_px = (BAR_WIDTH_DIP * scale).round() as i32;
        let desired_offset = drag_offset_override.unwrap_or(self.config.offset_x);
        let current_x = if self.win_w > 0 {
            Some(self.win_x)
        } else {
            None
        };
        let peer_span = peer_taskbar_bar_span(TaskbarBarSlot::MediaBar);
        let (x, y, w, h, resolved_offset) =
            compute_bar_rect(&tb, width_px, desired_offset, current_x, peer_span, mode);
        if mode == BarCollisionMode::Drag {
            self.config.offset_x = resolved_offset;
        }
        update_taskbar_bar_bounds(TaskbarBarSlot::MediaBar, Some((x, x + w)));

        let geom_changed =
            self.win_x != x || self.win_y != y || self.win_w != w || self.win_h != h || self.dpi != tb.dpi;
        self.win_x = x;
        self.win_y = y;
        self.win_w = w;
        self.win_h = h;
        self.dpi = tb.dpi;

        if geom_changed {
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    x,
                    y,
                    w,
                    h,
                    SWP_NOACTIVATE,
                );
            }
            self.refresh_backdrop_sample();
            self.dirty = true;
        } else {
            ensure_above_taskbar(self.hwnd, tb.taskbar_hwnd);
        }

        self.set_bar_visible(true);
        if force_render || self.dirty {
            self.render_now();
        }
        self.popup.ensure_topmost();
        self.mini_player.ensure_topmost();
    }

    fn set_bar_visible(&mut self, show: bool) {
        if show {
            let os_hidden = unsafe {
                !IsWindowVisible(self.hwnd).as_bool() || IsIconic(self.hwnd).as_bool()
            };
            if self.visible && !os_hidden {
                return;
            }
            self.visible = true;
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.dirty = true;
            self.render_now();
        } else {
            if !self.visible {
                return;
            }
            self.visible = false;
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }

    fn refresh_smtc_selection(&mut self) {
        self.snapshot = self.smtc.snapshot();
        let summaries: Vec<SessionSummary> = self
            .snapshot
            .sources
            .iter()
            .map(|s| SessionSummary {
                id: s.id.clone(),
                status: s.status,
                has_media_info: !s.title.is_empty() || !s.artist.is_empty(),
            })
            .collect();

        self.active_id = self
            .arbitrator
            .select(&summaries, self.snapshot.system_current_id.as_deref());

        let (is_playing, active_pid) = self
            .active_source()
            .map(|s| (s.status == PlaybackState::Playing, s.pid))
            .unwrap_or((false, None));

        self.audio.set_target(is_playing, active_pid);
        self.sync_shared_accent();
        if self.popup.is_visible() {
            self.popup.update_live_sources(
                self.snapshot.sources.clone(),
                self.arbitrator.mode().clone(),
                self.active_id.clone(),
            );
        }
        if self.mini_player.is_visible() {
            self.mini_player
                .update_live_source(self.active_source().cloned());
        }
        self.sync_marquee();
        self.dirty = true;
        self.render_now();
        self.update_anim_timer();
    }

    fn sync_marquee(&mut self) {
        let (is_idle, is_playing, title, artist, cover, accent) = self.frame_metadata();
        let mut frame = RenderFrame {
            hwnd: self.hwnd,
            win_x: self.win_x,
            win_y: self.win_y,
            width_px: self.win_w,
            height_px: self.win_h,
            dpi: self.dpi,
            light_taskbar: self.light_taskbar,
            is_idle,
            is_playing,
            title: &title,
            artist: &artist,
            cover: cover.as_ref(),
            accent_color: accent,
            hover_t: self.hover_t,
            hover_zone: self.hover_zone,
            pressed_zone: self.pressed_zone,
            locked: self.config.locked,
            vol_hud_active: self.vol_hud_active,
            vol_hud_dragging: self.vol_hud_dragging,
            master_volume: self.master_vol,
            master_muted: self.master_muted,
            popup_open: false,
            progress: None,
            bar_levels: &self.last_bars,
            title_marquee: &mut self.title_marquee,
            artist_marquee: &mut self.artist_marquee,
        };
        self.renderer.sync_marquee_metrics(&mut frame);
    }

    fn frame_metadata(
        &self,
    ) -> (
        bool,
        bool,
        String,
        String,
        Option<Arc<MediaCover>>,
        Rgb,
    ) {
        let fallback_bg_accent = enforce_ui_accent_deadzone(
            derive_background_accent(self.backdrop_rgb, self.light_taskbar),
            self.light_taskbar,
        );
        if let Some(src) = self.active_source() {
            let accent = if self.config.auto_color {
                src.cover
                    .as_ref()
                    .map(|c| {
                        let raw = if self.light_taskbar {
                            c.accent_light
                        } else {
                            c.accent_dark
                        };
                        enforce_ui_accent_deadzone(raw, self.light_taskbar)
                    })
                    .unwrap_or(fallback_bg_accent)
            } else {
                fallback_bg_accent
            };
            (
                false,
                src.status == PlaybackState::Playing,
                src.title.clone(),
                src.artist.clone(),
                src.cover.clone(),
                accent,
            )
        } else {
            (
                true,
                false,
                String::new(),
                String::new(),
                None,
                fallback_bg_accent,
            )
        }
    }

    fn render_now(&mut self) {
        if !self.visible || self.hidden_by_guard {
            return;
        }
        let popup_open = self.any_popup_visible();
        let progress = self.active_progress();
        let (is_idle, is_playing, title, artist, cover, accent) = self.frame_metadata();
        let mut frame = RenderFrame {
            hwnd: self.hwnd,
            win_x: self.win_x,
            win_y: self.win_y,
            width_px: self.win_w,
            height_px: self.win_h,
            dpi: self.dpi,
            light_taskbar: self.light_taskbar,
            is_idle,
            is_playing,
            title: &title,
            artist: &artist,
            cover: cover.as_ref(),
            accent_color: accent,
            hover_t: self.hover_t,
            hover_zone: self.hover_zone,
            pressed_zone: self.pressed_zone,
            locked: self.config.locked,
            vol_hud_active: self.vol_hud_active,
            vol_hud_dragging: self.vol_hud_dragging,
            master_volume: self.master_vol,
            master_muted: self.master_muted,
            popup_open,
            progress,
            bar_levels: &self.last_bars,
            title_marquee: &mut self.title_marquee,
            artist_marquee: &mut self.artist_marquee,
        };
        let _ = self.renderer.render(&mut frame);
        self.last_progress_draw = Instant::now();
        self.dirty = false;
    }

    fn needs_anim_tick(&self) -> bool {
        if !self.visible || self.hidden_by_guard {
            return false;
        }
        let hover_animating = (self.hover_t - self.target_hover_t).abs() > 0.004;
        let is_playing = self.is_playing();
        let audio_animating = is_playing || self.audio.is_decaying_or_active();
        let marquee_animating = self.title_marquee.is_active() || self.artist_marquee.is_active();
        let progress_animating = is_playing
            && self
                .active_source()
                .and_then(|s| s.timeline)
                .is_some_and(|tl| tl.is_valid());
        hover_animating || audio_animating || marquee_animating || progress_animating
    }

    fn update_anim_timer(&mut self) {
        let needed = self.needs_anim_tick();
        unsafe {
            if needed && !self.anim_timer_running {
                self.last_anim_tick = Instant::now();
                SetTimer(self.hwnd, TIMER_ANIM_ID, ANIM_INTERVAL_MS, None);
                self.anim_timer_running = true;
            } else if !needed && self.anim_timer_running {
                let _ = KillTimer(self.hwnd, TIMER_ANIM_ID);
                self.anim_timer_running = false;
            }
        }
    }

    fn tick_animation(&mut self) {
        let dt = self.last_anim_tick.elapsed().as_secs_f32().clamp(0.001, 0.05);
        self.last_anim_tick = Instant::now();

        let mut need_draw = self.dirty;

        // 1. Hover transition
        let hover_diff = self.target_hover_t - self.hover_t;
        if hover_diff.abs() > 0.004 {
            let step = (dt * 9.5).min(1.0);
            self.hover_t += hover_diff * step;
            if (self.target_hover_t - self.hover_t).abs() <= 0.004 {
                self.hover_t = self.target_hover_t;
            }
            need_draw = true;
        }

        // 2. Audio spectrum bars
        let bars = self.audio.read_bars();
        let bars_changed = bars
            .iter()
            .zip(self.last_bars.iter())
            .any(|(&a, &b)| (a - b).abs() > 0.003);
        if bars_changed {
            self.last_bars = bars;
            if self.mini_player.is_visible() {
                self.mini_player.update_bars(&self.last_bars);
            }
            need_draw = true;
        }

        // 3. Marquee scrolling & taskbar hairline progress tick
        let is_playing = self.is_playing();
        if self.title_marquee.tick(dt) {
            need_draw = true;
        }
        if self.artist_marquee.tick(dt) {
            need_draw = true;
        }
        if !need_draw
            && is_playing
            && !self.vol_hud_active
            && self.last_progress_draw.elapsed() >= Duration::from_millis(200)
            && self.active_progress().is_some()
        {
            need_draw = true;
        }

        if need_draw {
            self.render_now();
        }
        self.update_anim_timer();
    }

    fn current_layout(&self) -> BarLayout {
        BarLayout::compute(self.win_h, self.dpi, self.hover_t, &self.last_bars)
    }

    fn clear_vol_hud(&mut self) {
        self.vol_hud_active = false;
        self.vol_hud_dragging = false;
        self.vol_hud_deadline = None;
        self.master_vol_ctrl = None;
        if matches!(
            self.hover_zone,
            HitZone::VolumeMuteButton | HitZone::VolumeSlider
        ) {
            self.hover_zone = HitZone::None;
        }
        if matches!(
            self.pressed_zone,
            HitZone::VolumeMuteButton | HitZone::VolumeSlider
        ) {
            self.pressed_zone = HitZone::None;
        }
        self.dirty = true;
    }

    fn apply_master_wheel(&mut self, wheel_delta: i16) {
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        if let Some((actual_vol, actual_muted)) = self
            .master_vol_ctrl
            .as_mut()
            .and_then(|ctrl| ctrl.step_wheel_and_readback(wheel_delta))
        {
            self.master_vol = actual_vol;
            self.master_muted = actual_muted;
        }
        self.vol_hud_active = true;
        self.vol_hud_deadline = Some(Instant::now() + VOL_HUD_TIMEOUT);
        self.target_hover_t = 1.0;

        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt).is_ok() } {
            let lx = (pt.x - self.win_x) as f32;
            let ly = (pt.y - self.win_y) as f32;
            let layout = self.current_layout();
            self.hover_zone = hit_test_mode(&layout, lx, ly, true, true);
        }
        self.dirty = true;
        self.render_now();
        self.update_anim_timer();
    }

    fn set_hud_volume_by_cursor_x(&mut self, local_x: f32) {
        let layout = self.current_layout();
        let ratio = slider_ratio_from_x(
            layout.vol_track_rect.left,
            layout.vol_track_rect.right,
            local_x,
        );
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        if let Some((actual_vol, actual_muted)) = self
            .master_vol_ctrl
            .as_mut()
            .and_then(|ctrl| ctrl.set_volume_and_readback(ratio))
        {
            self.master_vol = actual_vol;
            self.master_muted = actual_muted;
        } else {
            self.master_vol = ratio;
            if self.master_muted && ratio > 0.0 {
                self.master_muted = false;
            }
        }
        self.vol_hud_deadline = Some(Instant::now() + VOL_HUD_TIMEOUT);
        self.dirty = true;
    }

    fn toggle_hud_mute(&mut self) {
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        let next_muted = !self.master_muted;
        if let Some((actual_vol, actual_muted)) = self
            .master_vol_ctrl
            .as_mut()
            .and_then(|ctrl| ctrl.set_mute_and_readback(next_muted))
        {
            self.master_vol = actual_vol;
            self.master_muted = actual_muted;
        } else {
            self.master_muted = next_muted;
        }
        self.vol_hud_deadline = Some(Instant::now() + VOL_HUD_TIMEOUT);
        self.dirty = true;
    }

    fn refresh_master_volume_state(&mut self) {
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        if let Some((vol, muted)) = self
            .master_vol_ctrl
            .as_mut()
            .and_then(|c| c.get_state())
        {
            self.master_vol = vol;
            self.master_muted = muted;
        }
    }

    fn apply_mini_player_wheel(&mut self, wheel_delta: i16) {
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        if let Some((next_vol, next_muted)) = self
            .master_vol_ctrl
            .as_mut()
            .and_then(|ctrl| ctrl.step_wheel_and_readback(wheel_delta))
        {
            self.master_vol = next_vol;
            self.master_muted = next_muted;
        } else {
            self.refresh_master_volume_state();
        }
        self.mini_player
            .update_volume(self.master_vol, self.master_muted, true);
    }

    fn toggle_mini_player_mute(&mut self) {
        self.refresh_master_volume_state();
        let next_muted = !self.master_muted;
        if let Some((actual_vol, actual_muted)) = self
            .master_vol_ctrl
            .as_mut()
            .and_then(|ctrl| ctrl.set_mute_and_readback(next_muted))
        {
            self.master_vol = actual_vol;
            self.master_muted = actual_muted;
        } else {
            self.master_muted = next_muted;
        }
        self.mini_player
            .update_volume(self.master_vol, self.master_muted, true);
    }

    fn handle_external_volume_events(&mut self, msg_flag: usize) {
        let pending = self
            .volume_watcher
            .as_ref()
            .map_or(0, VolumeWatcher::take_pending_events);
        let flags = msg_flag | pending;

        if (flags & VOL_EVENT_DEVICE) != 0 {
            self.master_vol_ctrl = None;
            let (render_changed, latest) = self
                .volume_watcher
                .as_mut()
                .map_or((true, None), VolumeWatcher::rebind_default_endpoint);
            if render_changed {
                self.audio.notify_device_changed();
            }
            if let Some((vol, muted)) = latest {
                self.master_vol = vol;
                self.master_muted = muted;
            } else {
                self.refresh_master_volume_state();
            }
            if self.popup.is_visible() {
                self.popup
                    .sync_external_devices(self.master_vol, self.master_muted);
            }
            if self.mini_player.is_visible() {
                self.mini_player
                    .update_volume(self.master_vol, self.master_muted, false);
            }
            if self.vol_hud_active && !self.vol_hud_dragging {
                self.dirty = true;
                self.render_now();
            }
            return;
        }

        if (flags & VOL_EVENT_MASTER) != 0 {
            let prev = (self.master_vol, self.master_muted);
            if let Some((vol, muted)) = self
                .volume_watcher
                .as_ref()
                .and_then(VolumeWatcher::current_master_state)
            {
                self.master_vol = vol;
                self.master_muted = muted;
            } else {
                self.refresh_master_volume_state();
            }
            let changed =
                (self.master_vol - prev.0).abs() > 0.001 || self.master_muted != prev.1;
            if changed {
                if self.vol_hud_active && !self.vol_hud_dragging {
                    self.dirty = true;
                    self.render_now();
                }
                if self.mini_player.is_visible() {
                    self.mini_player
                        .update_volume(self.master_vol, self.master_muted, true);
                }
            }
            if self.popup.is_visible() {
                self.popup
                    .sync_external_master_volume(self.master_vol, self.master_muted);
            }
        }

        if (flags & VOL_EVENT_SESSIONS) != 0 && self.popup.is_visible() {
            self.popup.sync_external_sessions();
        }
    }

    fn sync_hover_with_cursor(&mut self) {
        if !self.visible || self.hidden_by_guard || self.any_popup_visible() {
            return;
        }
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt).is_err() } {
            return;
        }
        let inside = pt.x >= self.win_x
            && pt.x < self.win_x + self.win_w
            && pt.y >= self.win_y
            && pt.y < self.win_y + self.win_h;

        if self.vol_hud_active && !self.vol_hud_dragging {
            let expired = self.vol_hud_deadline.is_some_and(|d| Instant::now() >= d);
            if !inside || expired {
                self.clear_vol_hud();
                if inside {
                    let local_x = (pt.x - self.win_x) as f32;
                    let local_y = (pt.y - self.win_y) as f32;
                    let layout = self.current_layout();
                    self.hover_zone =
                        hit_test_mode(&layout, local_x, local_y, self.hover_t > 0.35, false);
                }
                self.render_now();
            }
        }

        if inside {
            let local_x = (pt.x - self.win_x) as f32;
            let local_y = (pt.y - self.win_y) as f32;
            let layout = self.current_layout();
            let next_zone = if self.config.locked {
                hit_test_mode(
                    &layout,
                    local_x,
                    local_y,
                    self.hover_t > 0.35,
                    self.vol_hud_active,
                )
            } else {
                HitZone::None
            };
            let mut changed = false;
            if (self.target_hover_t - 1.0).abs() > 0.01 {
                self.target_hover_t = 1.0;
                changed = true;
            }
            if self.hover_zone != next_zone {
                self.hover_zone = next_zone;
                changed = true;
            }
            if changed {
                self.dirty = true;
                self.render_now();
            }
        } else if !self.is_mouse_down
            && !self.is_dragging
            && !self.vol_hud_dragging
            && self.target_hover_t > 0.01
        {
            self.target_hover_t = 0.0;
            self.hover_zone = HitZone::None;
            self.pressed_zone = HitZone::None;
            self.dirty = true;
            self.render_now();
        }
    }

    fn show_mini_player(&mut self) {
        if self.vol_hud_active {
            self.clear_vol_hud();
            self.render_now();
        }
        if self.popup.is_visible() {
            self.popup.hide();
        }
        if self.mini_player.is_open() {
            self.mini_player.hide_animated();
            return;
        }
        if self.mini_player.is_closing_recently() {
            return;
        }
        if let Some(closed_at) = self.mini_player_closed_at.take() {
            if closed_at.elapsed() < Duration::from_millis(POPUP_REOPEN_DEBOUNCE_MS) {
                return;
            }
        }
        self.refresh_master_volume_state();
        self.hover_t = 1.0;
        self.target_hover_t = 1.0;
        let layout = self.current_layout();
        let anchor = RECT {
            left: self.win_x + layout.bg_rect.left.round() as i32,
            top: self.win_y,
            right: self.win_x + layout.bg_rect.right.round() as i32,
            bottom: self.win_y + self.win_h,
        };
        let active_src = self.active_source().cloned();
        self.mini_player.show(
            anchor,
            self.dpi,
            self.light_taskbar,
            self.backdrop_rgb,
            self.config.auto_color,
            active_src,
            &self.last_bars,
            self.master_vol,
            self.master_muted,
        );
        self.dirty = true;
        self.render_now();
        self.mini_player.ensure_topmost();
    }

    fn show_source_popup(&mut self, click_screen_x: i32) {
        if self.vol_hud_active {
            self.clear_vol_hud();
            self.render_now();
        }
        if self.mini_player.is_visible() {
            self.mini_player.hide();
        }
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
        self.target_hover_t = 1.0;
        let layout = self.current_layout();
        let anchor = RECT {
            left: self.win_x + layout.bg_rect.left.round() as i32,
            top: self.win_y,
            right: self.win_x + layout.bg_rect.right.round() as i32,
            bottom: self.win_y + self.win_h,
        };
        self.popup.show(
            anchor,
            click_screen_x,
            self.dpi,
            self.light_taskbar,
            self.backdrop_rgb,
            self.snapshot.sources.clone(),
            self.arbitrator.mode().clone(),
            self.active_id.clone(),
            self.config.locked,
            self.config.auto_color,
        );
        self.dirty = true;
        self.render_now();
        self.popup.ensure_topmost();
    }

    fn save_config(&self) {
        let _ = self.config.save_to_path(&self.config_path);
    }
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

/// Live hosted instance of the MediaBar taskbar controller.
pub struct MediaBarInstance {
    hwnd: HWND,
}

impl MediaBarInstance {
    /// Creates and launches a MediaBar instance hosted by `host_hwnd` (does not own a message loop).
    pub fn start(
        host_hwnd: HWND,
        close_request_msg: u32,
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
    ) -> windows::core::Result<Self> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarCreated"));

            let arrow_cursor = LoadCursorW(None, IDC_ARROW)?;
            REGISTER_MAIN_CLASS.call_once(|| {
                let wc = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(main_wnd_proc),
                    hInstance: hinst.into(),
                    hCursor: arrow_cursor,
                    lpszClassName: MAIN_CLASS_NAME,
                    ..Default::default()
                };
                RegisterClassExW(&wc);
            });

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                MAIN_CLASS_NAME,
                w!("MediaBar"),
                WS_POPUP,
                0,
                0,
                BAR_WIDTH_DIP as i32,
                48,
                None,
                None,
                hinst,
                None,
            )?;
            let mut hwnd_guard = InitHwndGuard(Some(hwnd));
            exclude_from_peek(hwnd);

            let app = Rc::new(RefCell::new(AppState::new(
                hwnd,
                host_hwnd,
                close_request_msg,
                taskbar_created_msg,
                d2d_factory,
                dwrite_factory,
            )?));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::into_raw(app) as isize);
            hwnd_guard.disarm();
            crate::system::register_taskbar_overlay(hwnd);
            register_taskbar_bar(TaskbarBarSlot::MediaBar, hwnd, WM_APP_MEDIABAR_RESYNC_POS);
            notify_peer_taskbar_bar_resync(TaskbarBarSlot::MediaBar);

            SetTimer(hwnd, TIMER_GUARD_ID, GUARD_INTERVAL_MS, None);
            let _ = PostMessageW(hwnd, WM_APP_SMTC_UPDATED, WPARAM(0), LPARAM(0));

            Ok(Self { hwnd })
        }
    }

    /// Stops and destroys the MediaBar window and its worker threads.
    pub fn stop(&mut self) {
        if !self.hwnd.is_invalid() && unsafe { IsWindow(self.hwnd).as_bool() } {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
        self.hwnd = HWND::default();
    }

    /// Returns `true` if the MediaBar window is currently alive.
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        !self.hwnd.is_invalid() && unsafe { IsWindow(self.hwnd).as_bool() }
    }
}

impl Drop for MediaBarInstance {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe extern "system" fn main_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let raw_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<AppState>;
    if raw_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    if msg == WM_DESTROY {
        crate::system::unregister_taskbar_overlay(hwnd);
        unregister_taskbar_bar(TaskbarBarSlot::MediaBar, hwnd);
        set_mediabar_shared_accent(None);
        let rc = Rc::from_raw(raw_ptr);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let _ = KillTimer(hwnd, TIMER_GUARD_ID);
        let _ = KillTimer(hwnd, TIMER_ANIM_ID);
        if let Ok(mut state) = rc.try_borrow_mut() {
            if let Some(ref mut watcher) = state.volume_watcher {
                watcher.shutdown();
            }
            state.popup.hide();
            state.mini_player.hide();
            state.audio.shutdown();
            state.smtc.send(SmtcCommand::Shutdown);
        }
        return LRESULT(0);
    }

    // Keep an extra Rc alive across this dispatch in case a message triggers destruction.
    Rc::increment_strong_count(raw_ptr);
    let rc = Rc::from_raw(raw_ptr);

    let taskbar_created_msg = rc
        .try_borrow()
        .map(|s| s.taskbar_created_msg)
        .unwrap_or(0);

    if taskbar_created_msg != 0 && msg == taskbar_created_msg {
        crate::system::refresh_taskbar_overlay_hooks();
        if let Ok(mut state) = rc.try_borrow_mut() {
            state.sync_taskbar_geometry(true);
        }
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
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                let cursor = rc
                    .try_borrow()
                    .map(|s| {
                        if s.is_dragging || !s.config.locked {
                            s.grab_cursor
                        } else {
                            s.arrow_cursor
                        }
                    })
                    .unwrap_or_else(|_| LoadCursorW(None, IDC_ARROW).unwrap_or_default());
                SetCursor(cursor);
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEWHEEL => {
            let wheel_delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
            if wheel_delta != 0 {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    if state.visible && !state.hidden_by_guard && !state.any_popup_open() {
                        state.apply_master_wheel(wheel_delta);
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_SMTC_UPDATED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.refresh_smtc_selection();
            }
            LRESULT(0)
        }
        WM_APP_MEDIABAR_RESYNC_POS => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.sync_taskbar_geometry(false);
            }
            LRESULT(0)
        }
        WM_APP_POPUP_CLOSED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.popup_closed_at = Some(Instant::now());
                state.dirty = true;
                state.sync_hover_with_cursor();
                if state.dirty {
                    state.render_now();
                }
                state.update_anim_timer();
            }
            LRESULT(0)
        }
        WM_APP_MINI_PLAYER_CLOSED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.mini_player_closed_at = Some(Instant::now());
                state.dirty = true;
                state.sync_hover_with_cursor();
                if state.dirty {
                    state.render_now();
                }
                state.update_anim_timer();
            }
            LRESULT(0)
        }
        WM_APP_MINI_PLAYER_ACTION => {
            let mut open_menu = false;
            let mut activate_target: Option<(String, Option<u32>)> = None;
            if let Ok(mut state) = rc.try_borrow_mut() {
                match wparam.0 {
                    MINI_ACTION_PREV => {
                        if let Some(id) = state.active_id.clone() {
                            state.smtc.send(SmtcCommand::Previous(id));
                        }
                    }
                    MINI_ACTION_PLAY_PAUSE => {
                        if let Some(id) = state.active_id.clone() {
                            state.smtc.send(SmtcCommand::TogglePlayPause(id));
                        }
                    }
                    MINI_ACTION_NEXT => {
                        if let Some(id) = state.active_id.clone() {
                            state.smtc.send(SmtcCommand::Next(id));
                        }
                    }
                    MINI_ACTION_TOGGLE_MUTE => {
                        state.toggle_mini_player_mute();
                    }
                    MINI_ACTION_ACTIVATE => {
                        activate_target = state
                            .active_source()
                            .map(|s| (s.id.clone(), s.pid));
                    }
                    MINI_ACTION_OPEN_MENU => {
                        state.mini_player.hide();
                        state.popup_closed_at = None;
                        open_menu = true;
                    }
                    _ => {}
                }
            }
            if let Some((id, pid)) = activate_target {
                let raw_id = id.split('#').next().unwrap_or(&id);
                activate_player_window(raw_id, pid);
            }
            if open_menu {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.show_source_popup(pt.x);
                }
            }
            LRESULT(0)
        }
        WM_APP_MINI_PLAYER_SEEK => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if let (Some(id), Some(target_100ns)) =
                    (state.active_id.clone(), state.mini_player.take_pending_seek())
                {
                    state.smtc.send(SmtcCommand::Seek(id, target_100ns));
                }
            }
            LRESULT(0)
        }
        WM_APP_MINI_PLAYER_WHEEL => {
            let wheel_delta = wparam.0 as isize as i16;
            if wheel_delta != 0 {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.apply_mini_player_wheel(wheel_delta);
                }
            }
            LRESULT(0)
        }
        WM_APP_SOURCE_SELECTED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if wparam.0 == usize::MAX {
                    state.arbitrator.set_auto();
                } else if let Some(id) = state.popup.take_selected_source_id() {
                    state.arbitrator.set_manual(id);
                }
                state.refresh_smtc_selection();
            }
            LRESULT(0)
        }
        WM_APP_SETTING_ACTION => {
            match wparam.0 {
                SETTING_TOGGLE_AUTO_COLOR => {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.config.auto_color = !state.config.auto_color;
                        state.save_config();
                        state.sync_shared_accent();
                        let auto_color = state.config.auto_color;
                        state.popup.set_auto_color(auto_color);
                        state.mini_player.set_auto_color(auto_color);
                        state.dirty = true;
                        state.render_now();
                    }
                }
                SETTING_TOGGLE_LOCK => {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.config.locked = !state.config.locked;
                        if !state.config.locked {
                            state.hover_zone = HitZone::None;
                            state.pressed_zone = HitZone::None;
                            if state.vol_hud_active {
                                state.clear_vol_hud();
                            }
                        }
                        state.save_config();
                        state.dirty = true;
                        state.render_now();
                    }
                }
                SETTING_RESET_POS => {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        if state.config.offset_x != 0 {
                            state.popup.hide();
                            state.mini_player.hide();
                        }
                        state.config.offset_x = 0;
                        state.save_config();
                        state.sync_taskbar_geometry(true);
                        notify_peer_taskbar_bar_resync(TaskbarBarSlot::MediaBar);
                    }
                }
                SETTING_CLOSE => {
                    if let Ok(state) = rc.try_borrow() {
                        let _ = PostMessageW(
                            state.host_hwnd,
                            state.close_request_msg,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_APP_AUDIO_OUTPUT_CHANGED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.master_vol_ctrl = None;
                if let Some(ref mut watcher) = state.volume_watcher {
                    let (_, latest) = watcher.rebind_default_endpoint();
                    if let Some((vol, muted)) = latest {
                        state.master_vol = vol;
                        state.master_muted = muted;
                    }
                } else {
                    state.refresh_master_volume_state();
                }
                state.audio.notify_device_changed();
                state.smtc.send(SmtcCommand::Refresh);
            }
            LRESULT(0)
        }
        WM_APP_VOLUME_UPDATED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.handle_external_volume_events(wparam.0);
            }
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                let light = is_light_taskbar();
                state.light_taskbar = light;
                state.refresh_backdrop_sample();
                let bg = state.backdrop_rgb;
                state.popup.set_system_theme(light, bg);
                state.mini_player.set_system_theme(light, bg);
                state.dirty = true;
                state.sync_taskbar_geometry(true);
                if msg == WM_DPICHANGED {
                    if let Ok((new_grab, new_owned)) =
                        state.renderer.create_grabbing_cursor(state.dpi)
                    {
                        if state.grab_cursor_owned && !state.grab_cursor.is_invalid() {
                            let _ = DestroyIcon(HICON(state.grab_cursor.0));
                        }
                        state.grab_cursor = new_grab;
                        state.grab_cursor_owned = new_owned;
                    }
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_GUARD_ID {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    let light = is_light_taskbar();
                    if light != state.light_taskbar {
                        state.light_taskbar = light;
                        state.refresh_backdrop_sample();
                        let bg = state.backdrop_rgb;
                        state.popup.set_system_theme(light, bg);
                        state.mini_player.set_system_theme(light, bg);
                        state.dirty = true;
                    }
                    state.sync_taskbar_geometry(false);
                    state.sync_hover_with_cursor();
                    state.update_anim_timer();
                }
            } else if wparam.0 == TIMER_ANIM_ID {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.tick_animation();
                }
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if state.vol_hud_dragging || state.is_mouse_down || state.is_dragging {
                    state.vol_hud_dragging = false;
                    state.is_mouse_down = false;
                    state.is_dragging = false;
                    state.pressed_zone = HitZone::None;
                    SetCursor(if state.config.locked {
                        state.arrow_cursor
                    } else {
                        state.grab_cursor
                    });
                    state.dirty = true;
                    state.render_now();
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Ok(mut state) = rc.try_borrow_mut() {
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

                if state.vol_hud_dragging {
                    state.set_hud_volume_by_cursor_x(x);
                    state.render_now();
                    return LRESULT(0);
                }

                if state.is_mouse_down {
                    let can_drag = !state.config.locked;
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let dx = pt.x - state.drag_start_screen_x;
                    if can_drag && (state.is_dragging || dx.abs() >= 4) {
                        state.is_dragging = true;
                        SetCursor(state.grab_cursor);
                        let candidate = state.drag_start_offset_x + dx;
                        state.sync_taskbar_geometry_with_mode(
                            true,
                            Some(candidate),
                            BarCollisionMode::Drag,
                        );
                        return LRESULT(0);
                    }
                }

                let layout = state.current_layout();
                let next_zone = if state.config.locked {
                    hit_test_mode(
                        &layout,
                        x,
                        y,
                        state.hover_t > 0.35,
                        state.vol_hud_active,
                    )
                } else {
                    HitZone::None
                };
                let mut changed = false;
                if (state.target_hover_t - 1.0).abs() > 0.01 {
                    state.target_hover_t = 1.0;
                    changed = true;
                }
                if state.hover_zone != next_zone {
                    state.hover_zone = next_zone;
                    changed = true;
                }
                if changed {
                    state.dirty = true;
                    state.render_now();
                    state.update_anim_timer();
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.tracking_leave = false;
                if !state.is_dragging && !state.vol_hud_dragging {
                    if state.vol_hud_active {
                        state.clear_vol_hud();
                    }
                    state.hover_zone = HitZone::None;
                    state.pressed_zone = HitZone::None;
                    state.target_hover_t = if state.any_popup_visible() { 1.0 } else { 0.0 };
                    state.dirty = true;
                    state.render_now();
                    state.update_anim_timer();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if !state.config.locked {
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let current_offset = if state.win_w > 0 {
                        query_taskbar_state()
                            .or(state.taskbar)
                            .map(|tb| state.win_x - default_bar_x(&tb))
                            .unwrap_or(state.config.offset_x)
                    } else {
                        state.config.offset_x
                    };
                    state.is_mouse_down = true;
                    state.is_dragging = false;
                    state.drag_start_screen_x = pt.x;
                    state.drag_start_offset_x = current_offset;
                    state.pressed_zone = HitZone::None;
                    let _ = SetCapture(hwnd);
                    SetCursor(state.grab_cursor);
                    return LRESULT(0);
                }

                let x = ((lparam.0 & 0xFFFF) as i16) as f32;
                let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
                let layout = state.current_layout();
                let zone = hit_test_mode(
                    &layout,
                    x,
                    y,
                    state.hover_t > 0.35,
                    state.vol_hud_active,
                );

                if state.vol_hud_active && zone == HitZone::VolumeSlider {
                    state.is_mouse_down = false;
                    state.is_dragging = false;
                    state.vol_hud_dragging = true;
                    state.pressed_zone = HitZone::VolumeSlider;
                    let _ = SetCapture(hwnd);
                    state.set_hud_volume_by_cursor_x(x);
                    state.render_now();
                    return LRESULT(0);
                }

                if state.vol_hud_active && zone == HitZone::VolumeMuteButton {
                    state.is_mouse_down = false;
                    state.is_dragging = false;
                    state.pressed_zone = HitZone::VolumeMuteButton;
                    let _ = SetCapture(hwnd);
                    state.dirty = true;
                    state.render_now();
                    return LRESULT(0);
                }

                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                state.is_mouse_down = true;
                state.is_dragging = false;
                state.drag_start_screen_x = pt.x;
                state.drag_start_offset_x = state.config.offset_x;
                state.pressed_zone = zone;
                let _ = SetCapture(hwnd);
                state.dirty = true;
                state.render_now();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let mut open_popup = false;
            let mut hud_drag_done = false;
            if let Ok(mut state) = rc.try_borrow_mut() {
                if state.vol_hud_dragging {
                    state.vol_hud_dragging = false;
                    state.pressed_zone = HitZone::None;
                    state.set_hud_volume_by_cursor_x(x);
                    let layout = state.current_layout();
                    state.hover_zone = hit_test_mode(
                        &layout,
                        x,
                        y,
                        state.hover_t > 0.35,
                        state.vol_hud_active,
                    );
                    state.render_now();
                    hud_drag_done = true;
                } else {
                    let was_dragging = state.is_dragging;
                    let pressed = state.pressed_zone;
                    state.is_mouse_down = false;
                    state.is_dragging = false;
                    state.pressed_zone = HitZone::None;

                    if was_dragging {
                        SetCursor(if state.config.locked {
                            state.arrow_cursor
                        } else {
                            state.grab_cursor
                        });
                        state.save_config();
                        notify_peer_taskbar_bar_resync(TaskbarBarSlot::MediaBar);
                        state.dirty = true;
                        state.render_now();
                    } else if state.config.locked {
                        let layout = state.current_layout();
                        let zone = hit_test_mode(
                            &layout,
                            x,
                            y,
                            state.hover_t > 0.35,
                            state.vol_hud_active,
                        );

                        if zone == pressed {
                            match zone {
                                HitZone::PrevButton => {
                                    if let Some(id) = state.active_id.clone() {
                                        state.smtc.send(SmtcCommand::Previous(id));
                                    }
                                }
                                HitZone::PlayPauseButton => {
                                    if let Some(id) = state.active_id.clone() {
                                        state.smtc.send(SmtcCommand::TogglePlayPause(id));
                                    }
                                }
                                HitZone::NextButton => {
                                    if let Some(id) = state.active_id.clone() {
                                        state.smtc.send(SmtcCommand::Next(id));
                                    }
                                }
                                HitZone::VolumeMuteButton => {
                                    state.toggle_hud_mute();
                                }
                                HitZone::MediaInfo => {
                                    open_popup = true;
                                }
                                HitZone::VolumeSlider | HitZone::None => {}
                            }
                        }
                        state.dirty = true;
                        state.render_now();
                    }
                }
            }
            // Release capture after clearing state flags so synchronous WM_CAPTURECHANGED does not clobber pressed_zone.
            let _ = ReleaseCapture();
            if hud_drag_done {
                return LRESULT(0);
            }
            if open_popup {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.show_mini_player();
                }
            }
            LRESULT(0)
        }
        WM_MBUTTONUP => {
            let target = rc.try_borrow().ok().and_then(|state| {
                state
                    .active_source()
                    .map(|s| (s.id.clone(), s.pid))
            });
            if let Some((id, pid)) = target {
                let raw_id = id.split('#').next().unwrap_or(&id);
                activate_player_window(raw_id, pid);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.show_source_popup(pt.x);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
