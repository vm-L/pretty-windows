/// Extracts the executable filename from a Windows/POSIX path.
pub(crate) fn extract_exe_basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path).trim()
}

/// Returns `true` if `class_name` belongs to Windows desktop surfaces or modern shell flyouts
/// (Start, Quick Settings, Search, System Tray overflow, Input switcher, etc.).
pub(crate) fn is_shell_or_flyout_class_name(class_name: &str) -> bool {
    matches!(
        class_name,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "ControlCenterWindow"
            | "TopLevelWindowForOverflowXamlIsland"
            | "XamlExplorerHostIslandWindow"
            | "NotifyIconOverflowWindow"
            | "Shell_InputSwitchTopLevelWindow"
            | "Windows.UI.Core.CoreWindow"
            | "Windows.UI.Input.InputSite.WindowClass"
    )
}

/// Returns `true` if `exe_name` is a known Windows shell flyout host process.
///
/// Intentionally excludes `explorer.exe` because fullscreen File Explorer (`CabinetWClass`)
/// should still count as fullscreen occlusion.
pub(crate) fn is_shell_flyout_process_name(exe_name: &str) -> bool {
    let name = exe_name.trim();
    name.eq_ignore_ascii_case("shellhost.exe")
        || name.eq_ignore_ascii_case("shellexperiencehost.exe")
        || name.eq_ignore_ascii_case("startmenuexperiencehost.exe")
        || name.eq_ignore_ascii_case("searchhost.exe")
        || name.eq_ignore_ascii_case("searchapp.exe")
        || name.eq_ignore_ascii_case("textinputhost.exe")
        || name.eq_ignore_ascii_case("widgets.exe")
}

