use crate::arbitration::{PlaybackState, SessionArbitrator, SessionSummary};
use crate::audio::{start_audio_worker, AudioHandle};
use crate::color::{adjust_accent_for_theme, Rgb};
use crate::config::{portable_config_path, Config};
use crate::dsp::NUM_BARS;
use crate::layout::{hit_test_mode, BarLayout, HitZone, BAR_WIDTH_DIP};
use crate::marquee::MarqueeState;
use crate::popup::{
    SourcePopup, SETTING_EXIT, SETTING_RESET_POS, SETTING_TOGGLE_LOCK, WM_APP_AUDIO_OUTPUT_CHANGED,
    WM_APP_POPUP_CLOSED, WM_APP_SETTING_ACTION, WM_APP_SOURCE_SELECTED,
};
use crate::process_util::activate_player_window;
use crate::render::{BarRenderer, RenderFrame};
use crate::smtc::{start_smtc_worker, SmtcCommand, SmtcHandle, SmtcSnapshot, WM_APP_SMTC_UPDATED};
use crate::taskbar::{
    compute_bar_rect, ensure_above_taskbar, is_autostart_enabled, is_fullscreen_active,
    is_light_taskbar, query_taskbar_state, system_accent_color, TaskbarState,
};
use crate::volume::{slider_ratio_from_x, step_volume_by_wheel, MasterVolumeControl};
use std::cell::RefCell;
use std::mem::size_of;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    VK_CONTROL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, KillTimer, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassExW, RegisterWindowMessageW, SetCursor, SetTimer, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TranslateMessage, GWLP_USERDATA, HCURSOR, HICON, HTCLIENT,
    HWND_TOPMOST, IDC_ARROW, MA_NOACTIVATE, MSG, SWP_NOACTIVATE, SW_HIDE, SW_SHOW,
    WM_CAPTURECHANGED, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP, WM_SETCURSOR,
    WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const MAIN_CLASS_NAME: PCWSTR = w!("MediaBarMainWnd");
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;

const TIMER_GUARD_ID: usize = 1;
const TIMER_ANIM_ID: usize = 2;
const GUARD_INTERVAL_MS: u32 = 150;
const ANIM_INTERVAL_MS: u32 = 16;
const VOL_HUD_TIMEOUT: Duration = Duration::from_millis(1500);

struct AppState {
    hwnd: HWND,
    taskbar_created_msg: u32,
    config_path: PathBuf,
    config: Config,
    light_taskbar: bool,
    fallback_accent: Rgb,
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

    hover_t: f32,
    target_hover_t: f32,
    hover_zone: HitZone,
    pressed_zone: HitZone,
    tracking_leave: bool,

    is_mouse_down: bool,
    is_dragging: bool,
    drag_start_screen_x: i32,
    drag_start_offset_x: i32,

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
    fn new(hwnd: HWND, taskbar_created_msg: u32) -> windows::core::Result<Self> {
        let config_path = portable_config_path();
        let config = Config::load_from_path(&config_path);
        let light_taskbar = is_light_taskbar();
        let fallback_accent = system_accent_color();

        let renderer = BarRenderer::new()?;
        let popup = SourcePopup::new(
            hwnd,
            renderer.d2d_factory.clone(),
            renderer.dwrite_factory.clone(),
        )?;

        let smtc = start_smtc_worker(hwnd, fallback_accent);
        let audio = start_audio_worker();
        let taskbar = query_taskbar_state();
        let init_dpi = taskbar.map(|t| t.dpi).unwrap_or(96);
        let arrow_cursor = unsafe { LoadCursorW(None, IDC_ARROW)? };
        let (grab_cursor, grab_cursor_owned) = renderer.create_grabbing_cursor(init_dpi)?;

        let mut app = Self {
            hwnd,
            taskbar_created_msg,
            config_path,
            config,
            light_taskbar,
            fallback_accent,
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
            hover_t: 0.0,
            target_hover_t: 0.0,
            hover_zone: HitZone::None,
            pressed_zone: HitZone::None,
            tracking_leave: false,
            is_mouse_down: false,
            is_dragging: false,
            drag_start_screen_x: 0,
            drag_start_offset_x: 0,
            master_vol_ctrl: None,
            master_vol: 1.0,
            master_muted: false,
            vol_hud_active: false,
            vol_hud_dragging: false,
            vol_hud_deadline: None,
            last_bars: [0.0; NUM_BARS],
            title_marquee: MarqueeState::default(),
            artist_marquee: MarqueeState::default(),
            anim_timer_running: false,
            last_anim_tick: Instant::now(),
            dirty: true,
            arrow_cursor,
            grab_cursor,
            grab_cursor_owned,
        };

        app.sync_taskbar_geometry(true);
        Ok(app)
    }

    fn active_source(&self) -> Option<&crate::smtc::SourceItem> {
        self.active_id
            .as_ref()
            .and_then(|id| self.snapshot.sources.iter().find(|s| &s.id == id))
    }

    fn is_playing(&self) -> bool {
        self.active_source()
            .is_some_and(|s| s.status == PlaybackState::Playing)
    }

