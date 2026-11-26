use super::inject::{
    current_user_sid_string, decode_flyout_rects_payload, invoke_xaml_diagnostics_ex,
    is_any_tap_dll_loaded_in_process, is_process_alive, prepare_tap_dll_for_injection,
    reverse_pipe_name_wide, verify_pipe_client_pid, AltTabBoundsCache, ComMtaGuard,
    WatchedProcessHandle, CMD_HIDE, CMD_MENU_FLYOUT_RECTS, CMD_RESTORE,
    CMD_SET_MENU_FLYOUT_FALLBACK_STYLE, CMD_SET_MENU_FLYOUT_SUPPRESS, CMD_SET_START_MODAL_STYLE,
    CMD_SHELL_CARD_RECTS, MAINTAIN_PING_INTERVAL, MAX_MENU_FLYOUT_RECTS, MAX_SHELL_CARD_RECTS,
};
use super::{ProcessExitKind, TapSafetyGuard};
use std::mem::size_of;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE,
    HLOCAL, HWND, INVALID_HANDLE_VALUE, LPARAM, WAIT_EVENT, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PeekNamedPipe,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcessId, ResetEvent, SetEvent, WaitForMultipleObjects,
    WaitForSingleObject, INFINITE,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

struct OwnedEvent(HANDLE);

unsafe impl Send for OwnedEvent {}
unsafe impl Sync for OwnedEvent {}

impl OwnedEvent {
    fn new_manual_reset() -> Option<Self> {
        let h = unsafe { CreateEventW(None, true, false, None).ok()? };
        if h.0.is_null() || h == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Self(h))
        }
    }

    #[inline]
    fn handle(&self) -> HANDLE {
        self.0
    }

    #[inline]
    fn set(&self) {
        if !self.0.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                let _ = SetEvent(self.0);
            }
        }
    }

    #[inline]
    fn reset(&self) {
        if !self.0.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                let _ = ResetEvent(self.0);
            }
        }
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        if !self.0.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                let _ = CloseHandle(self.0);
            }
            self.0 = HANDLE::default();
        }
    }
}

struct ShellFlyoutWorkerShared {
    pipe_prefix: &'static str,
    grant_appcontainer_acl: bool,
    guard: TapSafetyGuard,
    desired_hidden: bool,
    target_pid: u32,
    attached_pid: Option<u32>,
    wake_seq: u64,
    last_processed_seq: u64,
    query_seq: u64,
    last_query_seq: u64,
    flyout_active: bool,
    flyout_bounds: Vec<AltTabBoundsCache>,
    flyout_suppressed: bool,
    synced_flyout_suppressed: Option<bool>,
    flyout_fallback_colors: Option<(u32, u32)>,
    synced_flyout_fallback_colors: Option<(u32, u32)>,
    modal_style: Option<(u8, bool, u32, u32)>,
    synced_modal_style: Option<(u8, bool, u32, u32)>,
    notify_hwnd_raw: usize,
    notify_msg: u32,
    last_inject_try: Option<Instant>,
    idle_inject_failures: u32,
    watched_process: Option<WatchedProcessHandle>,
    last_ping_ok: Option<Instant>,
    stop_worker: bool,
    shutdown_done: bool,
}

impl ShellFlyoutWorkerShared {
    fn track_process_pid(&mut self, pid: u32) {
        if pid == 0 {
            return;
        }
        if self.watched_process.as_ref().is_some_and(|w| w.pid() == pid) {
            return;
        }
        if let Some(handle) = WatchedProcessHandle::open(pid) {
            self.watched_process = Some(handle);
        }
    }

    fn inspect_old_pid_exit_kind(&mut self, current_pid: Option<u32>) -> ProcessExitKind {
        let Some(ref watched) = self.watched_process else {
            return ProcessExitKind::Unknown;
        };
        if current_pid == Some(watched.pid()) {
            return ProcessExitKind::Unknown;
        }
        let kind = watched.query_exit_kind();
        if current_pid.is_some() || kind != ProcessExitKind::Unknown {
            self.watched_process = None;
        }
        kind
    }
}

/// Dedicated TAP controller for Shell flyout hosts (`ShellExperienceHost.exe` and
/// `ShellHost.exe`), using a persistent overlapped reverse Named Pipe server compatible with
/// UWP PLM suspend/resume and an independent crash-loop circuit breaker per host process.
pub struct ShellFlyoutTapClient {
    shared: Arc<(Mutex<ShellFlyoutWorkerShared>, OwnedEvent, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl ShellFlyoutTapClient {
    fn with_config(pipe_prefix: &'static str, grant_appcontainer_acl: bool) -> Self {
        let wake_event =
            OwnedEvent::new_manual_reset().unwrap_or_else(|| OwnedEvent(HANDLE::default()));
        Self {
            shared: Arc::new((
                Mutex::new(ShellFlyoutWorkerShared {
                    pipe_prefix,
                    grant_appcontainer_acl,
                    guard: TapSafetyGuard::new(),
                    desired_hidden: false,
                    target_pid: 0,
                    attached_pid: None,
                    wake_seq: 0,
                    last_processed_seq: 0,
                    query_seq: 0,
                    last_query_seq: 0,
                    flyout_active: false,
                    flyout_bounds: Vec::new(),
                    flyout_suppressed: false,
                    synced_flyout_suppressed: None,
                    flyout_fallback_colors: None,
                    synced_flyout_fallback_colors: None,
                    modal_style: None,
                    synced_modal_style: None,
                    notify_hwnd_raw: 0,
                    notify_msg: 0,
                    last_inject_try: None,
                    idle_inject_failures: 0,
                    watched_process: None,
                    last_ping_ok: None,
                    stop_worker: false,
                    shutdown_done: false,
                }),
                wake_event,
                Condvar::new(),
            )),
            worker: None,
        }
    }

    pub fn new_shell_experience() -> Self {
        Self::with_config(r"\\.\pipe\PrettyWindows_ShellExpTap_", true)
    }

    pub fn new_shell_host() -> Self {
        Self::with_config(r"\\.\pipe\PrettyWindows_ShellHostTap_", false)
    }

    pub fn new_start_menu() -> Self {
        Self::with_config(r"\\.\pipe\PrettyWindows_StartMenuTap_", true)
    }

    pub fn new_search_host() -> Self {
        Self::with_config(r"\\.\pipe\PrettyWindows_SearchHostTap_", true)
    }

    pub fn set_notify_target(&mut self, hwnd: HWND, msg: u32) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.notify_hwnd_raw = hwnd.0 as usize;
            st.notify_msg = msg;
        }
    }

