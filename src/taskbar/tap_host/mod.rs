use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg_attr(not(windows), allow(dead_code))]
const SHADOW_PREFIX: &str = "PrettyWindowsTap_";
#[cfg_attr(not(windows), allow(dead_code))]
const SHADOW_EXT: &str = ".dll";

/// Time window after a TAP injection during which an Explorer PID change or restart
/// is treated as an injection-induced crash, permanently halting further injection.
#[cfg_attr(not(windows), allow(dead_code))]
pub const CRASH_DETECT_WINDOW: Duration = Duration::from_secs(30);
#[cfg_attr(not(windows), allow(dead_code))]
pub const MAX_CONSECUTIVE_FAILURES: u32 = 3;
#[cfg_attr(not(windows), allow(dead_code))]
pub const MAX_UNKNOWN_CRASH_STRIKES: u32 = 2;
#[cfg_attr(not(windows), allow(dead_code))]
pub const UNKNOWN_STRIKE_WINDOW: Duration = Duration::from_secs(60);

/// Classification of a watched target process's exit status when its PID disappears or changes.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessExitKind {
    /// Clean exit or manual termination (e.g. `0`, Task Manager restart `1`, `DBG_TERMINATE_PROCESS`, `STATUS_CONTROL_C_EXIT`).
    NormalOrManual,
    /// Fatal NTSTATUS / unhandled exception crash (e.g. `0xC0000005`, `0xC0000409`, `0xC00000FD`, `0xE0434352`).
    Crash,
    /// Process exit code unavailable or still active (`259`).
    Unknown,
}

/// Classifies a Win32 process exit code into normal/manual termination, fatal crash, or unknown.
#[cfg_attr(not(windows), allow(dead_code))]
pub const fn classify_process_exit_code(exit_code: Option<u32>) -> ProcessExitKind {
    const STILL_ACTIVE: u32 = 259;
    const DBG_TERMINATE_PROCESS: u32 = 0x4001_0004;
    const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;
    const STATUS_DLL_INIT_FAILED_LOGOFF: u32 = 0xC000_026B;
    match exit_code {
        None | Some(STILL_ACTIVE) => ProcessExitKind::Unknown,
        Some(
            0 | 1 | DBG_TERMINATE_PROCESS | STATUS_CONTROL_C_EXIT | STATUS_DLL_INIT_FAILED_LOGOFF,
        ) => ProcessExitKind::NormalOrManual,
        Some(code) if code >= 0x8000_0000 => ProcessExitKind::Crash,
        Some(_) => ProcessExitKind::NormalOrManual,
    }
}

/// Computes the backoff interval before retrying XAML Diagnostics injection into `explorer.exe`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn explorer_inject_retry_cooldown(not_ready_retries: u32) -> Duration {
    match not_ready_retries {
        0 => Duration::from_secs(2),
        1 => Duration::from_millis(500),
        2 => Duration::from_secs(1),
        3 => Duration::from_millis(1500),
        4..=9 => Duration::from_secs(2),
        _ => Duration::from_secs(5),
    }
}

/// Cross-platform crash-loop circuit breaker protecting `explorer.exe`.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Default)]
pub struct TapSafetyGuard {
    last_inject_pid: Option<u32>,
    last_inject_at: Option<Instant>,
    consecutive_failures: u32,
    unknown_crash_strikes: u32,
    last_unknown_strike_at: Option<Instant>,
    tripped: bool,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl TapSafetyGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_tripped(&self) -> bool {
        self.tripped
    }

    pub fn can_inject(&self) -> bool {
        !self.tripped && self.consecutive_failures < MAX_CONSECUTIVE_FAILURES
    }

    pub fn record_inject_attempt(&mut self, pid: u32, now: Instant) {
        if pid != 0 {
            self.last_inject_pid = Some(pid);
            self.last_inject_at = Some(now);
        }
    }

    fn record_rapid_exit(&mut self, exit_kind: ProcessExitKind, now: Instant) {
        match exit_kind {
            ProcessExitKind::NormalOrManual => {
                self.unknown_crash_strikes = 0;
                self.last_unknown_strike_at = None;
                self.consecutive_failures = 0;
            }
            ProcessExitKind::Crash => {
                self.tripped = true;
            }
            ProcessExitKind::Unknown => {
                let expired = self
                    .last_unknown_strike_at
                    .is_none_or(|t| now.saturating_duration_since(t) >= UNKNOWN_STRIKE_WINDOW);
                self.unknown_crash_strikes = if expired {
                    1
                } else {
                    self.unknown_crash_strikes.saturating_add(1)
                };
                self.last_unknown_strike_at = Some(now);
                if self.unknown_crash_strikes >= MAX_UNKNOWN_CRASH_STRIKES {
                    self.tripped = true;
                }
            }
        }
    }

