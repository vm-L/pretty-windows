mod layout;
mod render;
mod submenus;
mod wndproc;

#[cfg(test)]
mod tests;

use wndproc::{begin_close_source_popup, popup_wnd_proc};

use super::arbitration::SelectionMode;
use super::audio_devices::{enumerate_audio_endpoints, AudioEndpointItem};
use super::layout::RectF;
use super::smtc::{MediaCover, SourceItem};
use super::volume::{AppVolumeEntry, MasterVolumeControl, SessionWatchGuard, VolumeSharedState};
use crate::color::{enforce_ui_accent_deadzone, Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::system::{active_taskbar_surface_style, visible_transient_overlay_rects};
use crate::ui::glass::{capture_blurred_desktop_backdrop_excluding, create_dc_render_target, create_round_cap_stroke, LayeredSurface, PopupNoiseCache};
use crate::ui::{compute_halo_pair, popup_glass_recipe, PopupAnimPhase, Spring1D, POPUP_FLOAT_GAP_DIP, POPUP_REOPEN_DEBOUNCE_MS, ROW_H_DIP, SHADOW_PAD_DIP, SILK_H, SILK_W};
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::size_of;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory1, ID2D1Layer, ID2D1StrokeStyle};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, IsWindowVisible, KillTimer, LoadCursorW, RegisterClassExW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, HCURSOR, HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOW, WM_APP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP};

pub const WM_APP_SOURCE_SELECTED: u32 = WM_APP + 2;
pub const WM_APP_POPUP_CLOSED: u32 = WM_APP + 3;
pub const WM_APP_SETTING_ACTION: u32 = WM_APP + 4;
pub const WM_APP_AUDIO_OUTPUT_CHANGED: u32 = WM_APP + 5;

pub const SETTING_TOGGLE_LOCK: usize = 1;
pub const SETTING_RESET_POS: usize = 2;
pub const SETTING_CLOSE: usize = 3;
pub const SETTING_TOGGLE_AUTO_COLOR: usize = 4;

const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const POPUP_CLASS_NAME: windows::core::PCWSTR = w!("MediaBarSourcePopupWnd");

const TIMER_POPUP_FLOW_ID: usize = 1;
const SUB_WIDTH_STIFFNESS: f32 = 920.0;
const SUB_WIDTH_DAMPING: f32 = 46.0;
const SUB_VERT_STIFFNESS: f32 = 860.0;
const SUB_VERT_DAMPING: f32 = 44.5;

const PRIMARY_WIDTH_DIP: f32 = 148.0;
const PRIMARY_ROW_H_DIP: f32 = ROW_H_DIP;
const SUBMENU_GAP_DIP: f32 = 0.0;

const SOURCES_SUB_WIDTH_DIP: f32 = 280.0;
const VOLUME_SUB_WIDTH_DIP: f32 = 276.0;
const DEVICES_SUB_WIDTH_DIP: f32 = 264.0;
const SETTINGS_SUB_WIDTH_DIP: f32 = 172.0;

