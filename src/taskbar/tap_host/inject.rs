use super::{is_tap_dll_module_name, prepare_shadow_dll_in_dir};
use std::collections::HashSet;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::core::{s, w, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, FreeLibrary, GetLastError, LocalFree, ERROR_BAD_LENGTH, ERROR_SUCCESS,
    GENERIC_EXECUTE, GENERIC_READ, HANDLE, HLOCAL, HMODULE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSidToSidW, GetNamedSecurityInfoW, SetEntriesInAclW,
    SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS,
    TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
    PSECURITY_DESCRIPTOR, PSID, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
    MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

#[link(name = "kernel32")]
extern "system" {
    fn ProcessIdToSessionId(dw_process_id: u32, p_session_id: *mut u32) -> i32;
}

pub(super) struct ComMtaGuard(bool);

impl ComMtaGuard {
    pub(super) fn enter() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self(hr.is_ok())
    }
}

impl Drop for ComMtaGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct LibraryGuard(HMODULE);

impl Drop for LibraryGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = FreeLibrary(self.0);
            }
        }
    }
}

pub(super) struct WatchedProcessHandle {
    pid: u32,
    handle: HANDLE,
}

unsafe impl Send for WatchedProcessHandle {}

impl WatchedProcessHandle {
    pub(super) fn open(pid: u32) -> Option<Self> {
        if pid == 0 {
            return None;
        }
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()? };
        if handle.is_invalid() || handle == INVALID_HANDLE_VALUE {
            return None;
        }
        Some(Self { pid, handle })
    }

    pub(super) fn pid(&self) -> u32 {
        self.pid
    }

    pub(super) fn query_exit_kind(&self) -> super::ProcessExitKind {
        let mut code = 0u32;
        let ok = unsafe { GetExitCodeProcess(self.handle, &mut code).is_ok() };
        super::classify_process_exit_code(ok.then_some(code))
    }
}

impl Drop for WatchedProcessHandle {
    fn drop(&mut self) {
        if !self.handle.is_invalid() && self.handle != INVALID_HANDLE_VALUE {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
            self.handle = HANDLE::default();
        }
    }
}

pub(super) unsafe fn is_process_alive(pid: u32) -> bool {
    const STILL_ACTIVE_CODE: u32 = 259;
    if pid == 0 {
        return false;
    }
    let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
        return false;
    };
    let mut code = 0u32;
    let ok = GetExitCodeProcess(handle, &mut code).is_ok();
    let _ = CloseHandle(handle);
    ok && code == STILL_ACTIVE_CODE
}

/// Shared protocol with `tap/src/lib.rs` (`PrettyWindowsTap.dll`).
const CLSID_PRETTY_WINDOWS_TAP_U128: u128 = 0x8a4e6f21_9c3b_4d7a_b1e5_3f7c2a9d8410;
const CLSID_PRETTY_WINDOWS_TAP: GUID = GUID::from_u128(CLSID_PRETTY_WINDOWS_TAP_U128);

