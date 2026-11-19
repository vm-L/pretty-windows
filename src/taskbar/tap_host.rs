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

/// Cross-platform crash-loop circuit breaker protecting `explorer.exe`.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Default)]
pub struct TapSafetyGuard {
    last_inject_pid: Option<u32>,
    last_inject_at: Option<Instant>,
    consecutive_failures: u32,
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

    pub fn on_explorer_pid_changed(&mut self, new_pid: u32, now: Instant) -> bool {
        if new_pid == 0 {
            return self.tripped;
        }
        if let Some(prev_pid) = self.last_inject_pid {
            if prev_pid != new_pid {
                if self
                    .last_inject_at
                    .is_some_and(|at| now.saturating_duration_since(at) < CRASH_DETECT_WINDOW)
                {
                    self.tripped = true;
                }
                self.last_inject_pid = None;
                self.last_inject_at = None;
            }
        }
        self.tripped
    }

    pub fn on_explorer_restarted(&mut self, now: Instant) -> bool {
        if self
            .last_inject_at
            .is_some_and(|at| now.saturating_duration_since(at) < CRASH_DETECT_WINDOW)
        {
            self.tripped = true;
        }
        self.last_inject_pid = None;
        self.last_inject_at = None;
        self.tripped
    }

    pub fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    pub fn record_success(&mut self) {
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

/// Removes stale Shadow Copy DLLs and `.tmp` staging files in `cache_dir`, ignoring locked handles.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn cleanup_stale_shadow_dlls(cache_dir: &Path, keep_filename: Option<&str>) {
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

/// Copies `src_dll` into `cache_dir` under a content-addressed name so `explorer.exe`
/// never locks the source DLL beside `PrettyWindows.exe`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn prepare_shadow_dll_in_dir(src_dll: &Path, cache_dir: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(src_dll).ok()?;
    if bytes.is_empty() {
        return None;
    }

    let filename = shadow_dll_filename(&bytes);
    let dest = cache_dir.join(&filename);
    std::fs::create_dir_all(cache_dir).ok()?;
    cleanup_stale_shadow_dlls(cache_dir, Some(&filename));

    if dest.is_file()
        && std::fs::metadata(&dest).ok().map(|m| m.len()) == Some(bytes.len() as u64)
        && std::fs::read(&dest).ok().as_deref() == Some(bytes.as_slice())
    {
        return Some(dest);
    }

    let tmp = cache_dir.join(format!("{filename}.{}.tmp", std::process::id()));
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
mod win {
    use super::{
        cleanup_stale_shadow_dlls, prepare_shadow_dll_in_dir, TapSafetyGuard,
    };
    use std::mem::size_of;
    use std::path::PathBuf;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use windows::core::{s, w, GUID, HRESULT, PCWSTR};
    use windows::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::LibraryLoader::{
        GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    use windows::Win32::System::Pipes::WaitNamedPipeW;
    use windows::Win32::System::Threading::GetCurrentProcessId;

    /// Shared protocol with `tap/src/lib.rs` (`PrettyWindowsTap.dll`).
    const CLSID_PRETTY_WINDOWS_TAP_U128: u128 = 0x8a4e6f21_9c3b_4d7a_b1e5_3f7c2a9d8410;
    const CLSID_PRETTY_WINDOWS_TAP: GUID = GUID::from_u128(CLSID_PRETTY_WINDOWS_TAP_U128);

    const CMD_RESTORE: u8 = 0x00;
    const CMD_HIDE: u8 = 0x01;
    const CMD_PING: u8 = 0x02;
    const CMD_SHUTDOWN: u8 = 0x03;

    const E_NOTFOUND_HR: HRESULT = HRESULT(0x80070490u32 as i32);
    const MAINTAIN_PING_INTERVAL: Duration = Duration::from_secs(2);
    const INJECT_RETRY_COOLDOWN: Duration = Duration::from_secs(2);
    const CONFLICT_RECHECK_COOLDOWN: Duration = Duration::from_secs(3);

    type InitializeXamlDiagnosticsExFn = unsafe extern "system" fn(
        end_point_name: PCWSTR,
        pid: u32,
        wsz_dll_xaml_diagnostics: PCWSTR,
        wsz_tap_dll_name: PCWSTR,
        tap_clsid: GUID,
        wsz_initialization_data: PCWSTR,
    ) -> HRESULT;

    fn pipe_name_wide(explorer_pid: u32) -> Vec<u16> {
        format!(r"\\.\pipe\PrettyWindows_TaskbarTap_{explorer_pid}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn tap_dll_path() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dll = exe.parent()?.join("PrettyWindowsTap.dll");
        dll.is_file().then_some(dll)
    }

    fn shadow_cache_dir() -> PathBuf {
        std::env::temp_dir().join("PrettyWindowsTap")
    }

    /// Detects known single-consumer XAML Diagnostics taskbar tools (e.g., TranslucentTB)
    /// so we never collide on the sole XAML Diagnostics slot in `explorer.exe`.
    pub fn is_conflicting_taskbar_tool_running() -> bool {
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return false;
            };
            if snap.is_invalid() || snap == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut pe = PROCESSENTRY32W {
                dwSize: size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut found = false;
            if Process32FirstW(snap, &mut pe).is_ok() {
                loop {
                    let len = pe
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(pe.szExeFile.len());
                    let name = String::from_utf16_lossy(&pe.szExeFile[..len]);
                    if name.eq_ignore_ascii_case("TranslucentTB.exe") {
                        found = true;
                        break;
                    }
                    if Process32NextW(snap, &mut pe).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
            found
        }
    }

    struct TapWorkerShared {
        guard: TapSafetyGuard,
        desired_hidden: bool,
        target_pid: u32,
        attached_pid: Option<u32>,
        conflict_detected: bool,
        wake_seq: u64,
        last_processed_seq: u64,
        last_inject_try: Option<Instant>,
        last_ping_ok: Option<Instant>,
        stop_worker: bool,
        shutdown_done: bool,
    }

    /// Asynchronous Named Pipe + XAML Diagnostics client; never blocks the tray UI thread.
    pub struct TapClient {
        shared: Arc<(Mutex<TapWorkerShared>, Condvar, Condvar)>,
        worker: Option<JoinHandle<()>>,
    }

    impl TapClient {
        pub fn new() -> Self {
            Self {
                shared: Arc::new((
                    Mutex::new(TapWorkerShared {
                        guard: TapSafetyGuard::new(),
                        desired_hidden: false,
                        target_pid: 0,
                        attached_pid: None,
                        conflict_detected: false,
                        wake_seq: 0,
                        last_processed_seq: 0,
                        last_inject_try: None,
                        last_ping_ok: None,
                        stop_worker: false,
                        shutdown_done: false,
                    }),
                    Condvar::new(),
                    Condvar::new(),
                )),
                worker: None,
            }
        }

        pub fn is_circuit_tripped(&self) -> bool {
            self.shared
                .0
                .lock()
                .map(|g| g.guard.is_tripped())
                .unwrap_or(false)
        }

        pub fn on_explorer_restarted(&mut self) {
            let now = Instant::now();
            if let Ok(mut st) = self.shared.0.lock() {
                if st.guard.on_explorer_restarted(now) {
                    st.desired_hidden = false;
                }
                st.attached_pid = None;
                st.last_ping_ok = None;
                st.target_pid = 0;
                st.wake_seq = st.wake_seq.wrapping_add(1);
                self.shared.1.notify_all();
            }
        }

        pub fn set_hide_background(&mut self, explorer_pid: u32, hide: bool, user_initiated: bool) {
            let need_worker = if let Ok(mut st) = self.shared.0.lock() {
                let now = Instant::now();
                if explorer_pid != 0 && st.guard.on_explorer_pid_changed(explorer_pid, now) {
                    st.desired_hidden = false;
                } else {
                    if user_initiated {
                        st.guard.reset_failures_on_user_action();
                    }
                    st.desired_hidden = hide && !st.guard.is_tripped();
                }
                st.target_pid = explorer_pid;
                if !hide {
                    st.conflict_detected = false;
                }
                st.wake_seq = st.wake_seq.wrapping_add(1);
                hide || st.attached_pid.is_some()
            } else {
                false
            };

            if need_worker {
                self.ensure_worker_started();
            }
            self.shared.1.notify_all();
        }

        pub fn maintain(&mut self, explorer_pid: u32) {
            if explorer_pid == 0 {
                return;
            }
            let now = Instant::now();
            let mut should_wake = false;
            if let Ok(mut st) = self.shared.0.lock() {
                if st.guard.on_explorer_pid_changed(explorer_pid, now) {
                    st.desired_hidden = false;
                    st.attached_pid = None;
                    st.last_ping_ok = None;
                    st.target_pid = explorer_pid;
                    return;
                }

                st.target_pid = explorer_pid;
                if st.attached_pid.is_some_and(|p| p != explorer_pid) {
                    st.attached_pid = None;
                    st.last_ping_ok = None;
                }

                if st.desired_hidden && !st.guard.is_tripped() {
                    let ping_due = st
                        .last_ping_ok
                        .is_none_or(|t| now.saturating_duration_since(t) >= MAINTAIN_PING_INTERVAL);
                    let need_attach = st.attached_pid != Some(explorer_pid)
                        && st.guard.can_inject()
                        && st.last_inject_try.is_none_or(|t| {
                            now.saturating_duration_since(t) >= INJECT_RETRY_COOLDOWN
                        });
                    if need_attach || (st.attached_pid == Some(explorer_pid) && ping_due) {
                        st.wake_seq = st.wake_seq.wrapping_add(1);
                        should_wake = true;
                    }
                }
            }
            if should_wake {
                self.ensure_worker_started();
                self.shared.1.notify_all();
            }
        }

        pub fn shutdown(&mut self) {
            let has_worker = self.worker.is_some();
            if let Ok(mut st) = self.shared.0.lock() {
                st.desired_hidden = false;
                st.stop_worker = true;
                st.wake_seq = st.wake_seq.wrapping_add(1);
            }
            self.shared.1.notify_all();

            if has_worker {
                let (lock, _, done_cv) = &*self.shared;
                let exited_cleanly = if let Ok(st) = lock.lock() {
                    done_cv
                        .wait_timeout_while(st, Duration::from_millis(800), |s| !s.shutdown_done)
                        .is_ok_and(|(guard, _)| guard.shutdown_done)
                } else {
                    false
                };
                if exited_cleanly {
                    if let Some(handle) = self.worker.take() {
                        let _ = handle.join();
                    }
                } else {
                    self.worker.take();
                }
            }
        }

        fn ensure_worker_started(&mut self) {
            if self.worker.is_some() {
                return;
            }
            let shared = Arc::clone(&self.shared);
            self.worker = Some(std::thread::spawn(move || run_tap_worker(shared)));
        }
    }

    impl Drop for TapClient {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    fn run_tap_worker(shared: Arc<(Mutex<TapWorkerShared>, Condvar, Condvar)>) {
        cleanup_stale_shadow_dlls(&shadow_cache_dir(), None);
        let (lock, wake_cv, done_cv) = &*shared;
        loop {
            let (desired_hidden, target_pid, attached_pid, stop_worker, last_try) = {
                let Ok(mut st) = lock.lock() else {
                    break;
                };
                while !st.stop_worker && st.wake_seq == st.last_processed_seq {
                    st = match wake_cv.wait(st) {
                        Ok(next) => next,
                        Err(_) => return,
                    };
                }
                st.last_processed_seq = st.wake_seq;
                (
                    st.desired_hidden && !st.guard.is_tripped(),
                    st.target_pid,
                    st.attached_pid,
                    st.stop_worker,
                    st.last_inject_try,
                )
            };

            if stop_worker {
                let pid = attached_pid.unwrap_or(target_pid);
                if pid != 0 && (attached_pid.is_some() || send_command(pid, CMD_PING, 30)) {
                    let _ = send_command(pid, CMD_RESTORE, 200);
                    let _ = send_command(pid, CMD_SHUTDOWN, 400);
                }
                if let Ok(mut st) = lock.lock() {
                    st.attached_pid = None;
                    st.shutdown_done = true;
                }
                cleanup_stale_shadow_dlls(&shadow_cache_dir(), None);
                done_cv.notify_all();
                break;
            }

            if target_pid == 0 {
                continue;
            }

            let mut cur_attached = attached_pid.filter(|&p| p == target_pid);
            if let Some(pid) = cur_attached {
                if send_command(pid, CMD_PING, 35) {
                    if let Ok(mut st) = lock.lock() {
                        st.last_ping_ok = Some(Instant::now());
                        st.guard.record_success();
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        if st.attached_pid == Some(pid) {
                            st.attached_pid = None;
                            st.last_ping_ok = None;
                        }
                    }
                }
            }

            if cur_attached.is_none() && send_command(target_pid, CMD_PING, 40) {
                cur_attached = Some(target_pid);
                if let Ok(mut st) = lock.lock() {
                    st.attached_pid = Some(target_pid);
                    st.last_ping_ok = Some(Instant::now());
                    st.conflict_detected = false;
                    st.guard.record_success();
                }
            }

            if cur_attached.is_none() && desired_hidden {
                let can_try = lock
                    .lock()
                    .map(|st| {
                        st.guard.can_inject()
                            && last_try.is_none_or(|t| t.elapsed() >= INJECT_RETRY_COOLDOWN)
                            && (!st.conflict_detected
                                || last_try.is_none_or(|t| t.elapsed() >= CONFLICT_RECHECK_COOLDOWN))
                    })
                    .unwrap_or(false);

                if can_try {
                    if is_conflicting_taskbar_tool_running() {
                        if let Ok(mut st) = lock.lock() {
                            st.conflict_detected = true;
                            st.last_inject_try = Some(Instant::now());
                        }
                        continue;
                    }

                    if let Ok(mut st) = lock.lock() {
                        st.conflict_detected = false;
                        st.last_inject_try = Some(Instant::now());
                        st.guard.record_inject_attempt(target_pid, Instant::now());
                    }

                    if inject_and_wait_pipe(target_pid) {
                        cur_attached = Some(target_pid);
                        if let Ok(mut st) = lock.lock() {
                            st.attached_pid = Some(target_pid);
                            st.last_ping_ok = Some(Instant::now());
                            st.guard.record_success();
                        }
                    } else if let Ok(mut st) = lock.lock() {
                        st.guard.record_failure();
                    }
                }
            }

            if let Some(pid) = cur_attached {
                let latest_hide = lock
                    .lock()
                    .map(|st| st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker)
                    .unwrap_or(false);
                let cmd = if latest_hide { CMD_HIDE } else { CMD_RESTORE };
                if send_command(pid, cmd, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else if let Ok(mut st) = lock.lock() {
                    if st.attached_pid == Some(pid) {
                        st.attached_pid = None;
                        st.last_ping_ok = None;
                    }
                }
            }
        }
    }

    fn inject_and_wait_pipe(explorer_pid: u32) -> bool {
        let Some(src_dll) = tap_dll_path() else {
            return false;
        };
        let dll_path =
            prepare_shadow_dll_in_dir(&src_dll, &shadow_cache_dir()).unwrap_or(src_dll);
        let dll_wide: Vec<u16> = dll_path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let Ok(h_xaml) =
                LoadLibraryExW(w!("Windows.UI.Xaml.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
            else {
                return false;
            };
            let Some(proc) = GetProcAddress(h_xaml, s!("InitializeXamlDiagnosticsEx")) else {
                return false;
            };
            let init_xaml: InitializeXamlDiagnosticsExFn = std::mem::transmute(proc);

            let mut connected = false;
            for i in 1..=60 {
                let endpoint: Vec<u16> = format!("VisualDiagConnection{i}")
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let hr = init_xaml(
                    PCWSTR(endpoint.as_ptr()),
                    explorer_pid,
                    w!(""),
                    PCWSTR(dll_wide.as_ptr()),
                    CLSID_PRETTY_WINDOWS_TAP,
                    PCWSTR::null(),
                );
                if hr.is_ok() {
                    connected = true;
                    break;
                }
                if hr != E_NOTFOUND_HR {
                    break;
                }
            }

            if !connected && !send_command(explorer_pid, CMD_PING, 40) {
                return false;
            }
        }

        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline {
            if send_command(explorer_pid, CMD_PING, 50) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        false
    }

    fn send_command(explorer_pid: u32, cmd: u8, wait_ms: u32) -> bool {
        let pipe_wide = pipe_name_wide(explorer_pid);
        unsafe {
            let handle: HANDLE = match CreateFileW(
                PCWSTR(pipe_wide.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            ) {
                Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE => h,
                _ => {
                    if wait_ms == 0 || !WaitNamedPipeW(PCWSTR(pipe_wide.as_ptr()), wait_ms).as_bool()
                    {
                        return false;
                    }
                    match CreateFileW(
                        PCWSTR(pipe_wide.as_ptr()),
                        GENERIC_READ.0 | GENERIC_WRITE.0,
                        FILE_SHARE_NONE,
                        None,
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL,
                        None,
                    ) {
                        Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE => h,
                        _ => return false,
                    }
                }
            };

            let host_pid = GetCurrentProcessId().to_le_bytes();
            let req = [cmd, host_pid[0], host_pid[1], host_pid[2], host_pid[3]];
            let mut written = 0u32;
            let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
                && written == req.len() as u32;

            let mut ack = [0u8; 1];
            let mut read = 0u32;
            let read_ok = write_ok
                && ReadFile(handle, Some(&mut ack), Some(&mut read), None).is_ok()
                && read == 1
                && ack[0] == cmd;

            let _ = CloseHandle(handle);
            read_ok
        }
    }
}

#[cfg(windows)]
pub use win::TapClient;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_change_within_crash_window_trips_circuit_breaker_permanently() {
        let mut g = TapSafetyGuard::new();
        let t0 = Instant::now();
        assert!(g.can_inject());
        g.record_inject_attempt(1001, t0);

        let tripped = g.on_explorer_pid_changed(2002, t0 + Duration::from_secs(8));
        assert!(tripped);
        assert!(g.is_tripped());
        assert!(!g.can_inject());

        // Even explicit user style changes must NOT re-arm a tripped crash guard.
        g.reset_failures_on_user_action();
        assert!(g.is_tripped());
        assert!(!g.can_inject());
    }

    #[test]
    fn restart_outside_crash_window_does_not_trip_circuit_breaker() {
        let mut g = TapSafetyGuard::new();
        let t0 = Instant::now();
        g.record_inject_attempt(1001, t0);

        let tripped = g.on_explorer_restarted(t0 + Duration::from_secs(35));
        assert!(!tripped);
        assert!(!g.is_tripped());
        assert!(g.can_inject());
    }

    #[test]
    fn transient_failures_cap_retries_until_user_action_resets() {
        let mut g = TapSafetyGuard::new();
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            g.record_failure();
        }
        assert!(!g.is_tripped());
        assert!(!g.can_inject());

        g.reset_failures_on_user_action();
        assert!(g.can_inject());

        g.record_failure();
        g.record_success();
        assert!(g.can_inject());
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
}
