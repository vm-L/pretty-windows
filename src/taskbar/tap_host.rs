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
        cleanup_stale_shadow_dlls, prepare_shadow_dll_in_dir, TapSafetyGuard, SHADOW_PREFIX,
    };
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr::null_mut;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use windows::core::{s, w, GUID, HRESULT, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_BAD_LENGTH, ERROR_IO_PENDING,
        ERROR_PIPE_CONNECTED, ERROR_SUCCESS, GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE, HANDLE,
        HLOCAL, HWND, INVALID_HANDLE_VALUE, LPARAM, WAIT_EVENT, WAIT_OBJECT_0, WPARAM,
    };
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW,
        GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
        GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SDDL_REVISION_1, SE_FILE_OBJECT, TRUSTEE_IS_SID,
        TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
    };
    use windows::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID,
        SECURITY_ATTRIBUTES, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED,
        FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
        MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};
    use windows::Win32::System::LibraryLoader::{
        GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PeekNamedPipe, WaitNamedPipeW,
        PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, GetCurrentProcessId, ResetEvent, SetEvent, WaitForMultipleObjects,
        WaitForSingleObject, INFINITE,
    };
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

    #[link(name = "kernel32")]
    extern "system" {
        fn ProcessIdToSessionId(dw_process_id: u32, p_session_id: *mut u32) -> i32;
    }

    /// Shared protocol with `tap/src/lib.rs` (`PrettyWindowsTap.dll`).
    const CLSID_PRETTY_WINDOWS_TAP_U128: u128 = 0x8a4e6f21_9c3b_4d7a_b1e5_3f7c2a9d8410;
    const CLSID_PRETTY_WINDOWS_TAP: GUID = GUID::from_u128(CLSID_PRETTY_WINDOWS_TAP_U128);

    const CMD_RESTORE: u8 = 0x00;
    const CMD_HIDE: u8 = 0x01;
    const CMD_PING: u8 = 0x02;
    const CMD_SHUTDOWN: u8 = 0x03;
    const CMD_VERSION: u8 = 0x05;
    const CMD_ALTTAB_RECT: u8 = 0x06;
    const CMD_SET_ALTTAB_ITEM_STYLE: u8 = 0x07;
    const CMD_HOVER_FLYOUT_RECT: u8 = 0x08;
    const CMD_OSD_RECT: u8 = 0x09;
    const CMD_SET_OSD_SUPPRESS: u8 = 0x0A;
    const CMD_SET_OSD_FALLBACK_STYLE: u8 = 0x0B;
    const CMD_TOOLTIP_RECT: u8 = 0x0C;
    const CMD_SET_TOOLTIP_SUPPRESS: u8 = 0x0D;
    const CMD_SET_TOOLTIP_FALLBACK_STYLE: u8 = 0x0E;
    const CMD_MENU_FLYOUT_RECTS: u8 = 0x0F;
    const CMD_SET_MENU_FLYOUT_SUPPRESS: u8 = 0x10;
    const CMD_SET_MENU_FLYOUT_FALLBACK_STYLE: u8 = 0x11;
    const CMD_SHELL_CARD_RECTS: u8 = 0x12;
    const MAX_MENU_FLYOUT_RECTS: usize = 3;
    const MAX_SHELL_CARD_RECTS: usize = 8;
    const EXPECTED_TAP_PROTOCOL_VERSION: u8 = 11;

    #[derive(Clone, Copy, Debug)]
    pub struct AltTabBoundsCache {
        pub thread_id: u32,
        pub rect_dip: [f32; 4],
        #[allow(dead_code)]
        pub updated_at: Instant,
    }

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

    fn reverse_pipe_name_wide(pipe_prefix: &str, target_pid: u32) -> Vec<u16> {
        format!("{pipe_prefix}{target_pid}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn tap_dll_path() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dll = exe.parent()?.join("PrettyWindowsTap.dll");
        dll.is_file().then_some(dll)
    }

    fn expected_shadow_dll_filename() -> Option<String> {
        let src_dll = tap_dll_path()?;
        let bytes = std::fs::read(src_dll).ok()?;
        if bytes.is_empty() {
            return None;
        }
        Some(super::shadow_dll_filename(&bytes))
    }

    fn shadow_cache_dir() -> PathBuf {
        std::env::temp_dir().join("PrettyWindowsTap")
    }

    fn find_process_pid_in_session(exe_name: &str) -> u32 {
        unsafe {
            let my_pid = GetCurrentProcessId();
            let mut my_session = 0u32;
            let have_session = ProcessIdToSessionId(my_pid, &mut my_session) != 0;

            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return 0;
            };
            if snap.is_invalid() || snap == INVALID_HANDLE_VALUE {
                return 0;
            }
            let mut pe = PROCESSENTRY32W {
                dwSize: size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut found = 0u32;
            if Process32FirstW(snap, &mut pe).is_ok() {
                loop {
                    let len = pe
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(pe.szExeFile.len());
                    let name = String::from_utf16_lossy(&pe.szExeFile[..len]);
                    if name.eq_ignore_ascii_case(exe_name) && pe.th32ProcessID != 0 {
                        if have_session {
                            let mut proc_session = 0u32;
                            if ProcessIdToSessionId(pe.th32ProcessID, &mut proc_session) != 0
                                && proc_session == my_session
                            {
                                found = pe.th32ProcessID;
                                break;
                            }
                        } else {
                            found = pe.th32ProcessID;
                            break;
                        }
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

    /// Locates the current desktop session's `ShellExperienceHost.exe` process ID, or `0`.
    pub fn find_shell_experience_host_pid() -> u32 {
        find_process_pid_in_session("ShellExperienceHost.exe")
    }

    /// Locates the current desktop session's `ShellHost.exe` process ID, or `0`.
    pub fn find_shell_host_pid() -> u32 {
        find_process_pid_in_session("ShellHost.exe")
    }

    /// Checks via a toolhelp module snapshot (works even when UWP PLM has suspended all
    /// AppContainer threads) whether the current `PrettyWindowsTap` DLL build is loaded in `target_pid`.
    fn is_current_tap_dll_loaded_in_process(
        target_pid: u32,
        expected_shadow: Option<&str>,
    ) -> bool {
        if target_pid == 0 {
            return false;
        }
        unsafe {
            let mut snap = INVALID_HANDLE_VALUE;
            for attempt in 0..3 {
                match CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, target_pid) {
                    Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE => {
                        snap = h;
                        break;
                    }
                    Err(_) if GetLastError() == ERROR_BAD_LENGTH && attempt < 2 => {
                        std::thread::yield_now();
                    }
                    _ => return false,
                }
            }
            if snap.is_invalid() || snap == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut me = MODULEENTRY32W {
                dwSize: size_of::<MODULEENTRY32W>() as u32,
                ..Default::default()
            };
            let mut loaded = false;
            if Module32FirstW(snap, &mut me).is_ok() {
                loop {
                    let len = me
                        .szModule
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(me.szModule.len());
                    let name = String::from_utf16_lossy(&me.szModule[..len]);
                    let matches_current = if let Some(exp) = expected_shadow {
                        name.eq_ignore_ascii_case(exp)
                            || name.eq_ignore_ascii_case("PrettyWindowsTap.dll")
                    } else {
                        name.get(..SHADOW_PREFIX.len())
                            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(SHADOW_PREFIX))
                            || name.eq_ignore_ascii_case("PrettyWindowsTap.dll")
                    };
                    if matches_current {
                        loaded = true;
                        break;
                    }
                    if Module32NextW(snap, &mut me).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
            loaded
        }
    }

    /// Grants `ALL APPLICATION PACKAGES` (`S-1-15-2-1`) and `ALL RESTRICTED APPLICATION PACKAGES`
    /// (`S-1-15-2-2`) read + execute access to the shadow cache directory or DLL so
    /// `ShellExperienceHost.exe` can load `PrettyWindowsTap.dll` inside AppContainer.
    fn grant_appcontainer_rx_acl(path: &Path, is_dir: bool) {
        let mut wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let mut sid_all = PSID::default();
            let mut sid_restricted = PSID::default();
            if ConvertStringSidToSidW(w!("S-1-15-2-1"), &mut sid_all).is_err()
                || sid_all.0.is_null()
            {
                return;
            }
            let have_restricted =
                ConvertStringSidToSidW(w!("S-1-15-2-2"), &mut sid_restricted).is_ok()
                    && !sid_restricted.0.is_null();

            let mut old_dacl: *mut ACL = null_mut();
            let mut sd = PSECURITY_DESCRIPTOR::default();
            if GetNamedSecurityInfoW(
                PCWSTR(wide_path.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut old_dacl),
                None,
                &mut sd,
            ) == ERROR_SUCCESS
            {
                let inheritance = if is_dir {
                    SUB_CONTAINERS_AND_OBJECTS_INHERIT
                } else {
                    NO_INHERITANCE
                };
                let perms = GENERIC_READ.0 | GENERIC_EXECUTE.0;
                let make_ea = |sid: PSID| EXPLICIT_ACCESS_W {
                    grfAccessPermissions: perms,
                    grfAccessMode: GRANT_ACCESS,
                    grfInheritance: inheritance,
                    Trustee: TRUSTEE_W {
                        pMultipleTrustee: null_mut(),
                        MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                        TrusteeForm: TRUSTEE_IS_SID,
                        TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                        ptstrName: PWSTR(sid.0.cast()),
                    },
                };
                let mut entries = [make_ea(sid_all), make_ea(sid_all)];
                let count = if have_restricted {
                    entries[1] = make_ea(sid_restricted);
                    2usize
                } else {
                    1usize
                };
                let mut new_dacl: *mut ACL = null_mut();
                if SetEntriesInAclW(Some(&entries[..count]), Some(old_dacl), &mut new_dacl)
                    == ERROR_SUCCESS
                    && !new_dacl.is_null()
                {
                    let _ = SetNamedSecurityInfoW(
                        PWSTR(wide_path.as_mut_ptr()),
                        SE_FILE_OBJECT,
                        DACL_SECURITY_INFORMATION,
                        PSID::default(),
                        PSID::default(),
                        Some(new_dacl),
                        None,
                    );
                    let _ = LocalFree(HLOCAL(new_dacl.cast()));
                }
            }
            if !sd.0.is_null() {
                let _ = LocalFree(HLOCAL(sd.0));
            }
            if !sid_all.0.is_null() {
                let _ = LocalFree(HLOCAL(sid_all.0));
            }
            if !sid_restricted.0.is_null() {
                let _ = LocalFree(HLOCAL(sid_restricted.0));
            }
        }
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
        alt_tab_query_seq: u64,
        last_alt_tab_query_seq: u64,
        alt_tab_bounds: Option<AltTabBoundsCache>,
        hover_flyout_query_seq: u64,
        last_hover_flyout_query_seq: u64,
        hover_flyout_bounds: Option<AltTabBoundsCache>,
        osd_query_seq: u64,
        last_osd_query_seq: u64,
        osd_bounds: Option<AltTabBoundsCache>,
        osd_suppressed: bool,
        synced_osd_suppressed: Option<bool>,
        osd_fallback_colors: Option<(u32, u32)>,
        synced_osd_fallback_colors: Option<(u32, u32)>,
        tooltip_query_seq: u64,
        last_tooltip_query_seq: u64,
        tooltip_bounds: Option<AltTabBoundsCache>,
        tooltip_suppressed: bool,
        synced_tooltip_suppressed: Option<bool>,
        tooltip_fallback_colors: Option<(u32, u32)>,
        synced_tooltip_fallback_colors: Option<(u32, u32)>,
        menu_flyout_query_seq: u64,
        last_menu_flyout_query_seq: u64,
        menu_flyout_bounds: Vec<AltTabBoundsCache>,
        menu_flyout_suppressed: bool,
        synced_menu_flyout_suppressed: Option<bool>,
        menu_flyout_fallback_colors: Option<(u32, u32)>,
        synced_menu_flyout_fallback_colors: Option<(u32, u32)>,
        alt_tab_item_colors: Option<(u32, u32)>,
        synced_alt_tab_item_colors: Option<(u32, u32)>,
        notify_hwnd_raw: usize,
        notify_msg: u32,
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
                        alt_tab_query_seq: 0,
                        last_alt_tab_query_seq: 0,
                        alt_tab_bounds: None,
                        hover_flyout_query_seq: 0,
                        last_hover_flyout_query_seq: 0,
                        hover_flyout_bounds: None,
                        osd_query_seq: 0,
                        last_osd_query_seq: 0,
                        osd_bounds: None,
                        osd_suppressed: false,
                        synced_osd_suppressed: None,
                        osd_fallback_colors: None,
                        synced_osd_fallback_colors: None,
                        tooltip_query_seq: 0,
                        last_tooltip_query_seq: 0,
                        tooltip_bounds: None,
                        tooltip_suppressed: false,
                        synced_tooltip_suppressed: None,
                        tooltip_fallback_colors: None,
                        synced_tooltip_fallback_colors: None,
                        menu_flyout_query_seq: 0,
                        last_menu_flyout_query_seq: 0,
                        menu_flyout_bounds: Vec::new(),
                        menu_flyout_suppressed: false,
                        synced_menu_flyout_suppressed: None,
                        menu_flyout_fallback_colors: None,
                        synced_menu_flyout_fallback_colors: None,
                        alt_tab_item_colors: None,
                        synced_alt_tab_item_colors: None,
                        notify_hwnd_raw: 0,
                        notify_msg: 0,
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

        pub fn set_notify_target(&mut self, hwnd: HWND, msg: u32) {
            if let Ok(mut st) = self.shared.0.lock() {
                st.notify_hwnd_raw = hwnd.0 as usize;
                st.notify_msg = msg;
            }
        }

        pub fn request_alt_tab_rect(&mut self, explorer_pid: u32) {
            if explorer_pid == 0 {
                return;
            }
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                if st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker {
                    st.target_pid = explorer_pid;
                    st.alt_tab_query_seq = st.alt_tab_query_seq.wrapping_add(1);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if should_wake {
                self.ensure_worker_started();
                self.shared.1.notify_all();
            }
        }

        pub fn clear_alt_tab_rect(&mut self) {
            if let Ok(mut st) = self.shared.0.lock() {
                st.alt_tab_bounds = None;
            }
        }

        pub fn cached_alt_tab_rect(&self) -> Option<AltTabBoundsCache> {
            self.shared.0.lock().ok().and_then(|st| st.alt_tab_bounds)
        }

        pub fn request_hover_flyout_rect(&mut self, explorer_pid: u32) {
            if explorer_pid == 0 {
                return;
            }
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                if st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker {
                    st.target_pid = explorer_pid;
                    st.hover_flyout_query_seq = st.hover_flyout_query_seq.wrapping_add(1);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if should_wake {
                self.ensure_worker_started();
                self.shared.1.notify_all();
            }
        }

        pub fn clear_hover_flyout_rect(&mut self) {
            if let Ok(mut st) = self.shared.0.lock() {
                st.hover_flyout_bounds = None;
            }
        }

        pub fn cached_hover_flyout_rect(&self) -> Option<AltTabBoundsCache> {
            self.shared
                .0
                .lock()
                .ok()
                .and_then(|st| st.hover_flyout_bounds)
        }

        pub fn request_osd_rect(&mut self, explorer_pid: u32) {
            if explorer_pid == 0 {
                return;
            }
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                if st.desired_hidden
                    && !st.osd_suppressed
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                {
                    st.target_pid = explorer_pid;
                    st.osd_query_seq = st.osd_query_seq.wrapping_add(1);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if should_wake {
                self.ensure_worker_started();
                self.shared.1.notify_all();
            }
        }

        pub fn clear_osd_rect(&mut self) {
            if let Ok(mut st) = self.shared.0.lock() {
                st.osd_bounds = None;
            }
        }

        pub fn cached_osd_rect(&self) -> Option<AltTabBoundsCache> {
            self.shared.0.lock().ok().and_then(|st| st.osd_bounds)
        }

        pub fn set_osd_suppressed(&mut self, explorer_pid: u32, suppressed: bool) {
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                let changed = st.osd_suppressed != suppressed;
                st.osd_suppressed = suppressed;
                if suppressed {
                    st.osd_bounds = None;
                }
                if explorer_pid != 0 {
                    st.target_pid = explorer_pid;
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
                if explorer_pid != 0 {
                    self.ensure_worker_started();
                }
                if self.worker.is_some() {
                    self.shared.1.notify_all();
                }
            }
        }

        pub fn request_tooltip_rect(&mut self, explorer_pid: u32) {
            if explorer_pid == 0 {
                return;
            }
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                if st.desired_hidden
                    && !st.tooltip_suppressed
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                {
                    st.target_pid = explorer_pid;
                    st.tooltip_query_seq = st.tooltip_query_seq.wrapping_add(1);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if should_wake {
                self.ensure_worker_started();
                self.shared.1.notify_all();
            }
        }

        pub fn clear_tooltip_rect(&mut self) {
            if let Ok(mut st) = self.shared.0.lock() {
                st.tooltip_bounds = None;
            }
        }

        pub fn cached_tooltip_rect(&self) -> Option<AltTabBoundsCache> {
            self.shared.0.lock().ok().and_then(|st| st.tooltip_bounds)
        }

        pub fn set_tooltip_suppressed(&mut self, explorer_pid: u32, suppressed: bool) {
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                let changed = st.tooltip_suppressed != suppressed;
                st.tooltip_suppressed = suppressed;
                if suppressed {
                    st.tooltip_bounds = None;
                }
                if explorer_pid != 0 {
                    st.target_pid = explorer_pid;
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
                if explorer_pid != 0 {
                    self.ensure_worker_started();
                }
                if self.worker.is_some() {
                    self.shared.1.notify_all();
                }
            }
        }

        fn update_style_color_pair(
            &mut self,
            explorer_pid: u32,
            colors: Option<(u32, u32)>,
            select: fn(
                &mut TapWorkerShared,
            ) -> (&mut Option<(u32, u32)>, &mut Option<(u32, u32)>),
        ) {
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                let changed = {
                    let (desired_slot, synced_slot) = select(&mut st);
                    let changed = *desired_slot != colors;
                    *desired_slot = colors;
                    if colors.is_none() {
                        *synced_slot = None;
                    }
                    changed
                };
                if explorer_pid != 0 {
                    st.target_pid = explorer_pid;
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
                if explorer_pid != 0 {
                    self.ensure_worker_started();
                }
                if self.worker.is_some() {
                    self.shared.1.notify_all();
                }
            }
        }

        pub fn set_alt_tab_item_colors(
            &mut self,
            explorer_pid: u32,
            colors: Option<(u32, u32)>,
        ) {
            self.update_style_color_pair(explorer_pid, colors, |st| {
                (&mut st.alt_tab_item_colors, &mut st.synced_alt_tab_item_colors)
            });
        }

        pub fn set_osd_fallback_colors(
            &mut self,
            explorer_pid: u32,
            colors: Option<(u32, u32)>,
        ) {
            self.update_style_color_pair(explorer_pid, colors, |st| {
                (&mut st.osd_fallback_colors, &mut st.synced_osd_fallback_colors)
            });
        }

        pub fn set_tooltip_fallback_colors(
            &mut self,
            explorer_pid: u32,
            colors: Option<(u32, u32)>,
        ) {
            self.update_style_color_pair(explorer_pid, colors, |st| {
                (
                    &mut st.tooltip_fallback_colors,
                    &mut st.synced_tooltip_fallback_colors,
                )
            });
        }

        pub fn request_menu_flyout_rects(&mut self, explorer_pid: u32) {
            if explorer_pid == 0 {
                return;
            }
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                if st.desired_hidden
                    && !st.menu_flyout_suppressed
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                {
                    st.target_pid = explorer_pid;
                    st.menu_flyout_query_seq = st.menu_flyout_query_seq.wrapping_add(1);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if should_wake {
                self.ensure_worker_started();
                self.shared.1.notify_all();
            }
        }

        pub fn clear_menu_flyout_rects(&mut self) {
            if let Ok(mut st) = self.shared.0.lock() {
                st.menu_flyout_bounds.clear();
            }
        }

        pub fn cached_menu_flyout_rects(&self) -> Vec<AltTabBoundsCache> {
            self.shared
                .0
                .lock()
                .map(|st| st.menu_flyout_bounds.clone())
                .unwrap_or_default()
        }

        pub fn set_menu_flyout_suppressed(&mut self, explorer_pid: u32, suppressed: bool) {
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                let changed = st.menu_flyout_suppressed != suppressed;
                st.menu_flyout_suppressed = suppressed;
                if suppressed {
                    st.menu_flyout_bounds.clear();
                }
                if explorer_pid != 0 {
                    st.target_pid = explorer_pid;
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
                if explorer_pid != 0 {
                    self.ensure_worker_started();
                }
                if self.worker.is_some() {
                    self.shared.1.notify_all();
                }
            }
        }

        pub fn set_menu_flyout_fallback_colors(
            &mut self,
            explorer_pid: u32,
            colors: Option<(u32, u32)>,
        ) {
            self.update_style_color_pair(explorer_pid, colors, |st| {
                (
                    &mut st.menu_flyout_fallback_colors,
                    &mut st.synced_menu_flyout_fallback_colors,
                )
            });
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
                st.synced_alt_tab_item_colors = None;
                st.synced_osd_fallback_colors = None;
                st.synced_osd_suppressed = None;
                st.osd_suppressed = false;
                st.osd_bounds = None;
                st.synced_tooltip_fallback_colors = None;
                st.synced_tooltip_suppressed = None;
                st.tooltip_suppressed = false;
                st.tooltip_bounds = None;
                st.synced_menu_flyout_fallback_colors = None;
                st.synced_menu_flyout_suppressed = None;
                st.menu_flyout_suppressed = false;
                st.menu_flyout_bounds.clear();
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
                    st.osd_suppressed = false;
                    st.synced_osd_suppressed = None;
                    st.synced_osd_fallback_colors = None;
                    st.osd_bounds = None;
                    st.tooltip_suppressed = false;
                    st.synced_tooltip_suppressed = None;
                    st.synced_tooltip_fallback_colors = None;
                    st.tooltip_bounds = None;
                    st.menu_flyout_suppressed = false;
                    st.synced_menu_flyout_suppressed = None;
                    st.synced_menu_flyout_fallback_colors = None;
                    st.menu_flyout_bounds.clear();
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
                    st.synced_alt_tab_item_colors = None;
                    st.synced_osd_fallback_colors = None;
                    st.synced_osd_suppressed = None;
                    st.osd_bounds = None;
                    st.synced_tooltip_fallback_colors = None;
                    st.synced_tooltip_suppressed = None;
                    st.tooltip_bounds = None;
                    st.synced_menu_flyout_fallback_colors = None;
                    st.synced_menu_flyout_suppressed = None;
                    st.menu_flyout_bounds.clear();
                    st.target_pid = explorer_pid;
                    return;
                }

                st.target_pid = explorer_pid;
                if st.attached_pid.is_some_and(|p| p != explorer_pid) {
                    st.attached_pid = None;
                    st.last_ping_ok = None;
                    st.synced_alt_tab_item_colors = None;
                    st.synced_osd_fallback_colors = None;
                    st.synced_osd_suppressed = None;
                    st.osd_bounds = None;
                    st.synced_tooltip_fallback_colors = None;
                    st.synced_tooltip_suppressed = None;
                    st.tooltip_bounds = None;
                    st.synced_menu_flyout_fallback_colors = None;
                    st.synced_menu_flyout_suppressed = None;
                    st.menu_flyout_bounds.clear();
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

        /// Re-sends `CMD_HIDE` asynchronously if attached and hiding is desired (e.g. when the
        /// overflow flyout opens and may need fresh VisualTree background brushes updated).
        pub fn reassert_hide(&mut self, explorer_pid: u32) {
            if explorer_pid == 0 {
                return;
            }
            let should_wake = if let Ok(mut st) = self.shared.0.lock() {
                if st.desired_hidden
                    && !st.guard.is_tripped()
                    && st.attached_pid == Some(explorer_pid)
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
            let (
                desired_hidden,
                osd_suppressed,
                tooltip_suppressed,
                menu_flyout_suppressed,
                target_pid,
                attached_pid,
                stop_worker,
                last_try,
                need_cmd_sync,
                want_alt_tab_query,
                want_hover_flyout_query,
                want_osd_query,
                want_tooltip_query,
                want_menu_flyout_query,
            ) = {
                let Ok(mut st) = lock.lock() else {
                    break;
                };
                while !st.stop_worker
                    && st.wake_seq == st.last_processed_seq
                    && st.alt_tab_query_seq == st.last_alt_tab_query_seq
                    && st.hover_flyout_query_seq == st.last_hover_flyout_query_seq
                    && st.osd_query_seq == st.last_osd_query_seq
                    && st.tooltip_query_seq == st.last_tooltip_query_seq
                    && st.menu_flyout_query_seq == st.last_menu_flyout_query_seq
                {
                    st = match wake_cv.wait(st) {
                        Ok(next) => next,
                        Err(_) => return,
                    };
                }
                let cmd_dirty = st.wake_seq != st.last_processed_seq;
                let rect_dirty = st.alt_tab_query_seq != st.last_alt_tab_query_seq;
                let hover_rect_dirty =
                    st.hover_flyout_query_seq != st.last_hover_flyout_query_seq;
                let osd_rect_dirty = st.osd_query_seq != st.last_osd_query_seq;
                let tooltip_rect_dirty = st.tooltip_query_seq != st.last_tooltip_query_seq;
                let menu_flyout_rect_dirty =
                    st.menu_flyout_query_seq != st.last_menu_flyout_query_seq;
                st.last_processed_seq = st.wake_seq;
                st.last_alt_tab_query_seq = st.alt_tab_query_seq;
                st.last_hover_flyout_query_seq = st.hover_flyout_query_seq;
                st.last_osd_query_seq = st.osd_query_seq;
                st.last_tooltip_query_seq = st.tooltip_query_seq;
                st.last_menu_flyout_query_seq = st.menu_flyout_query_seq;
                (
                    st.desired_hidden && !st.guard.is_tripped(),
                    st.osd_suppressed,
                    st.tooltip_suppressed,
                    st.menu_flyout_suppressed,
                    st.target_pid,
                    st.attached_pid,
                    st.stop_worker,
                    st.last_inject_try,
                    cmd_dirty,
                    rect_dirty,
                    hover_rect_dirty,
                    osd_rect_dirty,
                    tooltip_rect_dirty,
                    menu_flyout_rect_dirty,
                )
            };

            if stop_worker {
                let pid = attached_pid.unwrap_or(target_pid);
                if pid != 0 && (attached_pid.is_some() || send_command(pid, CMD_PING, 30)) {
                    let _ = send_command(pid, CMD_RESTORE, 250);
                }
                if let Ok(mut st) = lock.lock() {
                    st.attached_pid = None;
                    st.alt_tab_bounds = None;
                    st.hover_flyout_bounds = None;
                    st.osd_bounds = None;
                    st.osd_suppressed = false;
                    st.synced_osd_suppressed = None;
                    st.synced_osd_fallback_colors = None;
                    st.tooltip_bounds = None;
                    st.tooltip_suppressed = false;
                    st.synced_tooltip_suppressed = None;
                    st.synced_tooltip_fallback_colors = None;
                    st.menu_flyout_bounds.clear();
                    st.menu_flyout_suppressed = false;
                    st.synced_menu_flyout_suppressed = None;
                    st.synced_menu_flyout_fallback_colors = None;
                    st.synced_alt_tab_item_colors = None;
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
            let mut did_cmd_sync = false;
            if let Some(pid) = cur_attached {
                if need_cmd_sync {
                    let latest_hide = lock
                        .lock()
                        .map(|st| st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker)
                        .unwrap_or(false);
                    let cmd = if latest_hide { CMD_HIDE } else { CMD_RESTORE };
                    if send_command(pid, cmd, 250) {
                        did_cmd_sync = true;
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
                                st.synced_alt_tab_item_colors = None;
                                st.synced_osd_fallback_colors = None;
                                st.synced_osd_suppressed = None;
                                st.osd_bounds = None;
                                st.synced_tooltip_fallback_colors = None;
                                st.synced_tooltip_suppressed = None;
                                st.tooltip_bounds = None;
                                st.synced_menu_flyout_fallback_colors = None;
                                st.synced_menu_flyout_suppressed = None;
                                st.menu_flyout_bounds.clear();
                            }
                        }
                    }
                }
            }

            if cur_attached.is_none() && send_command(target_pid, CMD_PING, 40) {
                cur_attached = Some(target_pid);
                if let Ok(mut st) = lock.lock() {
                    st.attached_pid = Some(target_pid);
                    st.last_ping_ok = Some(Instant::now());
                    st.synced_alt_tab_item_colors = None;
                    st.synced_osd_fallback_colors = None;
                    st.synced_osd_suppressed = None;
                    st.synced_tooltip_fallback_colors = None;
                    st.synced_tooltip_suppressed = None;
                    st.synced_menu_flyout_fallback_colors = None;
                    st.synced_menu_flyout_suppressed = None;
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
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                            st.synced_menu_flyout_fallback_colors = None;
                            st.synced_menu_flyout_suppressed = None;
                            st.guard.record_success();
                        }
                    } else if let Ok(mut st) = lock.lock() {
                        st.guard.record_failure();
                    }
                }
            }

            if !did_cmd_sync {
                if let Some(pid) = cur_attached {
                    if need_cmd_sync
                        || (!want_alt_tab_query
                            && !want_hover_flyout_query
                            && !want_osd_query
                            && !want_tooltip_query
                            && !want_menu_flyout_query)
                    {
                        let latest_hide = lock
                            .lock()
                            .map(|st| {
                                st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker
                            })
                            .unwrap_or(false);
                        let cmd = if latest_hide { CMD_HIDE } else { CMD_RESTORE };
                        if send_command(pid, cmd, 250) {
                            if let Ok(mut st) = lock.lock() {
                                st.last_ping_ok = Some(Instant::now());
                            }
                        } else {
                            cur_attached = None;
                            if let Ok(mut st) = lock.lock() {
                                if st.attached_pid == Some(pid) {
                                    st.attached_pid = None;
                                    st.last_ping_ok = None;
                                    st.synced_alt_tab_item_colors = None;
                                    st.synced_osd_fallback_colors = None;
                                    st.synced_osd_suppressed = None;
                                    st.osd_bounds = None;
                                    st.synced_tooltip_fallback_colors = None;
                                    st.synced_tooltip_suppressed = None;
                                    st.tooltip_bounds = None;
                                    st.synced_menu_flyout_fallback_colors = None;
                                    st.synced_menu_flyout_suppressed = None;
                                    st.menu_flyout_bounds.clear();
                                }
                            }
                        }
                    }
                }
            }

            if let Some(pid) = cur_attached {
                let desired_colors = lock.lock().ok().and_then(|st| {
                    (st.desired_hidden
                        && !st.guard.is_tripped()
                        && !st.stop_worker
                        && st.alt_tab_item_colors != st.synced_alt_tab_item_colors)
                        .then_some(st.alt_tab_item_colors)
                        .flatten()
                });
                if let Some((bg_argb, border_argb)) = desired_colors {
                    if send_alt_tab_item_style(pid, bg_argb, border_argb, 250) {
                        if let Ok(mut st) = lock.lock() {
                            st.synced_alt_tab_item_colors = Some((bg_argb, border_argb));
                            st.last_ping_ok = Some(Instant::now());
                        }
                    } else {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            if st.attached_pid == Some(pid) {
                                st.attached_pid = None;
                                st.last_ping_ok = None;
                            }
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.osd_bounds = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                            st.tooltip_bounds = None;
                            st.synced_menu_flyout_fallback_colors = None;
                            st.synced_menu_flyout_suppressed = None;
                            st.menu_flyout_bounds.clear();
                        }
                    }
                }
            }

            if let Some(pid) = cur_attached {
                let desired_osd_colors = lock.lock().ok().and_then(|st| {
                    (st.desired_hidden
                        && !st.guard.is_tripped()
                        && !st.stop_worker
                        && st.osd_fallback_colors != st.synced_osd_fallback_colors)
                        .then_some(st.osd_fallback_colors)
                        .flatten()
                });
                if let Some((bg_argb, border_argb)) = desired_osd_colors {
                    if send_osd_fallback_style(pid, bg_argb, border_argb, 250) {
                        if let Ok(mut st) = lock.lock() {
                            st.synced_osd_fallback_colors = Some((bg_argb, border_argb));
                            st.last_ping_ok = Some(Instant::now());
                        }
                    } else {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            if st.attached_pid == Some(pid) {
                                st.attached_pid = None;
                                st.last_ping_ok = None;
                            }
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.osd_bounds = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                            st.tooltip_bounds = None;
                            st.synced_menu_flyout_fallback_colors = None;
                            st.synced_menu_flyout_suppressed = None;
                            st.menu_flyout_bounds.clear();
                        }
                    }
                }
            }

            if let Some(pid) = cur_attached {
                let desired_osd_suppress = lock.lock().ok().and_then(|st| {
                    (st.desired_hidden
                        && !st.guard.is_tripped()
                        && !st.stop_worker
                        && st.synced_osd_suppressed != Some(st.osd_suppressed))
                        .then_some(st.osd_suppressed)
                });
                if let Some(suppressed) = desired_osd_suppress {
                    if send_osd_suppress(pid, suppressed, 250) {
                        if let Ok(mut st) = lock.lock() {
                            st.synced_osd_suppressed = Some(suppressed);
                            st.last_ping_ok = Some(Instant::now());
                        }
                    } else {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            if st.attached_pid == Some(pid) {
                                st.attached_pid = None;
                                st.last_ping_ok = None;
                            }
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.osd_bounds = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                            st.tooltip_bounds = None;
                            st.synced_menu_flyout_fallback_colors = None;
                            st.synced_menu_flyout_suppressed = None;
                            st.menu_flyout_bounds.clear();
                        }
                    }
                }
            }

            if let Some(pid) = cur_attached {
                let desired_tooltip_colors = lock.lock().ok().and_then(|st| {
                    (st.desired_hidden
                        && !st.guard.is_tripped()
                        && !st.stop_worker
                        && st.tooltip_fallback_colors != st.synced_tooltip_fallback_colors)
                        .then_some(st.tooltip_fallback_colors)
                        .flatten()
                });
                if let Some((bg_argb, border_argb)) = desired_tooltip_colors {
                    if send_tooltip_fallback_style(pid, bg_argb, border_argb, 250) {
                        if let Ok(mut st) = lock.lock() {
                            st.synced_tooltip_fallback_colors = Some((bg_argb, border_argb));
                            st.last_ping_ok = Some(Instant::now());
                        }
                    } else {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            if st.attached_pid == Some(pid) {
                                st.attached_pid = None;
                                st.last_ping_ok = None;
                            }
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.osd_bounds = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                            st.tooltip_bounds = None;
                            st.synced_menu_flyout_fallback_colors = None;
                            st.synced_menu_flyout_suppressed = None;
                            st.menu_flyout_bounds.clear();
                        }
                    }
                }
            }

            if let Some(pid) = cur_attached {
                let desired_tooltip_suppress = lock.lock().ok().and_then(|st| {
                    (st.desired_hidden
                        && !st.guard.is_tripped()
                        && !st.stop_worker
                        && st.synced_tooltip_suppressed != Some(st.tooltip_suppressed))
                        .then_some(st.tooltip_suppressed)
                });
                if let Some(suppressed) = desired_tooltip_suppress {
                    if send_tooltip_suppress(pid, suppressed, 250) {
                        if let Ok(mut st) = lock.lock() {
                            st.synced_tooltip_suppressed = Some(suppressed);
                            st.last_ping_ok = Some(Instant::now());
                        }
                    } else {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            if st.attached_pid == Some(pid) {
                                st.attached_pid = None;
                                st.last_ping_ok = None;
                            }
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.osd_bounds = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                            st.tooltip_bounds = None;
                            st.synced_menu_flyout_fallback_colors = None;
                            st.synced_menu_flyout_suppressed = None;
                            st.menu_flyout_bounds.clear();
                        }
                    }
                }
            }

            if let Some(pid) = cur_attached {
                let desired_menu_colors = lock.lock().ok().and_then(|st| {
                    (st.desired_hidden
                        && !st.guard.is_tripped()
                        && !st.stop_worker
                        && st.menu_flyout_fallback_colors != st.synced_menu_flyout_fallback_colors)
                        .then_some(st.menu_flyout_fallback_colors)
                        .flatten()
                });
                if let Some((bg_argb, border_argb)) = desired_menu_colors {
                    if send_menu_flyout_fallback_style(pid, bg_argb, border_argb, 250) {
                        if let Ok(mut st) = lock.lock() {
                            st.synced_menu_flyout_fallback_colors = Some((bg_argb, border_argb));
                            st.last_ping_ok = Some(Instant::now());
                        }
                    } else {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            if st.attached_pid == Some(pid) {
                                st.attached_pid = None;
                                st.last_ping_ok = None;
                            }
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.osd_bounds = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                            st.tooltip_bounds = None;
                            st.synced_menu_flyout_fallback_colors = None;
                            st.synced_menu_flyout_suppressed = None;
                            st.menu_flyout_bounds.clear();
                        }
                    }
                }
            }

            if let Some(pid) = cur_attached {
                let desired_menu_suppress = lock.lock().ok().and_then(|st| {
                    (st.desired_hidden
                        && !st.guard.is_tripped()
                        && !st.stop_worker
                        && st.synced_menu_flyout_suppressed != Some(st.menu_flyout_suppressed))
                        .then_some(st.menu_flyout_suppressed)
                });
                if let Some(suppressed) = desired_menu_suppress {
                    if send_menu_flyout_suppress(pid, suppressed, 250) {
                        if let Ok(mut st) = lock.lock() {
                            st.synced_menu_flyout_suppressed = Some(suppressed);
                            st.last_ping_ok = Some(Instant::now());
                        }
                    } else {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            if st.attached_pid == Some(pid) {
                                st.attached_pid = None;
                                st.last_ping_ok = None;
                            }
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.osd_bounds = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                            st.tooltip_bounds = None;
                            st.synced_menu_flyout_fallback_colors = None;
                            st.synced_menu_flyout_suppressed = None;
                            st.menu_flyout_bounds.clear();
                        }
                    }
                }
            }

            if want_alt_tab_query {
                let query_pid = cur_attached.filter(|_| {
                    lock.lock()
                        .map(|st| st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker)
                        .unwrap_or(false)
                });
                if let Some(pid) = query_pid {
                    match query_card_rect(pid, CMD_ALTTAB_RECT, 120) {
                        Ok(Some((thread_id, rect_dip))) => {
                            let notify = if let Ok(mut st) = lock.lock() {
                                st.alt_tab_bounds = Some(AltTabBoundsCache {
                                    thread_id,
                                    rect_dip,
                                    updated_at: Instant::now(),
                                });
                                st.last_ping_ok = Some(Instant::now());
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
                        Ok(None) => {
                            if let Ok(mut st) = lock.lock() {
                                st.alt_tab_bounds = None;
                            }
                        }
                        Err(()) => {
                            cur_attached = None;
                            if let Ok(mut st) = lock.lock() {
                                st.alt_tab_bounds = None;
                                st.hover_flyout_bounds = None;
                                st.osd_bounds = None;
                                st.tooltip_bounds = None;
                                st.menu_flyout_bounds.clear();
                                if st.attached_pid == Some(pid) {
                                    st.attached_pid = None;
                                    st.last_ping_ok = None;
                                    st.synced_alt_tab_item_colors = None;
                                    st.synced_osd_fallback_colors = None;
                                    st.synced_osd_suppressed = None;
                                    st.synced_tooltip_fallback_colors = None;
                                    st.synced_tooltip_suppressed = None;
                                    st.synced_menu_flyout_fallback_colors = None;
                                    st.synced_menu_flyout_suppressed = None;
                                }
                            }
                        }
                    }
                } else if let Ok(mut st) = lock.lock() {
                    st.alt_tab_bounds = None;
                }
            }

            if want_hover_flyout_query {
                let query_pid = cur_attached.filter(|_| {
                    lock.lock()
                        .map(|st| st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker)
                        .unwrap_or(false)
                });
                if let Some(pid) = query_pid {
                    match query_card_rect(pid, CMD_HOVER_FLYOUT_RECT, 120) {
                        Ok(Some((thread_id, rect_dip))) => {
                            let notify = if let Ok(mut st) = lock.lock() {
                                let changed = match st.hover_flyout_bounds {
                                    None => true,
                                    Some(prev) => {
                                        prev.thread_id != thread_id
                                            || (prev.rect_dip[0] - rect_dip[0]).abs() > 0.25
                                            || (prev.rect_dip[1] - rect_dip[1]).abs() > 0.25
                                            || (prev.rect_dip[2] - rect_dip[2]).abs() > 0.25
                                            || (prev.rect_dip[3] - rect_dip[3]).abs() > 0.25
                                    }
                                };
                                st.hover_flyout_bounds = Some(AltTabBoundsCache {
                                    thread_id,
                                    rect_dip,
                                    updated_at: Instant::now(),
                                });
                                st.last_ping_ok = Some(Instant::now());
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
                        Ok(None) => {
                            let notify = if let Ok(mut st) = lock.lock() {
                                let had_bounds = st.hover_flyout_bounds.take().is_some();
                                if had_bounds {
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
                            cur_attached = None;
                            if let Ok(mut st) = lock.lock() {
                                st.hover_flyout_bounds = None;
                                st.alt_tab_bounds = None;
                                st.osd_bounds = None;
                                st.tooltip_bounds = None;
                                st.menu_flyout_bounds.clear();
                                if st.attached_pid == Some(pid) {
                                    st.attached_pid = None;
                                    st.last_ping_ok = None;
                                    st.synced_alt_tab_item_colors = None;
                                    st.synced_osd_fallback_colors = None;
                                    st.synced_osd_suppressed = None;
                                    st.synced_tooltip_fallback_colors = None;
                                    st.synced_tooltip_suppressed = None;
                                    st.synced_menu_flyout_fallback_colors = None;
                                    st.synced_menu_flyout_suppressed = None;
                                }
                            }
                        }
                    }
                } else if let Ok(mut st) = lock.lock() {
                    st.hover_flyout_bounds = None;
                }
            }

            if want_osd_query {
                sync_suppressed_card_rect_query(
                    lock,
                    cur_attached,
                    CMD_OSD_RECT,
                    osd_suppressed,
                    |st| st.osd_suppressed,
                    |st| &mut st.osd_bounds,
                );
            }

            if want_tooltip_query {
                sync_suppressed_card_rect_query(
                    lock,
                    cur_attached,
                    CMD_TOOLTIP_RECT,
                    tooltip_suppressed,
                    |st| st.tooltip_suppressed,
                    |st| &mut st.tooltip_bounds,
                );
            }

            if want_menu_flyout_query {
                sync_menu_flyout_rects_query(lock, cur_attached, menu_flyout_suppressed);
            }
        }
    }

    fn sync_menu_flyout_rects_query(
        lock: &Mutex<TapWorkerShared>,
        cur_attached: Option<u32>,
        cached_suppressed: bool,
    ) {
        let query_pid = if cached_suppressed {
            None
        } else {
            cur_attached.filter(|_| {
                lock.lock()
                    .map(|st| {
                        st.desired_hidden
                            && !st.menu_flyout_suppressed
                            && !st.guard.is_tripped()
                            && !st.stop_worker
                    })
                    .unwrap_or(false)
            })
        };
        if let Some(pid) = query_pid {
            match query_menu_flyout_rects(pid, 120) {
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
                        let changed = st.menu_flyout_bounds.len() != next_bounds.len()
                            || st.menu_flyout_bounds.iter().zip(next_bounds.iter()).any(
                                |(prev, next)| {
                                    prev.thread_id != next.thread_id
                                        || (prev.rect_dip[0] - next.rect_dip[0]).abs() > 0.25
                                        || (prev.rect_dip[1] - next.rect_dip[1]).abs() > 0.25
                                        || (prev.rect_dip[2] - next.rect_dip[2]).abs() > 0.25
                                        || (prev.rect_dip[3] - next.rect_dip[3]).abs() > 0.25
                                },
                            );
                        st.menu_flyout_bounds = next_bounds;
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
                    if let Ok(mut st) = lock.lock() {
                        st.osd_bounds = None;
                        st.tooltip_bounds = None;
                        st.menu_flyout_bounds.clear();
                        st.hover_flyout_bounds = None;
                        st.alt_tab_bounds = None;
                        if st.attached_pid == Some(pid) {
                            st.attached_pid = None;
                            st.last_ping_ok = None;
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                            st.synced_menu_flyout_fallback_colors = None;
                            st.synced_menu_flyout_suppressed = None;
                        }
                    }
                }
            }
        } else if let Ok(mut st) = lock.lock() {
            st.menu_flyout_bounds.clear();
        }
    }

    fn sync_suppressed_card_rect_query(
        lock: &Mutex<TapWorkerShared>,
        cur_attached: Option<u32>,
        cmd: u8,
        cached_suppressed: bool,
        get_suppressed: fn(&TapWorkerShared) -> bool,
        get_bounds_mut: fn(&mut TapWorkerShared) -> &mut Option<AltTabBoundsCache>,
    ) {
        let query_pid = if cached_suppressed {
            None
        } else {
            cur_attached.filter(|_| {
                lock.lock()
                    .map(|st| {
                        st.desired_hidden
                            && !get_suppressed(&st)
                            && !st.guard.is_tripped()
                            && !st.stop_worker
                    })
                    .unwrap_or(false)
            })
        };
        if let Some(pid) = query_pid {
            match query_card_rect(pid, cmd, 120) {
                Ok(Some((thread_id, rect_dip))) => {
                    let notify = if let Ok(mut st) = lock.lock() {
                        let slot = get_bounds_mut(&mut st);
                        let changed = match *slot {
                            None => true,
                            Some(prev) => {
                                prev.thread_id != thread_id
                                    || (prev.rect_dip[0] - rect_dip[0]).abs() > 0.25
                                    || (prev.rect_dip[1] - rect_dip[1]).abs() > 0.25
                                    || (prev.rect_dip[2] - rect_dip[2]).abs() > 0.25
                                    || (prev.rect_dip[3] - rect_dip[3]).abs() > 0.25
                            }
                        };
                        *slot = Some(AltTabBoundsCache {
                            thread_id,
                            rect_dip,
                            updated_at: Instant::now(),
                        });
                        st.last_ping_ok = Some(Instant::now());
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
                Ok(None) => {
                    let notify = if let Ok(mut st) = lock.lock() {
                        let had_bounds = get_bounds_mut(&mut st).take().is_some();
                        if had_bounds {
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
                    if let Ok(mut st) = lock.lock() {
                        st.osd_bounds = None;
                        st.tooltip_bounds = None;
                        st.hover_flyout_bounds = None;
                        st.alt_tab_bounds = None;
                        if st.attached_pid == Some(pid) {
                            st.attached_pid = None;
                            st.last_ping_ok = None;
                            st.synced_alt_tab_item_colors = None;
                            st.synced_osd_fallback_colors = None;
                            st.synced_osd_suppressed = None;
                            st.synced_tooltip_fallback_colors = None;
                            st.synced_tooltip_suppressed = None;
                        }
                    }
                }
            }
        } else if let Ok(mut st) = lock.lock() {
            *get_bounds_mut(&mut st) = None;
        }
    }

    fn prepare_tap_dll_for_injection(grant_appcontainer: bool) -> Option<PathBuf> {
        let src_dll = tap_dll_path()?;
        let cache_dir = shadow_cache_dir();
        let _ = std::fs::create_dir_all(&cache_dir);
        if grant_appcontainer {
            grant_appcontainer_rx_acl(&cache_dir, true);
        }
        let dll_path = prepare_shadow_dll_in_dir(&src_dll, &cache_dir).unwrap_or(src_dll);
        if grant_appcontainer {
            grant_appcontainer_rx_acl(&dll_path, false);
        }
        Some(dll_path)
    }

    fn invoke_xaml_diagnostics_ex(target_pid: u32, dll_path: &Path) -> bool {
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

            for i in 1..=128 {
                let endpoint: Vec<u16> = format!("VisualDiagConnection{i}")
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let hr = init_xaml(
                    PCWSTR(endpoint.as_ptr()),
                    target_pid,
                    w!(""),
                    PCWSTR(dll_wide.as_ptr()),
                    CLSID_PRETTY_WINDOWS_TAP,
                    PCWSTR::null(),
                );
                if hr.is_ok() {
                    return true;
                }
                if hr != E_NOTFOUND_HR {
                    break;
                }
            }
            false
        }
    }

    fn inject_and_wait_pipe(explorer_pid: u32) -> bool {
        let Some(dll_path) = prepare_tap_dll_for_injection(false) else {
            return false;
        };
        let connected = invoke_xaml_diagnostics_ex(explorer_pid, &dll_path);
        if !connected && !send_command(explorer_pid, CMD_PING, 40) {
            return false;
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

    fn open_pipe(explorer_pid: u32, wait_ms: u32) -> Option<HANDLE> {
        let pipe_wide = pipe_name_wide(explorer_pid);
        unsafe {
            match CreateFileW(
                PCWSTR(pipe_wide.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            ) {
                Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE => Some(h),
                _ => {
                    if wait_ms == 0 || !WaitNamedPipeW(PCWSTR(pipe_wide.as_ptr()), wait_ms).as_bool()
                    {
                        return None;
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
                        Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE => Some(h),
                        _ => None,
                    }
                }
            }
        }
    }

    fn query_card_rect(
        explorer_pid: u32,
        cmd: u8,
        wait_ms: u32,
    ) -> Result<Option<(u32, [f32; 4])>, ()> {
        let handle = open_pipe(explorer_pid, wait_ms).ok_or(())?;
        unsafe {
            let host_pid = GetCurrentProcessId().to_le_bytes();
            let req = [cmd, host_pid[0], host_pid[1], host_pid[2], host_pid[3]];
            let mut written = 0u32;
            let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
                && written == req.len() as u32;
            if !write_ok {
                let _ = CloseHandle(handle);
                return Err(());
            }

            let mut tag = [0u8; 1];
            let mut tag_read = 0u32;
            if ReadFile(handle, Some(&mut tag), Some(&mut tag_read), None).is_err() || tag_read != 1
            {
                let _ = CloseHandle(handle);
                return Err(());
            }
            if tag[0] == 0xFF {
                let _ = CloseHandle(handle);
                return Ok(None);
            }
            if tag[0] != cmd {
                let _ = CloseHandle(handle);
                return Err(());
            }

            let mut payload = [0u8; 21];
            let mut off = 0usize;
            while off < payload.len() {
                let mut r = 0u32;
                if ReadFile(handle, Some(&mut payload[off..]), Some(&mut r), None).is_err()
                    || r == 0
                {
                    let _ = CloseHandle(handle);
                    return Err(());
                }
                off += r as usize;
            }
            let _ = CloseHandle(handle);

            if payload[0] == 0 {
                return Ok(None);
            }
            let tid = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
            let l = f32::from_le_bytes([payload[5], payload[6], payload[7], payload[8]]);
            let t = f32::from_le_bytes([payload[9], payload[10], payload[11], payload[12]]);
            let r = f32::from_le_bytes([payload[13], payload[14], payload[15], payload[16]]);
            let b = f32::from_le_bytes([payload[17], payload[18], payload[19], payload[20]]);
            Ok(Some((tid, [l, t, r, b])))
        }
    }

    fn send_style_colors_cmd(
        explorer_pid: u32,
        cmd: u8,
        bg_argb: u32,
        border_argb: u32,
        wait_ms: u32,
    ) -> bool {
        let Some(handle) = open_pipe(explorer_pid, wait_ms) else {
            return false;
        };
        unsafe {
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
            let mut written = 0u32;
            let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
                && written == req.len() as u32;

            let mut ack = [0u8; 1];
            let mut read = 0u32;
            let read_ok = write_ok
                && ReadFile(handle, Some(&mut ack), Some(&mut read), None).is_ok()
                && read == 1
                && (ack[0] == cmd || ack[0] == 0xFF);

            let _ = CloseHandle(handle);
            read_ok
        }
    }

    fn send_alt_tab_item_style(
        explorer_pid: u32,
        bg_argb: u32,
        border_argb: u32,
        wait_ms: u32,
    ) -> bool {
        send_style_colors_cmd(
            explorer_pid,
            CMD_SET_ALTTAB_ITEM_STYLE,
            bg_argb,
            border_argb,
            wait_ms,
        )
    }

    fn send_osd_fallback_style(
        explorer_pid: u32,
        bg_argb: u32,
        border_argb: u32,
        wait_ms: u32,
    ) -> bool {
        send_style_colors_cmd(
            explorer_pid,
            CMD_SET_OSD_FALLBACK_STYLE,
            bg_argb,
            border_argb,
            wait_ms,
        )
    }

    fn send_tooltip_fallback_style(
        explorer_pid: u32,
        bg_argb: u32,
        border_argb: u32,
        wait_ms: u32,
    ) -> bool {
        send_style_colors_cmd(
            explorer_pid,
            CMD_SET_TOOLTIP_FALLBACK_STYLE,
            bg_argb,
            border_argb,
            wait_ms,
        )
    }

    fn send_menu_flyout_fallback_style(
        explorer_pid: u32,
        bg_argb: u32,
        border_argb: u32,
        wait_ms: u32,
    ) -> bool {
        send_style_colors_cmd(
            explorer_pid,
            CMD_SET_MENU_FLYOUT_FALLBACK_STYLE,
            bg_argb,
            border_argb,
            wait_ms,
        )
    }

    fn decode_flyout_rects_payload(payload: &[u8], max_rects: usize) -> Vec<(u32, [f32; 4])> {
        let count = (payload[0] as usize).min(max_rects);
        let mut out = Vec::with_capacity(count);
        for slot in 0..count {
            let base = 1 + slot * 20;
            let tid = u32::from_le_bytes([
                payload[base],
                payload[base + 1],
                payload[base + 2],
                payload[base + 3],
            ]);
            let l = f32::from_le_bytes([
                payload[base + 4],
                payload[base + 5],
                payload[base + 6],
                payload[base + 7],
            ]);
            let t = f32::from_le_bytes([
                payload[base + 8],
                payload[base + 9],
                payload[base + 10],
                payload[base + 11],
            ]);
            let r = f32::from_le_bytes([
                payload[base + 12],
                payload[base + 13],
                payload[base + 14],
                payload[base + 15],
            ]);
            let b = f32::from_le_bytes([
                payload[base + 16],
                payload[base + 17],
                payload[base + 18],
                payload[base + 19],
            ]);
            out.push((tid, [l, t, r, b]));
        }
        out
    }

    fn decode_menu_flyout_rects_payload(payload: &[u8]) -> Vec<(u32, [f32; 4])> {
        decode_flyout_rects_payload(payload, MAX_MENU_FLYOUT_RECTS)
    }

    fn query_menu_flyout_rects(
        explorer_pid: u32,
        wait_ms: u32,
    ) -> Result<Vec<(u32, [f32; 4])>, ()> {
        let handle = open_pipe(explorer_pid, wait_ms).ok_or(())?;
        unsafe {
            let host_pid = GetCurrentProcessId().to_le_bytes();
            let req = [
                CMD_MENU_FLYOUT_RECTS,
                host_pid[0],
                host_pid[1],
                host_pid[2],
                host_pid[3],
            ];
            let mut written = 0u32;
            let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
                && written == req.len() as u32;
            if !write_ok {
                let _ = CloseHandle(handle);
                return Err(());
            }

            let mut tag = [0u8; 1];
            let mut tag_read = 0u32;
            if ReadFile(handle, Some(&mut tag), Some(&mut tag_read), None).is_err() || tag_read != 1
            {
                let _ = CloseHandle(handle);
                return Err(());
            }
            if tag[0] == 0xFF {
                let _ = CloseHandle(handle);
                return Ok(Vec::new());
            }
            if tag[0] != CMD_MENU_FLYOUT_RECTS {
                let _ = CloseHandle(handle);
                return Err(());
            }

            let mut payload = [0u8; 1 + MAX_MENU_FLYOUT_RECTS * 20];
            let mut off = 0usize;
            while off < payload.len() {
                let mut r = 0u32;
                if ReadFile(handle, Some(&mut payload[off..]), Some(&mut r), None).is_err()
                    || r == 0
                {
                    let _ = CloseHandle(handle);
                    return Err(());
                }
                off += r as usize;
            }
            let _ = CloseHandle(handle);
            Ok(decode_menu_flyout_rects_payload(&payload))
        }
    }

    fn send_bool_flag_cmd(explorer_pid: u32, cmd: u8, flag: bool, wait_ms: u32) -> bool {
        let Some(handle) = open_pipe(explorer_pid, wait_ms) else {
            return false;
        };
        unsafe {
            let host_pid = GetCurrentProcessId().to_le_bytes();
            let req = [
                cmd,
                host_pid[0],
                host_pid[1],
                host_pid[2],
                host_pid[3],
                u8::from(flag),
            ];
            let mut written = 0u32;
            let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
                && written == req.len() as u32;

            let mut ack = [0u8; 1];
            let mut read = 0u32;
            let read_ok = write_ok
                && ReadFile(handle, Some(&mut ack), Some(&mut read), None).is_ok()
                && read == 1
                && (ack[0] == cmd || ack[0] == 0xFF);

            let _ = CloseHandle(handle);
            read_ok
        }
    }

    fn send_osd_suppress(explorer_pid: u32, suppressed: bool, wait_ms: u32) -> bool {
        send_bool_flag_cmd(explorer_pid, CMD_SET_OSD_SUPPRESS, suppressed, wait_ms)
    }

    fn send_tooltip_suppress(explorer_pid: u32, suppressed: bool, wait_ms: u32) -> bool {
        send_bool_flag_cmd(explorer_pid, CMD_SET_TOOLTIP_SUPPRESS, suppressed, wait_ms)
    }

    fn send_menu_flyout_suppress(explorer_pid: u32, suppressed: bool, wait_ms: u32) -> bool {
        send_bool_flag_cmd(explorer_pid, CMD_SET_MENU_FLYOUT_SUPPRESS, suppressed, wait_ms)
    }

    fn send_command(explorer_pid: u32, cmd: u8, wait_ms: u32) -> bool {
        let Some(handle) = open_pipe(explorer_pid, wait_ms) else {
            return false;
        };
        unsafe {
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

    // ---------- ShellExperienceHost AppContainer TAP Client (Reverse Pipe Server) ----------

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
        notify_hwnd_raw: usize,
        notify_msg: u32,
        last_inject_try: Option<Instant>,
        last_ping_ok: Option<Instant>,
        stop_worker: bool,
        shutdown_done: bool,
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
                        notify_hwnd_raw: 0,
                        notify_msg: 0,
                        last_inject_try: None,
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
                if target_pid != 0 && st.guard.on_explorer_pid_changed(target_pid, now) {
                    st.desired_hidden = false;
                } else {
                    if user_initiated {
                        st.guard.reset_failures_on_user_action();
                    }
                    st.desired_hidden = hide && !st.guard.is_tripped();
                }
                if target_pid != 0 {
                    st.target_pid = target_pid;
                }
                if !hide {
                    st.flyout_active = false;
                    st.flyout_suppressed = false;
                    st.synced_flyout_suppressed = None;
                    st.synced_flyout_fallback_colors = None;
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
                if st.guard.on_explorer_pid_changed(target_pid, now) {
                    st.desired_hidden = false;
                    st.attached_pid = None;
                    st.last_ping_ok = None;
                    st.flyout_active = false;
                    st.synced_flyout_suppressed = None;
                    st.synced_flyout_fallback_colors = None;
                    st.flyout_bounds.clear();
                    st.target_pid = target_pid;
                    return;
                }

                st.target_pid = target_pid;
                if st.attached_pid.is_some_and(|p| p != target_pid) {
                    st.attached_pid = None;
                    st.last_ping_ok = None;
                    st.flyout_active = false;
                    st.synced_flyout_suppressed = None;
                    st.synced_flyout_fallback_colors = None;
                    st.flyout_bounds.clear();
                }

                if st.desired_hidden && !st.guard.is_tripped() {
                    let ping_due = st
                        .last_ping_ok
                        .is_none_or(|t| now.saturating_duration_since(t) >= MAINTAIN_PING_INTERVAL);
                    let need_attach = st.attached_pid != Some(target_pid);
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

        pub fn shutdown(&mut self) {
            let has_worker = self.worker.is_some();
            if let Ok(mut st) = self.shared.0.lock() {
                st.desired_hidden = false;
                st.flyout_active = false;
                st.stop_worker = true;
                st.wake_seq = st.wake_seq.wrapping_add(1);
            }
            self.shared.1.set();

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
            let mut written = 0u32;
            let res = WriteFile(pipe, Some(&buf[off..]), Some(&mut written), Some(&mut ov));
            let put = if res.is_ok() {
                written as usize
            } else if GetLastError() == ERROR_IO_PENDING {
                if WaitForSingleObject(io_event, timeout_ms) != WAIT_OBJECT_0 {
                    let _ = CancelIo(pipe);
                    let mut tr = 0u32;
                    let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                    return false;
                }
                let mut tr = 0u32;
                if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
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
            let mut read = 0u32;
            let res = ReadFile(pipe, Some(&mut buf[off..]), Some(&mut read), Some(&mut ov));
            let got = if res.is_ok() {
                read as usize
            } else if GetLastError() == ERROR_IO_PENDING {
                if WaitForSingleObject(io_event, timeout_ms) != WAIT_OBJECT_0 {
                    let _ = CancelIo(pipe);
                    let mut tr = 0u32;
                    let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                    return false;
                }
                let mut tr = 0u32;
                if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
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

    unsafe fn pipe_query_version(pipe: HANDLE, io_event: HANDLE, wait_ms: u32) -> Option<u8> {
        let host_pid = GetCurrentProcessId().to_le_bytes();
        let req = [CMD_VERSION, host_pid[0], host_pid[1], host_pid[2], host_pid[3]];
        if !pipe_write_exact_timeout(pipe, io_event, &req, wait_ms) {
            return None;
        }
        let mut tag = [0u8; 1];
        if !pipe_read_exact_timeout(pipe, io_event, &mut tag, wait_ms) || tag[0] != CMD_VERSION {
            return None;
        }
        let mut ver = [0u8; 1];
        pipe_read_exact_timeout(pipe, io_event, &mut ver, wait_ms).then_some(ver[0])
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

    fn create_reverse_pipe_server(pipe_prefix: &str, target_pid: u32) -> Option<HANDLE> {
        let pipe_wide = reverse_pipe_name_wide(pipe_prefix, target_pid);
        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;IU)(A;;GRGW;;;S-1-15-2-1)(A;;GRGW;;;S-1-15-2-2)S:(ML;;NW;;;LW)"),
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
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
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

    fn close_pipe_state(
        pipe_state: &mut ShellFlyoutPipeState,
        lock: &Mutex<ShellFlyoutWorkerShared>,
        broken_pid: Option<u32>,
    ) {
        match std::mem::replace(pipe_state, ShellFlyoutPipeState::Disconnected) {
            ShellFlyoutPipeState::Disconnected => {}
            ShellFlyoutPipeState::Listening { pipe, ov, .. } => unsafe {
                let _ = CancelIo(pipe);
                let mut tr = 0u32;
                let _ = GetOverlappedResult(pipe, &*ov, &mut tr, true);
                let _ = CloseHandle(pipe);
            },
            ShellFlyoutPipeState::Connected { pipe, .. } => unsafe {
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
            },
        }
        if let Ok(mut st) = lock.lock() {
            if broken_pid.is_none_or(|pid| st.attached_pid == Some(pid)) {
                st.attached_pid = None;
                st.last_ping_ok = None;
            }
            st.synced_flyout_suppressed = None;
            st.synced_flyout_fallback_colors = None;
            st.flyout_bounds.clear();
        }
    }

    fn mark_shell_flyout_connected(lock: &Mutex<ShellFlyoutWorkerShared>, pid: u32) {
        let notify = if let Ok(mut st) = lock.lock() {
            st.attached_pid = Some(pid);
            st.last_ping_ok = Some(Instant::now());
            st.synced_flyout_suppressed = None;
            st.synced_flyout_fallback_colors = None;
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
        let connected_pid = match std::mem::replace(pipe_state, ShellFlyoutPipeState::Disconnected) {
            ShellFlyoutPipeState::Listening { pid, pipe, ov } => {
                let mut tr = 0u32;
                if unsafe { GetOverlappedResult(pipe, &*ov, &mut tr, false).is_ok() } {
                    *pipe_state = ShellFlyoutPipeState::Connected {
                        pid,
                        pipe,
                        synced_hide: None,
                    };
                    Some(pid)
                } else {
                    unsafe {
                        let _ = CancelIo(pipe);
                        let mut tr2 = 0u32;
                        let _ = GetOverlappedResult(pipe, &*ov, &mut tr2, true);
                        let _ = CloseHandle(pipe);
                    }
                    None
                }
            }
            other => {
                *pipe_state = other;
                return false;
            }
        };
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
        conn_event: HANDLE,
        pipe_state: &mut ShellFlyoutPipeState,
        lock: &Mutex<ShellFlyoutWorkerShared>,
    ) -> bool {
        if !matches!(pipe_state, ShellFlyoutPipeState::Disconnected) || target_pid == 0 {
            return false;
        }
        let Some(pipe) = create_reverse_pipe_server(pipe_prefix, target_pid) else {
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
                *pipe_state = ShellFlyoutPipeState::Connected {
                    pid: target_pid,
                    pipe,
                    synced_hide: None,
                };
                mark_shell_flyout_connected(lock, target_pid);
                true
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
        let mut upgraded_pids: Vec<u32> = Vec::new();

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
                        let _ = pipe_send_simple(pipe, io_event, CMD_RESTORE, 120);
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
                            let _ = pipe_send_simple(pipe, io_event, CMD_RESTORE, 150);
                        }
                    }
                }
                close_pipe_state(&mut pipe_state, lock, None);
                continue;
            }

            if matches!(pipe_state, ShellFlyoutPipeState::Disconnected) {
                if try_start_listening_pipe(pipe_prefix, target_pid, conn_event, &mut pipe_state, lock)
                {
                    just_connected = true;
                    continue;
                }
            }

            if matches!(pipe_state, ShellFlyoutPipeState::Listening { .. })
                && unsafe { WaitForSingleObject(conn_event, 0) } == WAIT_OBJECT_0
                && try_complete_listening_pipe(&mut pipe_state, lock)
            {
                just_connected = true;
                continue;
            }

            if let ShellFlyoutPipeState::Connected { pid, pipe, .. } = pipe_state {
                if was_just_connected && !upgraded_pids.contains(&pid) {
                    match unsafe { pipe_query_version(pipe, io_event, 500) } {
                        Some(ver) if ver < EXPECTED_TAP_PROTOCOL_VERSION => {
                            upgraded_pids.push(pid);
                            unsafe {
                                let _ = pipe_send_simple(pipe, io_event, CMD_SHUTDOWN, 400);
                            }
                            close_pipe_state(&mut pipe_state, lock, Some(pid));
                            if let Ok(mut st) = lock.lock() {
                                st.last_inject_try = None;
                                st.guard.reset_failures_on_user_action();
                            }
                            std::thread::sleep(Duration::from_millis(120));
                            if try_start_listening_pipe(
                                pipe_prefix,
                                pid,
                                conn_event,
                                &mut pipe_state,
                                lock,
                            ) {
                                just_connected = true;
                                continue;
                            }
                        }
                        None => {
                            close_pipe_state(&mut pipe_state, lock, Some(pid));
                            if try_start_listening_pipe(
                                pipe_prefix,
                                pid,
                                conn_event,
                                &mut pipe_state,
                                lock,
                            ) {
                                just_connected = true;
                            }
                            continue;
                        }
                        Some(_) => {}
                    }
                }
            }

            if let ShellFlyoutPipeState::Listening { pid, .. } = pipe_state {
                let inject_cooldown = if flyout_active {
                    Duration::from_millis(500)
                } else {
                    INJECT_RETRY_COOLDOWN
                };
                let can_try = lock
                    .lock()
                    .map(|st| {
                        st.guard.can_inject()
                            && st
                                .last_inject_try
                                .is_none_or(|t| t.elapsed() >= inject_cooldown)
                    })
                    .unwrap_or(false);
                if can_try {
                    let expected_shadow = expected_shadow_dll_filename();
                    if is_current_tap_dll_loaded_in_process(pid, expected_shadow.as_deref()) {
                        if let Ok(mut st) = lock.lock() {
                            st.last_inject_try = Some(Instant::now());
                        }
                    } else {
                        if let Ok(mut st) = lock.lock() {
                            st.last_inject_try = Some(Instant::now());
                            st.guard.record_inject_attempt(pid, Instant::now());
                        }
                        if let Some(dll_path) =
                            prepare_tap_dll_for_injection(grant_appcontainer_acl)
                        {
                            let ok = invoke_xaml_diagnostics_ex(pid, &dll_path);
                            if !ok && flyout_active {
                                if let Ok(mut st) = lock.lock() {
                                    st.guard.record_failure();
                                }
                            }
                        } else if flyout_active {
                            if let Ok(mut st) = lock.lock() {
                                st.guard.record_failure();
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
                        pipe_state = ShellFlyoutPipeState::Connected {
                            pid,
                            pipe,
                            synced_hide: Some(true),
                        };
                        if let Ok(mut st) = lock.lock() {
                            st.last_ping_ok = Some(Instant::now());
                            st.guard.record_success();
                        }
                    } else {
                        close_pipe_state(&mut pipe_state, lock, Some(pid));
                        if try_start_listening_pipe(
                            pipe_prefix,
                            pid,
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
}

#[cfg(windows)]
pub use win::{
    find_shell_experience_host_pid, find_shell_host_pid, ShellFlyoutTapClient, TapClient,
};

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
