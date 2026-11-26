//! Windows desktop layer attachment, non-destructive icon visibility guard, folder picker, and shell file ops.

use super::layout::{
    clamp_box_bounds_to_work_area, resolve_monitor_and_clamp_bounds, MonitorWorkRect,
};
use super::DesktopZOrderSnapshot;
use crate::system::window_band;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{BOOL, HMODULE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HDC, HMONITOR,
    MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellItem, SHFileOperationW, ShellExecuteW, FOF_ALLOWUNDO,
    FOF_NOCONFIRMMKDIR, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FO_COPY, FO_MOVE,
    SHFILEOPSTRUCTW, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetAncestor, GetClassNameW, GetForegroundWindow, GetTopWindow,
    GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsChild, IsIconic,
    IsWindowVisible, PostMessageW, ShowWindow, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, GA_ROOT, GWL_EXSTYLE, GW_HWNDNEXT, SW_HIDE, SW_SHOW,
    SW_SHOWNORMAL, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};

/// Work area geometry and stable device name of a Windows monitor.
#[derive(Clone, Debug)]
pub struct MonitorWorkInfo {
    pub monitor_id: String,
    pub work: RECT,
    pub dpi: u32,
}

/// Locates `(desktop_host_hwnd, defview_hwnd)` hosting Windows desktop icons across
/// both Win11 22H2/23H2 (`WorkerW -> SHELLDLL_DefView`) and Win11 24H2 (`Progman -> SHELLDLL_DefView`).
pub fn find_desktop_defview_host() -> Option<(HWND, HWND)> {
    unsafe {
        if let Ok(progman) = FindWindowW(w!("Progman"), None) {
            if !progman.is_invalid() {
                if let Ok(defview) = FindWindowExW(progman, None, w!("SHELLDLL_DefView"), None) {
                    if !defview.is_invalid() {
                        return Some((progman, defview));
                    }
                }
            }
        }

        let mut prev = HWND::default();
        loop {
            let Ok(worker) = FindWindowExW(None, prev, w!("WorkerW"), None) else {
                break;
            };
            if worker.is_invalid() {
                break;
            }
            if let Ok(defview) = FindWindowExW(worker, None, w!("SHELLDLL_DefView"), None) {
                if !defview.is_invalid() {
                    return Some((worker, defview));
                }
            }
            prev = worker;
        }
    }
    None
}

fn find_desktop_listview() -> Option<HWND> {
    let (_, defview) = find_desktop_defview_host()?;
    unsafe {
        let lv = FindWindowExW(defview, None, w!("SysListView32"), None).ok()?;
        (!lv.is_invalid()).then_some(lv)
    }
}

/// Single-pass top-level Z-order probe result used to place DeskBox cleanly across normal & `Win+D` states.
pub struct DesktopZOrderProbe {
    pub snapshot: DesktopZOrderSnapshot,
    pub apps_above_progman_bottom_up: Vec<HWND>,
    pub lowest_same_band_visible_topmost: Option<HWND>,
}

/// Shared single-pass top-level Z-order scan reused across all DeskBox windows on a tick/sync pass.
pub struct DesktopZOrderScan {
    pub desktop_host: HWND,
    progman_rank: Option<usize>,
    /// `(hwnd, rank, is_topmost)` for main `PrettyWindowsDeskBoxWnd` windows.
    box_entries: Vec<(HWND, usize, bool)>,
    real_apps: Vec<(usize, HWND)>,
    same_band_topmosts: Vec<(usize, HWND)>,
    third_party_app_foreground: bool,
}

#[inline]
fn eq_utf16_ascii(buf: &[u16], ascii: &str) -> bool {
    let bytes = ascii.as_bytes();
    if buf.len() != bytes.len() {
        return false;
    }
    buf.iter()
        .zip(bytes.iter())
        .all(|(&w, &b)| w == u16::from(b))
}