pub(super) const CMD_RESTORE: u8 = 0x00;
pub(super) const CMD_HIDE: u8 = 0x01;
pub(super) const CMD_PING: u8 = 0x02;
pub(super) const CMD_ALTTAB_RECT: u8 = 0x06;
pub(super) const CMD_SET_ALTTAB_ITEM_STYLE: u8 = 0x07;
pub(super) const CMD_HOVER_FLYOUT_RECT: u8 = 0x08;
pub(super) const CMD_OSD_RECT: u8 = 0x09;
pub(super) const CMD_SET_OSD_SUPPRESS: u8 = 0x0A;
pub(super) const CMD_SET_OSD_FALLBACK_STYLE: u8 = 0x0B;
pub(super) const CMD_TOOLTIP_RECT: u8 = 0x0C;
pub(super) const CMD_SET_TOOLTIP_SUPPRESS: u8 = 0x0D;
pub(super) const CMD_SET_TOOLTIP_FALLBACK_STYLE: u8 = 0x0E;
pub(super) const CMD_MENU_FLYOUT_RECTS: u8 = 0x0F;
pub(super) const CMD_SET_MENU_FLYOUT_SUPPRESS: u8 = 0x10;
pub(super) const CMD_SET_MENU_FLYOUT_FALLBACK_STYLE: u8 = 0x11;
pub(super) const CMD_SHELL_CARD_RECTS: u8 = 0x12;
pub(super) const CMD_SET_VIRTUAL_DESKTOP_BAR_STYLE: u8 = 0x13;
pub(super) const CMD_INPUT_SWITCHER_RECT: u8 = 0x14;
pub(super) const CMD_SET_INPUT_SWITCHER_SUPPRESS: u8 = 0x15;
pub(super) const CMD_SET_INPUT_SWITCHER_FALLBACK_STYLE: u8 = 0x16;
pub(super) const CMD_SET_START_MODAL_STYLE: u8 = 0x17;
pub(super) const MAX_MENU_FLYOUT_RECTS: usize = 3;
pub(super) const MAX_SHELL_CARD_RECTS: usize = 8;

#[derive(Clone, Copy, Debug)]
pub struct AltTabBoundsCache {
    pub thread_id: u32,
    pub rect_dip: [f32; 4],
    #[allow(dead_code)]
    pub updated_at: Instant,
}

const E_NOTFOUND_HR: HRESULT = HRESULT(0x80070490u32 as i32);
pub(super) const MAINTAIN_PING_INTERVAL: Duration = Duration::from_secs(2);
pub(super) const CONFLICT_RECHECK_COOLDOWN: Duration = Duration::from_secs(3);

type InitializeXamlDiagnosticsExFn = unsafe extern "system" fn(
    end_point_name: PCWSTR,
    pid: u32,
    wsz_dll_xaml_diagnostics: PCWSTR,
    wsz_tap_dll_name: PCWSTR,
    tap_clsid: GUID,
    wsz_initialization_data: PCWSTR,
) -> HRESULT;