    pub fn on_explorer_pid_changed(
        &mut self,
        new_pid: u32,
        exit_kind: ProcessExitKind,
        now: Instant,
    ) -> bool {
        if new_pid == 0 {
            return self.tripped;
        }
        if let Some(prev_pid) = self.last_inject_pid {
            if prev_pid != new_pid {
                if self
                    .last_inject_at
                    .is_some_and(|at| now.saturating_duration_since(at) < CRASH_DETECT_WINDOW)
                {
                    self.record_rapid_exit(exit_kind, now);
                }
                self.last_inject_pid = None;
                self.last_inject_at = None;
                self.consecutive_failures = 0;
            }
        }
        self.tripped
    }

    pub fn on_explorer_restarted(
        &mut self,
        current_pid: Option<u32>,
        exit_kind: ProcessExitKind,
        now: Instant,
    ) -> bool {
        let pid_changed = match (self.last_inject_pid, current_pid) {
            (Some(old_pid), Some(new_pid)) => old_pid != new_pid,
            (Some(_), None) => true,
            _ => false,
        };
        if pid_changed
            && self
                .last_inject_at
                .is_some_and(|at| now.saturating_duration_since(at) < CRASH_DETECT_WINDOW)
        {
            self.record_rapid_exit(exit_kind, now);
        }
        self.last_inject_pid = None;
        self.last_inject_at = None;
        if !self.tripped && (pid_changed || current_pid.is_none()) {
            self.consecutive_failures = 0;
        }
        self.tripped
    }

    pub fn clear_failed_attempt_if_same_pid(&mut self, pid: u32) {
        if self.last_inject_pid == Some(pid) {
            self.last_inject_pid = None;
            self.last_inject_at = None;
        }
    }

    pub fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Resets transient failure backoff when switching to a freshly started target PID.
    /// Never clears a tripped crash-loop circuit breaker within the current process lifetime.
    pub fn reset_failures_on_new_pid(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Resets transient failure backoff when the user explicitly selects a style.
    /// Never clears a tripped crash-loop circuit breaker within the current process lifetime.
    pub fn reset_failures_on_user_action(&mut self) {
        self.consecutive_failures = 0;
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Derives a content-addressed Shadow Copy filename (`PrettyWindowsTap_<fnv1a>_<len>.dll`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn shadow_dll_filename(bytes: &[u8]) -> String {
    format!("{SHADOW_PREFIX}{:016x}_{}{SHADOW_EXT}", fnv1a_64(bytes), bytes.len())
}

/// Returns `true` if `name` matches any loaded `PrettyWindowsTap` DLL build
/// (`PrettyWindowsTap.dll` or a content-addressed shadow copy `PrettyWindowsTap_*.dll`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn is_tap_dll_module_name(name: &str) -> bool {
    if name.eq_ignore_ascii_case("PrettyWindowsTap.dll") {
        return true;
    }
    name.len() > SHADOW_PREFIX.len() + SHADOW_EXT.len()
        && name
            .get(..SHADOW_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(SHADOW_PREFIX))
        && name
            .get(name.len() - SHADOW_EXT.len()..)
            .is_some_and(|ext| ext.eq_ignore_ascii_case(SHADOW_EXT))
}

static SHADOW_COPY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn cleanup_stale_shadow_dlls_locked(cache_dir: &Path, keep_filename: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with(SHADOW_PREFIX)
            || (!name_str.ends_with(SHADOW_EXT) && !name_str.ends_with(".tmp"))
        {
            continue;
        }
        if keep_filename == Some(name_str.as_ref()) {
            continue;
        }
        let _ = std::fs::remove_file(entry.path());
    }
}

/// Removes stale Shadow Copy DLLs and `.tmp` staging files in `cache_dir`, ignoring locked handles.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn cleanup_stale_shadow_dlls(cache_dir: &Path, keep_filename: Option<&str>) {
    let _guard = SHADOW_COPY_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cleanup_stale_shadow_dlls_locked(cache_dir, keep_filename);
}

/// Copies `src_dll` into `cache_dir` under a content-addressed name so `explorer.exe`
/// never locks the source DLL beside `PrettyWindows.exe`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn prepare_shadow_dll_in_dir(src_dll: &Path, cache_dir: &Path) -> Option<PathBuf> {
    static SHADOW_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    // Serialize intra-process callers (Taskbar/ShellExp/ShellHost workers) so cleanup and `.tmp`
    // staging never race and fall back to locking the source DLL directly.
    let _guard = SHADOW_COPY_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let bytes = std::fs::read(src_dll).ok()?;
    if bytes.is_empty() {
        return None;
    }

