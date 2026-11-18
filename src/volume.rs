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
    use super::*;
    use crate::process_util::snapshot_process_lookup;
    use windows::core::{Interface, GUID, PWSTR};
    use windows::Win32::Foundation::S_OK;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eConsole, eMultimedia, eRender, AudioSessionStateActive, AudioSessionStateExpired,
        IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume,
        MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

    /// Controls master volume and mute state on the default playback audio endpoint.
    pub struct MasterVolumeControl {
        endpoint: IAudioEndpointVolume,
    }

    impl MasterVolumeControl {
        /// Opens the active default render endpoint (`eMultimedia`, falling back to `eConsole`).
        pub fn open_default() -> Option<Self> {
            unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                let device = enumerator
                    .GetDefaultAudioEndpoint(eRender, eMultimedia)
                    .or_else(|_| enumerator.GetDefaultAudioEndpoint(eRender, eConsole))
                    .ok()?;
                let endpoint = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?;
                Some(Self { endpoint })
            }
        }

        pub fn get_state(&self) -> Option<(f32, bool)> {
            unsafe {
                let vol = self.endpoint.GetMasterVolumeLevelScalar().ok()?.clamp(0.0, 1.0);
                let muted = self.endpoint.GetMute().ok()?.as_bool();
                Some((vol, muted))
            }
        }

        pub fn set_volume(&self, level: f32) -> bool {
            unsafe {
                self.endpoint
                    .SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), &GUID::zeroed())
                    .is_ok()
            }
        }

        pub fn set_mute(&self, muted: bool) -> bool {
            unsafe { self.endpoint.SetMute(muted, &GUID::zeroed()).is_ok() }
        }
    }

    pub type AppVolumeEntry = GroupedAppVolume<ISimpleAudioVolume>;

    impl AppVolumeEntry {
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
                    let _ = handle.SetMasterVolume(clamped, &GUID::zeroed());
                    if unmute {
                        let _ = handle.SetMute(false, &GUID::zeroed());
                    }
                }
            }
        }

        /// Steps volume via wheel delta across all grouped COM session handles for this application.
        pub fn step_group_wheel(&mut self, wheel_delta: i16) {
            let (next_vol, next_muted) = step_volume_by_wheel(self.volume, self.muted, wheel_delta);
            let unmute = self.muted && !next_muted;
            self.volume = next_vol;
            self.muted = next_muted;
            unsafe {
                for handle in &self.handles {
                    let _ = handle.SetMasterVolume(next_vol, &GUID::zeroed());
                    if unmute {
                        let _ = handle.SetMute(false, &GUID::zeroed());
                    }
                }
            }
        }

        /// Toggles mute state across all grouped COM session handles for this application.
        pub fn toggle_group_mute(&mut self) {
            let next_muted = !self.muted;
            self.muted = next_muted;
            unsafe {
                for handle in &self.handles {
                    let _ = handle.SetMute(next_muted, &GUID::zeroed());
                }
            }
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

    /// Enumerates and groups non-system audio sessions on the default render device.
    pub fn enumerate_app_volumes() -> Vec<AppVolumeEntry> {
        unsafe {
            let Ok(enumerator): windows::core::Result<IMMDeviceEnumerator> =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            else {
                return Vec::new();
            };

            let Ok(device) = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .or_else(|_| enumerator.GetDefaultAudioEndpoint(eRender, eConsole))
            else {
                return Vec::new();
            };

            let Ok(mgr) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                return Vec::new();
            };
            let Ok(sessions) = mgr.GetSessionEnumerator() else {
                return Vec::new();
            };

            let count = sessions.GetCount().unwrap_or(0);
            let lookup = snapshot_process_lookup();
            let mut records = Vec::with_capacity(count as usize);

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

            merge_session_records(records)
        }
    }
}

#[cfg(windows)]
pub use win::{enumerate_app_volumes, AppVolumeEntry, MasterVolumeControl};

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