    pub fn is_circuit_tripped(&self) -> bool {
        self.shared
            .0
            .lock()
            .map(|g| g.guard.is_tripped())
            .unwrap_or(false)
    }

    pub fn set_hide_background(&mut self, target_pid: u32, hide: bool, user_initiated: bool) {
        let need_worker = if let Ok(mut st) = self.shared.0.lock() {
            let now = Instant::now();
            if target_pid != 0 {
                let pid_changed = st.target_pid != 0 && st.target_pid != target_pid;
                let exit_kind = st.inspect_old_pid_exit_kind(Some(target_pid));
                if st.guard.on_explorer_pid_changed(target_pid, exit_kind, now) {
                    st.desired_hidden = false;
                } else {
                    if user_initiated {
                        st.guard.reset_failures_on_user_action();
                    } else if pid_changed {
                        st.guard.reset_failures_on_new_pid();
                    }
                    if pid_changed {
                        st.last_inject_try = None;
                        st.idle_inject_failures = 0;
                    }
                    st.desired_hidden = hide && !st.guard.is_tripped();
                }
                st.target_pid = target_pid;
                st.track_process_pid(target_pid);
            } else {
                if user_initiated {
                    st.guard.reset_failures_on_user_action();
                }
                st.desired_hidden = hide && !st.guard.is_tripped();
            }
            if !hide {
                st.flyout_active = false;
                st.flyout_suppressed = false;
                st.synced_flyout_suppressed = None;
                st.synced_flyout_fallback_colors = None;
                st.synced_modal_style = None;
                st.flyout_bounds.clear();
            }
            st.wake_seq = st.wake_seq.wrapping_add(1);
            hide || st.attached_pid.is_some()
        } else {
            false
        };

        if need_worker {
            self.ensure_worker_started();
        }
        if self.worker.is_some() {
            self.shared.1.set();
        }
    }

