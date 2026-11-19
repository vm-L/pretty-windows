use std::collections::HashMap;
use std::path::Path;
use windows::core::{Interface, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, MAX_PATH};
use windows::Win32::Media::Audio::{
    eConsole, eMultimedia, eRender, AudioSessionStateActive, IAudioSessionControl2,
    IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_RESTORE,
    WS_EX_TOOLWINDOW,
};

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ProcNode {
    pid: u32,
    parent_pid: u32,
    exe_name: String,
}

/// Snapshot-backed process tree & audio application resolver.
pub struct ProcessLookup {
    nodes: Vec<ProcNode>,
    index_by_pid: HashMap<u32, usize>,
}

impl ProcessLookup {
    fn from_nodes(nodes: Vec<ProcNode>) -> Self {
        let index_by_pid = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.pid, i))
            .collect();
        Self {
            nodes,
            index_by_pid,
        }
    }

    fn promote_to_tree_root(&self, mut pid: u32) -> Option<(u32, &str)> {
        let start_idx = *self.index_by_pid.get(&pid)?;
        let target_exe = self.nodes[start_idx].exe_name.to_ascii_lowercase();
        for _ in 0..8 {
            let Some(&idx) = self.index_by_pid.get(&pid) else {
                break;
            };
            let parent_pid = self.nodes[idx].parent_pid;
            let Some(&pidx) = self.index_by_pid.get(&parent_pid) else {
                break;
            };
            let parent = &self.nodes[pidx];
            if parent.pid == pid || parent.exe_name.to_ascii_lowercase() != target_exe {
                break;
            }
            pid = parent.pid;
        }
        let root_idx = *self.index_by_pid.get(&pid)?;
        Some((pid, &self.nodes[root_idx].exe_name))
    }

    /// Resolves an audio session PID into `(group_key, friendly_app_name)`.
    /// Returns `None` for invalid/exited processes or Windows system host processes.
    pub fn resolve_audio_app(&self, pid: u32, session_display_name: &str) -> Option<(String, String)> {
        if pid == 0 {
            return None;
        }

        let exe_stem = if let Some((_, exe_name)) = self.promote_to_tree_root(pid) {
            exe_name
                .strip_suffix(".exe")
                .or_else(|| exe_name.strip_suffix(".EXE"))
                .unwrap_or(exe_name)
                .to_string()
        } else {
            query_process_exe_stem(pid)?
        };

        let stem_trimmed = exe_stem.trim();
        if stem_trimmed.is_empty() {
            return None;
        }

        let stem_lower = stem_trimmed.to_ascii_lowercase();
        if matches!(stem_lower.as_str(), "audiodg" | "svchost" | "system" | "idle") {
            return None;
        }

        let group_key = normalize_audio_group_key(&stem_lower);
        let friendly = if let Some(known) = lookup_known_app_name(&group_key, Some(&stem_lower)) {
            known.to_string()
        } else {
            let disp = session_display_name.trim();
            if !disp.is_empty()
                && !disp.starts_with('@')
                && !disp.contains('\\')
                && !disp.contains('/')
            {
                disp.to_string()
            } else {
                stem_trimmed.to_string()
            }
        };

        Some((group_key, friendly))
    }
}

/// Captures a reusable process snapshot for resolving audio session applications.
pub fn snapshot_process_lookup() -> ProcessLookup {
    ProcessLookup::from_nodes(snapshot_processes())
}

fn wide_slice_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn query_process_exe_stem(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let _guard = OwnedHandle(handle);
        let mut buf = [0u16; MAX_PATH as usize * 2];
        let mut len = buf.len() as u32;
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
            .ok()?;
        let path_str = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&path_str)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    }
}

fn query_process_aumid(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let _guard = OwnedHandle(handle);
        let mut len = 0u32;
        let _ = GetApplicationUserModelId(handle, &mut len, PWSTR::null());
        if len == 0 || len > 512 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        let err = GetApplicationUserModelId(handle, &mut len, PWSTR(buf.as_mut_ptr()));
        if err.0 == 0 {
            Some(wide_slice_to_string(&buf))
        } else {
            None
        }
    }
}

fn snapshot_processes() -> Vec<ProcNode> {
    let mut nodes = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return nodes;
        };
        let _guard = OwnedHandle(snap);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                nodes.push(ProcNode {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    exe_name: wide_slice_to_string(&entry.szExeFile),
                });
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
    }
    nodes
}

fn normalize_token(raw: &str) -> String {
    let base = raw.rsplit(['\\', '/']).next().unwrap_or(raw);
    let before_bang = base.split('!').next().unwrap_or(base);
    let no_ext = before_bang
        .strip_suffix(".exe")
        .or_else(|| before_bang.strip_suffix(".EXE"))
        .unwrap_or(before_bang);
    no_ext.to_ascii_lowercase()
}