impl DesktopZOrderScan {
    /// Walks top-level Z-order once (up to 4096 windows) and caches relationships for all DeskBox windows.
    /// Skips invisible non-box windows up front and compares class names on the stack without heap allocations.
    pub fn capture(
        desktop_host: HWND,
        reference_box_hwnd: HWND,
        known_box_hwnds: &[HWND],
    ) -> Self {
        let self_pid = unsafe { GetCurrentProcessId() };
        let box_band = window_band(reference_box_hwnd);
        let mut progman_rank: Option<usize> = None;
        let mut box_entries: Vec<(HWND, usize, bool)> = Vec::with_capacity(known_box_hwnds.len());
        let mut real_apps: Vec<(usize, HWND)> = Vec::new();
        let mut same_band_topmosts: Vec<(usize, HWND)> = Vec::new();

        unsafe {
            if let Ok(first) = GetTopWindow(None) {
                let mut cur = first;
                let mut cls_buf = [0u16; 64];
                for rank in 0..4096usize {
                    if cur.is_invalid() {
                        break;
                    }
                    if cur == desktop_host {
                        progman_rank = Some(progman_rank.map_or(rank, |prev| prev.min(rank)));
                    } else if known_box_hwnds.contains(&cur) {
                        if IsWindowVisible(cur).as_bool() {
                            let ex = GetWindowLongPtrW(cur, GWL_EXSTYLE) as u32;
                            box_entries.push((cur, rank, (ex & WS_EX_TOPMOST.0) != 0));
                        }
                    } else if IsWindowVisible(cur).as_bool() {
                        let cls_len = GetClassNameW(cur, &mut cls_buf);
                        let cls = if cls_len > 0 {
                            &cls_buf[..cls_len as usize]
                        } else {
                            &[]
                        };
                        if eq_utf16_ascii(cls, "PrettyWindowsDeskBoxWnd")
                            || eq_utf16_ascii(cls, "PrettyWindowsDeskBoxBlurWnd")
                        {
                            // Ignore always-topmost edge drawers not in `known_box_hwnds` and internal blur layers.
                        } else if (eq_utf16_ascii(cls, "Progman")
                            || eq_utf16_ascii(cls, "WorkerW"))
                            && is_desktop_sized_rect(cur)
                            && !is_window_cloaked(cur)
                        {
                            progman_rank = Some(progman_rank.map_or(rank, |prev| prev.min(rank)));
                        } else {
                            let ex = GetWindowLongPtrW(cur, GWL_EXSTYLE) as u32;
                            if (ex & WS_EX_TOPMOST.0) != 0 {
                                let mut pid = 0u32;
                                let _ = GetWindowThreadProcessId(cur, Some(&mut pid));
                                // Exclude all own-process windows (transient menus/tooltips/bars) and desktop shells
                                // so tucking below an anchor never targets an ephemeral or demoting window.
                                if pid != self_pid
                                    && !eq_utf16_ascii(cls, "Progman")
                                    && !eq_utf16_ascii(cls, "WorkerW")
                                    && !is_window_cloaked(cur)
                                {
                                    let same_band = match (box_band, window_band(cur)) {
                                        (Some(bb), Some(cb)) => bb == cb,
                                        _ => true,
                                    };
                                    if same_band {
                                        same_band_topmosts.push((rank, cur));
                                    }
                                }
                            } else if is_real_normal_app_window_visible(cur, ex, self_pid, desktop_host)
                            {
                                real_apps.push((rank, cur));
                            }
                        }
                    }
                    let Ok(next) = GetWindow(cur, GW_HWNDNEXT) else {
                        break;
                    };
                    cur = next;
                }
            }
        }

        Self {
            desktop_host,
            progman_rank,
            box_entries,
            real_apps,
            same_band_topmosts,
            third_party_app_foreground: is_third_party_app_foreground(desktop_host),
        }
    }