    pub fn maintain(&mut self, target_pid: u32) {
        if target_pid == 0 {
            return;
        }
        let now = Instant::now();
        let mut should_wake = false;
        if let Ok(mut st) = self.shared.0.lock() {
            let exit_kind = st.inspect_old_pid_exit_kind(Some(target_pid));
            if st.guard.on_explorer_pid_changed(target_pid, exit_kind, now) {
                st.desired_hidden = false;
                st.attached_pid = None;
                st.last_ping_ok = None;
                st.flyout_active = false;
                st.synced_flyout_suppressed = None;
                st.synced_flyout_fallback_colors = None;
                st.synced_modal_style = None;
                st.flyout_bounds.clear();
                st.target_pid = target_pid;
                st.track_process_pid(target_pid);
                return;
            }

            if st.target_pid != 0 && st.target_pid != target_pid {
                st.guard.reset_failures_on_new_pid();
                st.last_inject_try = None;
                st.idle_inject_failures = 0;
            }
            st.target_pid = target_pid;
            st.track_process_pid(target_pid);
            if st.attached_pid.is_some_and(|p| p != target_pid) {
                st.attached_pid = None;
                st.last_ping_ok = None;
                st.flyout_active = false;
                st.synced_flyout_suppressed = None;
                st.synced_flyout_fallback_colors = None;
                st.synced_modal_style = None;
                st.flyout_bounds.clear();
            }

            if st.desired_hidden && !st.guard.is_tripped() {
                let ping_due = st
                    .last_ping_ok
                    .is_none_or(|t| now.saturating_duration_since(t) >= MAINTAIN_PING_INTERVAL);
                let attach_cooldown = if st.flyout_active {
                    Duration::from_millis(500)
                } else {
                    super::shell_flyout_idle_inject_backoff(st.idle_inject_failures)
                };
                let need_attach = st.attached_pid != Some(target_pid)
                    && st
                        .last_inject_try
                        .is_none_or(|t| now.saturating_duration_since(t) >= attach_cooldown);
                if need_attach || (st.attached_pid == Some(target_pid) && ping_due) {
                    st.wake_seq = st.wake_seq.wrapping_add(1);
                    should_wake = true;
                }
            }
        }
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.set();
        }
    }

    pub fn request_flyout_rects(&mut self, target_pid: u32) {
        if target_pid == 0 {
            return;
        }
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            st.flyout_active = true;
            if st.desired_hidden
                && !st.flyout_suppressed
                && !st.guard.is_tripped()
                && !st.stop_worker
            {
                st.target_pid = target_pid;
                st.query_seq = st.query_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.set();
        }
    }

    pub fn clear_flyout_rects(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.flyout_active = false;
            st.flyout_bounds.clear();
        }
    }

    pub fn cached_flyout_rects(&self) -> Vec<AltTabBoundsCache> {
        self.shared
            .0
            .lock()
            .map(|st| st.flyout_bounds.clone())
            .unwrap_or_default()
    }

    pub fn set_flyout_suppressed(&mut self, target_pid: u32, suppressed: bool) {
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            let changed = st.flyout_suppressed != suppressed;
            st.flyout_suppressed = suppressed;
            if suppressed {
                st.flyout_bounds.clear();
            }
            if target_pid != 0 {
                st.target_pid = target_pid;
            }
            if changed && st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker {
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            if target_pid != 0 {
                self.ensure_worker_started();
            }
            if self.worker.is_some() {
                self.shared.1.set();
            }
        }
    }

    pub fn set_flyout_fallback_colors(
        &mut self,
        target_pid: u32,
        colors: Option<(u32, u32)>,
    ) {
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            let changed = st.flyout_fallback_colors != colors;
            st.flyout_fallback_colors = colors;
            if colors.is_none() {
                st.synced_flyout_fallback_colors = None;
            }
            if target_pid != 0 {
                st.target_pid = target_pid;
            }
            if changed
                && colors.is_some()
                && st.desired_hidden
                && !st.guard.is_tripped()
                && !st.stop_worker
            {
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            if target_pid != 0 {
                self.ensure_worker_started();
            }
            if self.worker.is_some() {
                self.shared.1.set();
            }
        }
    }

    pub fn set_modal_style(&mut self, target_pid: u32, style: Option<(u8, bool, u32, u32)>) {
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            let changed = st.modal_style != style;
            st.modal_style = style;
            if style.is_none() {
                st.synced_modal_style = None;
            }
            if target_pid != 0 {
                st.target_pid = target_pid;
            }
            if changed
                && style.is_some()
                && st.desired_hidden
                && !st.guard.is_tripped()
                && !st.stop_worker
            {
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            if target_pid != 0 {
                self.ensure_worker_started();
            }
            if self.worker.is_some() {
                self.shared.1.set();
            }
        }
    }

    /// Triggers a throttled reconnect/injection retry or dirty-state flush when a hosted
    /// flyout opens (without resetting a tripped crash guard).
    pub fn retry_attach_on_flyout_open(&mut self, target_pid: u32) {
        if target_pid == 0 {
            return;
        }
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            st.flyout_active = true;
            if !st.desired_hidden || st.guard.is_tripped() || st.stop_worker {
                false
            } else if st.attached_pid != Some(target_pid) {
                if st
                    .last_inject_try
                    .is_none_or(|t| t.elapsed() >= Duration::from_millis(500))
                {
                    st.guard.reset_failures_on_user_action();
                    st.target_pid = target_pid;
                    st.wake_seq = st.wake_seq.wrapping_add(1);
                    true
                } else {
                    false
                }
            } else if st.synced_flyout_suppressed != Some(st.flyout_suppressed)
                || (st.flyout_fallback_colors.is_some()
                    && st.flyout_fallback_colors != st.synced_flyout_fallback_colors)
                || (st.modal_style.is_some() && st.modal_style != st.synced_modal_style)
            {
                st.target_pid = target_pid;
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            self.ensure_worker_started();
            if self.worker.is_some() {
                self.shared.1.set();
            }
        }
    }

    pub fn begin_shutdown(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.desired_hidden = false;
            st.flyout_active = false;
            st.stop_worker = true;
            st.wake_seq = st.wake_seq.wrapping_add(1);
        }
        self.shared.1.set();
    }

    pub fn finish_shutdown(&mut self, deadline: Instant) {
        let Some(handle) = self.worker.take() else {
            return;
        };
        let timeout = deadline.saturating_duration_since(Instant::now());
        let (lock, _, done_cv) = &*self.shared;
        let exited_cleanly = if let Ok(st) = lock.lock() {
            if st.shutdown_done {
                true
            } else {
                done_cv
                    .wait_timeout_while(st, timeout, |s| !s.shutdown_done)
                    .is_ok_and(|(guard, _)| guard.shutdown_done)
            }
        } else {
            false
        };
        if exited_cleanly {
            let _ = handle.join();
        }
    }

    pub fn shutdown(&mut self) {
        self.begin_shutdown();
        self.finish_shutdown(Instant::now() + Duration::from_millis(800));
    }

    fn ensure_worker_started(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        self.worker = Some(std::thread::spawn(move || run_shell_flyout_worker(shared)));
    }
}

impl Drop for ShellFlyoutTapClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

unsafe fn pipe_write_exact_timeout(
    pipe: HANDLE,
    io_event: HANDLE,
    buf: &[u8],
    timeout_ms: u32,
) -> bool {
    let mut off = 0usize;
    while off < buf.len() {
        let _ = ResetEvent(io_event);
        let mut ov = OVERLAPPED {
            hEvent: io_event,
            ..Default::default()
        };
        let res = WriteFile(pipe, Some(&buf[off..]), None, Some(&mut ov));
        let put = if res.is_ok() {
            let mut tr = 0u32;
            if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
                return false;
            }
            tr as usize
        } else if GetLastError() == ERROR_IO_PENDING {
            if WaitForSingleObject(io_event, timeout_ms) != WAIT_OBJECT_0 {
                let _ = CancelIo(pipe);
                let mut tr = 0u32;
                let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                return false;
            }
            let mut tr = 0u32;
            if GetOverlappedResult(pipe, &ov, &mut tr, true).is_err() {
                return false;
            }
            tr as usize
        } else {
            return false;
        };
        if put == 0 {
            return false;
        }
        off += put;
    }
    true
}

