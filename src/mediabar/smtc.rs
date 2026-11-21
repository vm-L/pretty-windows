use super::arbitration::PlaybackState;
use super::process_util::{friendly_app_name, resolve_pid_for_source};
use super::timeline::{current_filetime_100ns, MediaTimeline};
use crate::color::{extract_accent_from_bgra, Rgb};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{IUnknown, Interface};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub const WM_APP_SMTC_UPDATED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

const THUMB_SIZE: u32 = 64;
const ART_SIZE: u32 = 192;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const COVER_RETRY_DELAY: Duration = Duration::from_millis(360);
const OVERRIDE_TTL: Duration = Duration::from_millis(650);

/// Decoded album artwork (compact taskbar thumbnail + crisp mini-player art) and theme-calibrated accent colors.
#[derive(Clone, Debug)]
pub struct MediaCover {
    pub hash: u64,
    pub thumb_w: u32,
    pub thumb_h: u32,
    pub thumb_pbgra: Vec<u8>,
    pub art_w: u32,
    pub art_h: u32,
    pub art_pbgra: Vec<u8>,
    pub accent_dark: Rgb,
    pub accent_light: Rgb,
}

/// Snapshot of one active SMTC session.
#[derive(Clone, Debug)]
pub struct SourceItem {
    pub id: String,
    pub app_name: String,
    pub title: String,
    pub artist: String,
    pub status: PlaybackState,
    pub pid: Option<u32>,
    pub cover: Option<Arc<MediaCover>>,
    pub timeline: Option<MediaTimeline>,
}

/// Immutable snapshot shared from the SMTC worker thread to the UI thread.
#[derive(Clone, Debug, Default)]
pub struct SmtcSnapshot {
    pub sources: Vec<SourceItem>,
    pub system_current_id: Option<String>,
}

/// Commands dispatched from UI interactions to the SMTC MTA thread.
#[derive(Clone, Debug)]
pub enum SmtcCommand {
    Refresh,
    TogglePlayPause(String),
    Next(String),
    Previous(String),
    Seek(String, i64),
    Shutdown,
}

pub struct SmtcHandle {
    snapshot: Arc<Mutex<SmtcSnapshot>>,
    cmd_tx: Sender<SmtcCommand>,
}