    fn sync_taskbar_geometry(&mut self, force_render: bool) {
        let tb_opt = query_taskbar_state();
        self.taskbar = tb_opt;

        let Some(tb) = tb_opt else {
            self.set_bar_visible(false);
            return;
        };

        let occluded = tb.auto_hide_collapsed || is_fullscreen_active(tb.taskbar_hwnd);
        if occluded {
            self.hidden_by_guard = true;
            if self.popup.is_visible() {
                self.popup.hide();
            }
            self.clear_vol_hud();
            self.set_bar_visible(false);
            return;
        }
        self.hidden_by_guard = false;

        let scale = (tb.dpi as f32) / 96.0;
        let width_px = (BAR_WIDTH_DIP * scale).round() as i32;
        let (x, y, w, h, clamped_offset) = compute_bar_rect(&tb, width_px, self.config.offset_x);
        if clamped_offset != self.config.offset_x {
            self.config.offset_x = clamped_offset;
        }

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
            self.dirty = true;
        } else {
            ensure_above_taskbar(self.hwnd, tb.taskbar_hwnd);
        }

        self.set_bar_visible(true);
        if force_render || self.dirty {
            self.render_now();
        }
        self.popup.ensure_topmost();
    }

    fn set_bar_visible(&mut self, show: bool) {
        if self.visible == show {
            return;
        }
        self.visible = show;
        unsafe {
            let _ = ShowWindow(self.hwnd, if show { SW_SHOW } else { SW_HIDE });
        }
        if show {
            self.dirty = true;
            self.render_now();
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
        if self.popup.is_visible() {
            self.popup.update_live_sources(
                self.snapshot.sources.clone(),
                self.arbitrator.mode().clone(),
                self.active_id.clone(),
            );
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
            popup_open_span_x: None,
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
        Option<Arc<crate::smtc::MediaCover>>,
        Rgb,
    ) {
        if let Some(src) = self.active_source() {
            let accent = src
                .cover
                .as_ref()
                .map(|c| {
                    if self.light_taskbar {
                        c.accent_light
                    } else {
                        c.accent_dark
                    }
                })
                .unwrap_or_else(|| adjust_accent_for_theme(self.fallback_accent, self.light_taskbar));
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
                adjust_accent_for_theme(self.fallback_accent, self.light_taskbar),
            )
        }
    }

    fn render_now(&mut self) {
        if !self.visible || self.hidden_by_guard {
            return;
        }
        let popup_open_span_x = self
            .popup
            .attached_bar_span_screen_x()
            .map(|(l, r)| (l - self.win_x as f32, r - self.win_x as f32));
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
            popup_open_span_x,
            bar_levels: &self.last_bars,
            title_marquee: &mut self.title_marquee,
            artist_marquee: &mut self.artist_marquee,
        };
        let _ = self.renderer.render(&mut frame);
        self.dirty = false;
    }

    fn needs_anim_tick(&self) -> bool {
        if !self.visible || self.hidden_by_guard {
            return false;
        }
        let hover_animating = (self.hover_t - self.target_hover_t).abs() > 0.004;
        let is_playing = self.is_playing();
        let audio_animating = is_playing || self.audio.is_decaying_or_active();
        let marquee_animating =
            self.title_marquee.is_active(is_playing) || self.artist_marquee.is_active(is_playing);
        hover_animating || audio_animating || marquee_animating
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
            need_draw = true;
        }

        // 3. Marquee scrolling
        let is_playing = self.is_playing();
        if self.title_marquee.tick(dt, is_playing) {
            need_draw = true;
        }
        if self.artist_marquee.tick(dt, is_playing) {
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
        let unmute = self.master_muted && ratio > 0.0;
        self.master_vol = ratio;
        if unmute {
            self.master_muted = false;
        }
        if let Some(ref ctrl) = self.master_vol_ctrl {
            let _ = ctrl.set_volume(ratio);
            if unmute {
                let _ = ctrl.set_mute(false);
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
        self.master_muted = next_muted;
        if let Some(ref ctrl) = self.master_vol_ctrl {
            let _ = ctrl.set_mute(next_muted);
        }
        self.vol_hud_deadline = Some(Instant::now() + VOL_HUD_TIMEOUT);
        self.dirty = true;
    }

    fn sync_hover_with_cursor(&mut self) {
        if !self.visible || self.hidden_by_guard || self.popup.is_visible() {
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
            let next_zone = hit_test_mode(
                &layout,
                local_x,
                local_y,
                self.hover_t > 0.35,
                self.vol_hud_active,
            );
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

    fn show_source_popup(&mut self, click_screen_x: i32) {
        if self.vol_hud_active {
            self.clear_vol_hud();
            self.render_now();
        }
        if self.popup.is_visible() {
            self.popup.hide();
            self.dirty = true;
            self.render_now();
            return;
        }
        if let Some(closed_at) = self.popup_closed_at.take() {
            if closed_at.elapsed() < Duration::from_millis(240) {
                return;
            }
        }
        self.hover_t = 1.0;
        self.target_hover_t = 1.0;
        let layout = self.current_layout();
        let pill_overlap_px = (layout.bg_rect.top + 0.5 * layout.scale).round() as i32;
        let anchor = RECT {
            left: self.win_x + layout.bg_rect.left.round() as i32,
            top: self.win_y + pill_overlap_px,
            right: self.win_x + layout.bg_rect.right.round() as i32,
            bottom: self.win_y + self.win_h,
        };
        self.popup.show(
            anchor,
            click_screen_x,
            self.dpi,
            self.light_taskbar,
            self.fallback_accent,
            self.snapshot.sources.clone(),
            self.arbitrator.mode().clone(),
            self.active_id.clone(),
            self.config.locked,
            is_autostart_enabled(),
            pill_overlap_px,
        );
        self.dirty = true;
        self.render_now();
        self.popup.ensure_topmost();
    }

    fn save_config(&self) {
        let _ = self.config.save_to_path(&self.config_path);
    }
}

pub fn run() -> windows::core::Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarCreated"));

        let arrow_cursor = LoadCursorW(None, IDC_ARROW)?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(main_wnd_proc),
            hInstance: hinst.into(),
            hCursor: arrow_cursor,
            lpszClassName: MAIN_CLASS_NAME,
            ..Default::default()
        };
        RegisterClassExW(&wc);

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

        let app = Rc::new(RefCell::new(AppState::new(hwnd, taskbar_created_msg)?));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::into_raw(app) as isize);

        SetTimer(hwnd, TIMER_GUARD_ID, GUARD_INTERVAL_MS, None);
        let _ = PostMessageW(hwnd, WM_APP_SMTC_UPDATED, WPARAM(0), LPARAM(0));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
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
        let rc = Rc::from_raw(raw_ptr);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let _ = KillTimer(hwnd, TIMER_GUARD_ID);
        let _ = KillTimer(hwnd, TIMER_ANIM_ID);
        if let Ok(mut state) = rc.try_borrow_mut() {
            state.popup.hide();
            state.audio.shutdown();
            state.smtc.send(SmtcCommand::Shutdown);
        }
        PostQuitMessage(0);
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
        if let Ok(mut state) = rc.try_borrow_mut() {
            state.sync_taskbar_geometry(true);
        }
        return LRESULT(0);
    }

    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                let cursor = rc
                    .try_borrow()
                    .map(|s| if s.is_dragging { s.grab_cursor } else { s.arrow_cursor })
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
                    if state.visible && !state.hidden_by_guard && !state.popup.is_visible() {
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
        WM_APP_SOURCE_SELECTED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if wparam.0 == usize::MAX {
                    state.arbitrator.set_auto();
                } else if let Some(id) = state.popup.source_id_at(wparam.0) {
                    state.arbitrator.set_manual(id);
                }
                state.refresh_smtc_selection();
            }
            LRESULT(0)
        }
        WM_APP_SETTING_ACTION => {
            match wparam.0 {
                SETTING_TOGGLE_LOCK => {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.config.locked = !state.config.locked;
                        state.save_config();
                        state.dirty = true;
                        state.render_now();
                    }
                }
                SETTING_RESET_POS => {
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        if state.config.offset_x != 0 {
                            state.popup.hide();
                        }
                        state.config.offset_x = 0;
                        state.save_config();
                        state.sync_taskbar_geometry(true);
                    }
                }
                SETTING_EXIT => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_APP_AUDIO_OUTPUT_CHANGED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.master_vol_ctrl = None;
                state.audio.notify_device_changed();
                state.smtc.send(SmtcCommand::Refresh);
            }
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                let light = is_light_taskbar();
                let accent = system_accent_color();
                state.light_taskbar = light;
                state.fallback_accent = accent;
                state.popup.set_system_theme(light, accent);
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
                        let accent = state.fallback_accent;
                        state.light_taskbar = light;
                        state.popup.set_system_theme(light, accent);
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
                    SetCursor(state.arrow_cursor);
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
                    let ctrl_held = (GetKeyState(i32::from(VK_CONTROL.0)) as u16 & 0x8000) != 0;
                    let can_drag = !state.config.locked || ctrl_held;
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let dx = pt.x - state.drag_start_screen_x;
                    if can_drag && (state.is_dragging || dx.abs() >= 4) {
                        state.is_dragging = true;
                        SetCursor(state.grab_cursor);
                        let candidate = state.drag_start_offset_x + dx;
                        state.config.offset_x = candidate;
                        state.sync_taskbar_geometry(true);
                        return LRESULT(0);
                    }
                }

                let layout = state.current_layout();
                let next_zone = hit_test_mode(
                    &layout,
                    x,
                    y,
                    state.hover_t > 0.35,
                    state.vol_hud_active,
                );
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
                    state.target_hover_t = if state.popup.is_visible() { 1.0 } else { 0.0 };
                    state.dirty = true;
                    state.render_now();
                    state.update_anim_timer();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Ok(mut state) = rc.try_borrow_mut() {
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
                        SetCursor(state.arrow_cursor);
                        state.save_config();
                        state.dirty = true;
                        state.render_now();
                    } else {
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
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.show_source_popup(pt.x);
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