    let filename = shadow_dll_filename(&bytes);
    let dest = cache_dir.join(&filename);
    std::fs::create_dir_all(cache_dir).ok()?;
    cleanup_stale_shadow_dlls_locked(cache_dir, Some(&filename));

    if dest.is_file()
        && std::fs::metadata(&dest).ok().map(|m| m.len()) == Some(bytes.len() as u64)
        && std::fs::read(&dest).ok().as_deref() == Some(bytes.as_slice())
    {
        return Some(dest);
    }

    let seq = SHADOW_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = cache_dir.join(format!("{filename}.{}.{seq}.tmp", std::process::id()));
    std::fs::write(&tmp, &bytes).ok()?;
    if std::fs::rename(&tmp, &dest).is_err() {
        let _ = std::fs::remove_file(&tmp);
        if dest.is_file()
            && std::fs::read(&dest).ok().as_deref() == Some(bytes.as_slice())
        {
            return Some(dest);
        }
        return None;
    }

    Some(dest)
}


#[cfg(windows)]
mod explorer;
#[cfg(windows)]
mod inject;
#[cfg(windows)]
mod shell_flyout;

#[cfg(windows)]
pub use explorer::TapClient;
#[cfg(windows)]
pub use inject::find_shell_flyout_pids_in_session;
#[cfg(windows)]
pub use shell_flyout::ShellFlyoutTapClient;

