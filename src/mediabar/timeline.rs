/// Normalized media playback timeline reported by Windows SMTC (`100ns` ticks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaTimeline {
    pub start_100ns: i64,
    pub end_100ns: i64,
    pub position_100ns: i64,
    /// Windows FILETIME (`100ns` intervals since 1601-01-01 UTC) when `position_100ns` was recorded.
    pub last_updated_100ns: i64,
    /// Whether the active player advertises seek support (`IsPlaybackPositionEnabled`).
    pub can_seek: bool,
}

const TICKS_PER_SEC: f64 = 10_000_000.0;
/// Ignore tracks shorter than 1 second or longer than 24 hours (typically live streams or sentinel values).
const MIN_VALID_DURATION_100NS: i64 = 10_000_000;
const MAX_VALID_DURATION_100NS: i64 = 24 * 3600 * 10_000_000;

impl MediaTimeline {
    /// Total track duration in `100ns` ticks (`end - start`).
    #[inline]
    pub fn duration_100ns(&self) -> i64 {
        self.end_100ns.saturating_sub(self.start_100ns)
    }

    /// Returns `true` when the timeline represents a finite, displayable track duration.
    #[inline]
    pub fn is_valid(&self) -> bool {
        let dur = self.duration_100ns();
        (MIN_VALID_DURATION_100NS..=MAX_VALID_DURATION_100NS).contains(&dur)
    }

    /// Computes the live playback position in `100ns` ticks, extrapolating forward while playing.
    pub fn current_position_100ns(&self, now_filetime_100ns: i64, is_playing: bool) -> i64 {
        let base = self.position_100ns.clamp(self.start_100ns, self.end_100ns);
        if !is_playing || self.last_updated_100ns <= 0 || now_filetime_100ns <= self.last_updated_100ns
        {
            return base;
        }
        let delta = now_filetime_100ns
            .saturating_sub(self.last_updated_100ns)
            .clamp(0, MAX_VALID_DURATION_100NS);
        base.saturating_add(delta)
            .clamp(self.start_100ns, self.end_100ns)
    }

    /// Returns normalized playback progress in `0.0..=1.0`, or `None` when no finite timeline exists.
    pub fn progress_ratio(&self, now_filetime_100ns: i64, is_playing: bool) -> Option<f32> {
        if !self.is_valid() {
            return None;
        }
        let dur = self.duration_100ns() as f64;
        let cur = self
            .current_position_100ns(now_filetime_100ns, is_playing)
            .saturating_sub(self.start_100ns) as f64;
        Some(((cur / dur) as f32).clamp(0.0, 1.0))
    }

    /// Returns `(elapsed_secs, total_secs)` relative to `start_100ns`.
    #[allow(dead_code)]
    pub fn elapsed_and_total_secs(
        &self,
        now_filetime_100ns: i64,
        is_playing: bool,
    ) -> Option<(f64, f64)> {
        if !self.is_valid() {
            return None;
        }
        let dur_secs = (self.duration_100ns() as f64) / TICKS_PER_SEC;
        let cur_secs = (self
            .current_position_100ns(now_filetime_100ns, is_playing)
            .saturating_sub(self.start_100ns) as f64)
            / TICKS_PER_SEC;
        Some((cur_secs.clamp(0.0, dur_secs), dur_secs))
    }

    /// Maps a normalized `[0.0, 1.0]` slider ratio to an absolute SMTC seek target in `100ns` ticks.
    #[allow(dead_code)]
    pub fn seek_target_100ns(&self, ratio: f32) -> Option<i64> {
        if !self.is_valid() || !self.can_seek {
            return None;
        }
        let r = ratio.clamp(0.0, 1.0) as f64;
        let offset = ((self.duration_100ns() as f64) * r).round() as i64;
        Some(
            self.start_100ns
                .saturating_add(offset)
                .clamp(self.start_100ns, self.end_100ns),
        )
    }

