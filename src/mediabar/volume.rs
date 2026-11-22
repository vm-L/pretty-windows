use std::collections::HashMap;

/// Scalar step applied per standard mouse wheel notch (`WHEEL_DELTA = 120`).
#[cfg_attr(not(windows), allow(dead_code))]
pub const WHEEL_STEP_SCALAR: f32 = 0.02;

/// Maps a horizontal pointer coordinate along `[track_left, track_right]` to a normalized `[0.0, 1.0]` slider ratio.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn slider_ratio_from_x(track_left: f32, track_right: f32, cursor_x: f32) -> f32 {
    let span = track_right - track_left;
    if span <= 0.0 {
        return 0.0;
    }
    ((cursor_x - track_left) / span).clamp(0.0, 1.0)
}

/// Applies mouse wheel delta to volume scalar (`±2%` per notch) and automatically unmutes on upward scroll above 0.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn step_volume_by_wheel(current: f32, current_muted: bool, wheel_delta: i16) -> (f32, bool) {
    let notches = f32::from(wheel_delta) / 120.0;
    let next_vol = (current + notches * WHEEL_STEP_SCALAR).clamp(0.0, 1.0);
    let next_muted = if current_muted && wheel_delta > 0 && next_vol > 0.0 {
        false
    } else {
        current_muted
    };
    (next_vol, next_muted)
}

/// Formats normalized volume `[0.0, 1.0]` as an integer percentage string (`"0%"`..`"100%"`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn format_volume_pct(volume: f32) -> String {
    let pct = (volume.clamp(0.0, 1.0) * 100.0).round() as u32;
    format!("{pct}%")
}

/// Single raw audio session record before application grouping.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct SessionRecord<T> {
    pub key: String,
    pub name: String,
    pub volume: f32,
    pub muted: bool,
    pub is_active: bool,
    pub handle: T,
}

/// Grouped per-application audio volume state aggregating one or more underlying session handles.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct GroupedAppVolume<T> {
    #[allow(dead_code)]
    pub key: String,
    pub name: String,
    pub volume: f32,
    pub muted: bool,
    pub is_active: bool,
    pub handles: Vec<T>,
}