    /// Derives a per-box Z-order probe from this shared scan in O(1) without re-walking system windows.
    pub fn probe_for(&self, box_hwnd: HWND) -> DesktopZOrderProbe {
        let box_is_topmost = if box_hwnd.is_invalid() {
            false
        } else {
            let ex = unsafe { GetWindowLongPtrW(box_hwnd, GWL_EXSTYLE) } as u32;
            (ex & WS_EX_TOPMOST.0) != 0
        };
        let box_rank = self
            .box_entries
            .iter()
            .find(|&&(h, _, _)| h == box_hwnd)
            .map(|&(_, r, _)| r);

        // Only consider topmost windows strictly above `desktop_host` so tucking below one can never
        // place a DeskBox behind a raised desktop during `Win+D`.
        let lowest_same_band_topmost = self
            .same_band_topmosts
            .iter()
            .rfind(|&&(r, _)| self.progman_rank.is_none_or(|p| r < p))
            .copied();

        let is_box_below_progman = match (box_rank, self.progman_rank) {
            (Some(b), Some(p)) => b > p,
            _ => false,
        };
        let any_peer_below_progman = self
            .progman_rank
            .is_some_and(|p| self.box_entries.iter().any(|&(_, r, _)| r > p));
        let any_peer_topmost = self.box_entries.iter().any(|&(_, _, top)| top);

        let any_real_app_below_progman = self
            .progman_rank
            .is_some_and(|p| self.real_apps.iter().any(|&(r, _)| r > p));

        let apps_above_progman_bottom_up: Vec<HWND> = match self.progman_rank {
            Some(p) => self
                .real_apps
                .iter()
                .filter(|&&(r, _)| r < p)
                .rev()
                .map(|&(_, h)| h)
                .collect(),
            None => self.real_apps.iter().rev().map(|&(_, h)| h).collect(),
        };
        let any_real_app_above_progman = !apps_above_progman_bottom_up.is_empty();
        let lowest_app_above_progman_rank = match self.progman_rank {
            Some(p) => self
                .real_apps
                .iter()
                .filter(|&&(r, _)| r < p)
                .map(|&(r, _)| r)
                .max(),
            None => self.real_apps.iter().map(|&(r, _)| r).max(),
        };

        let needs_normal_reanchor = match (box_rank, lowest_app_above_progman_rank) {
            (Some(b), Some(app_r)) => b < app_r,
            (None, _) => true,
            _ => false,
        };

        let box_above_same_band_topmost = match (box_rank, lowest_same_band_topmost) {
            (Some(b), Some((top_r, _))) => box_is_topmost && b < top_r,
            _ => false,
        };

        DesktopZOrderProbe {
            snapshot: DesktopZOrderSnapshot {
                box_is_topmost,
                is_box_below_progman,
                any_peer_below_progman,
                any_peer_topmost,
                any_real_app_below_progman,
                any_real_app_above_progman,
                third_party_app_foreground: self.third_party_app_foreground,
                needs_normal_reanchor,
                box_above_same_band_topmost,
            },
            apps_above_progman_bottom_up,
            lowest_same_band_visible_topmost: lowest_same_band_topmost.map(|(_, h)| h),
        }
    }
}

fn is_desktop_sized_rect(hwnd: HWND) -> bool {
    let mut r = RECT::default();
    let ok = unsafe { GetWindowRect(hwnd, &mut r).is_ok() };
    ok && (r.right - r.left) >= 640 && (r.bottom - r.top) >= 480
}

pub fn is_window_cloaked(hwnd: HWND) -> bool {
    unsafe {
        let mut cloaked: u32 = 0;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
    }
}

fn is_real_normal_app_window_visible(
    hwnd: HWND,
    ex_style: u32,
    self_pid: u32,
    desktop_host: HWND,
) -> bool {
    if hwnd.is_invalid() || hwnd == desktop_host {
        return false;
    }
    unsafe {
        if (ex_style & (WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0)) != 0 {
            return false;
        }
        if IsIconic(hwnd).as_bool() {
            return false;
        }
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == self_pid {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err()
            || (rect.right - rect.left) < 24
            || (rect.bottom - rect.top) < 24
        {
            return false;
        }
        if is_window_cloaked(hwnd) {
            return false;
        }
        !crate::system::is_shell_or_flyout_window(hwnd)
    }
}

/// Returns `true` when the active foreground window belongs to a visible uncloaked third-party app
/// (excluding desktop hosts, self, and shell flyouts).
pub fn is_third_party_app_foreground(desktop_host: HWND) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let root = GetAncestor(fg, GA_ROOT);
        let target = if root.is_invalid() { fg } else { root };
        if target == desktop_host
            || (!desktop_host.is_invalid() && IsChild(desktop_host, target).as_bool())
            || !IsWindowVisible(target).as_bool()
            || IsIconic(target).as_bool()
            || is_window_cloaked(target)
        {
            return false;
        }
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(target, Some(&mut pid));
        if pid == 0 || pid == GetCurrentProcessId() {
            return false;
        }
        !crate::system::is_shell_or_flyout_window(target)
    }
}