impl SmtcHandle {
    pub fn snapshot(&self) -> SmtcSnapshot {
        self.snapshot
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn send(&self, cmd: SmtcCommand) {
        let _ = self.cmd_tx.send(cmd);
    }
}

struct ComGuard;

impl ComGuard {
    fn new_mta() -> Self {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        Self
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct TrackedSession {
    identity_ptr: usize,
    session: GlobalSystemMediaTransportControlsSession,
    playback_token: EventRegistrationToken,
    props_token: EventRegistrationToken,
    timeline_token: EventRegistrationToken,
}

impl Drop for TrackedSession {
    fn drop(&mut self) {
        let _ = self.session.RemovePlaybackInfoChanged(self.playback_token);
        let _ = self.session.RemoveMediaPropertiesChanged(self.props_token);
        let _ = self.session.RemoveTimelinePropertiesChanged(self.timeline_token);
    }
}

/// Spawns the background MTA thread that monitors SMTC sessions and executes playback commands.
pub fn start_smtc_worker(ui_hwnd: HWND, fallback_accent: Rgb) -> SmtcHandle {
    let snapshot = Arc::new(Mutex::new(SmtcSnapshot::default()));
    let snapshot_clone = Arc::clone(&snapshot);
    let (cmd_tx, cmd_rx) = mpsc::channel::<SmtcCommand>();
    let worker_tx = cmd_tx.clone();
    let hwnd_raw = ui_hwnd.0 as isize;

    std::thread::Builder::new()
        .name("mediabar-smtc".into())
        .spawn(move || {
            run_smtc_loop(hwnd_raw, fallback_accent, snapshot_clone, worker_tx, cmd_rx);
        })
        .expect("failed to spawn SMTC worker thread");

    SmtcHandle { snapshot, cmd_tx }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn session_identity_ptr(sess: &GlobalSystemMediaTransportControlsSession) -> usize {
    sess.cast::<IUnknown>()
        .map(|unk| unk.as_raw() as usize)
        .unwrap_or_else(|_| sess.as_raw() as usize)
}

fn extract_playback_state(sess: &GlobalSystemMediaTransportControlsSession) -> PlaybackState {
    let Ok(pi) = sess.GetPlaybackInfo() else {
        return PlaybackState::Other;
    };
    let raw = pi
        .PlaybackStatus()
        .unwrap_or(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Closed);
    match raw {
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing => PlaybackState::Playing,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused => PlaybackState::Paused,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped => PlaybackState::Stopped,
        _ => {
            if let Ok(ctrl) = pi.Controls() {
                let can_pause = ctrl.IsPauseEnabled().unwrap_or(false);
                let can_play = ctrl.IsPlayEnabled().unwrap_or(false);
                if can_pause && !can_play {
                    return PlaybackState::Playing;
                }
                if can_play && !can_pause {
                    return PlaybackState::Paused;
                }
            }
            PlaybackState::Other
        }
    }
}

fn extract_timeline(sess: &GlobalSystemMediaTransportControlsSession) -> Option<MediaTimeline> {
    let tl = sess.GetTimelineProperties().ok()?;
    let start_100ns = tl.StartTime().ok()?.Duration;
    let end_100ns = tl.EndTime().ok()?.Duration;
    let position_100ns = tl.Position().ok()?.Duration;
    let raw_updated = tl
        .LastUpdatedTime()
        .ok()
        .map(|dt| dt.UniversalTime)
        .unwrap_or(0);
    let last_updated_100ns = if raw_updated > 0 {
        raw_updated
    } else {
        current_filetime_100ns()
    };
    let can_seek = sess
        .GetPlaybackInfo()
        .and_then(|pi| pi.Controls())
        .and_then(|c| c.IsPlaybackPositionEnabled())
        .unwrap_or(false);
    let parsed = MediaTimeline {
        start_100ns,
        end_100ns,
        position_100ns,
        last_updated_100ns,
        can_seek,
    };
    parsed.is_valid().then_some(parsed)
}

struct SessionCacheEntry {
    pid: Option<u32>,
    app_name: String,
    title: String,
    artist: String,
    last_status: PlaybackState,
    last_track_key: String,
    cover: Option<Arc<MediaCover>>,
    pending_cover_retry: bool,
}

fn snapshots_equal(a: &SmtcSnapshot, b: &SmtcSnapshot) -> bool {
    if a.system_current_id != b.system_current_id || a.sources.len() != b.sources.len() {
        return false;
    }
    a.sources.iter().zip(b.sources.iter()).all(|(x, y)| {
        x.id == y.id
            && x.app_name == y.app_name
            && x.title == y.title
            && x.artist == y.artist
            && x.status == y.status
            && x.pid == y.pid
            && x.cover.as_ref().map(|c| c.hash) == y.cover.as_ref().map(|c| c.hash)
            && x.timeline == y.timeline
    })
}

fn find_live_session(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
    tracked: &HashMap<String, TrackedSession>,
    target_id: &str,
) -> Option<GlobalSystemMediaTransportControlsSession> {
    if let Ok(vec_view) = manager.GetSessions() {
        let count = vec_view.Size().unwrap_or(0);
        let mut seen_ids: HashMap<String, usize> = HashMap::new();
        let mut fallback: Option<GlobalSystemMediaTransportControlsSession> = None;
        let raw_target = target_id.split('#').next().unwrap_or(target_id);
        for i in 0..count {
            let Ok(sess) = vec_view.GetAt(i) else {
                continue;
            };
            let raw_id = sess
                .SourceAppUserModelId()
                .map(|s| s.to_string_lossy())
                .unwrap_or_else(|_| format!("unknown-{i}"));
            let occ = seen_ids.entry(raw_id.clone()).or_insert(0);
            *occ += 1;
            let unique_id = if *occ == 1 {
                raw_id.clone()
            } else {
                format!("{raw_id}#{}", *occ)
            };
            if unique_id == target_id {
                return Some(sess);
            }
            if fallback.is_none() && raw_id == raw_target {
                fallback = Some(sess);
            }
        }
        if fallback.is_some() {
            return fallback;
        }
    }
    tracked.get(target_id).map(|t| t.session.clone())
}

fn run_smtc_loop(
    hwnd_raw: isize,
    fallback_accent: Rgb,
    snapshot: Arc<Mutex<SmtcSnapshot>>,
    self_tx: Sender<SmtcCommand>,
    cmd_rx: Receiver<SmtcCommand>,
) {
    let _com = ComGuard::new_mta();
    let wic: Option<IWICImagingFactory> =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok() };

    let Ok(op) = GlobalSystemMediaTransportControlsSessionManager::RequestAsync() else {
        return;
    };
    let Ok(manager) = op.get() else {
        return;
    };

    let tx_sessions = self_tx.clone();
    let sessions_token = manager
        .SessionsChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx_sessions.send(SmtcCommand::Refresh);
            Ok(())
        }))
        .ok();