    /// Computes `(preview_elapsed_secs, total_secs)` for an interactive slider ratio `[0.0, 1.0]`.
    #[allow(dead_code)]
    pub fn preview_secs_for_ratio(&self, ratio: f32) -> Option<(f64, f64)> {
        if !self.is_valid() {
            return None;
        }
        let dur_secs = (self.duration_100ns() as f64) / TICKS_PER_SEC;
        let r = ratio.clamp(0.0, 1.0) as f64;
        Some(((dur_secs * r).clamp(0.0, dur_secs), dur_secs))
    }
}

/// Formats seconds as `MM:SS` (or `H:MM:SS` for tracks >= 1 hour).
#[allow(dead_code)]
pub fn format_media_time(secs: f64) -> String {
    let total = if secs.is_finite() && secs > 0.0 {
        secs.floor() as u64
    } else {
        0
    };
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Returns current UTC system time as Windows FILETIME `100ns` ticks (same epoch as `winrt::DateTime`).
#[cfg(windows)]
pub fn current_filetime_100ns() -> i64 {
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    let ft = unsafe { GetSystemTimeAsFileTime() };
    ((u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: i64 = 10_000_000;

    fn sample_timeline() -> MediaTimeline {
        MediaTimeline {
            start_100ns: 0,
            end_100ns: 200 * SEC,
            position_100ns: 50 * SEC,
            last_updated_100ns: 1_000 * SEC,
            can_seek: true,
        }
    }

    #[test]
    fn extrapolates_while_playing_and_holds_when_paused() {
        let tl = sample_timeline();
        assert!(tl.is_valid());

        // 10s elapsed while playing -> 60s / 200s = 0.30
        let playing_ratio = tl.progress_ratio(1_010 * SEC, true).expect("valid ratio");
        assert!((playing_ratio - 0.30).abs() < 1e-4);

        // When paused, position does not drift forward
        let paused_ratio = tl.progress_ratio(1_080 * SEC, false).expect("valid ratio");
        assert!((paused_ratio - 0.25).abs() < 1e-4);

        // Clamps cleanly at track end
        let clamped_ratio = tl.progress_ratio(1_500 * SEC, true).expect("valid ratio");
        assert_eq!(clamped_ratio, 1.0);
    }

    #[test]
    fn rejects_zero_negative_or_absurd_durations() {
        let live = MediaTimeline {
            start_100ns: 0,
            end_100ns: 0,
            position_100ns: 0,
            last_updated_100ns: 1_000 * SEC,
            can_seek: false,
        };
        assert!(!live.is_valid());
        assert_eq!(live.progress_ratio(1_005 * SEC, true), None);

        let inverted = MediaTimeline {
            start_100ns: 100 * SEC,
            end_100ns: 50 * SEC,
            position_100ns: 50 * SEC,
            last_updated_100ns: 1_000 * SEC,
            can_seek: true,
        };
        assert!(!inverted.is_valid());

        let absurd = MediaTimeline {
            start_100ns: 0,
            end_100ns: 48 * 3600 * SEC,
            position_100ns: 0,
            last_updated_100ns: 1_000 * SEC,
            can_seek: true,
        };
        assert!(!absurd.is_valid());
    }

    #[test]
    fn seek_target_maps_ratio_and_respects_can_seek() {
        let mut tl = MediaTimeline {
            start_100ns: 10 * SEC,
            end_100ns: 110 * SEC,
            position_100ns: 30 * SEC,
            last_updated_100ns: 1_000 * SEC,
            can_seek: true,
        };
        assert_eq!(tl.seek_target_100ns(0.0), Some(10 * SEC));
        assert_eq!(tl.seek_target_100ns(0.5), Some(60 * SEC));
        assert_eq!(tl.seek_target_100ns(1.2), Some(110 * SEC));

        tl.can_seek = false;
        assert_eq!(tl.seek_target_100ns(0.5), None);
    }

    #[test]
    fn formats_mm_ss_and_h_mm_ss_cleanly() {
        assert_eq!(format_media_time(-5.0), "00:00");
        assert_eq!(format_media_time(0.0), "00:00");
        assert_eq!(format_media_time(65.9), "01:05");
        assert_eq!(format_media_time(238.0), "03:58");
        assert_eq!(format_media_time(3661.0), "1:01:01");
    }
}