/// Captures a single-pass top-level Z-order snapshot around `box_hwnd` and `desktop_host` (up to 4096 windows).
pub fn probe_desktop_zorder(box_hwnd: HWND, desktop_host: HWND) -> DesktopZOrderProbe {
    DesktopZOrderScan::capture(desktop_host, box_hwnd, &[box_hwnd]).probe_for(box_hwnd)
}

static ZORDER_HOOK_HOST: AtomicIsize = AtomicIsize::new(0);
static ZORDER_HOOK_MSG: AtomicU32 = AtomicU32::new(0);
static ZORDER_NOTIFY_PENDING: AtomicBool = AtomicBool::new(false);

/// Clears the coalescing flag so subsequent foreground/minimize WinEvents can trigger an immediate Z-order sync.
pub fn clear_zorder_notify_pending() {
    ZORDER_NOTIFY_PENDING.store(false, Ordering::Release);
}

unsafe extern "system" fn deskbox_zorder_win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if id_object != 0 {
        return;
    }
    if event == EVENT_OBJECT_HIDE_CODE || event == EVENT_OBJECT_CLOAKED_CODE {
        let mut buf = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut buf);
        if len <= 0 || !eq_utf16_ascii(&buf[..len as usize], "PrettyWindowsDeskBoxWnd") {
            return;
        }
    } else if event == EVENT_SYSTEM_FOREGROUND {
        // Ignore own-process focus shifts (e.g., BoxContextMenu or TrayMenu opening/closing) so
        // popup interactions never trigger redundant Z-order scans or 1.2s follow-up polling.
        let mut fg_pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut fg_pid));
        if fg_pid == GetCurrentProcessId() {
            return;
        }
        // Clicking taskbar/tray icons briefly focuses `Shell_TrayWnd` right as our tray/bar
        // popup menus open; ignore taskbar chrome focus so we don't trigger a Z-order scan
        // storm during popup open animations.
        let mut buf = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut buf);
        if len > 0 {
            let cls = &buf[..len as usize];
            if eq_utf16_ascii(cls, "Shell_TrayWnd")
                || eq_utf16_ascii(cls, "Shell_SecondaryTrayWnd")
                || eq_utf16_ascii(cls, "NotifyIconOverflowWindow")
            {
                return;
            }
        }
    }
    let host_raw = ZORDER_HOOK_HOST.load(Ordering::Acquire);
    let msg = ZORDER_HOOK_MSG.load(Ordering::Acquire);
    if host_raw == 0 || msg == 0 {
        return;
    }
    if ZORDER_NOTIFY_PENDING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let target = HWND(host_raw as *mut core::ffi::c_void);
        if PostMessageW(target, msg, WPARAM(0), LPARAM(0)).is_err() {
            ZORDER_NOTIFY_PENDING.store(false, Ordering::Release);
        }
    }
}

const EVENT_OBJECT_HIDE_CODE: u32 = 0x8003;
const EVENT_OBJECT_CLOAKED_CODE: u32 = 0x8017;

/// Lightweight RAII WinEvent hook listening to foreground, minimize/restore, and own-process
/// hide/cloak transitions for instant DeskBox Z-order and visibility recovery.
#[derive(Default)]
pub struct DeskBoxZOrderEventHook {
    active: bool,
    fg_hook: HWINEVENTHOOK,
    min_hook: HWINEVENTHOOK,
    own_hide_hook: HWINEVENTHOOK,
    own_cloak_hook: HWINEVENTHOOK,
}