    let tx_current = self_tx.clone();
    let current_token = manager
        .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx_current.send(SmtcCommand::Refresh);
            Ok(())
        }))
        .ok();

    let mut tracked: HashMap<String, TrackedSession> = HashMap::new();
    let mut cache: HashMap<String, SessionCacheEntry> = HashMap::new();
    let mut cover_by_hash: HashMap<u64, Arc<MediaCover>> = HashMap::new();
    let mut cover_retry_deadline: Option<Instant> = None;
    let mut pending_override: Option<(String, PlaybackState, Instant)> = None;
    let mut pending_seek_override: Option<(String, i64, i64, Instant)> = None;
    let mut last_snapshot: Option<SmtcSnapshot> = None;
    let mut poll_cycle: u32 = 0;

    // Initial load
    let _ = self_tx.send(SmtcCommand::Refresh);

    loop {
        let poll_deadline = Instant::now() + POLL_INTERVAL;
        let deadline = match cover_retry_deadline {
            Some(c_dl) if c_dl < poll_deadline => c_dl,
            _ => poll_deadline,
        };
        let wait = deadline.saturating_duration_since(Instant::now());

        let mut need_full_refresh = false;
        let mut need_light_refresh = false;
        let mut pending_cmds = Vec::new();

        match cmd_rx.recv_timeout(wait) {
            Ok(c) => {
                if matches!(c, SmtcCommand::Refresh) {
                    need_full_refresh = true;
                } else {
                    pending_cmds.push(c);
                }
                while let Ok(extra) = cmd_rx.try_recv() {
                    if matches!(extra, SmtcCommand::Refresh) {
                        need_full_refresh = true;
                    } else {
                        pending_cmds.push(extra);
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if cover_retry_deadline.is_some_and(|dl| Instant::now() >= dl) {
                    cover_retry_deadline = None;
                    need_full_refresh = true;
                } else {
                    poll_cycle = poll_cycle.wrapping_add(1);
                    need_light_refresh = true;
                    if poll_cycle.is_multiple_of(8) {
                        need_full_refresh = true;
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let mut shutdown = false;
        for c in pending_cmds {
            match c {
                SmtcCommand::Shutdown => {
                    shutdown = true;
                    break;
                }
                SmtcCommand::Refresh => need_full_refresh = true,
                SmtcCommand::TogglePlayPause(id) => {
                    if let Some(sess) = find_live_session(&manager, &tracked, &id) {
                        let cur_state = extract_playback_state(&sess);
                        let want_playing = cur_state != PlaybackState::Playing;
                        let ctrl = sess.GetPlaybackInfo().and_then(|pi| pi.Controls()).ok();
                        let op_res = if want_playing {
                            if ctrl
                                .as_ref()
                                .and_then(|c| c.IsPlayEnabled().ok())
                                .unwrap_or(false)
                            {
                                sess.TryPlayAsync()
                            } else {
                                sess.TryTogglePlayPauseAsync()
                            }
                        } else if ctrl
                            .as_ref()
                            .and_then(|c| c.IsPauseEnabled().ok())
                            .unwrap_or(false)
                        {
                            sess.TryPauseAsync()
                        } else {
                            sess.TryTogglePlayPauseAsync()
                        };
                        if let Ok(op) = op_res {
                            let _ = op.get();
                        }
                        let target_state = if want_playing {
                            PlaybackState::Playing
                        } else {
                            PlaybackState::Paused
                        };
                        pending_override = Some((id, target_state, Instant::now() + OVERRIDE_TTL));
                    }
                    need_full_refresh = true;
                }
                SmtcCommand::Next(id) => {
                    if let Some(sess) = find_live_session(&manager, &tracked, &id) {
                        if let Ok(op) = sess.TrySkipNextAsync() {
                            let _ = op.get();
                        }
                    }
                    need_full_refresh = true;
                }
                SmtcCommand::Previous(id) => {
                    if let Some(sess) = find_live_session(&manager, &tracked, &id) {
                        if let Ok(op) = sess.TrySkipPreviousAsync() {
                            let _ = op.get();
                        }
                    }
                    need_full_refresh = true;
                }
                SmtcCommand::Seek(id, target_100ns) => {
                    if let Some(sess) = find_live_session(&manager, &tracked, &id) {
                        if let Ok(op) = sess.TryChangePlaybackPositionAsync(target_100ns) {
                            let _ = op.get();
                        }
                        pending_seek_override = Some((
                            id,
                            target_100ns,
                            current_filetime_100ns(),
                            Instant::now() + OVERRIDE_TTL,
                        ));
                    }
                    need_full_refresh = true;
                }
            }
        }
        if shutdown {
            break;
        }

        if need_full_refresh || need_light_refresh {
            let (new_snapshot, needs_retry) = refresh_sessions(
                &manager,
                wic.as_ref(),
                fallback_accent,
                &self_tx,
                &mut tracked,
                &mut cache,
                &mut cover_by_hash,
                need_full_refresh,
                &mut pending_override,
                &mut pending_seek_override,
            );
            if needs_retry {
                cover_retry_deadline = Some(Instant::now() + COVER_RETRY_DELAY);
            }
            let changed = last_snapshot
                .as_ref()
                .is_none_or(|prev| !snapshots_equal(prev, &new_snapshot));
            if changed {
                if let Ok(mut guard) = snapshot.lock() {
                    *guard = new_snapshot.clone();
                }
                last_snapshot = Some(new_snapshot);
                unsafe {
                    let _ = PostMessageW(
                        HWND(hwnd_raw as *mut _),
                        WM_APP_SMTC_UPDATED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        }
    }

    drop(tracked);
    if let Some(tok) = sessions_token {
        let _ = manager.RemoveSessionsChanged(tok);
    }
    if let Some(tok) = current_token {
        let _ = manager.RemoveCurrentSessionChanged(tok);
    }
}

#[allow(clippy::too_many_arguments)]
fn refresh_sessions(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
    wic: Option<&IWICImagingFactory>,
    fallback_accent: Rgb,
    self_tx: &Sender<SmtcCommand>,
    tracked: &mut HashMap<String, TrackedSession>,
    cache: &mut HashMap<String, SessionCacheEntry>,
    cover_by_hash: &mut HashMap<u64, Arc<MediaCover>>,
    fetch_properties: bool,
    pending_override: &mut Option<(String, PlaybackState, Instant)>,
    pending_seek_override: &mut Option<(String, i64, i64, Instant)>,
) -> (SmtcSnapshot, bool) {
    let mut live_sessions: Vec<(String, GlobalSystemMediaTransportControlsSession)> = Vec::new();
    let mut seen_ids: HashMap<String, usize> = HashMap::new();

    if let Ok(vec_view) = manager.GetSessions() {
        let count = vec_view.Size().unwrap_or(0);
        for i in 0..count {
            let Ok(sess) = vec_view.GetAt(i) else {
                continue;
            };
            let raw_id = sess
                .SourceAppUserModelId()
                .map(|s| s.to_string_lossy())
                .unwrap_or_else(|_| format!("unknown-{i}"));
            let occ = seen_ids.entry(raw_id.clone()).or_insert(0);
            *occ += 1;
            let unique_id = if *occ == 1 {
                raw_id
            } else {
                format!("{raw_id}#{}", *occ)
            };
            live_sessions.push((unique_id, sess));
        }
    }

    // Remove stale tracked sessions & stale cache entries
    tracked.retain(|id, _| live_sessions.iter().any(|(lid, _)| lid == id));
    cache.retain(|id, _| live_sessions.iter().any(|(lid, _)| lid == id));

    if let Some((_, _, dl)) = pending_override.as_ref() {
        if Instant::now() >= *dl {
            *pending_override = None;
        }
    }
    if let Some((_, _, _, dl)) = pending_seek_override.as_ref() {
        if Instant::now() >= *dl {
            *pending_seek_override = None;
        }
    }

    let system_current_raw = manager
        .GetCurrentSession()
        .ok()
        .and_then(|s| s.SourceAppUserModelId().ok())
        .map(|s| s.to_string_lossy());

    let mut schedule_retry = false;
    let mut items = Vec::with_capacity(live_sessions.len());

    for (id, sess) in live_sessions {
        let identity_ptr = session_identity_ptr(&sess);
        let need_rebind = tracked
            .get(&id)
            .is_none_or(|t| t.identity_ptr != identity_ptr);
        if need_rebind {
            tracked.remove(&id);
            let tx_p = self_tx.clone();
            let playback_token = sess
                .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = tx_p.send(SmtcCommand::Refresh);
                    Ok(())
                }))
                .unwrap_or_default();

            let tx_m = self_tx.clone();
            let props_token = sess
                .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = tx_m.send(SmtcCommand::Refresh);
                    Ok(())
                }))
                .unwrap_or_default();

            let tx_t = self_tx.clone();
            let timeline_token = sess
                .TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = tx_t.send(SmtcCommand::Refresh);
                    Ok(())
                }))
                .unwrap_or_default();

            tracked.insert(
                id.clone(),
                TrackedSession {
                    identity_ptr,
                    session: sess.clone(),
                    playback_token,
                    props_token,
                    timeline_token,
                },
            );
        }

        let mut status = extract_playback_state(&sess);
        if let Some((ref ov_id, ov_status, _)) = pending_override {
            if ov_id == &id {
                if status == *ov_status {
                    *pending_override = None;
                } else {
                    status = *ov_status;
                }
            }
        }

        let is_new_entry = !cache.contains_key(&id);
        let raw_source_id = id.split('#').next().unwrap_or(&id);
        let entry = cache.entry(id.clone()).or_insert_with(|| {
            let pid = resolve_pid_for_source(raw_source_id);
            let app_name = friendly_app_name(raw_source_id, pid);
            SessionCacheEntry {
                pid,
                app_name,
                title: String::new(),
                artist: String::new(),
                last_status: status,
                last_track_key: String::new(),
                cover: None,
                pending_cover_retry: false,
            }
        });

        let status_changed = entry.last_status != status;
        entry.last_status = status;

        // Re-resolve PID if missing or when switching to Playing so audio isolation stays accurate.
        if entry.pid.is_none() || (status == PlaybackState::Playing && status_changed) {
            if let Some(pid) = resolve_pid_for_source(raw_source_id) {
                entry.pid = Some(pid);
                entry.app_name = friendly_app_name(raw_source_id, Some(pid));
            }
        }

        let fetch_this_session =
            fetch_properties || is_new_entry || status_changed || entry.pending_cover_retry;

        if fetch_this_session {
            let (title, artist, thumb_ref) =
                match sess.TryGetMediaPropertiesAsync().and_then(|op| op.get()) {
                    Ok(props) => {
                        let t = props
                            .Title()
                            .map(|s| s.to_string_lossy().trim().to_string())
                            .unwrap_or_default();
                        let a = props
                            .Artist()
                            .map(|s| s.to_string_lossy().trim().to_string())
                            .unwrap_or_default();
                        let tr = props.Thumbnail().ok();
                        (t, a, tr)
                    }
                    Err(_) => (entry.title.clone(), entry.artist.clone(), None),
                };

            entry.title = title;
            entry.artist = artist;

            let track_key = format!("{}\0{}", entry.title, entry.artist);
            let track_changed = entry.last_track_key != track_key;
            if track_changed {
                entry.last_track_key = track_key;
                entry.pending_cover_retry = true;
            }

            let should_decode_cover =
                track_changed || entry.pending_cover_retry || entry.cover.is_none();

            if should_decode_cover {
                if let Some(thumb) = thumb_ref {
                    if let Some(bytes) = read_thumbnail_bytes(&thumb) {
                        let hash = fnv1a_64(&bytes);
                        let prev_hash = entry.cover.as_ref().map(|c| c.hash);
                        if prev_hash != Some(hash) {
                            if let Some(cached) = cover_by_hash.get(&hash) {
                                entry.cover = Some(Arc::clone(cached));
                                entry.pending_cover_retry = false;
                            } else if let Some(factory) = wic {
                                if let Some(decoded) =
                                    decode_and_process_cover(factory, &bytes, hash, fallback_accent)
                                {
                                    let arc = Arc::new(decoded);
                                    if cover_by_hash.len() > 12 {
                                        cover_by_hash.clear();
                                    }
                                    cover_by_hash.insert(hash, Arc::clone(&arc));
                                    entry.cover = Some(arc);
                                    entry.pending_cover_retry = false;
                                }
                            }
                        } else if track_changed {
                            // Thumbnail bytes match the previous song -> player may update artwork shortly.
                            schedule_retry = true;
                        } else {
                            entry.pending_cover_retry = false;
                        }
                    } else if track_changed {
                        entry.cover = None;
                        schedule_retry = true;
                    } else {
                        entry.pending_cover_retry = false;
                    }
                } else if track_changed {
                    entry.cover = None;
                    schedule_retry = true;
                } else {
                    entry.pending_cover_retry = false;
                }
            }
        }

        let mut timeline = extract_timeline(&sess);
        if let Some((ref seek_id, target_pos, seek_ts, _)) = pending_seek_override {
            if seek_id == &id {
                if let Some(ref mut tl) = timeline {
                    if (tl.position_100ns - *target_pos).abs() <= 20_000_000
                        && tl.last_updated_100ns >= *seek_ts
                    {
                        *pending_seek_override = None;
                    } else {
                        tl.position_100ns = (*target_pos).clamp(tl.start_100ns, tl.end_100ns);
                        tl.last_updated_100ns = *seek_ts;
                    }
                }
            }
        }

        items.push(SourceItem {
            id: id.clone(),
            app_name: entry.app_name.clone(),
            title: if entry.title.is_empty() {
                entry.app_name.clone()
            } else {
                entry.title.clone()
            },
            artist: entry.artist.clone(),
            status,
            pid: entry.pid,
            cover: entry.cover.clone(),
            timeline,
        });
    }

    let system_current_id = system_current_raw.and_then(|raw| {
        items
            .iter()
            .find(|s| s.id == raw || s.id.split('#').next() == Some(raw.as_str()))
            .map(|s| s.id.clone())
    });

    (
        SmtcSnapshot {
            sources: items,
            system_current_id,
        },
        schedule_retry,
    )
}