unsafe fn pipe_read_exact_timeout(
    pipe: HANDLE,
    io_event: HANDLE,
    buf: &mut [u8],
    timeout_ms: u32,
) -> bool {
    let mut off = 0usize;
    while off < buf.len() {
        let _ = ResetEvent(io_event);
        let mut ov = OVERLAPPED {
            hEvent: io_event,
            ..Default::default()
        };
        let res = ReadFile(pipe, Some(&mut buf[off..]), None, Some(&mut ov));
        let got = if res.is_ok() {
            let mut tr = 0u32;
            if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
                return false;
            }
            tr as usize
        } else if GetLastError() == ERROR_IO_PENDING {
            if WaitForSingleObject(io_event, timeout_ms) != WAIT_OBJECT_0 {
                let _ = CancelIo(pipe);
                let mut tr = 0u32;
                let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                return false;
            }
            let mut tr = 0u32;
            if GetOverlappedResult(pipe, &ov, &mut tr, true).is_err() {
                return false;
            }
            tr as usize
        } else {
            return false;
        };
        if got == 0 {
            return false;
        }
        off += got;
    }
    true
}

unsafe fn pipe_send_simple(pipe: HANDLE, io_event: HANDLE, cmd: u8, wait_ms: u32) -> bool {
    let host_pid = GetCurrentProcessId().to_le_bytes();
    let req = [cmd, host_pid[0], host_pid[1], host_pid[2], host_pid[3]];
    if !pipe_write_exact_timeout(pipe, io_event, &req, wait_ms) {
        return false;
    }
    let mut ack = [0u8; 1];
    pipe_read_exact_timeout(pipe, io_event, &mut ack, wait_ms) && ack[0] == cmd
}

unsafe fn pipe_send_bool(
    pipe: HANDLE,
    io_event: HANDLE,
    cmd: u8,
    flag: bool,
    wait_ms: u32,
) -> bool {
    let host_pid = GetCurrentProcessId().to_le_bytes();
    let req = [
        cmd,
        host_pid[0],
        host_pid[1],
        host_pid[2],
        host_pid[3],
        u8::from(flag),
    ];
    if !pipe_write_exact_timeout(pipe, io_event, &req, wait_ms) {
        return false;
    }
    let mut ack = [0u8; 1];
    pipe_read_exact_timeout(pipe, io_event, &mut ack, wait_ms)
        && (ack[0] == cmd || ack[0] == 0xFF)
}

unsafe fn pipe_send_style_colors(
    pipe: HANDLE,
    io_event: HANDLE,
    cmd: u8,
    bg_argb: u32,
    border_argb: u32,
    wait_ms: u32,
) -> bool {
    let host_pid = GetCurrentProcessId().to_le_bytes();
    let bg = bg_argb.to_le_bytes();
    let bd = border_argb.to_le_bytes();
    let req = [
        cmd,
        host_pid[0],
        host_pid[1],
        host_pid[2],
        host_pid[3],
        bg[0],
        bg[1],
        bg[2],
        bg[3],
        bd[0],
        bd[1],
        bd[2],
        bd[3],
    ];
    if !pipe_write_exact_timeout(pipe, io_event, &req, wait_ms) {
        return false;
    }
    let mut ack = [0u8; 1];
    pipe_read_exact_timeout(pipe, io_event, &mut ack, wait_ms)
        && (ack[0] == cmd || ack[0] == 0xFF)
}

unsafe fn pipe_send_start_modal_style(
    pipe: HANDLE,
    io_event: HANDLE,
    style_id: u8,
    light_glass: bool,
    halo0_rgb: u32,
    halo1_rgb: u32,
    wait_ms: u32,
) -> bool {
    let host_pid = GetCurrentProcessId().to_le_bytes();
    let h0 = halo0_rgb.to_le_bytes();
    let h1 = halo1_rgb.to_le_bytes();
    let req = [
        CMD_SET_START_MODAL_STYLE,
        host_pid[0],
        host_pid[1],
        host_pid[2],
        host_pid[3],
        style_id,
        u8::from(light_glass),
        h0[0],
        h0[1],
        h0[2],
        h0[3],
        h1[0],
        h1[1],
        h1[2],
        h1[3],
    ];
    if !pipe_write_exact_timeout(pipe, io_event, &req, wait_ms) {
        return false;
    }
    let mut ack = [0u8; 1];
    pipe_read_exact_timeout(pipe, io_event, &mut ack, wait_ms)
        && (ack[0] == CMD_SET_START_MODAL_STYLE || ack[0] == 0xFF)
}

unsafe fn pipe_query_menu_flyout_rects(
    pipe: HANDLE,
    io_event: HANDLE,
    wait_ms: u32,
) -> Result<Vec<(u32, [f32; 4])>, ()> {
    let host_pid = GetCurrentProcessId().to_le_bytes();
    let req = [
        CMD_MENU_FLYOUT_RECTS,
        host_pid[0],
        host_pid[1],
        host_pid[2],
        host_pid[3],
    ];
    if !pipe_write_exact_timeout(pipe, io_event, &req, wait_ms) {
        return Err(());
    }
    let mut tag = [0u8; 1];
    if !pipe_read_exact_timeout(pipe, io_event, &mut tag, wait_ms) {
        return Err(());
    }
    if tag[0] == 0xFF {
        return Ok(Vec::new());
    }
    if tag[0] != CMD_MENU_FLYOUT_RECTS {
        return Err(());
    }
    let mut payload = [0u8; 1 + MAX_MENU_FLYOUT_RECTS * 20];
    if !pipe_read_exact_timeout(pipe, io_event, &mut payload, wait_ms) {
        return Err(());
    }
    Ok(decode_flyout_rects_payload(&payload, MAX_MENU_FLYOUT_RECTS))
}