pub(super) fn pipe_name_wide(explorer_pid: u32) -> Vec<u16> {
    format!(r"\\.\pipe\PrettyWindows_TaskbarTap_{explorer_pid}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

pub(super) fn reverse_pipe_name_wide(pipe_prefix: &str, target_pid: u32) -> Vec<u16> {
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

pub(super) fn shadow_cache_dir() -> PathBuf {
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

/// Locates the current desktop session's `StartMenuExperienceHost.exe` process ID, or `0`.
pub fn find_start_menu_pid() -> u32 {
    find_process_pid_in_session("StartMenuExperienceHost.exe")
}

/// Locates the current desktop session's `SearchHost.exe` process ID, or `0`.
pub fn find_search_host_pid() -> u32 {
    find_process_pid_in_session("SearchHost.exe")
}

/// Checks via a toolhelp module snapshot (works even when UWP PLM has suspended all
/// AppContainer threads) whether ANY `PrettyWindowsTap` DLL build is already loaded in
/// `target_pid`. Re-injecting a second DLL build into the same XAML host process creates
/// competing reverse-pipe clients / `AdviseVisualTreeChange` conflicts.
pub(super) fn is_any_tap_dll_loaded_in_process(target_pid: u32) -> bool {
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
                if is_tap_dll_module_name(&name) {
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
    static ACL_APPLIED: OnceLock<Mutex<HashSet<(String, bool)>>> = OnceLock::new();
    let cache = ACL_APPLIED.get_or_init(|| Mutex::new(HashSet::new()));
    let acl_key = (
        String::from_utf16_lossy(&wide_path[..wide_path.len().saturating_sub(1)])
            .to_ascii_lowercase(),
        is_dir,
    );
    if cache.lock().is_ok_and(|set| set.contains(&acl_key)) {
        return;
    }
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
                grfAccessMode: SET_ACCESS,
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
                if SetNamedSecurityInfoW(
                    PWSTR(wide_path.as_mut_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    PSID::default(),
                    PSID::default(),
                    Some(new_dacl),
                    None,
                ) == ERROR_SUCCESS
                {
                    if let Ok(mut set) = cache.lock() {
                        set.insert(acl_key);
                    }
                }
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

pub(super) fn prepare_tap_dll_for_injection(grant_appcontainer: bool) -> Option<PathBuf> {
    let src_dll = tap_dll_path()?;
    let cache_dir = shadow_cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);
    if grant_appcontainer {
        grant_appcontainer_rx_acl(&cache_dir, true);
    }
    let dll_path = prepare_shadow_dll_in_dir(&src_dll, &cache_dir)?;
    if grant_appcontainer {
        grant_appcontainer_rx_acl(&dll_path, false);
    }
    Some(dll_path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum XamlDiagInjectStatus {
    Connected,
    EndpointNotReady,
    Failed,
}

pub(super) fn invoke_xaml_diagnostics_ex_status(
    target_pid: u32,
    dll_path: &Path,
) -> XamlDiagInjectStatus {
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
            return XamlDiagInjectStatus::Failed;
        };
        let _xaml_lib = LibraryGuard(h_xaml);
        let Some(proc) = GetProcAddress(h_xaml, s!("InitializeXamlDiagnosticsEx")) else {
            return XamlDiagInjectStatus::Failed;
        };
        let init_xaml: InitializeXamlDiagnosticsExFn = std::mem::transmute(proc);

        let mut saw_unexpected_error = false;
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
                return XamlDiagInjectStatus::Connected;
            }
            if hr != E_NOTFOUND_HR {
                saw_unexpected_error = true;
                break;
            }
        }
        if saw_unexpected_error {
            XamlDiagInjectStatus::Failed
        } else {
            XamlDiagInjectStatus::EndpointNotReady
        }
    }
}

pub(super) fn invoke_xaml_diagnostics_ex(target_pid: u32, dll_path: &Path) -> bool {
    invoke_xaml_diagnostics_ex_status(target_pid, dll_path) == XamlDiagInjectStatus::Connected
}


pub(super) unsafe fn verify_pipe_server_pid(pipe: HANDLE, expected_pid: u32) -> bool {
    let mut server_pid = 0u32;
    GetNamedPipeServerProcessId(pipe, &mut server_pid).is_ok() && server_pid == expected_pid
}

pub(super) unsafe fn verify_pipe_client_pid(pipe: HANDLE, expected_pid: u32) -> bool {
    let mut client_pid = 0u32;
    GetNamedPipeClientProcessId(pipe, &mut client_pid).is_ok() && client_pid == expected_pid
}

pub(super) unsafe fn current_user_sid_string() -> Option<String> {
    let mut token = HANDLE::default();
    OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
    if token.is_invalid() {
        return None;
    }
    struct TokenHandleGuard(HANDLE);
    impl Drop for TokenHandleGuard {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
    }
    let _token_guard = TokenHandleGuard(token);

    let mut needed = 0u32;
    let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    GetTokenInformation(
        token,
        TokenUser,
        Some(buf.as_mut_ptr().cast()),
        (buf.len() * size_of::<u64>()) as u32,
        &mut needed,
    )
    .ok()?;
    let user = &*(buf.as_ptr().cast::<TOKEN_USER>());
    let mut sid_pwstr = PWSTR::null();
    ConvertSidToStringSidW(user.User.Sid, &mut sid_pwstr).ok()?;
    if sid_pwstr.is_null() {
        return None;
    }
    let sid_str = sid_pwstr.to_string().ok();
    let _ = LocalFree(HLOCAL(sid_pwstr.0.cast()));
    sid_str
}


pub(super) fn decode_flyout_rects_payload(payload: &[u8], max_rects: usize) -> Vec<(u32, [f32; 4])> {
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

pub(super) fn decode_menu_flyout_rects_payload(payload: &[u8]) -> Vec<(u32, [f32; 4])> {
    decode_flyout_rects_payload(payload, MAX_MENU_FLYOUT_RECTS)
}