impl DeskBoxZOrderEventHook {
    pub fn set_active(&mut self, active: bool, host_hwnd: HWND, notify_msg: u32) {
        if active && !host_hwnd.is_invalid() && notify_msg != 0 {
            ZORDER_HOOK_HOST.store(host_hwnd.0 as isize, Ordering::Release);
            ZORDER_HOOK_MSG.store(notify_msg, Ordering::Release);
            if self.active
                && !self.fg_hook.is_invalid()
                && !self.min_hook.is_invalid()
                && !self.own_hide_hook.is_invalid()
                && !self.own_cloak_hook.is_invalid()
            {
                return;
            }
            self.unhook();
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            unsafe {
                let self_pid = GetCurrentProcessId();
                self.fg_hook = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    HMODULE::default(),
                    Some(deskbox_zorder_win_event_proc),
                    0,
                    0,
                    flags,
                );
                self.min_hook = SetWinEventHook(
                    EVENT_SYSTEM_MINIMIZESTART,
                    EVENT_SYSTEM_MINIMIZEEND,
                    HMODULE::default(),
                    Some(deskbox_zorder_win_event_proc),
                    0,
                    0,
                    flags,
                );
                self.own_hide_hook = SetWinEventHook(
                    EVENT_OBJECT_HIDE_CODE,
                    EVENT_OBJECT_HIDE_CODE,
                    HMODULE::default(),
                    Some(deskbox_zorder_win_event_proc),
                    self_pid,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                self.own_cloak_hook = SetWinEventHook(
                    EVENT_OBJECT_CLOAKED_CODE,
                    EVENT_OBJECT_CLOAKED_CODE,
                    HMODULE::default(),
                    Some(deskbox_zorder_win_event_proc),
                    self_pid,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
            }
            self.active = true;
        } else {
            self.unhook();
            ZORDER_NOTIFY_PENDING.store(false, Ordering::Release);
            ZORDER_HOOK_HOST.store(0, Ordering::Release);
            ZORDER_HOOK_MSG.store(0, Ordering::Release);
            self.active = false;
        }
    }

    fn unhook(&mut self) {
        unsafe {
            if !self.fg_hook.is_invalid() {
                let _ = UnhookWinEvent(self.fg_hook);
                self.fg_hook = HWINEVENTHOOK::default();
            }
            if !self.min_hook.is_invalid() {
                let _ = UnhookWinEvent(self.min_hook);
                self.min_hook = HWINEVENTHOOK::default();
            }
            if !self.own_hide_hook.is_invalid() {
                let _ = UnhookWinEvent(self.own_hide_hook);
                self.own_hide_hook = HWINEVENTHOOK::default();
            }
            if !self.own_cloak_hook.is_invalid() {
                let _ = UnhookWinEvent(self.own_cloak_hook);
                self.own_cloak_hook = HWINEVENTHOOK::default();
            }
        }
    }
}

impl Drop for DeskBoxZOrderEventHook {
    fn drop(&mut self) {
        self.unhook();
        ZORDER_NOTIFY_PENDING.store(false, Ordering::Release);
        ZORDER_HOOK_HOST.store(0, Ordering::Release);
        ZORDER_HOOK_MSG.store(0, Ordering::Release);
    }
}

/// Non-destructively hides the Windows desktop `SysListView32` via `ShowWindow` (zero registry writes)
/// and automatically restores visibility if dropped while hidden.
#[derive(Default)]
pub struct DesktopIconGuard {
    want_hidden: bool,
    hidden_by_us: bool,
}

impl DesktopIconGuard {
    pub fn set_hidden(&mut self, hide: bool) {
        self.want_hidden = hide;
        self.sync();
    }

    pub fn reapply_after_shell_restart(&mut self) {
        if self.want_hidden {
            self.hidden_by_us = false;
            self.sync();
        }
    }

    fn sync(&mut self) {
        let Some(lv) = find_desktop_listview() else {
            return;
        };
        unsafe {
            let currently_visible = IsWindowVisible(lv).as_bool();
            if self.want_hidden {
                if currently_visible {
                    let _ = ShowWindow(lv, SW_HIDE);
                    self.hidden_by_us = true;
                }
            } else if self.hidden_by_us {
                if !currently_visible {
                    let _ = ShowWindow(lv, SW_SHOW);
                }
                self.hidden_by_us = false;
            }
        }
    }