fn match_score(source_id: &str, exe_stem: &str, aumid: Option<&str>) -> u32 {
    let src_lower = source_id.to_ascii_lowercase();
    let src_norm = normalize_token(source_id);
    let exe_lower = exe_stem.to_ascii_lowercase();

    if let Some(id) = aumid {
        let id_lower = id.to_ascii_lowercase();
        if id_lower == src_lower {
            return 100;
        }
        if let (Some(a), Some(b)) = (id_lower.split('!').next(), src_lower.split('!').next()) {
            if !a.is_empty() && a == b {
                return 90;
            }
        }
    }

    if exe_lower == src_norm {
        return 85;
    }
    if !exe_lower.is_empty() && (src_lower.contains(&exe_lower) || exe_lower.contains(&src_norm)) {
        return 60;
    }
    if src_lower.contains("zunemusic") && exe_lower.contains("microsoft.media.player") {
        return 75;
    }
    if src_lower.contains("applemusic") && exe_lower.contains("applemusic") {
        return 75;
    }
    0
}

fn find_audio_session_pid(source_id: &str) -> Option<u32> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let mut best: Option<(u32, u32)> = None;

        for role in [eMultimedia, eConsole] {
            let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, role) else {
                continue;
            };
            let Ok(mgr) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = mgr.GetSessionEnumerator() else {
                continue;
            };
            let count = sessions.GetCount().unwrap_or(0);

            for i in 0..count {
                let Ok(ctrl) = sessions.GetSession(i) else {
                    continue;
                };
                let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let Ok(pid) = ctrl2.GetProcessId() else {
                    continue;
                };
                if pid == 0 {
                    continue;
                }

                let state = ctrl2.GetState().ok();
                let is_active = state == Some(AudioSessionStateActive);

                let session_ident = ctrl2
                    .GetSessionIdentifier()
                    .ok()
                    .map(|pwstr| {
                        let s = pwstr.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(pwstr.0 as *const _));
                        s
                    })
                    .unwrap_or_default();

                let exe_stem = query_process_exe_stem(pid).unwrap_or_default();
                let aumid = query_process_aumid(pid);
                let mut score = match_score(source_id, &exe_stem, aumid.as_deref());
                if score == 0 && !session_ident.is_empty() {
                    let norm = normalize_token(source_id);
                    if !norm.is_empty() && session_ident.to_ascii_lowercase().contains(&norm) {
                        score = 55;
                    }
                }

                if score > 0 {
                    let final_score = score + if is_active { 25 } else { 0 };
                    if best.is_none_or(|(bs, _)| final_score > bs) {
                        best = Some((final_score, pid));
                    }
                }
            }
        }
        best.map(|(_, pid)| pid)
    }
}

/// Resolves the best process ID (preferably tree root or active audio host) for an SMTC SourceAppUserModelId.
pub fn resolve_pid_for_source(source_id: &str) -> Option<u32> {
    let nodes = snapshot_processes();
    let by_pid: HashMap<u32, &ProcNode> = nodes.iter().map(|n| (n.pid, n)).collect();

    // First check active audio sessions (especially important when browser audio runs in a utility process).
    let audio_pid = find_audio_session_pid(source_id);

    // Promote a child process to its same-executable root ancestor so Process Loopback captures the tree.
    let promote_to_tree_root = |mut pid: u32| -> u32 {
        let target_exe = by_pid
            .get(&pid)
            .map(|n| n.exe_name.to_ascii_lowercase())
            .unwrap_or_default();
        for _ in 0..8 {
            let Some(node) = by_pid.get(&pid) else {
                break;
            };
            let Some(parent) = by_pid.get(&node.parent_pid) else {
                break;
            };
            if parent.pid == pid || parent.exe_name.to_ascii_lowercase() != target_exe {
                break;
            }
            pid = parent.pid;
        }
        pid
    };

    if let Some(pid) = audio_pid {
        return Some(promote_to_tree_root(pid));
    }

    // Fallback: match against all running processes via AUMID or executable name.
    let mut best: Option<(u32, u32)> = None;
    for node in &nodes {
        let stem = node
            .exe_name
            .strip_suffix(".exe")
            .or_else(|| node.exe_name.strip_suffix(".EXE"))
            .unwrap_or(&node.exe_name);
        let aumid = if source_id.contains('!') {
            query_process_aumid(node.pid)
        } else {
            None
        };
        let score = match_score(source_id, stem, aumid.as_deref());
        if score > 0 {
            let root = promote_to_tree_root(node.pid);
            let bonus = if root == node.pid { 10 } else { 0 };
            let total = score + bonus;
            if best.is_none_or(|(bs, _)| total > bs) {
                best = Some((total, root));
            }
        }
    }

    best.map(|(_, pid)| pid)
}

fn normalize_audio_group_key(stem_lower: &str) -> String {
    if stem_lower == "steamwebhelper" || stem_lower.starts_with("steamwebhelper") {
        return "steam".to_string();
    }
    if stem_lower == "weixin" || stem_lower.starts_with("wechat") {
        return "wechat".to_string();
    }
    if stem_lower == "obs64" || stem_lower == "obs32" {
        return "obs".to_string();
    }
    if stem_lower.starts_with("dingtalk") {
        return "dingtalk".to_string();
    }
    stem_lower.to_string()
}