const AUTO_ROW_H_DIP: f32 = ROW_H_DIP;
const SOURCE_ROW_H_DIP: f32 = 46.0;
const VOLUME_ROW_H_DIP: f32 = 44.0;
const DEVICE_ROW_H_DIP: f32 = 42.0;
const SETTING_ROW_H_DIP: f32 = ROW_H_DIP;

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
    SubSettingAutoColor,
    SubSettingLock,
    SubSettingResetPos,
    SubSettingClose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SliderTarget {
    Master,
    App(usize),
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

struct PopupInner {
    hwnd: HWND,
    main_hwnd: HWND,
    d2d_factory: ID2D1Factory1,
    dwrite_factory: IDWriteFactory,
    // Declare RT-owned resources and `rt` before `surface` so `rt` releases before `DeleteDC`.
    backdrop_bmp: Option<ID2D1Bitmap>,
    noise_cache: PopupNoiseCache,
    silk_bmp: Option<ID2D1Bitmap>,
    cover_bmps: HashMap<u64, ID2D1Bitmap>,
    layer: ID2D1Layer,
    content_layer: ID2D1Layer,
    thumb_layer: ID2D1Layer,
    rt: ID2D1DCRenderTarget,
    surface: LayeredSurface,
    round_stroke: Option<ID2D1StrokeStyle>,
    dpi: u32,
    backdrop_rgb: Rgb,
    sources: Vec<SourceItem>,
    mode: SelectionMode,
    active_id: Option<String>,
    pending_selected_source_id: Option<String>,
    master_vol_ctrl: Option<MasterVolumeControl>,
    master_vol: f32,
    master_muted: bool,
    app_volumes: Vec<AppVolumeEntry>,
    session_watch: Option<SessionWatchGuard>,
    volume_shared: Option<Arc<VolumeSharedState>>,
    pending_session_reload: bool,
    dragging_slider: Option<SliderTarget>,
    playback_devices: Vec<AudioEndpointItem>,
    recording_devices: Vec<AudioEndpointItem>,
    locked: bool,
    auto_color: bool,
    active_category: Option<PrimaryCategory>,
    hover_target: HoverTarget,
    open_left: bool,
    tracking_leave: bool,
    light_glass: bool,
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
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
    sub_anim: Option<SubSpringState>,
    timer_interval_ms: u32,
    cached_text_formats: Option<CachedTextFormats>,
    arrow_cursor: HCURSOR,
}

pub struct SourcePopup {
    hwnd: HWND,
    inner: Rc<RefCell<PopupInner>>,
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
            let mut hwnd_guard = InitHwndGuard(Some(hwnd));

            let rt = create_dc_render_target(&d2d_factory)?;
            let layer = rt.CreateLayer(None)?;
            let content_layer = rt.CreateLayer(None)?;
            let thumb_layer = rt.CreateLayer(None)?;
            let surface = LayeredSurface::new()?;
            let round_stroke = create_round_cap_stroke(&d2d_factory);

            let inner = Rc::new(RefCell::new(PopupInner {
                hwnd,
                main_hwnd,
                d2d_factory,
                dwrite_factory,
                backdrop_bmp: None,
                noise_cache: PopupNoiseCache::default(),
                silk_bmp: None,
                cover_bmps: HashMap::new(),
                layer,
                content_layer,
                thumb_layer,
                rt,
                surface,
                round_stroke,
                dpi: 96,
                backdrop_rgb: NEUTRAL_BACKDROP_FALLBACK,
                sources: Vec::new(),
                mode: SelectionMode::Auto,
                active_id: None,
                pending_selected_source_id: None,
                master_vol_ctrl: None,
                master_vol: 1.0,
                master_muted: false,
                app_volumes: Vec::new(),
                session_watch: None,
                volume_shared: None,
                pending_session_reload: false,
                dragging_slider: None,
                playback_devices: Vec::new(),
                recording_devices: Vec::new(),
                locked: true,
                auto_color: true,
                active_category: None,
                hover_target: HoverTarget::None,
                open_left: false,
                tracking_leave: false,
                light_glass: false,
                win_x: 0,
                win_y: 0,
                win_w: 1,
                win_h: 1,
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
                sub_anim: None,
                timer_interval_ms: 0,
                cached_text_formats: None,
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
            begin_close_source_popup(self.hwnd, &self.inner);
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

    pub fn take_selected_source_id(&mut self) -> Option<String> {
        self.inner
            .try_borrow_mut()
            .ok()
            .and_then(|mut s| s.pending_selected_source_id.take())
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.reset_transient_state();
        }
    }

    pub fn update_live_sources(
        &mut self,
        sources: Vec<SourceItem>,
        mode: SelectionMode,
        active_id: Option<String>,
    ) {
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.sources = sources;
            s.mode = mode;
            s.active_id = active_id;
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

    pub fn set_system_theme(&mut self, light_taskbar: bool, backdrop_rgb: Rgb) {
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            let changed = s.light_glass != light_taskbar || s.backdrop_rgb != backdrop_rgb;
            s.light_glass = light_taskbar;
            s.backdrop_rgb = backdrop_rgb;
            if changed {
                s.halo_colors_ready = false;
                s.last_silk_tick = None;
                self.is_visible()
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
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            if s.auto_color != auto_color {
                s.auto_color = auto_color;
                s.halo_colors_ready = false;
                s.last_silk_tick = None;
                self.is_visible()
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

    pub fn set_volume_shared(&mut self, shared: Arc<VolumeSharedState>) {
        if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.volume_shared = Some(shared);
        }
    }

    pub fn sync_external_master_volume(&mut self, vol: f32, muted: bool) {
        if !self.is_visible() {
            return;
        }
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            if s.anim_phase == PopupAnimPhase::Closing
                || s.dragging_slider == Some(SliderTarget::Master)
            {
                false
            } else {
                let clamped = vol.clamp(0.0, 1.0);
                let changed = (s.master_vol - clamped).abs() > 0.001 || s.master_muted != muted;
                if changed {
                    s.master_vol = clamped;
                    s.master_muted = muted;
                    s.active_category == Some(PrimaryCategory::VolumeControl)
                } else {
                    false
                }
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

    pub fn sync_external_sessions(&mut self) {
        if !self.is_visible() {
            return;
        }
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            if s.anim_phase == PopupAnimPhase::Closing {
                false
            } else if s.dragging_slider.is_some() {
                s.pending_session_reload = true;
                false
            } else {
                let prev_count = s.app_volumes.len();
                s.reload_app_sessions_only();
                s.active_category == Some(PrimaryCategory::VolumeControl)
                    || prev_count != s.app_volumes.len()
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

    pub fn sync_external_devices(&mut self, vol: f32, muted: bool) {
        if !self.is_visible() {
            return;
        }
        let should_render = if let Ok(mut s) = self.inner.try_borrow_mut() {
            if s.anim_phase == PopupAnimPhase::Closing {
                false
            } else {
                s.playback_devices = enumerate_audio_endpoints(false);
                s.recording_devices = enumerate_audio_endpoints(true);
                if s.dragging_slider.is_none() {
                    s.reload_volumes();
                } else {
                    if s.dragging_slider != Some(SliderTarget::Master) {
                        s.master_vol = vol.clamp(0.0, 1.0);
                        s.master_muted = muted;
                    }
                    s.pending_session_reload = true;
                }
                true
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
        anchor_rect: RECT,
        click_screen_x: i32,
        dpi: u32,
        light_taskbar: bool,
        fallback_backdrop_rgb: Rgb,
        sources: Vec<SourceItem>,
        mode: SelectionMode,
        active_id: Option<String>,
        locked: bool,
        auto_color: bool,
    ) {
        let Some((win_x, win_y, win_w, win_h)) = (|| {
            let mut s = self.inner.try_borrow_mut().ok()?;
            s.dpi = dpi.max(96);
            s.light_glass = light_taskbar;
            s.backdrop_rgb = fallback_backdrop_rgb;
            s.sources = sources;
            s.mode = mode;
            s.active_id = active_id;
            s.pending_selected_source_id = None;
            s.locked = locked;
            s.auto_color = auto_color;
            s.playback_devices = enumerate_audio_endpoints(false);
            s.recording_devices = enumerate_audio_endpoints(true);
            s.reload_volumes();
            s.dragging_slider = None;
            s.active_category = None;
            s.hover_target = HoverTarget::None;
            // Reset cached height before sizing so stale height from a previous open never clips items.
            s.win_h = 0;

            let sc = s.scale();
            let side_pad = (SHADOW_PAD_DIP * sc).round() as i32;
            let float_gap = (POPUP_FLOAT_GAP_DIP * sc).round() as i32;
            let primary_w = (PRIMARY_WIDTH_DIP * sc).round() as i32;
            let sub_gap = (SUBMENU_GAP_DIP * sc).round() as i32;
            let max_sub_w = (SOURCES_SUB_WIDTH_DIP.max(VOLUME_SUB_WIDTH_DIP) * sc).round() as i32;

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
            let win_w = side_pad * 2 + primary_w + sub_gap + max_sub_w;
            let win_h = (side_pad * 2 + max_reach)
                .min((desired_card_bottom + side_pad - work.top - top_margin).max(180));

            // Center/align primary card cleanly above the MediaBar pill within safe screen bounds.
            let pill_inset = (8.0 * sc).round() as i32;
            let bar_min = anchor_rect.left + pill_inset;
            let bar_max = (anchor_rect.right - pill_inset - primary_w).max(bar_min);
            let work_min = work.left + edge_margin;
            let work_max = (work.right - primary_w - edge_margin).max(work_min);
            let min_x = bar_min.max(work_min).min(work_max);
            let max_x = bar_max.min(work_max).max(min_x);
            let ideal_primary_left =
                (click_screen_x - (24.0 * sc).round() as i32).clamp(min_x, max_x);

            let open_left =
                ideal_primary_left + primary_w + sub_gap + max_sub_w + edge_margin > work.right;
            s.open_left = open_left;

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
                popup_glass_recipe(active_taskbar_surface_style(), light_taskbar, 0.5);
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
            s.sub_anim = None;

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
        // Stamp clock after window presentation so first WM_TIMER does not absorb SetWindowPos/DWM latency.
        if let Ok(mut s) = self.inner.try_borrow_mut() {
            s.last_flow_tick = Instant::now();
            s.last_color_tick = s.last_flow_tick;
            s.sync_timer_rate();
        }
    }
}

impl Drop for SourcePopup {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
            let raw = SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0) as *const RefCell<PopupInner>;
            if !raw.is_null() {
                drop(Rc::from_raw(raw));
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}


/// Resolves the menu's dynamic UI accent color from the active source cover (matching spectrum bars),
/// falling back to any available source cover or the backdrop-derived neutral accent.
fn resolve_dynamic_menu_accent(
    sources: &[SourceItem],
    active_id: Option<&str>,
    fallback_ui: Rgb,
    light_theme: bool,
) -> Rgb {
    let cover = active_id
        .and_then(|aid| sources.iter().find(|src| src.id == aid))
        .and_then(|src| src.cover.as_deref())
        .or_else(|| sources.iter().find_map(|src| src.cover.as_deref()));
    let raw = cover
        .map(|cov| {
            if light_theme {
                cov.accent_light
            } else {
                cov.accent_dark
            }
        })
        .unwrap_or(fallback_ui);
    enforce_ui_accent_deadzone(raw, light_theme)
}

/// Computes two soft, restrained silk-tint colors derived from the active cover's purified accent color.
fn compute_cover_halo_pair(
    cover: Option<&MediaCover>,
    fallback_dark: Rgb,
    light_theme: bool,
) -> [Rgb; 2] {
    let base_rgb = cover.map_or(fallback_dark, |cov| cov.accent_dark);
    compute_halo_pair(base_rgb, light_theme)
}