#[cfg(windows)]
mod win {
    use crate::color::Rgb;
    use std::cell::RefCell;
    use std::mem::size_of;
    use windows::core::{w, PCWSTR, PWSTR};
    use windows::UI::ViewManagement::{UIColorType, UISettings};
    use windows::Win32::Foundation::{CloseHandle, HMODULE, HWND, RECT};
    use windows::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXCLUDED_FROM_PEEK,
    };
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_SZ, REG_VALUE_TYPE,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetAncestor, GetClassNameW, GetDesktopWindow, GetForegroundWindow,
        GetShellWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
        IsIconic, IsWindow, IsWindowVisible, SetWindowPos, EVENT_OBJECT_FOCUS,
        EVENT_OBJECT_REORDER, EVENT_SYSTEM_CAPTUREEND, EVENT_SYSTEM_CAPTURESTART,
        EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GA_ROOT,
        GWL_EXSTYLE, GW_HWNDPREV, GW_OWNER, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOOWNERZORDER, SWP_NOSIZE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
        WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    };

    const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const RUN_VALUE_NAME: PCWSTR = w!("PrettyWindows");
    const LEGACY_RUN_VALUE_NAME: PCWSTR = w!("MediaBar");

    /// Reads a `REG_DWORD` value from the Windows Registry.
    pub(crate) fn read_reg_dword(root: HKEY, subkey: PCWSTR, value_name: PCWSTR) -> Option<u32> {
        unsafe {
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(root, subkey, 0, KEY_READ, &mut hkey).0 != 0 {
                return None;
            }
            let mut val = 0u32;
            let mut val_type = REG_VALUE_TYPE::default();
            let mut size = size_of::<u32>() as u32;
            let q = RegQueryValueExW(
                hkey,
                value_name,
                None,
                Some(&mut val_type),
                Some((&mut val as *mut u32).cast::<u8>()),
                Some(&mut size),
            );
            let _ = RegCloseKey(hkey);
            (q.0 == 0 && val_type == REG_DWORD).then_some(val)
        }
    }

    /// Reads Windows 11 taskbar/system theme (`true` = light theme, `false` = dark theme).
    pub fn is_light_taskbar() -> bool {
        read_reg_dword(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
        ) == Some(1)
    }

    /// Reads the current Windows System Accent Color, falling back to classic Win11 blue.
    pub fn system_accent_color() -> Rgb {
        if let Ok(settings) = UISettings::new() {
            if let Ok(c) = settings.GetColorValue(UIColorType::Accent) {
                return Rgb::new(c.R, c.G, c.B);
            }
        }
        Rgb::new(0, 120, 212)
    }

    /// Returns `true` if Pretty Windows is configured to launch at Windows startup.
    pub fn is_autostart_enabled() -> bool {
        unsafe {
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_READ, &mut hkey).0 != 0 {
                return false;
            }
            let mut size = 0u32;
            let res = RegQueryValueExW(hkey, RUN_VALUE_NAME, None, None, None, Some(&mut size));
            let _ = RegCloseKey(hkey);
            res.0 == 0 && size > 0
        }
    }

    /// Enables or disables auto-start in HKCU Run registry key.
    pub fn set_autostart_enabled(enable: bool) -> bool {
        unsafe {
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_WRITE, &mut hkey).0 != 0 {
                return false;
            }
            let ok = if enable {
                let Ok(exe_path) = std::env::current_exe() else {
                    let _ = RegCloseKey(hkey);
                    return false;
                };
                let quoted = format!("\"{}\"", exe_path.display());
                let wide: Vec<u16> = quoted.encode_utf16().chain(std::iter::once(0)).collect();
                let byte_slice = std::slice::from_raw_parts(
                    wide.as_ptr().cast::<u8>(),
                    wide.len() * size_of::<u16>(),
                );
                let set_ok =
                    RegSetValueExW(hkey, RUN_VALUE_NAME, 0, REG_SZ, Some(byte_slice)).0 == 0;
                if set_ok {
                    let _ = RegDeleteValueW(hkey, LEGACY_RUN_VALUE_NAME);
                }
                set_ok
            } else {
                let err = RegDeleteValueW(hkey, RUN_VALUE_NAME);
                let _ = RegDeleteValueW(hkey, LEGACY_RUN_VALUE_NAME);
                err.0 == 0 || err.0 == 2 // ERROR_FILE_NOT_FOUND
            };
            let _ = RegCloseKey(hkey);
            ok
        }
    }

    /// Exempts a floating taskbar widget from Windows Aero Peek / Show Desktop fade.
    pub fn exclude_from_peek(hwnd: HWND) {
        let val: u32 = 1;
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_EXCLUDED_FROM_PEEK,
                (&val as *const u32).cast(),
                size_of::<u32>() as u32,
            );
        }
    }

    fn is_shell_or_flyout_class(hwnd: HWND) -> bool {
        let mut buf = [0u16; 96];
        let len = unsafe { GetClassNameW(hwnd, &mut buf) };
        if len <= 0 {
            return false;
        }
        let name = String::from_utf16_lossy(&buf[..len as usize]);
        super::is_shell_or_flyout_class_name(&name)
    }

    fn is_shell_flyout_process(hwnd: HWND) -> bool {
        unsafe {
            let mut pid = 0u32;
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return false;
            }
            let Ok(hproc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                hproc,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(hproc);
            if !ok || len == 0 {
                return false;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            super::is_shell_flyout_process_name(super::extract_exe_basename(&path))
        }
    }

    fn is_taskbar_above_window(bar_hwnd: HWND, taskbar_hwnd: HWND) -> bool {
        if bar_hwnd.is_invalid() || taskbar_hwnd.is_invalid() {
            return false;
        }
        unsafe {
            let mut cur = bar_hwnd;
            for _ in 0..512 {
                match GetWindow(cur, GW_HWNDPREV) {
                    Ok(prev) if !prev.is_invalid() => {
                        if prev == taskbar_hwnd {
                            return true;
                        }
                        cur = prev;
                    }
                    _ => break,
                }
            }
            false
        }
    }

    /// Detects whether a genuine fullscreen/presentation window is currently occluding `taskbar_hwnd`.
    ///
    /// Desktop surfaces (`Progman`/`WorkerW`), system shell flyouts (Quick Settings, Start Menu,
    /// Search, TextInputHost, `Windows.UI.Core.CoreWindow`), DWM-cloaked/minimized windows, and
    /// windows sitting below `taskbar_hwnd` in Z-order (such as after `Win+D`) do not count.
    pub fn is_fullscreen_occluding_taskbar(taskbar_hwnd: HWND) -> bool {
        unsafe {
            if let Ok(state) = SHQueryUserNotificationState() {
                if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                    return true;
                }
            }

            let fg = GetForegroundWindow();
            if fg.is_invalid() {
                return false;
            }
            let root = GetAncestor(fg, GA_ROOT);
            let target = if root.is_invalid() { fg } else { root };

            if target == taskbar_hwnd
                || target == GetShellWindow()
                || target == GetDesktopWindow()
                || !IsWindowVisible(target).as_bool()
                || IsIconic(target).as_bool()
            {
                return false;
            }

            let mut cloaked: u32 = 0;
            if DwmGetWindowAttribute(
                target,
                DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0
            {
                return false;
            }

            if is_shell_or_flyout_class(target) || is_shell_flyout_process(target) {
                return false;
            }

            let ex_style = GetWindowLongW(target, GWL_EXSTYLE) as u32;
            if (ex_style & (WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0)) != 0 {
                return false;
            }

            if let Ok(owner) = GetWindow(target, GW_OWNER) {
                if !owner.is_invalid()
                    && (is_shell_or_flyout_class(owner) || is_shell_flyout_process(owner))
                {
                    return false;
                }
            }

            let bar_mon = MonitorFromWindow(taskbar_hwnd, MONITOR_DEFAULTTONEAREST);
            let fg_mon = MonitorFromWindow(target, MONITOR_DEFAULTTONEAREST);
            if bar_mon != fg_mon {
                return false;
            }

            let mut mi = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(bar_mon, &mut mi).as_bool() {
                return false;
            }

            let mut fg_rect = RECT::default();
            if GetWindowRect(target, &mut fg_rect).is_err() {
                return false;
            }
            let covers_monitor = fg_rect.left <= mi.rcMonitor.left
                && fg_rect.top <= mi.rcMonitor.top
                && fg_rect.right >= mi.rcMonitor.right
                && fg_rect.bottom >= mi.rcMonitor.bottom;
            if !covers_monitor {
                return false;
            }

            // If `taskbar_hwnd` sits above `target` in Z-order (e.g. `Win+D` or desktop companion),
            // the taskbar is visible and not occluded.
            !is_taskbar_above_window(target, taskbar_hwnd)
        }
    }

    /// Places `bar_hwnd` immediately above `taskbar_hwnd` without covering higher shell flyouts,
    /// falling back to `HWND_TOPMOST` when needed.
    pub fn ensure_window_above_taskbar(bar_hwnd: HWND, taskbar_hwnd: HWND) {
        unsafe {
            if bar_hwnd.is_invalid()
                || taskbar_hwnd.is_invalid()
                || !IsWindowVisible(bar_hwnd).as_bool()
            {
                return;
            }
            if !is_taskbar_above_window(bar_hwnd, taskbar_hwnd) {
                return;
            }

            if let Ok(above_tb) = GetWindow(taskbar_hwnd, GW_HWNDPREV) {
                if !above_tb.is_invalid() && above_tb != bar_hwnd {
                    let _ = SetWindowPos(
                        bar_hwnd,
                        above_tb,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                    );
                    if !is_taskbar_above_window(bar_hwnd, taskbar_hwnd) {
                        return;
                    }
                }
            }

            let _ = SetWindowPos(
                bar_hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }

    struct TaskbarOverlayGuard {
        widgets: Vec<HWND>,
        in_callback: bool,
        global_fg_hook: HWINEVENTHOOK,
        global_min_hook: HWINEVENTHOOK,
        taskbar_capture_hook: HWINEVENTHOOK,
        taskbar_reorder_hook: HWINEVENTHOOK,
        bound_tb_pid: u32,
        bound_tb_tid: u32,
    }

    impl Default for TaskbarOverlayGuard {
        fn default() -> Self {
            Self {
                widgets: Vec::new(),
                in_callback: false,
                global_fg_hook: HWINEVENTHOOK::default(),
                global_min_hook: HWINEVENTHOOK::default(),
                taskbar_capture_hook: HWINEVENTHOOK::default(),
                taskbar_reorder_hook: HWINEVENTHOOK::default(),
                bound_tb_pid: 0,
                bound_tb_tid: 0,
            }
        }
    }

    impl TaskbarOverlayGuard {
        fn unhook_taskbar_specific(&mut self) {
            unsafe {
                if !self.taskbar_capture_hook.is_invalid() {
                    let _ = UnhookWinEvent(self.taskbar_capture_hook);
                    self.taskbar_capture_hook = HWINEVENTHOOK::default();
                }
                if !self.taskbar_reorder_hook.is_invalid() {
                    let _ = UnhookWinEvent(self.taskbar_reorder_hook);
                    self.taskbar_reorder_hook = HWINEVENTHOOK::default();
                }
            }
            self.bound_tb_pid = 0;
            self.bound_tb_tid = 0;
        }

        fn unhook_all(&mut self) {
            unsafe {
                if !self.global_fg_hook.is_invalid() {
                    let _ = UnhookWinEvent(self.global_fg_hook);
                    self.global_fg_hook = HWINEVENTHOOK::default();
                }
                if !self.global_min_hook.is_invalid() {
                    let _ = UnhookWinEvent(self.global_min_hook);
                    self.global_min_hook = HWINEVENTHOOK::default();
                }
            }
            self.unhook_taskbar_specific();
        }

        fn rebind_taskbar_hooks(&mut self, pid: u32, tid: u32) {
            if self.bound_tb_pid == pid
                && self.bound_tb_tid == tid
                && !self.taskbar_capture_hook.is_invalid()
                && !self.taskbar_reorder_hook.is_invalid()
            {
                return;
            }
            self.unhook_taskbar_specific();
            if pid == 0 || tid == 0 {
                return;
            }
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            unsafe {
                self.taskbar_capture_hook = SetWinEventHook(
                    EVENT_SYSTEM_CAPTURESTART,
                    EVENT_SYSTEM_CAPTUREEND,
                    HMODULE::default(),
                    Some(taskbar_zorder_win_event_proc),
                    pid,
                    tid,
                    flags,
                );
                self.taskbar_reorder_hook = SetWinEventHook(
                    EVENT_OBJECT_REORDER,
                    EVENT_OBJECT_FOCUS,
                    HMODULE::default(),
                    Some(taskbar_zorder_win_event_proc),
                    pid,
                    tid,
                    flags,
                );
            }
            self.bound_tb_pid = pid;
            self.bound_tb_tid = tid;
        }

        fn ensure_hooks(&mut self) {
            if self.widgets.is_empty() {
                self.unhook_all();
                return;
            }
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            unsafe {
                if self.global_fg_hook.is_invalid() {
                    self.global_fg_hook = SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        HMODULE::default(),
                        Some(taskbar_zorder_win_event_proc),
                        0,
                        0,
                        flags,
                    );
                }
                if self.global_min_hook.is_invalid() {
                    self.global_min_hook = SetWinEventHook(
                        EVENT_SYSTEM_MINIMIZESTART,
                        EVENT_SYSTEM_MINIMIZEEND,
                        HMODULE::default(),
                        Some(taskbar_zorder_win_event_proc),
                        0,
                        0,
                        flags,
                    );
                }
            }
            if let Ok(tb_hwnd) = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) } {
                if !tb_hwnd.is_invalid() {
                    let mut pid = 0u32;
                    let tid = unsafe { GetWindowThreadProcessId(tb_hwnd, Some(&mut pid)) };
                    self.rebind_taskbar_hooks(pid, tid);
                }
            }
        }
    }

    impl Drop for TaskbarOverlayGuard {
        fn drop(&mut self) {
            self.unhook_all();
        }
    }

    thread_local! {
        static OVERLAY_GUARD: RefCell<TaskbarOverlayGuard> = RefCell::new(TaskbarOverlayGuard::default());
    }

    struct ReentryResetGuard;

    impl Drop for ReentryResetGuard {
        fn drop(&mut self) {
            OVERLAY_GUARD.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    guard.in_callback = false;
                }
            });
        }
    }

    unsafe extern "system" fn taskbar_zorder_win_event_proc(
        _hwineventhook: HWINEVENTHOOK,
        _event: u32,
        _hwnd: HWND,
        _idobject: i32,
        _idchild: i32,
        _ideventthread: u32,
        _dwmseventtime: u32,
    ) {
        let Some(widgets) = OVERLAY_GUARD.with(|cell| {
            let Ok(mut guard) = cell.try_borrow_mut() else {
                return None;
            };
            if guard.in_callback || guard.widgets.is_empty() {
                return None;
            }
            guard.in_callback = true;
            Some(guard.widgets.clone())
        }) else {
            return;
        };
        let _reset = ReentryResetGuard;

        let Ok(tb_hwnd) = FindWindowW(w!("Shell_TrayWnd"), None) else {
            return;
        };
        if tb_hwnd.is_invalid() {
            return;
        }

        let any_below = widgets
            .iter()
            .copied()
            .any(|w| IsWindowVisible(w).as_bool() && is_taskbar_above_window(w, tb_hwnd));
        if any_below && !is_fullscreen_occluding_taskbar(tb_hwnd) {
            for w in widgets {
                if IsWindowVisible(w).as_bool() {
                    ensure_window_above_taskbar(w, tb_hwnd);
                }
            }
        }

        let mut cur_pid = 0u32;
        let cur_tid = GetWindowThreadProcessId(tb_hwnd, Some(&mut cur_pid));
        let need_rebind = OVERLAY_GUARD.with(|cell| {
            cell.try_borrow().is_ok_and(|g| {
                !g.widgets.is_empty()
                    && (g.bound_tb_pid != cur_pid || g.bound_tb_tid != cur_tid)
            })
        });
        if need_rebind {
            OVERLAY_GUARD.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    guard.rebind_taskbar_hooks(cur_pid, cur_tid);
                }
            });
        }
    }

    /// Registers a taskbar widget HWND with the thread-local event-driven Z-order guard.
    pub fn register_taskbar_overlay(hwnd: HWND) {
        if hwnd.is_invalid() {
            return;
        }
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard
                    .widgets
                    .retain(|&w| !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
                if !guard.widgets.contains(&hwnd) {
                    guard.widgets.push(hwnd);
                }
                guard.ensure_hooks();
            }
        });
    }

    /// Unregisters a taskbar widget HWND and unhooks global/taskbar listeners when no widgets remain.
    pub fn unregister_taskbar_overlay(hwnd: HWND) {
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard
                    .widgets
                    .retain(|&w| w != hwnd && !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
                if guard.widgets.is_empty() {
                    guard.unhook_all();
                }
            }
        });
    }

    /// Refreshes thread-scoped WinEvent hooks after Explorer recreates the taskbar (`TaskbarCreated`).
    pub fn refresh_taskbar_overlay_hooks() {
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard.ensure_hooks();
            }
        });
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_exe_basename_handles_windows_and_posix_paths() {
        assert_eq!(
            extract_exe_basename(r"C:\Windows\SystemApps\ShellHost.exe"),
            "ShellHost.exe"
        );
        assert_eq!(
            extract_exe_basename("C:/Windows/System32/explorer.exe"),
            "explorer.exe"
        );
        assert_eq!(extract_exe_basename("SearchHost.exe"), "SearchHost.exe");
        assert_eq!(extract_exe_basename(""), "");
    }

    #[test]
    fn shell_and_flyout_class_filter_matches_expected_windows_only() {
        assert!(is_shell_or_flyout_class_name("Progman"));
        assert!(is_shell_or_flyout_class_name("WorkerW"));
        assert!(is_shell_or_flyout_class_name("Shell_TrayWnd"));
        assert!(is_shell_or_flyout_class_name("Shell_SecondaryTrayWnd"));
        assert!(is_shell_or_flyout_class_name("ControlCenterWindow"));
        assert!(is_shell_or_flyout_class_name(
            "TopLevelWindowForOverflowXamlIsland"
        ));
        assert!(is_shell_or_flyout_class_name("XamlExplorerHostIslandWindow"));
        assert!(is_shell_or_flyout_class_name("Windows.UI.Core.CoreWindow"));

        assert!(!is_shell_or_flyout_class_name("Chrome_WidgetWin_1"));
        assert!(!is_shell_or_flyout_class_name("ApplicationFrameWindow"));
        assert!(!is_shell_or_flyout_class_name("CabinetWClass"));
    }

    #[test]
    fn shell_flyout_process_filter_is_case_insensitive_and_preserves_explorer() {
        assert!(is_shell_flyout_process_name("ShellHost.exe"));
        assert!(is_shell_flyout_process_name("SHELLEXPERIENCEHOST.EXE"));
        assert!(is_shell_flyout_process_name("StartMenuExperienceHost.exe"));
        assert!(is_shell_flyout_process_name("SearchHost.exe"));
        assert!(is_shell_flyout_process_name("TextInputHost.exe"));
        assert!(is_shell_flyout_process_name("Widgets.exe"));

        assert!(!is_shell_flyout_process_name("explorer.exe"));
        assert!(!is_shell_flyout_process_name("chrome.exe"));
    }
}