/// Merges sessions belonging to the same application key and sorts active sessions first, then alphabetically.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn merge_session_records<T>(records: Vec<SessionRecord<T>>) -> Vec<GroupedAppVolume<T>> {
    let mut groups: Vec<GroupedAppVolume<T>> = Vec::with_capacity(records.len());
    let mut index_by_key: HashMap<String, usize> = HashMap::new();

    for rec in records {
        let vol = rec.volume.clamp(0.0, 1.0);
        if let Some(&idx) = index_by_key.get(&rec.key) {
            let group = &mut groups[idx];
            // Prefer the first active session's volume & mute state as representative for the group.
            if !group.is_active && rec.is_active {
                group.is_active = true;
                group.volume = vol;
                group.muted = rec.muted;
            }
            group.handles.push(rec.handle);
        } else {
            let idx = groups.len();
            index_by_key.insert(rec.key.clone(), idx);
            groups.push(GroupedAppVolume {
                key: rec.key,
                name: rec.name,
                volume: vol,
                muted: rec.muted,
                is_active: rec.is_active,
                handles: vec![rec.handle],
            });
        }
    }

    groups.sort_by(|a, b| {
        b.is_active
            .cmp(&a.is_active)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    groups
}

#[cfg(windows)]
mod win {
    use super::super::process_util::snapshot_process_lookup;
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
    use std::sync::Arc;
    use windows::core::{implement, Interface, GUID, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, S_OK, WPARAM};
    use windows::Win32::Media::Audio::Endpoints::{
        IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
    };
    use windows::Win32::Media::Audio::{
        eConsole, eCapture, eMultimedia, eRender, AudioSessionDisconnectReason, AudioSessionState,
        AudioSessionStateActive, AudioSessionStateExpired, EDataFlow, ERole,
        IAudioSessionControl, IAudioSessionControl2, IAudioSessionEvents,
        IAudioSessionEvents_Impl, IAudioSessionManager2, IAudioSessionNotification,
        IAudioSessionNotification_Impl, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient,
        IMMNotificationClient_Impl, ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoTaskMemFree, IAgileObject, IAgileObject_Impl, CLSCTX_ALL,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, PostMessageW, WM_APP};

    /// Custom UI notification posted from lock-free CoreAudio callbacks when master volume,
    /// default audio devices, or per-app audio sessions change externally.
    pub const WM_APP_VOLUME_UPDATED: u32 = WM_APP + 11;
    pub const VOL_EVENT_MASTER: usize = 1 << 0;
    pub const VOL_EVENT_DEVICE: usize = 1 << 1;
    pub const VOL_EVENT_SESSIONS: usize = 1 << 2;

    const MEDIABAR_VOLUME_EVENT_CONTEXT: GUID =
        GUID::from_u128(0x9a4d_7c12_3b8e_4f51_9e20_6c1a_8b7d_4e31);

    unsafe fn device_id_string(device: &IMMDevice) -> Option<String> {
        let pwstr: PWSTR = device.GetId().ok()?;
        if pwstr.is_null() {
            return None;
        }
        let id = pwstr.to_string().ok();
        CoTaskMemFree(Some(pwstr.0.cast()));
        id
    }

    unsafe fn open_default_render_device(enumerator: &IMMDeviceEnumerator) -> Option<IMMDevice> {
        enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .or_else(|_| enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia))
            .ok()
    }

    /// Lock-free atomic state shared between CoreAudio MTA callback threads and the STA UI thread.
    pub struct VolumeSharedState {
        alive: AtomicBool,
        master_vol_bits: AtomicU32,
        master_muted: AtomicBool,
        has_master_snapshot: AtomicBool,
        pending_flags: AtomicUsize,
    }

    impl VolumeSharedState {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                alive: AtomicBool::new(true),
                master_vol_bits: AtomicU32::new(1.0f32.to_bits()),
                master_muted: AtomicBool::new(false),
                has_master_snapshot: AtomicBool::new(false),
                pending_flags: AtomicUsize::new(0),
            })
        }

        fn set_master_snapshot(&self, vol: f32, muted: bool) {
            self.master_vol_bits
                .store(vol.clamp(0.0, 1.0).to_bits(), Ordering::Release);
            self.master_muted.store(muted, Ordering::Release);
            self.has_master_snapshot.store(true, Ordering::Release);
        }

        fn emit_event(&self, hwnd_raw: usize, flag: usize) {
            if !self.alive.load(Ordering::Acquire) || hwnd_raw == 0 {
                return;
            }
            let prev = self.pending_flags.fetch_or(flag, Ordering::AcqRel);
            if (prev & flag) == 0 {
                let hwnd = HWND(hwnd_raw as *mut c_void);
                unsafe {
                    if IsWindow(hwnd).as_bool() {
                        let _ = PostMessageW(hwnd, WM_APP_VOLUME_UPDATED, WPARAM(flag), LPARAM(0));
                    }
                }
            }
        }
    }

    #[implement(IAudioEndpointVolumeCallback, IAgileObject)]
    struct EndpointVolumeCallback {
        hwnd_raw: usize,
        shared: Arc<VolumeSharedState>,
    }

    impl IAgileObject_Impl for EndpointVolumeCallback_Impl {}

    impl IAudioEndpointVolumeCallback_Impl for EndpointVolumeCallback_Impl {
        fn OnNotify(
            &self,
            pnotify: *mut windows::Win32::Media::Audio::AUDIO_VOLUME_NOTIFICATION_DATA,
        ) -> windows::core::Result<()> {
            if !self.shared.alive.load(Ordering::Acquire) || pnotify.is_null() {
                return Ok(());
            }
            let (vol, muted) = unsafe {
                (
                    (*pnotify).fMasterVolume.clamp(0.0, 1.0),
                    (*pnotify).bMuted.as_bool(),
                )
            };
            self.shared.set_master_snapshot(vol, muted);
            self.shared.emit_event(self.hwnd_raw, VOL_EVENT_MASTER);
            Ok(())
        }
    }

    #[implement(IMMNotificationClient, IAgileObject)]
    struct DeviceNotificationClient {
        hwnd_raw: usize,
        shared: Arc<VolumeSharedState>,
    }

    impl IAgileObject_Impl for DeviceNotificationClient_Impl {}

    impl IMMNotificationClient_Impl for DeviceNotificationClient_Impl {
        fn OnDeviceStateChanged(
            &self,
            _pwstrdeviceid: &PCWSTR,
            _dwnewstate: DEVICE_STATE,
        ) -> windows::core::Result<()> {
            self.shared.emit_event(self.hwnd_raw, VOL_EVENT_DEVICE);
            Ok(())
        }

        fn OnDeviceAdded(&self, _pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
            self.shared.emit_event(self.hwnd_raw, VOL_EVENT_DEVICE);
            Ok(())
        }

        fn OnDeviceRemoved(&self, _pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
            self.shared.emit_event(self.hwnd_raw, VOL_EVENT_DEVICE);
            Ok(())
        }

        fn OnDefaultDeviceChanged(
            &self,
            flow: EDataFlow,
            role: ERole,
            _pwstrdefaultdeviceid: &PCWSTR,
        ) -> windows::core::Result<()> {
            if (flow == eRender && (role == eConsole || role == eMultimedia)) || flow == eCapture {
                self.shared.emit_event(self.hwnd_raw, VOL_EVENT_DEVICE);
            }
            Ok(())
        }

        fn OnPropertyValueChanged(
            &self,
            _pwstrdeviceid: &PCWSTR,
            _key: &PROPERTYKEY,
        ) -> windows::core::Result<()> {
            Ok(())
        }
    }

    #[implement(IAudioSessionEvents, IAgileObject)]
    struct SessionEventsCallback {
        hwnd_raw: usize,
        shared: Arc<VolumeSharedState>,
    }

    impl IAgileObject_Impl for SessionEventsCallback_Impl {}

    impl IAudioSessionEvents_Impl for SessionEventsCallback_Impl {
        fn OnDisplayNameChanged(
            &self,
            _newdisplayname: &PCWSTR,
            _eventcontext: *const GUID,
        ) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnIconPathChanged(
            &self,
            _newiconpath: &PCWSTR,
            _eventcontext: *const GUID,
        ) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnSimpleVolumeChanged(
            &self,
            _newvolume: f32,
            _newmute: BOOL,
            eventcontext: *const GUID,
        ) -> windows::core::Result<()> {
            if !eventcontext.is_null() && unsafe { *eventcontext } == MEDIABAR_VOLUME_EVENT_CONTEXT
            {
                return Ok(());
            }
            self.shared.emit_event(self.hwnd_raw, VOL_EVENT_SESSIONS);
            Ok(())
        }

        fn OnChannelVolumeChanged(
            &self,
            _channelcount: u32,
            _newchannelvolumearray: *const f32,
            _changedchannel: u32,
            _eventcontext: *const GUID,
        ) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnGroupingParamChanged(
            &self,
            _newgroupingparam: *const GUID,
            _eventcontext: *const GUID,
        ) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnStateChanged(&self, _newstate: AudioSessionState) -> windows::core::Result<()> {
            self.shared.emit_event(self.hwnd_raw, VOL_EVENT_SESSIONS);
            Ok(())
        }

        fn OnSessionDisconnected(
            &self,
            _disconnectreason: AudioSessionDisconnectReason,
        ) -> windows::core::Result<()> {
            self.shared.emit_event(self.hwnd_raw, VOL_EVENT_SESSIONS);
            Ok(())
        }
    }

    #[implement(IAudioSessionNotification, IAgileObject)]
    struct SessionCreatedCallback {
        hwnd_raw: usize,
        shared: Arc<VolumeSharedState>,
    }

    impl IAgileObject_Impl for SessionCreatedCallback_Impl {}

    impl IAudioSessionNotification_Impl for SessionCreatedCallback_Impl {
        fn OnSessionCreated(
            &self,
            _newsession: Option<&IAudioSessionControl>,
        ) -> windows::core::Result<()> {
            self.shared.emit_event(self.hwnd_raw, VOL_EVENT_SESSIONS);
            Ok(())
        }
    }

    /// UI-thread owner for active per-session COM notifications while the mixer popup is open.
    pub struct SessionWatchGuard {
        mgr: IAudioSessionManager2,
        created_cb: IAudioSessionNotification,
        sessions: Vec<IAudioSessionControl>,
        events_cb: IAudioSessionEvents,
    }

    impl Drop for SessionWatchGuard {
        fn drop(&mut self) {
            unsafe {
                for ctrl in &self.sessions {
                    let _ = ctrl.UnregisterAudioSessionNotification(&self.events_cb);
                }
                let _ = self.mgr.UnregisterSessionNotification(&self.created_cb);
            }
        }
    }

    /// Watches default playback/capture endpoint switches and master endpoint volume/mute changes.
    pub struct VolumeWatcher {
        hwnd_raw: usize,
        shared: Arc<VolumeSharedState>,
        enumerator: IMMDeviceEnumerator,
        device_cb: IMMNotificationClient,
        device_cb_registered: bool,
        endpoint_id: String,
        endpoint: Option<IAudioEndpointVolume>,
        vol_cb: Option<IAudioEndpointVolumeCallback>,
    }

    impl VolumeWatcher {
        pub fn start(target_hwnd: HWND) -> Option<Self> {
            unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                let hwnd_raw = target_hwnd.0 as usize;
                let shared = VolumeSharedState::new();
                let device_cb: IMMNotificationClient = DeviceNotificationClient {
                    hwnd_raw,
                    shared: Arc::clone(&shared),
                }
                .into();
                let device_cb_registered = enumerator
                    .RegisterEndpointNotificationCallback(&device_cb)
                    .is_ok();
                let mut watcher = Self {
                    hwnd_raw,
                    shared,
                    enumerator,
                    device_cb,
                    device_cb_registered,
                    endpoint_id: String::new(),
                    endpoint: None,
                    vol_cb: None,
                };
                watcher.rebind_default_endpoint();
                Some(watcher)
            }
        }

        pub fn shared_state(&self) -> Arc<VolumeSharedState> {
            Arc::clone(&self.shared)
        }

        pub fn take_pending_events(&self) -> usize {
            self.shared.pending_flags.swap(0, Ordering::AcqRel)
        }

        pub fn current_master_state(&self) -> Option<(f32, bool)> {
            if self.shared.has_master_snapshot.load(Ordering::Acquire) {
                Some((
                    f32::from_bits(self.shared.master_vol_bits.load(Ordering::Acquire)),
                    self.shared.master_muted.load(Ordering::Acquire),
                ))
            } else {
                None
            }
        }

        fn unbind_endpoint_cb(&mut self) {
            if let (Some(ref ep), Some(ref cb)) = (&self.endpoint, &self.vol_cb) {
                unsafe {
                    let _ = ep.UnregisterControlChangeNotify(cb);
                }
            }
            self.vol_cb = None;
            self.endpoint = None;
        }

        /// Safely unregisters old endpoint notifications on the UI thread and binds to the
        /// active `eConsole` (fallback `eMultimedia`) render endpoint.
        pub fn rebind_default_endpoint(&mut self) -> (bool, Option<(f32, bool)>) {
            let prev_id = std::mem::take(&mut self.endpoint_id);
            self.unbind_endpoint_cb();

            unsafe {
                let Some(device) = open_default_render_device(&self.enumerator) else {
                    return (!prev_id.is_empty(), None);
                };
                let new_id = device_id_string(&device).unwrap_or_default();
                let render_changed = prev_id != new_id;
                self.endpoint_id = new_id;

                if let Ok(endpoint) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    let vol_cb: IAudioEndpointVolumeCallback = EndpointVolumeCallback {
                        hwnd_raw: self.hwnd_raw,
                        shared: Arc::clone(&self.shared),
                    }
                    .into();
                    if endpoint.RegisterControlChangeNotify(&vol_cb).is_ok() {
                        self.vol_cb = Some(vol_cb);
                    }
                    let state = endpoint
                        .GetMasterVolumeLevelScalar()
                        .ok()
                        .zip(endpoint.GetMute().ok())
                        .map(|(v, m)| (v.clamp(0.0, 1.0), m.as_bool()));
                    if let Some((vol, muted)) = state {
                        self.shared.set_master_snapshot(vol, muted);
                    }
                    self.endpoint = Some(endpoint);
                    (render_changed, state)
                } else {
                    (render_changed, None)
                }
            }
        }

        pub fn shutdown(&mut self) {
            self.shared.alive.store(false, Ordering::Release);
            self.unbind_endpoint_cb();
            if self.device_cb_registered {
                unsafe {
                    let _ = self
                        .enumerator
                        .UnregisterEndpointNotificationCallback(&self.device_cb);
                }
                self.device_cb_registered = false;
            }
        }
    }

    impl Drop for VolumeWatcher {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    /// Controls master volume and mute state on the active default playback audio endpoint (`eConsole` priority).
    pub struct MasterVolumeControl {
        enumerator: IMMDeviceEnumerator,
        device_id: String,
        endpoint: IAudioEndpointVolume,
    }

    impl MasterVolumeControl {
        /// Opens the active default render endpoint (`eConsole`, falling back to `eMultimedia` to align with OS OSD).
        pub fn open_default() -> Option<Self> {
            unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                let device = open_default_render_device(&enumerator)?;
                let device_id = device_id_string(&device).unwrap_or_default();
                let endpoint = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?;
                Some(Self {
                    enumerator,
                    device_id,
                    endpoint,
                })
            }
        }

        fn ensure_current_default(&mut self) {
            unsafe {
                let Some(device) = open_default_render_device(&self.enumerator) else {
                    return;
                };
                let Some(id) = device_id_string(&device) else {
                    return;
                };
                if !self.device_id.is_empty() && self.device_id == id {
                    return;
                }
                if let Ok(endpoint) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    self.device_id = id;
                    self.endpoint = endpoint;
                }
            }
        }

        pub fn get_state(&mut self) -> Option<(f32, bool)> {
            self.ensure_current_default();
            unsafe {
                let vol = self.endpoint.GetMasterVolumeLevelScalar().ok()?.clamp(0.0, 1.0);
                let muted = self.endpoint.GetMute().ok()?.as_bool();
                Some((vol, muted))
            }
        }

        pub fn set_volume_and_readback(&mut self, level: f32) -> Option<(f32, bool)> {
            self.ensure_current_default();
            let clamped = level.clamp(0.0, 1.0);
            unsafe {
                let prev_muted = self.endpoint.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                if self
                    .endpoint
                    .SetMasterVolumeLevelScalar(clamped, &MEDIABAR_VOLUME_EVENT_CONTEXT)
                    .is_err()
                {
                    return None;
                }
                if prev_muted && clamped > 0.0 {
                    let _ = self.endpoint.SetMute(false, &MEDIABAR_VOLUME_EVENT_CONTEXT);
                }
                let actual_vol = self
                    .endpoint
                    .GetMasterVolumeLevelScalar()
                    .ok()
                    .map(|v| v.clamp(0.0, 1.0))
                    .unwrap_or(clamped);
                let actual_muted = self
                    .endpoint
                    .GetMute()
                    .ok()
                    .map(|b| b.as_bool())
                    .unwrap_or(if prev_muted && clamped > 0.0 {
                        false
                    } else {
                        prev_muted
                    });
                Some((actual_vol, actual_muted))
            }
        }

        pub fn set_mute_and_readback(&mut self, muted: bool) -> Option<(f32, bool)> {
            self.ensure_current_default();
            unsafe {
                if self
                    .endpoint
                    .SetMute(muted, &MEDIABAR_VOLUME_EVENT_CONTEXT)
                    .is_err()
                {
                    return None;
                }
                Some(self.get_state().unwrap_or((1.0, muted)))
            }
        }

        pub fn step_wheel_and_readback(&mut self, wheel_delta: i16) -> Option<(f32, bool)> {
            let (base_vol, base_muted) = self.get_state()?;
            let (target_vol, next_muted) = step_volume_by_wheel(base_vol, base_muted, wheel_delta);
            unsafe {
                if base_muted != next_muted {
                    let _ = self
                        .endpoint
                        .SetMute(next_muted, &MEDIABAR_VOLUME_EVENT_CONTEXT);
                }
                if self
                    .endpoint
                    .SetMasterVolumeLevelScalar(target_vol, &MEDIABAR_VOLUME_EVENT_CONTEXT)
                    .is_err()
                {
                    return self.get_state();
                }
                let actual_vol = self
                    .endpoint
                    .GetMasterVolumeLevelScalar()
                    .ok()
                    .map(|v| v.clamp(0.0, 1.0))
                    .unwrap_or(target_vol);
                // Guard against coarse hardware volume steps (>2%) snapping scalar writes back to base_vol.
                if wheel_delta != 0
                    && (target_vol - base_vol).abs() > 0.005
                    && (actual_vol - base_vol).abs() < 0.004
                {
                    if wheel_delta > 0 {
                        let _ = self.endpoint.VolumeStepUp(&MEDIABAR_VOLUME_EVENT_CONTEXT);
                    } else {
                        let _ = self.endpoint.VolumeStepDown(&MEDIABAR_VOLUME_EVENT_CONTEXT);
                    }
                }
                Some(self.get_state().unwrap_or((actual_vol, next_muted)))
            }
        }
    }

    pub type AppVolumeEntry = GroupedAppVolume<ISimpleAudioVolume>;

    impl AppVolumeEntry {
        fn readback_from_first_handle(&mut self) {
            if let Some(first) = self.handles.first() {
                unsafe {
                    if let Ok(v) = first.GetMasterVolume() {
                        self.volume = v.clamp(0.0, 1.0);
                    }
                    if let Ok(m) = first.GetMute() {
                        self.muted = m.as_bool();
                    }
                }
            }
        }

        /// Sets scalar volume across all grouped COM session handles for this application.
        pub fn set_group_volume(&mut self, level: f32) {
            let clamped = level.clamp(0.0, 1.0);
            let unmute = self.muted && clamped > 0.0;
            self.volume = clamped;
            if unmute {
                self.muted = false;
            }
            unsafe {
                for handle in &self.handles {
                    let _ = handle.SetMasterVolume(clamped, &MEDIABAR_VOLUME_EVENT_CONTEXT);
                    if unmute {
                        let _ = handle.SetMute(false, &MEDIABAR_VOLUME_EVENT_CONTEXT);
                    }
                }
            }
            self.readback_from_first_handle();
        }

        /// Steps volume via wheel delta across all grouped COM session handles for this application.
        pub fn step_group_wheel(&mut self, wheel_delta: i16) {
            let (next_vol, next_muted) = step_volume_by_wheel(self.volume, self.muted, wheel_delta);
            let unmute = self.muted && !next_muted;
            self.volume = next_vol;
            self.muted = next_muted;
            unsafe {
                for handle in &self.handles {
                    let _ = handle.SetMasterVolume(next_vol, &MEDIABAR_VOLUME_EVENT_CONTEXT);
                    if unmute {
                        let _ = handle.SetMute(false, &MEDIABAR_VOLUME_EVENT_CONTEXT);
                    }
                }
            }
            self.readback_from_first_handle();
        }

        /// Toggles mute state across all grouped COM session handles for this application.
        pub fn toggle_group_mute(&mut self) {
            let next_muted = !self.muted;
            self.muted = next_muted;
            unsafe {
                for handle in &self.handles {
                    let _ = handle.SetMute(next_muted, &MEDIABAR_VOLUME_EVENT_CONTEXT);
                }
            }
            self.readback_from_first_handle();
        }
    }

    unsafe fn take_pwstr(ptr: PWSTR) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let s = ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(ptr.0.cast()));
        s
    }

    /// Enumerates non-system audio sessions on the default render device (`eConsole` priority)
    /// and optionally registers session-creation & per-session COM notifications for `target_hwnd`.
    pub fn enumerate_app_volumes_with_watch(
        target_hwnd: HWND,
        shared: Option<&Arc<VolumeSharedState>>,
    ) -> (Vec<AppVolumeEntry>, Option<SessionWatchGuard>) {
        unsafe {
            let Ok(enumerator): windows::core::Result<IMMDeviceEnumerator> =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            else {
                return (Vec::new(), None);
            };

            let Some(device) = open_default_render_device(&enumerator) else {
                return (Vec::new(), None);
            };

            let Ok(mgr) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                return (Vec::new(), None);
            };

            let watch_setup = shared.and_then(|sh| {
                let hwnd_raw = target_hwnd.0 as usize;
                let created_cb: IAudioSessionNotification = SessionCreatedCallback {
                    hwnd_raw,
                    shared: Arc::clone(sh),
                }
                .into();
                // RegisterSessionNotification requires a subsequent GetSessionEnumerator() call to activate events.
                if mgr.RegisterSessionNotification(&created_cb).is_ok() {
                    let events_cb: IAudioSessionEvents = SessionEventsCallback {
                        hwnd_raw,
                        shared: Arc::clone(sh),
                    }
                    .into();
                    Some((created_cb, events_cb))
                } else {
                    None
                }
            });

            let Ok(sessions) = mgr.GetSessionEnumerator() else {
                if let Some((created_cb, _)) = watch_setup {
                    let _ = mgr.UnregisterSessionNotification(&created_cb);
                }
                return (Vec::new(), None);
            };

            let count = sessions.GetCount().unwrap_or(0);
            let lookup = snapshot_process_lookup();
            let mut records = Vec::with_capacity(count as usize);
            let mut watched_ctrls = Vec::with_capacity(count as usize);

            for i in 0..count {
                let Ok(ctrl) = sessions.GetSession(i) else {
                    continue;
                };
                let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else {
                    continue;
                };

                // IsSystemSoundsSession returns S_OK for system sounds and S_FALSE otherwise;
                // never use .is_ok() since S_FALSE also counts as success.
                if ctrl2.IsSystemSoundsSession() == S_OK {
                    continue;
                }

                let Ok(state) = ctrl2.GetState() else {
                    continue;
                };
                if state == AudioSessionStateExpired {
                    continue;
                }
                let is_active = state == AudioSessionStateActive;

                let Ok(pid) = ctrl2.GetProcessId() else {
                    continue;
                };
                if pid == 0 {
                    continue;
                }

                let display_name = ctrl2
                    .GetDisplayName()
                    .ok()
                    .map(|p| take_pwstr(p))
                    .unwrap_or_default();

                let Some((key, name)) = lookup.resolve_audio_app(pid, &display_name) else {
                    continue;
                };

                let Ok(simple_vol) = ctrl.cast::<ISimpleAudioVolume>() else {
                    continue;
                };
                if let Some((_, ref events_cb)) = watch_setup {
                    if ctrl.RegisterAudioSessionNotification(events_cb).is_ok() {
                        watched_ctrls.push(ctrl.clone());
                    }
                }
                let volume = simple_vol.GetMasterVolume().unwrap_or(1.0).clamp(0.0, 1.0);
                let muted = simple_vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);

                records.push(SessionRecord {
                    key,
                    name,
                    volume,
                    muted,
                    is_active,
                    handle: simple_vol,
                });
            }

            let guard = watch_setup.map(|(created_cb, events_cb)| SessionWatchGuard {
                mgr,
                created_cb,
                sessions: watched_ctrls,
                events_cb,
            });
            (merge_session_records(records), guard)
        }
    }
}