fn read_thumbnail_bytes(
    thumb: &windows::Storage::Streams::IRandomAccessStreamReference,
) -> Option<Vec<u8>> {
    let stream = thumb.OpenReadAsync().ok()?.get().ok()?;
    let size_u64 = stream.Size().ok()?;
    if size_u64 == 0 || size_u64 > 16 * 1024 * 1024 {
        return None;
    }
    let size = size_u64 as u32;
    let input = stream.GetInputStreamAt(0).ok()?;
    let reader = DataReader::CreateDataReader(&input).ok()?;
    let loaded = reader.LoadAsync(size).ok()?.get().ok()?;
    if loaded == 0 {
        return None;
    }
    let mut buf = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut buf).ok()?;
    Some(buf)
}

fn center_crop_square_bgra(src: &[u8], w: u32, h: u32, target: u32) -> Vec<u8> {
    let t = target as usize;
    let sw = w as usize;
    let sh = h as usize;
    let start_x = sw.saturating_sub(t) / 2;
    let start_y = sh.saturating_sub(t) / 2;
    let mut out = vec![0u8; t * t * 4];

    for y in 0..t {
        let sy = (start_y + y).min(sh.saturating_sub(1));
        for x in 0..t {
            let sx = (start_x + x).min(sw.saturating_sub(1));
            let src_idx = (sy * sw + sx) * 4;
            let dst_idx = (y * t + x) * 4;
            out[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }
    out
}

fn straight_to_premul_bgra(src: &[u8]) -> Vec<u8> {
    let mut out = src.to_vec();
    for px in out.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        if a < 255 {
            px[0] = ((u16::from(px[0]) * a + 127) / 255) as u8;
            px[1] = ((u16::from(px[1]) * a + 127) / 255) as u8;
            px[2] = ((u16::from(px[2]) * a + 127) / 255) as u8;
        }
    }
    out
}

fn decode_wic_square_straight_bgra(
    wic: &IWICImagingFactory,
    bytes: &[u8],
    target_size: u32,
) -> Option<Vec<u8>> {
    unsafe {
        let stream = wic.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        let decoder = wic
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let mut orig_w = 0u32;
        let mut orig_h = 0u32;
        frame.GetSize(&mut orig_w, &mut orig_h).ok()?;
        if orig_w == 0 || orig_h == 0 {
            return None;
        }

        // Aspect-preserving scale so the shortest side equals `target_size`, then center-crop square.
        let min_side = orig_w.min(orig_h) as f32;
        let scale = (target_size as f32) / min_side;
        let scaled_w = ((orig_w as f32) * scale).round().max(target_size as f32) as u32;
        let scaled_h = ((orig_h as f32) * scale).round().max(target_size as f32) as u32;

        let scaler = wic.CreateBitmapScaler().ok()?;
        scaler
            .Initialize(&frame, scaled_w, scaled_h, WICBitmapInterpolationModeFant)
            .ok()?;

        let converter = wic.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &scaler,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .ok()?;

        let stride = scaled_w * 4;
        let mut full_buf = vec![0u8; (stride * scaled_h) as usize];
        converter
            .CopyPixels(std::ptr::null(), stride, &mut full_buf)
            .ok()?;

        if scaled_w == target_size && scaled_h == target_size {
            Some(full_buf)
        } else {
            Some(center_crop_square_bgra(
                &full_buf,
                scaled_w,
                scaled_h,
                target_size,
            ))
        }
    }
}

fn decode_and_process_cover(
    wic: &IWICImagingFactory,
    bytes: &[u8],
    hash: u64,
    fallback_accent: Rgb,
) -> Option<MediaCover> {
    let straight_thumb = decode_wic_square_straight_bgra(wic, bytes, THUMB_SIZE)?;
    let accent_dark =
        extract_accent_from_bgra(&straight_thumb, THUMB_SIZE, THUMB_SIZE, false, fallback_accent);
    let accent_light =
        extract_accent_from_bgra(&straight_thumb, THUMB_SIZE, THUMB_SIZE, true, fallback_accent);
    let thumb_pbgra = straight_to_premul_bgra(&straight_thumb);
    let (art_w, art_h, art_pbgra) =
        match decode_wic_square_straight_bgra(wic, bytes, ART_SIZE) {
            Some(straight_art) => (ART_SIZE, ART_SIZE, straight_to_premul_bgra(&straight_art)),
            None => (THUMB_SIZE, THUMB_SIZE, thumb_pbgra.clone()),
        };

    Some(MediaCover {
        hash,
        thumb_w: THUMB_SIZE,
        thumb_h: THUMB_SIZE,
        thumb_pbgra,
        art_w,
        art_h,
        art_pbgra,
        accent_dark,
        accent_light,
    })
}