pub fn shell_flyout_idle_inject_backoff(failures: u32) -> Duration {
    Duration::from_secs(2u64.saturating_mul(1u64 << failures.min(5)).min(60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_classification_separates_manual_exit_crash_and_unknown() {
        assert_eq!(
            classify_process_exit_code(Some(0)),
            ProcessExitKind::NormalOrManual
        );
        assert_eq!(
            classify_process_exit_code(Some(1)),
            ProcessExitKind::NormalOrManual
        );
        assert_eq!(
            classify_process_exit_code(Some(0x4001_0004)),
            ProcessExitKind::NormalOrManual
        );
        assert_eq!(
            classify_process_exit_code(Some(0xC000_013A)),
            ProcessExitKind::NormalOrManual
        );
        assert_eq!(
            classify_process_exit_code(Some(0xC000_026B)),
            ProcessExitKind::NormalOrManual
        );

        assert_eq!(classify_process_exit_code(None), ProcessExitKind::Unknown);
        assert_eq!(
            classify_process_exit_code(Some(259)),
            ProcessExitKind::Unknown
        );

        assert_eq!(
            classify_process_exit_code(Some(0xC000_0005)),
            ProcessExitKind::Crash
        );
        assert_eq!(
            classify_process_exit_code(Some(0xC000_0409)),
            ProcessExitKind::Crash
        );
        assert_eq!(
            classify_process_exit_code(Some(0xC000_00FD)),
            ProcessExitKind::Crash
        );
        assert_eq!(
            classify_process_exit_code(Some(0xE043_4352)),
            ProcessExitKind::Crash
        );
    }

    #[test]
    fn manual_explorer_restart_within_crash_window_does_not_trip_and_resets_failures() {
        let mut g = TapSafetyGuard::new();
        let t0 = Instant::now();
        for i in 0..5u32 {
            let pid = 1000 + i * 10;
            g.record_inject_attempt(pid, t0 + Duration::from_secs(u64::from(i) * 4));
            g.record_failure();
            g.record_failure();
            let tripped = g.on_explorer_pid_changed(
                pid + 10,
                ProcessExitKind::NormalOrManual,
                t0 + Duration::from_secs(u64::from(i) * 4 + 2),
            );
            assert!(!tripped, "manual restart must not trip circuit breaker");
            assert!(!g.is_tripped());
            assert!(g.can_inject(), "consecutive failures must reset for new PID");
        }

        let mut g2 = TapSafetyGuard::new();
        g2.record_inject_attempt(5001, t0);
        g2.record_failure();
        assert!(!g2.on_explorer_restarted(
            Some(5002),
            ProcessExitKind::NormalOrManual,
            t0 + Duration::from_secs(3),
        ));
        assert!(g2.can_inject());
    }

    #[test]
    fn crash_exit_within_crash_window_trips_circuit_breaker_permanently() {
        let mut g = TapSafetyGuard::new();
        let t0 = Instant::now();
        assert!(g.can_inject());
        g.record_inject_attempt(1001, t0);

        let tripped =
            g.on_explorer_pid_changed(2002, ProcessExitKind::Crash, t0 + Duration::from_secs(8));
        assert!(tripped);
        assert!(g.is_tripped());
        assert!(!g.can_inject());

        // Even explicit user style changes or new PIDs must NOT re-arm a tripped crash guard.
        g.reset_failures_on_user_action();
        g.reset_failures_on_new_pid();
        assert!(g.is_tripped());
        assert!(!g.can_inject());
    }

    #[test]
    fn unknown_exit_requires_two_strikes_within_window_to_trip() {
        let mut g = TapSafetyGuard::new();
        let t0 = Instant::now();
        g.record_inject_attempt(1001, t0);
        assert!(!g.on_explorer_pid_changed(
            1002,
            ProcessExitKind::Unknown,
            t0 + Duration::from_secs(5),
        ));
        assert!(g.can_inject());

        // Second unknown strike inside UNKNOWN_STRIKE_WINDOW (60s) trips breaker.
        g.record_inject_attempt(1002, t0 + Duration::from_secs(10));
        assert!(g.on_explorer_restarted(
            Some(1003),
            ProcessExitKind::Unknown,
            t0 + Duration::from_secs(15),
        ));
        assert!(g.is_tripped());
        assert!(!g.can_inject());

        // Strikes spaced beyond UNKNOWN_STRIKE_WINDOW decay cleanly and do not trip.
        let mut g2 = TapSafetyGuard::new();
        g2.record_inject_attempt(2001, t0);
        assert!(!g2.on_explorer_pid_changed(
            2002,
            ProcessExitKind::Unknown,
            t0 + Duration::from_secs(5),
        ));
        g2.record_inject_attempt(2002, t0 + Duration::from_secs(70));
        assert!(!g2.on_explorer_pid_changed(
            2003,
            ProcessExitKind::Unknown,
            t0 + Duration::from_secs(75),
        ));
        assert!(!g2.is_tripped());
    }

    #[test]
    fn restart_outside_crash_window_does_not_trip_circuit_breaker() {
        let mut g = TapSafetyGuard::new();
        let t0 = Instant::now();
        g.record_inject_attempt(1001, t0);

        let tripped = g.on_explorer_restarted(
            Some(1002),
            ProcessExitKind::Crash,
            t0 + Duration::from_secs(35),
        );
        assert!(!tripped);
        assert!(!g.is_tripped());
        assert!(g.can_inject());
    }

    #[test]
    fn transient_failures_cap_retries_until_user_action_or_new_pid_resets() {
        let mut g = TapSafetyGuard::new();
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            g.record_failure();
        }
        assert!(!g.is_tripped());
        assert!(!g.can_inject());

        g.reset_failures_on_user_action();
        assert!(g.can_inject());

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            g.record_failure();
        }
        assert!(!g.can_inject());
        g.reset_failures_on_new_pid();
        assert!(g.can_inject());

        g.record_failure();
        g.record_success();
        assert!(g.can_inject());
    }

    #[test]
    fn explorer_inject_retry_cooldown_steps_from_fast_startup_to_backoff() {
        assert_eq!(explorer_inject_retry_cooldown(0), Duration::from_secs(2));
        assert_eq!(explorer_inject_retry_cooldown(1), Duration::from_millis(500));
        assert_eq!(explorer_inject_retry_cooldown(2), Duration::from_secs(1));
        assert_eq!(explorer_inject_retry_cooldown(3), Duration::from_millis(1500));
        assert_eq!(explorer_inject_retry_cooldown(4), Duration::from_secs(2));
        assert_eq!(explorer_inject_retry_cooldown(9), Duration::from_secs(2));
        assert_eq!(explorer_inject_retry_cooldown(10), Duration::from_secs(5));
        assert_eq!(explorer_inject_retry_cooldown(99), Duration::from_secs(5));
    }

    struct WorkspaceTmpGuard {
        root: PathBuf,
        parent_tmp: PathBuf,
    }

    impl WorkspaceTmpGuard {
        fn new(name: &str) -> Self {
            let parent_tmp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp");
            let root = parent_tmp.join(format!("{name}_{}", std::process::id()));
            let _ = std::fs::create_dir_all(&root);
            Self { root, parent_tmp }
        }
    }

    impl Drop for WorkspaceTmpGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
            let _ = std::fs::remove_dir(&self.parent_tmp);
        }
    }

    #[test]
    fn shadow_dll_filename_is_deterministic_and_sensitive_to_content_changes() {
        let a1 = shadow_dll_filename(b"PrettyWindowsTap-v1");
        let a2 = shadow_dll_filename(b"PrettyWindowsTap-v1");
        let b1 = shadow_dll_filename(b"PrettyWindowsTap-v2");
        let b2 = shadow_dll_filename(b"PrettyWindowsTap-v10");

        assert_eq!(a1, a2);
        assert_ne!(a1, b1);
        assert_ne!(a1, b2);
        assert!(a1.starts_with(SHADOW_PREFIX));
        assert!(a1.ends_with(SHADOW_EXT));
    }

    #[test]
    fn tap_dll_module_name_matches_any_shadow_or_direct_build() {
        assert!(is_tap_dll_module_name("PrettyWindowsTap.dll"));
        assert!(is_tap_dll_module_name("prettywindowstap.DLL"));
        assert!(is_tap_dll_module_name(
            "PrettyWindowsTap_7dfd0aa87cb801c7_416768.dll"
        ));
        assert!(is_tap_dll_module_name(
            "prettywindowstap_179012b017fbe372_418304.DLL"
        ));
        assert!(!is_tap_dll_module_name("PrettyWindowsTap_.dll"));
        assert!(!is_tap_dll_module_name("PrettyWindowsTap_abc.tmp"));
        assert!(!is_tap_dll_module_name("Windows.UI.Xaml.dll"));
    }

    #[test]
    fn prepare_shadow_dll_reuses_identical_copy_and_rotates_on_update_with_cleanup() {
        let guard = WorkspaceTmpGuard::new("shadow_copy_test");
        let src_dir = guard.root.join("src");
        let cache_dir = guard.root.join("cache");
        let _ = std::fs::create_dir_all(&src_dir);
        let _ = std::fs::create_dir_all(&cache_dir);

        let src_dll = src_dir.join("PrettyWindowsTap.dll");
        std::fs::write(&src_dll, b"binary-payload-v1").expect("write v1 dll");

        let copy1 = prepare_shadow_dll_in_dir(&src_dll, &cache_dir).expect("prepare v1");
        assert!(copy1.is_file());
        assert_eq!(
            std::fs::read(&copy1).expect("read copy1"),
            b"binary-payload-v1"
        );

        let copy1_again =
            prepare_shadow_dll_in_dir(&src_dll, &cache_dir).expect("reuse v1");
        assert_eq!(copy1_again, copy1);

        let unrelated = cache_dir.join("keep_me.txt");
        let stale_tmp = cache_dir.join(format!("{SHADOW_PREFIX}deadbeef.tmp"));
        std::fs::write(&unrelated, b"keep").expect("write unrelated");
        std::fs::write(&stale_tmp, b"tmp").expect("write stale tmp");

        std::fs::write(&src_dll, b"binary-payload-v2-updated").expect("write v2 dll");
        let copy2 = prepare_shadow_dll_in_dir(&src_dll, &cache_dir).expect("prepare v2");
        assert_ne!(copy2, copy1);
        assert!(copy2.is_file());
        assert_eq!(
            std::fs::read(&copy2).expect("read copy2"),
            b"binary-payload-v2-updated"
        );

        assert!(!copy1.exists(), "stale v1 shadow DLL should be cleaned up");
        assert!(!stale_tmp.exists(), "stale tmp file should be cleaned up");
        assert!(unrelated.exists(), "unrelated files must remain untouched");
    }

    #[test]
    fn prepare_shadow_dll_is_safe_under_concurrent_threads() {
        let guard = WorkspaceTmpGuard::new("shadow_copy_concurrent_test");
        let src_dir = guard.root.join("src");
        let cache_dir = guard.root.join("cache");
        std::fs::create_dir_all(&src_dir).expect("create src dir");
        std::fs::create_dir_all(&cache_dir).expect("create cache dir");

        let payload = b"binary-payload-concurrent-v1";
        let src_dll = src_dir.join("PrettyWindowsTap.dll");
        std::fs::write(&src_dll, payload).expect("write src dll");
        let expected_dest = cache_dir.join(shadow_dll_filename(payload));

        let thread_count = 6;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(thread_count));
        let mut handles = Vec::with_capacity(thread_count);
        for _ in 0..thread_count {
            let barrier = std::sync::Arc::clone(&barrier);
            let src_dll = src_dll.clone();
            let cache_dir = cache_dir.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                prepare_shadow_dll_in_dir(&src_dll, &cache_dir)
            }));
        }

        for handle in handles {
            let resolved = handle
                .join()
                .expect("worker thread panicked")
                .expect("shadow copy must succeed under contention");
            assert_eq!(resolved, expected_dest);
            assert_eq!(
                std::fs::read(&resolved).expect("read resolved shadow copy"),
                payload
            );
        }

        let leftover_tmp = std::fs::read_dir(&cache_dir)
            .expect("read cache dir")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"));
        assert!(
            !leftover_tmp,
            "no stale .tmp staging files should remain after concurrent shadow copy"
        );
    }

    #[test]
    fn same_pid_explorer_restart_does_not_trip_circuit_breaker() {
        let mut g = TapSafetyGuard::new();
        let t0 = Instant::now();
        g.record_inject_attempt(1001, t0);
        let tripped =
            g.on_explorer_restarted(Some(1001), ProcessExitKind::Crash, t0 + Duration::from_secs(5));
        assert!(!tripped, "same-PID TaskbarCreated must not trip breaker");
        assert!(g.can_inject());
    }

    #[test]
    fn different_pid_or_missing_explorer_restart_within_window_trips_breaker() {
        let mut g = TapSafetyGuard::new();
        let t0 = Instant::now();
        g.record_inject_attempt(1001, t0);
        assert!(g.on_explorer_restarted(
            Some(1002),
            ProcessExitKind::Crash,
            t0 + Duration::from_secs(5),
        ));
        assert!(!g.can_inject());

        let mut g2 = TapSafetyGuard::new();
        g2.record_inject_attempt(2001, t0);
        assert!(g2.on_explorer_restarted(None, ProcessExitKind::Crash, t0 + Duration::from_secs(5)));
        assert!(!g2.can_inject());
    }

    #[test]
    fn clear_failed_attempt_and_idle_backoff_behave_as_expected() {
        let mut g = TapSafetyGuard::new();
        let t0 = Instant::now();
        g.record_inject_attempt(3001, t0);
        g.clear_failed_attempt_if_same_pid(9999);
        assert!(
            g.on_explorer_restarted(Some(3002), ProcessExitKind::Crash, t0 + Duration::from_secs(5)),
            "mismatched PID should not clear recorded attempt"
        );

        let mut g2 = TapSafetyGuard::new();
        g2.record_inject_attempt(3001, t0);
        g2.clear_failed_attempt_if_same_pid(3001);
        assert!(
            !g2.on_explorer_restarted(
                Some(3002),
                ProcessExitKind::Crash,
                t0 + Duration::from_secs(5),
            ),
            "same PID failed idle attempt should be cleared cleanly"
        );

        assert_eq!(shell_flyout_idle_inject_backoff(0), Duration::from_secs(2));
        assert_eq!(shell_flyout_idle_inject_backoff(1), Duration::from_secs(4));
        assert_eq!(shell_flyout_idle_inject_backoff(2), Duration::from_secs(8));
        assert_eq!(shell_flyout_idle_inject_backoff(3), Duration::from_secs(16));
        assert_eq!(shell_flyout_idle_inject_backoff(4), Duration::from_secs(32));
        assert_eq!(shell_flyout_idle_inject_backoff(5), Duration::from_secs(60));
        assert_eq!(shell_flyout_idle_inject_backoff(99), Duration::from_secs(60));
    }
}