fn lookup_known_app_name(token_lower: &str, exact_stem_lower: Option<&str>) -> Option<&'static str> {
    if let Some(stem) = exact_stem_lower {
        match stem {
            "qq" => return Some("QQ"),
            "tim" => return Some("TIM"),
            "obs" | "obs64" | "obs32" => return Some("OBS Studio"),
            _ => {}
        }
    }

    const KNOWN: &[(&str, &str)] = &[
        ("cloudmusic", "网易云音乐"),
        ("163music", "网易云音乐"),
        ("qqmusic", "QQ音乐"),
        ("kugou", "酷狗音乐"),
        ("kuwo", "酷我音乐"),
        ("spotify", "Spotify"),
        ("applemusic", "Apple Music"),
        ("apple.music", "Apple Music"),
        ("ytmusic", "YouTube Music"),
        ("lx-music", "LX Music"),
        ("yesplaymusic", "YesPlayMusic"),
        ("foobar2000", "foobar2000"),
        ("aimp", "AIMP"),
        ("potplayer", "PotPlayer"),
        ("vlc", "VLC"),
        ("mpv", "mpv"),
        ("msedge", "Microsoft Edge"),
        ("chrome", "Google Chrome"),
        ("firefox", "Firefox"),
        ("zunemusic", "媒体播放器"),
        ("microsoft.media.player", "媒体播放器"),
        ("bilibili", "哔哩哔哩"),
        ("tidal", "TIDAL"),
        ("wechat", "微信"),
        ("weixin", "微信"),
        ("dingtalk", "钉钉"),
        ("feishu", "飞书"),
        ("lark", "飞书"),
        ("discord", "Discord"),
        ("telegram", "Telegram"),
        ("steam", "Steam"),
        ("obs64", "OBS Studio"),
        ("obs32", "OBS Studio"),
    ];

    for (needle, label) in KNOWN {
        if token_lower.contains(needle) {
            return Some(*label);
        }
    }
    None
}

/// Maps raw SMTC AUMID and optional PID into a clean human-friendly player name.
pub fn friendly_app_name(source_id: &str, pid: Option<u32>) -> String {
    let lower = source_id.to_ascii_lowercase();
    if let Some(label) = lookup_known_app_name(&lower, None) {
        return label.to_string();
    }

    if let Some(p) = pid {
        if let Some(stem) = query_process_exe_stem(p) {
            let stem_lower = stem.to_ascii_lowercase();
            if let Some(label) = lookup_known_app_name(&stem_lower, Some(&stem_lower)) {
                return label.to_string();
            }
            if !stem.is_empty() {
                return stem;
            }
        }
    }

    let cleaned = normalize_token(source_id);
    if cleaned.is_empty() {
        "媒体播放器".to_string()
    } else {
        cleaned
    }
}

struct EnumTarget {
    pids: Vec<u32>,
    best_hwnd: Option<HWND>,
}

unsafe extern "system" fn enum_windows_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let target = &mut *(lparam.0 as *mut EnumTarget);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
    if !owner.is_invalid() && owner.0 as isize != 0 {
        return BOOL(1);
    }
    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
        return BOOL(1);
    }
    if GetWindowTextLengthW(hwnd) <= 0 {
        return BOOL(1);
    }

    let mut win_pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut win_pid));
    if target.pids.contains(&win_pid) {
        target.best_hwnd = Some(hwnd);
        return BOOL(0);
    }
    BOOL(1)
}

/// Brings the active media player's main window to the foreground (restoring if minimized).
pub fn activate_player_window(source_id: &str, hint_pid: Option<u32>) {
    let mut pids = Vec::new();
    if let Some(p) = hint_pid {
        pids.push(p);
    }

    // Include sibling processes sharing the same executable (useful for multi-process Electron/browsers).
    let nodes = snapshot_processes();
    let target_exe = hint_pid
        .and_then(|p| nodes.iter().find(|n| n.pid == p))
        .map(|n| n.exe_name.to_ascii_lowercase());
    let norm_src = normalize_token(source_id);

    for node in &nodes {
        let exe_lower = node.exe_name.to_ascii_lowercase();
        let matches = target_exe
            .as_ref()
            .is_some_and(|te| &exe_lower == te)
            || (!norm_src.is_empty() && exe_lower.contains(&norm_src));
        if matches && !pids.contains(&node.pid) {
            pids.push(node.pid);
        }
    }

    if pids.is_empty() {
        return;
    }

    let mut state = EnumTarget {
        pids,
        best_hwnd: None,
    };

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_cb),
            LPARAM((&mut state as *mut EnumTarget) as isize),
        );

        let Some(hwnd) = state.best_hwnd else {
            return;
        };

        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        if !SetForegroundWindow(hwnd).as_bool() {
            let cur_tid = GetCurrentThreadId();
            let fg_hwnd = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
            let fg_tid = if !fg_hwnd.is_invalid() {
                GetWindowThreadProcessId(fg_hwnd, None)
            } else {
                0
            };
            if fg_tid != 0 && fg_tid != cur_tid {
                let _ = AttachThreadInput(cur_tid, fg_tid, true);
                let _ = SetForegroundWindow(hwnd);
                let _ = AttachThreadInput(cur_tid, fg_tid, false);
            }
        }
    }
}