    pub fn restore_now(&mut self) {
        self.want_hidden = false;
        self.sync();
    }
}

impl Drop for DesktopIconGuard {
    fn drop(&mut self) {
        self.restore_now();
    }
}

/// Opens the Win11 modern modal folder picker and returns the selected filesystem directory.
/// Callers must dismiss any active popup menus and release `RefCell` borrows before invoking.
pub fn pick_folder_dialog(owner: HWND) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL).ok()?;
        let opts = dialog.GetOptions().ok()?;
        dialog
            .SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
            .ok()?;
        let title = HSTRING::from("选择要关联的文件夹");
        let _ = dialog.SetTitle(&title);
        if dialog.Show(owner).is_err() {
            return None;
        }
        let item: IShellItem = dialog.GetResult().ok()?;
        let pwstr: PWSTR = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path_str = pwstr.to_string().ok();
        CoTaskMemFree(Some(pwstr.0.cast()));
        path_str
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
    }
}

/// Launches a file, folder, application, or shortcut using Windows Shell default verb.
pub fn open_item_in_shell(path: &Path) -> bool {
    let target_wide = HSTRING::from(path.as_os_str());
    let parent_wide = if path.is_file() {
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| HSTRING::from(p.as_os_str()))
    } else {
        None
    };
    let dir_ptr = parent_wide
        .as_ref()
        .map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr()));
    let ret = unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(target_wide.as_ptr()),
            PCWSTR::null(),
            dir_ptr,
            SW_SHOWNORMAL,
        )
    };
    (ret.0 as isize) > 32
}

/// Opens a directory in Windows File Explorer.
pub fn open_folder_in_explorer(dir: &Path) -> bool {
    let dir_wide = HSTRING::from(dir.as_os_str());
    let ret = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(dir_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    (ret.0 as isize) > 32
}

/// Moves or copies `sources` into `dest_dir` using Explorer's `SHFileOperationW` (supports Undo/conflicts).
/// Must be called outside any `RefCell` borrow because Windows may display a progress/conflict dialog.
pub fn shell_move_or_copy_files(
    owner: HWND,
    sources: &[PathBuf],
    dest_dir: &Path,
    copy: bool,
) -> bool {
    if sources.is_empty() || !dest_dir.is_dir() {
        return false;
    }

    let dest_key = super::config::normalize_path_key(&dest_dir.to_string_lossy());
    if dest_key.is_empty() {
        return false;
    }

    let mut from_buf: Vec<u16> = Vec::new();
    for src in sources {
        if src.as_os_str().is_empty() {
            continue;
        }
        let src_key = super::config::normalize_path_key(&src.to_string_lossy());
        if src_key.is_empty() || src_key == dest_key {
            continue;
        }
        // Prevent moving OR copying a folder into one of its own descendants (case-insensitive).
        let is_descendant = if src_key.ends_with('\\') {
            dest_key.starts_with(&src_key)
        } else {
            dest_key.starts_with(&src_key)
                && dest_key.as_bytes().get(src_key.len()) == Some(&b'\\')
        };
        if is_descendant {
            continue;
        }
        let wide: Vec<u16> = src.as_os_str().encode_wide().collect();
        if wide.is_empty() || wide.contains(&0) {
            continue;
        }
        from_buf.extend_from_slice(&wide);
        from_buf.push(0);
    }
    if from_buf.is_empty() {
        return false;
    }
    from_buf.push(0);

    let mut to_buf: Vec<u16> = dest_dir.as_os_str().encode_wide().collect();
    to_buf.push(0);
    to_buf.push(0);

    let mut op = SHFILEOPSTRUCTW {
        hwnd: owner,
        wFunc: if copy { FO_COPY } else { FO_MOVE },
        pFrom: PCWSTR(from_buf.as_ptr()),
        pTo: PCWSTR(to_buf.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR).0 as u16,
        ..Default::default()
    };

    let code = unsafe { SHFileOperationW(&mut op) };
    code == 0 && !op.fAnyOperationsAborted.as_bool()
}

/// Queries work area, device name, and effective DPI for a specific `HMONITOR`.
pub fn query_monitor_info(mon: HMONITOR) -> MonitorWorkInfo {
    unsafe {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let ok = GetMonitorInfoW(mon, (&mut mi as *mut MONITORINFOEXW).cast()).as_bool();
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let dpi = if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok()
            && dpi_x >= 96
        {
            dpi_x
        } else {
            96
        };
        if ok {
            let end = mi
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(mi.szDevice.len());
            MonitorWorkInfo {
                monitor_id: String::from_utf16_lossy(&mi.szDevice[..end]),
                work: mi.monitorInfo.rcWork,
                dpi,
            }
        } else {
            MonitorWorkInfo {
                monitor_id: String::new(),
                work: RECT {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1032,
                },
                dpi,
            }
        }
    }
}

pub fn monitor_info_at_point(pt: POINT) -> MonitorWorkInfo {
    let mon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    query_monitor_info(mon)
}

pub fn monitor_info_for_window(hwnd: HWND) -> MonitorWorkInfo {
    let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    query_monitor_info(mon)
}

/// Finds the monitor matching `monitor_id`, falling back cleanly to the primary display.
pub fn resolve_monitor_info(monitor_id: &str) -> MonitorWorkInfo {
    let trimmed = monitor_id.trim();
    if !trimmed.is_empty() {
        let monitors = enumerate_monitors();
        if let Some(found) = monitors
            .into_iter()
            .find(|m| m.monitor_id.eq_ignore_ascii_case(trimmed))
        {
            return found;
        }
    }
    let primary_mon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTONEAREST) };
    query_monitor_info(primary_mon)
}