unsafe fn pipe_query_shell_card_rects(
    pipe: HANDLE,
    io_event: HANDLE,
    wait_ms: u32,
) -> Result<Vec<(u32, [f32; 4])>, ()> {
    let host_pid = GetCurrentProcessId().to_le_bytes();
    let req = [
        CMD_SHELL_CARD_RECTS,
        host_pid[0],
        host_pid[1],
        host_pid[2],
        host_pid[3],
    ];
    if !pipe_write_exact_timeout(pipe, io_event, &req, wait_ms) {
        return Err(());
    }
    let mut tag = [0u8; 1];
    if !pipe_read_exact_timeout(pipe, io_event, &mut tag, wait_ms) {
        return Err(());
    }
    if tag[0] == 0xFF {
        return pipe_query_menu_flyout_rects(pipe, io_event, wait_ms);
    }
    if tag[0] != CMD_SHELL_CARD_RECTS {
        return Err(());
    }
    let mut payload = [0u8; 1 + MAX_SHELL_CARD_RECTS * 20];
    if !pipe_read_exact_timeout(pipe, io_event, &mut payload, wait_ms) {
        return Err(());
    }
    Ok(decode_flyout_rects_payload(&payload, MAX_SHELL_CARD_RECTS))
}

fn create_reverse_pipe_server(
    pipe_prefix: &str,
    target_pid: u32,
    grant_appcontainer_acl: bool,
) -> Option<HANDLE> {
    let pipe_wide = reverse_pipe_name_wide(pipe_prefix, target_pid);
    unsafe {
        let owner_sid = current_user_sid_string().unwrap_or_else(|| "OW".to_string());
        let sddl = if grant_appcontainer_acl {
            format!(
                "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{owner_sid})(A;;GRGW;;;S-1-15-2-1)(A;;GRGW;;;S-1-15-2-2)S:(ML;;NW;;;LW)"
            )
        } else {
            format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{owner_sid})")
        };
        let sddl_wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_wide.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
        .is_err()
            || sd.0.is_null()
        {
            return None;
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let pipe = CreateNamedPipeW(
            PCWSTR(pipe_wide.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            512,
            512,
            0,
            Some(&sa),
        );
        let _ = LocalFree(HLOCAL(sd.0));
        if pipe == INVALID_HANDLE_VALUE || pipe.is_invalid() {
            None
        } else {
            Some(pipe)
        }
    }
}

enum ShellFlyoutPipeState {
    Disconnected,
    Listening {
        pid: u32,
        pipe: HANDLE,
        ov: Box<OVERLAPPED>,
    },
    Connected {
        pid: u32,
        pipe: HANDLE,
        synced_hide: Option<bool>,
    },
}

impl ShellFlyoutPipeState {
    fn release_raw_handles(&mut self) {
        match self {
            ShellFlyoutPipeState::Disconnected => {}
            ShellFlyoutPipeState::Listening { pipe, ov, .. } => {
                let handle = std::mem::take(pipe);
                if !handle.is_invalid() && !handle.0.is_null() {
                    unsafe {
                        let _ = CancelIo(handle);
                        let mut tr = 0u32;
                        let _ = GetOverlappedResult(handle, &**ov, &mut tr, true);
                        let _ = CloseHandle(handle);
                    }
                }
            }
            ShellFlyoutPipeState::Connected { pipe, .. } => {
                let handle = std::mem::take(pipe);
                if !handle.is_invalid() && !handle.0.is_null() {
                    unsafe {
                        let _ = DisconnectNamedPipe(handle);
                        let _ = CloseHandle(handle);
                    }
                }
            }
        }
    }

    fn close_handles(&mut self) {
        self.release_raw_handles();
        if !matches!(self, ShellFlyoutPipeState::Disconnected) {
            *self = ShellFlyoutPipeState::Disconnected;
        }
    }
}

impl Drop for ShellFlyoutPipeState {
    fn drop(&mut self) {
        self.release_raw_handles();
    }
}

fn close_pipe_state(
    pipe_state: &mut ShellFlyoutPipeState,
    lock: &Mutex<ShellFlyoutWorkerShared>,
    broken_pid: Option<u32>,
) {
    pipe_state.close_handles();
    if let Ok(mut st) = lock.lock() {
        if broken_pid.is_none_or(|pid| st.attached_pid == Some(pid)) {
            st.attached_pid = None;
            st.last_ping_ok = None;
        }
        st.synced_flyout_suppressed = None;
        st.synced_flyout_fallback_colors = None;
        st.synced_modal_style = None;
        st.flyout_bounds.clear();
    }
}

fn mark_shell_flyout_connected(lock: &Mutex<ShellFlyoutWorkerShared>, pid: u32) {
    let notify = if let Ok(mut st) = lock.lock() {
        st.attached_pid = Some(pid);
        st.last_ping_ok = Some(Instant::now());
        st.idle_inject_failures = 0;
        st.synced_flyout_suppressed = None;
        st.synced_flyout_fallback_colors = None;
        st.synced_modal_style = None;
        st.track_process_pid(pid);
        st.guard.record_success();
        (st.notify_hwnd_raw, st.notify_msg)
    } else {
        (0, 0)
    };
    if notify.0 != 0 && notify.1 != 0 {
        unsafe {
            let _ = PostMessageW(
                HWND(notify.0 as *mut std::ffi::c_void),
                notify.1,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

fn try_complete_listening_pipe(
    pipe_state: &mut ShellFlyoutPipeState,
    lock: &Mutex<ShellFlyoutWorkerShared>,
) -> bool {
    let (connected_pid, invalid_client) = match pipe_state {
        ShellFlyoutPipeState::Listening { pid, pipe, ov } => {
            let target_pid = *pid;
            let handle = *pipe;
            let mut tr = 0u32;
            if unsafe { GetOverlappedResult(handle, &**ov, &mut tr, false).is_ok() } {
                if unsafe { verify_pipe_client_pid(handle, target_pid) } {
                    let taken = std::mem::take(pipe);
                    *pipe_state = ShellFlyoutPipeState::Connected {
                        pid: target_pid,
                        pipe: taken,
                        synced_hide: None,
                    };
                    (Some(target_pid), None)
                } else {
                    let taken = std::mem::take(pipe);
                    (None, Some(taken))
                }
            } else {
                (None, None)
            }
        }
        _ => return false,
    };
    if let Some(handle) = invalid_client {
        unsafe {
            let _ = DisconnectNamedPipe(handle);
            let _ = CloseHandle(handle);
        }
    }
    if let Some(pid) = connected_pid {
        mark_shell_flyout_connected(lock, pid);
        true
    } else {
        close_pipe_state(pipe_state, lock, None);
        false
    }
}

fn try_start_listening_pipe(
    pipe_prefix: &str,
    target_pid: u32,
    grant_appcontainer_acl: bool,
    conn_event: HANDLE,
    pipe_state: &mut ShellFlyoutPipeState,
    lock: &Mutex<ShellFlyoutWorkerShared>,
) -> bool {
    if !matches!(pipe_state, ShellFlyoutPipeState::Disconnected) || target_pid == 0 {
        return false;
    }
    let Some(pipe) = create_reverse_pipe_server(pipe_prefix, target_pid, grant_appcontainer_acl)
    else {
        return false;
    };
    unsafe {
        let _ = ResetEvent(conn_event);
        let mut ov = Box::new(OVERLAPPED {
            hEvent: conn_event,
            ..Default::default()
        });
        let conn_res = ConnectNamedPipe(pipe, Some(&mut *ov));
        if conn_res.is_ok() || GetLastError() == ERROR_PIPE_CONNECTED {
            if verify_pipe_client_pid(pipe, target_pid) {
                *pipe_state = ShellFlyoutPipeState::Connected {
                    pid: target_pid,
                    pipe,
                    synced_hide: None,
                };
                mark_shell_flyout_connected(lock, target_pid);
                true
            } else {
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
                false
            }
        } else if GetLastError() == ERROR_IO_PENDING {
            *pipe_state = ShellFlyoutPipeState::Listening {
                pid: target_pid,
                pipe,
                ov,
            };
            false
        } else {
            let _ = CloseHandle(pipe);
            false
        }
    }
}

fn run_shell_flyout_worker(shared: Arc<(Mutex<ShellFlyoutWorkerShared>, OwnedEvent, Condvar)>) {
    let _com = ComMtaGuard::enter();
    let (lock, wake_event, done_cv) = &*shared;
    if wake_event.handle().0.is_null() || wake_event.handle() == INVALID_HANDLE_VALUE {
        if let Ok(mut st) = lock.lock() {
            st.shutdown_done = true;
        }
        done_cv.notify_all();
        return;
    }
    let Ok(conn_event) = (unsafe { CreateEventW(None, true, false, None) }) else {
        if let Ok(mut st) = lock.lock() {
            st.shutdown_done = true;
        }
        done_cv.notify_all();
        return;
    };
    let Ok(io_event) = (unsafe { CreateEventW(None, true, false, None) }) else {
        unsafe {
            let _ = CloseHandle(conn_event);
        }
        if let Ok(mut st) = lock.lock() {
            st.shutdown_done = true;
        }
        done_cv.notify_all();
        return;
    };

    let (pipe_prefix, grant_appcontainer_acl) = lock
        .lock()
        .map(|st| (st.pipe_prefix, st.grant_appcontainer_acl))
        .unwrap_or((r"\\.\pipe\PrettyWindows_ShellExpTap_", true));

    let mut pipe_state = ShellFlyoutPipeState::Disconnected;
    let mut just_connected = false;

    loop {
        let (desired_hidden, target_pid, stop_worker, want_query, flyout_active) = loop {
            let pending = {
                let Ok(mut st) = lock.lock() else {
                    close_pipe_state(&mut pipe_state, lock, None);
                    unsafe {
                        let _ = CloseHandle(io_event);
                        let _ = CloseHandle(conn_event);
                    }
                    return;
                };
                wake_event.reset();
                let desired = st.desired_hidden && !st.guard.is_tripped();
                let need_sync_hide = matches!(
                    pipe_state,
                    ShellFlyoutPipeState::Connected {
                        synced_hide: None,
                        ..
                    }
                );
                let has_work = st.stop_worker
                    || st.wake_seq != st.last_processed_seq
                    || st.query_seq != st.last_query_seq
                    || need_sync_hide
                    || just_connected;
                if has_work {
                    let query_dirty = st.query_seq != st.last_query_seq;
                    st.last_processed_seq = st.wake_seq;
                    st.last_query_seq = st.query_seq;
                    Some((
                        desired,
                        st.target_pid,
                        st.stop_worker,
                        query_dirty,
                        st.flyout_active,
                    ))
                } else {
                    None
                }
            };
            if let Some(work) = pending {
                break work;
            }

            if matches!(pipe_state, ShellFlyoutPipeState::Listening { .. }) {
                let handles = [wake_event.handle(), conn_event];
                let wait_res = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
                if wait_res == WAIT_EVENT(WAIT_OBJECT_0.0 + 1) {
                    if try_complete_listening_pipe(&mut pipe_state, lock) {
                        just_connected = true;
                    }
                } else if wait_res != WAIT_OBJECT_0 {
                    close_pipe_state(&mut pipe_state, lock, None);
                }
            } else {
                let wait_res = unsafe { WaitForSingleObject(wake_event.handle(), INFINITE) };
                if wait_res != WAIT_OBJECT_0 {
                    break (false, 0, true, false, false);
                }
            }
        };

        let was_just_connected = std::mem::take(&mut just_connected);

        if stop_worker {
            if let ShellFlyoutPipeState::Connected { pipe, .. } = pipe_state {
                unsafe {
                    let _ = pipe_send_simple(pipe, io_event, CMD_RESTORE, 300);
                }
            }
            close_pipe_state(&mut pipe_state, lock, None);
            if let Ok(mut st) = lock.lock() {
                st.flyout_active = false;
                st.flyout_suppressed = false;
                st.shutdown_done = true;
            }
            done_cv.notify_all();
            break;
        }

        let mismatched_pid = match &pipe_state {
            ShellFlyoutPipeState::Listening { pid, .. }
            | ShellFlyoutPipeState::Connected { pid, .. } => *pid != target_pid,
            ShellFlyoutPipeState::Disconnected => false,
        };
        if mismatched_pid {
            close_pipe_state(&mut pipe_state, lock, None);
            if let Ok(mut st) = lock.lock() {
                st.last_inject_try = None;
                st.idle_inject_failures = 0;
            }
        }

        if target_pid == 0 {
            continue;
        }

        if !desired_hidden {
            if let ShellFlyoutPipeState::Connected {
                pipe, synced_hide, ..
            } = pipe_state
            {
                if synced_hide != Some(false) {
                    unsafe {
                        let _ = pipe_send_simple(pipe, io_event, CMD_RESTORE, 300);
                    }
                }
            }
            close_pipe_state(&mut pipe_state, lock, None);
            continue;
        }

        if matches!(pipe_state, ShellFlyoutPipeState::Disconnected)
            && try_start_listening_pipe(
                pipe_prefix,
                target_pid,
                grant_appcontainer_acl,
                conn_event,
                &mut pipe_state,
                lock,
            )
        {
            just_connected = true;
            continue;
        }

        if matches!(pipe_state, ShellFlyoutPipeState::Listening { .. })
            && unsafe { WaitForSingleObject(conn_event, 0) } == WAIT_OBJECT_0
            && try_complete_listening_pipe(&mut pipe_state, lock)
        {
            just_connected = true;
            continue;
        }

        if let ShellFlyoutPipeState::Listening { pid, .. } = pipe_state {
            let (can_try, is_flyout_active) = lock
                .lock()
                .map(|st| {
                    let active = flyout_active || st.flyout_active;
                    let inject_cooldown = if active {
                        Duration::from_millis(500)
                    } else {
                        super::shell_flyout_idle_inject_backoff(st.idle_inject_failures)
                    };
                    let allow = st.guard.can_inject()
                        && st
                            .last_inject_try
                            .is_none_or(|t| t.elapsed() >= inject_cooldown);
                    (allow, active)
                })
                .unwrap_or((false, flyout_active));
            if can_try {
                if is_any_tap_dll_loaded_in_process(pid) {
                    if let Ok(mut st) = lock.lock() {
                        st.last_inject_try = Some(Instant::now());
                        st.idle_inject_failures = 0;
                    }
                } else {
                    if let Ok(mut st) = lock.lock() {
                        st.track_process_pid(pid);
                        let now = Instant::now();
                        st.last_inject_try = Some(now);
                        st.guard.record_inject_attempt(pid, now);
                    }
                    let inject_ok = prepare_tap_dll_for_injection(grant_appcontainer_acl)
                        .is_some_and(|dll_path| invoke_xaml_diagnostics_ex(pid, &dll_path));
                    if inject_ok {
                        if let Ok(mut st) = lock.lock() {
                            st.idle_inject_failures = 0;
                        }
                    } else if is_flyout_active {
                        if let Ok(mut st) = lock.lock() {
                            st.guard.record_failure();
                        }
                    } else {
                        let alive = unsafe { is_process_alive(pid) };
                        if let Ok(mut st) = lock.lock() {
                            st.idle_inject_failures =
                                st.idle_inject_failures.saturating_add(1);
                            if alive {
                                st.guard.clear_failed_attempt_if_same_pid(pid);
                            } else {
                                st.guard.record_failure();
                            }
                        }
                    }
                }
            }
            continue;
        }

        if let ShellFlyoutPipeState::Connected {
            pid,
            pipe,
            synced_hide,
        } = pipe_state
        {
            if synced_hide != Some(true) {
                if unsafe { pipe_send_simple(pipe, io_event, CMD_HIDE, 500) } {
                    if let ShellFlyoutPipeState::Connected { synced_hide, .. } = &mut pipe_state
                    {
                        *synced_hide = Some(true);
                    }
                    if let Ok(mut st) = lock.lock() {
                        st.last_ping_ok = Some(Instant::now());
                        st.guard.record_success();
                    }
                } else {
                    close_pipe_state(&mut pipe_state, lock, Some(pid));
                    if try_start_listening_pipe(
                        pipe_prefix,
                        pid,
                        grant_appcontainer_acl,
                        conn_event,
                        &mut pipe_state,
                        lock,
                    ) {
                        just_connected = true;
                    }
                    continue;
                }
            } else if !was_just_connected && !flyout_active && !want_query {
                // Idle inspection: use non-blocking PeekNamedPipe so PLM-suspended
                // host threads are never woken or falsely disconnected.
                let alive =
                    unsafe { PeekNamedPipe(pipe, None, 0, None, None, None).is_ok() };
                if alive {
                    if let Ok(mut st) = lock.lock() {
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    close_pipe_state(&mut pipe_state, lock, Some(pid));
                    if try_start_listening_pipe(
                        pipe_prefix,
                        pid,
                        grant_appcontainer_acl,
                        conn_event,
                        &mut pipe_state,
                        lock,
                    ) {
                        just_connected = true;
                    }
                }
                continue;
            }
        }

        if let ShellFlyoutPipeState::Connected { pid, pipe, .. } = pipe_state {
            let desired_colors = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.flyout_fallback_colors != st.synced_flyout_fallback_colors)
                    .then_some(st.flyout_fallback_colors)
                    .flatten()
            });
            if let Some((bg_argb, border_argb)) = desired_colors {
                if unsafe {
                    pipe_send_style_colors(
                        pipe,
                        io_event,
                        CMD_SET_MENU_FLYOUT_FALLBACK_STYLE,
                        bg_argb,
                        border_argb,
                        500,
                    )
                } {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_flyout_fallback_colors = Some((bg_argb, border_argb));
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    close_pipe_state(&mut pipe_state, lock, Some(pid));
                    if try_start_listening_pipe(
                        pipe_prefix,
                        pid,
                        grant_appcontainer_acl,
                        conn_event,
                        &mut pipe_state,
                        lock,
                    ) {
                        just_connected = true;
                    }
                    continue;
                }
            }
        }

        if let ShellFlyoutPipeState::Connected { pid, pipe, .. } = pipe_state {
            let desired_modal = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.modal_style != st.synced_modal_style)
                    .then_some(st.modal_style)
                    .flatten()
            });
            if let Some((style_id, light_glass, halo0_rgb, halo1_rgb)) = desired_modal {
                if unsafe {
                    pipe_send_start_modal_style(
                        pipe,
                        io_event,
                        style_id,
                        light_glass,
                        halo0_rgb,
                        halo1_rgb,
                        500,
                    )
                } {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_modal_style = Some((style_id, light_glass, halo0_rgb, halo1_rgb));
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    close_pipe_state(&mut pipe_state, lock, Some(pid));
                    if try_start_listening_pipe(
                        pipe_prefix,
                        pid,
                        grant_appcontainer_acl,
                        conn_event,
                        &mut pipe_state,
                        lock,
                    ) {
                        just_connected = true;
                    }
                    continue;
                }
            }
        }

        if let ShellFlyoutPipeState::Connected { pid, pipe, .. } = pipe_state {
            let desired_suppress = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.synced_flyout_suppressed != Some(st.flyout_suppressed))
                    .then_some(st.flyout_suppressed)
            });
            if let Some(suppressed) = desired_suppress {
                if unsafe {
                    pipe_send_bool(pipe, io_event, CMD_SET_MENU_FLYOUT_SUPPRESS, suppressed, 500)
                } {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_flyout_suppressed = Some(suppressed);
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    close_pipe_state(&mut pipe_state, lock, Some(pid));
                    if try_start_listening_pipe(
                        pipe_prefix,
                        pid,
                        grant_appcontainer_acl,
                        conn_event,
                        &mut pipe_state,
                        lock,
                    ) {
                        just_connected = true;
                    }
                    continue;
                }
            }
        }

        if want_query {
            let query_target = match pipe_state {
                ShellFlyoutPipeState::Connected { pid, pipe, .. } => {
                    let allow = lock
                        .lock()
                        .map(|st| {
                            st.desired_hidden
                                && !st.flyout_suppressed
                                && !st.guard.is_tripped()
                                && !st.stop_worker
                        })
                        .unwrap_or(false);
                    allow.then_some((pid, pipe))
                }
                _ => None,
            };
            if let Some((pid, pipe)) = query_target {
                match unsafe { pipe_query_shell_card_rects(pipe, io_event, 500) } {
                    Ok(rects) => {
                        let now = Instant::now();
                        let next_bounds: Vec<AltTabBoundsCache> = rects
                            .into_iter()
                            .map(|(thread_id, rect_dip)| AltTabBoundsCache {
                                thread_id,
                                rect_dip,
                                updated_at: now,
                            })
                            .collect();
                        let notify = if let Ok(mut st) = lock.lock() {
                            let changed = st.flyout_bounds.len() != next_bounds.len()
                                || st.flyout_bounds.iter().zip(next_bounds.iter()).any(
                                    |(prev, next)| {
                                        prev.thread_id != next.thread_id
                                            || (prev.rect_dip[0] - next.rect_dip[0]).abs() > 0.25
                                            || (prev.rect_dip[1] - next.rect_dip[1]).abs() > 0.25
                                            || (prev.rect_dip[2] - next.rect_dip[2]).abs() > 0.25
                                            || (prev.rect_dip[3] - next.rect_dip[3]).abs() > 0.25
                                    },
                                );
                            st.flyout_bounds = next_bounds;
                            st.last_ping_ok = Some(now);
                            if changed {
                                (st.notify_hwnd_raw, st.notify_msg)
                            } else {
                                (0, 0)
                            }
                        } else {
                            (0, 0)
                        };
                        if notify.0 != 0 && notify.1 != 0 {
                            unsafe {
                                let _ = PostMessageW(
                                    HWND(notify.0 as *mut std::ffi::c_void),
                                    notify.1,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        }
                    }
                    Err(()) => {
                        close_pipe_state(&mut pipe_state, lock, Some(pid));
                        if try_start_listening_pipe(
                            pipe_prefix,
                            pid,
                            grant_appcontainer_acl,
                            conn_event,
                            &mut pipe_state,
                            lock,
                        ) {
                            just_connected = true;
                        }
                    }
                }
            } else if let Ok(mut st) = lock.lock() {
                st.flyout_bounds.clear();
            }
        }
    }

    close_pipe_state(&mut pipe_state, lock, None);
    unsafe {
        let _ = CloseHandle(io_event);
        let _ = CloseHandle(conn_event);
    }
}