#[cfg(windows)]
pub use win::{
    enumerate_app_volumes_with_watch, AppVolumeEntry, MasterVolumeControl, SessionWatchGuard,
    VolumeSharedState, VolumeWatcher, VOL_EVENT_DEVICE, VOL_EVENT_MASTER, VOL_EVENT_SESSIONS,
    WM_APP_VOLUME_UPDATED,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_ratio_clamps_bounds_and_invalid_span() {
        assert_eq!(slider_ratio_from_x(10.0, 110.0, -5.0), 0.0);
        assert_eq!(slider_ratio_from_x(10.0, 110.0, 10.0), 0.0);
        assert!((slider_ratio_from_x(10.0, 110.0, 60.0) - 0.5).abs() < 1e-6);
        assert_eq!(slider_ratio_from_x(10.0, 110.0, 150.0), 1.0);
        assert_eq!(slider_ratio_from_x(100.0, 100.0, 50.0), 0.0);
        assert_eq!(slider_ratio_from_x(120.0, 80.0, 100.0), 0.0);
    }

    #[test]
    fn wheel_step_adjusts_volume_and_auto_unmutes_upwards() {
        let (v, m) = step_volume_by_wheel(0.50, false, 120);
        assert!((v - 0.52).abs() < 1e-6);
        assert!(!m);

        let (v, m) = step_volume_by_wheel(0.50, false, -120);
        assert!((v - 0.48).abs() < 1e-6);
        assert!(!m);

        // Clamp top and bottom
        assert_eq!(step_volume_by_wheel(0.99, false, 120), (1.0, false));
        assert_eq!(step_volume_by_wheel(0.01, false, -120), (0.0, false));

        // Muted: scrolling up unmutes when result > 0; scrolling down stays muted
        let (v, m) = step_volume_by_wheel(0.0, true, 120);
        assert!((v - 0.02).abs() < 1e-6);
        assert!(!m);

        let (v, m) = step_volume_by_wheel(0.40, true, -120);
        assert!((v - 0.38).abs() < 1e-6);
        assert!(m);
    }

    #[test]
    fn format_volume_pct_rounds_and_clamps() {
        assert_eq!(format_volume_pct(-0.2), "0%");
        assert_eq!(format_volume_pct(0.0), "0%");
        assert_eq!(format_volume_pct(0.376), "38%");
        assert_eq!(format_volume_pct(1.5), "100%");
    }

    #[test]
    fn merge_session_records_groups_same_app_and_prioritizes_active() {
        let records = vec![
            SessionRecord {
                key: "chrome".to_string(),
                name: "Google Chrome".to_string(),
                volume: 0.25,
                muted: true,
                is_active: false,
                handle: 1u32,
            },
            SessionRecord {
                key: "vlc".to_string(),
                name: "VLC".to_string(),
                volume: 0.90,
                muted: false,
                is_active: false,
                handle: 2u32,
            },
            SessionRecord {
                key: "chrome".to_string(),
                name: "Google Chrome".to_string(),
                volume: 0.72,
                muted: false,
                is_active: true,
                handle: 3u32,
            },
            SessionRecord {
                key: "chrome".to_string(),
                name: "Google Chrome".to_string(),
                volume: 0.10,
                muted: true,
                is_active: true,
                handle: 4u32,
            },
            SessionRecord {
                key: "cloudmusic".to_string(),
                name: "网易云音乐".to_string(),
                volume: 0.85,
                muted: false,
                is_active: true,
                handle: 5u32,
            },
            SessionRecord {
                key: "aimp".to_string(),
                name: "AIMP".to_string(),
                volume: 0.60,
                muted: false,
                is_active: false,
                handle: 6u32,
            },
        ];

        let merged = merge_session_records(records);
        assert_eq!(merged.len(), 4);

        // Active apps first, sorted alphabetically ("Google Chrome" before "网易云音乐")
        assert_eq!(merged[0].key, "chrome");
        assert!(merged[0].is_active);
        // Should pick the FIRST active session's volume & mute (0.72, false), not the inactive 0.25 or second active 0.10
        assert!((merged[0].volume - 0.72).abs() < 1e-6);
        assert!(!merged[0].muted);
        assert_eq!(merged[0].handles, vec![1, 3, 4]);

        assert_eq!(merged[1].key, "cloudmusic");
        assert!(merged[1].is_active);
        assert_eq!(merged[1].handles, vec![5]);

        // Inactive apps next, sorted alphabetically ("AIMP" before "VLC")
        assert_eq!(merged[2].key, "aimp");
        assert!(!merged[2].is_active);
        assert_eq!(merged[3].key, "vlc");
        assert!(!merged[3].is_active);
    }
}