/// Resolves clamped `(x_dip, y_dip, w_dip, h_dip, win_x, win_y, win_w, win_h, monitor)`
/// for a saved box specification using deterministic primary fallback when a saved monitor was unplugged.
pub fn resolve_box_window_geometry(
    monitor_id: &str,
    x_dip: f32,
    y_dip: f32,
    w_dip: f32,
    h_dip: f32,
    collapsed: bool,
) -> (f32, f32, f32, f32, i32, i32, i32, i32, MonitorWorkInfo) {
    let monitors = enumerate_monitors();
    if !monitors.is_empty() {
        let primary_mon =
            unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTONEAREST) };
        let primary_id = query_monitor_info(primary_mon).monitor_id;
        let rects: Vec<MonitorWorkRect> = monitors
            .iter()
            .map(|m| MonitorWorkRect {
                monitor_id: m.monitor_id.clone(),
                left: m.work.left,
                top: m.work.top,
                right: m.work.right,
                bottom: m.work.bottom,
                dpi: m.dpi,
                is_primary: !primary_id.is_empty() && m.monitor_id.eq_ignore_ascii_case(&primary_id),
            })
            .collect();
        if let Some(res) = resolve_monitor_and_clamp_bounds(
            &rects,
            monitor_id,
            x_dip,
            y_dip,
            w_dip,
            h_dip,
            collapsed,
        ) {
            return (
                res.x_dip,
                res.y_dip,
                res.w_dip,
                res.h_dip,
                res.win_x,
                res.win_y,
                res.win_w,
                res.win_h,
                monitors[res.monitor_idx].clone(),
            );
        }
    }

    let info = resolve_monitor_info(monitor_id);
    let (cx, cy, cw, ch, wx, wy, ww, wh) = clamp_box_bounds_to_work_area(
        x_dip,
        y_dip,
        w_dip,
        h_dip,
        collapsed,
        info.work.left,
        info.work.top,
        info.work.right,
        info.work.bottom,
        info.dpi,
    );
    (cx, cy, cw, ch, wx, wy, ww, wh, info)
}

fn enumerate_monitors() -> Vec<MonitorWorkInfo> {
    unsafe extern "system" fn enum_proc(
        mon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<MonitorWorkInfo>);
        list.push(query_monitor_info(mon));
        BOOL(1)
    }

    let mut list = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut list as *mut Vec<MonitorWorkInfo> as isize),
        );
    }
    list
}
