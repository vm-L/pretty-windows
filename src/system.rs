use crate::ui::GlassSurfaceStyle;
use std::sync::atomic::{AtomicU8, Ordering};

static ACTIVE_TASKBAR_SURFACE_STYLE: AtomicU8 =
    AtomicU8::new(GlassSurfaceStyle::LiquidGlass.to_u8());

/// Publishes the active taskbar visual style so all popup surfaces harmonize with the taskbar.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn set_active_taskbar_surface_style(style: GlassSurfaceStyle) {
    ACTIVE_TASKBAR_SURFACE_STYLE.store(style.to_u8(), Ordering::Relaxed);
}

/// Returns the active taskbar visual style published by `TaskbarController`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn active_taskbar_surface_style() -> GlassSurfaceStyle {
    GlassSurfaceStyle::from_u8(ACTIVE_TASKBAR_SURFACE_STYLE.load(Ordering::Relaxed))
}

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

/// Returns `true` if `class_name` represents a transient desktop shell flyout or tooltip window
/// that may vanish right after a popup activates and should be excluded from backdrop captures.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_transient_backdrop_occluder_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "TopLevelWindowForOverflowXamlIsland"
            | "NotifyIconOverflowWindow"
            | "XamlExplorerHostIslandWindow"
            | "ControlCenterWindow"
            | "tooltips_class32"
            | "Xaml_WindowedPopupClass"
            | "TaskListThumbnailWnd"
    )
}

/// Picks the lowest (largest `z_rank`) same-band window among the taskbar and visible taskbar
/// overlay widgets on the same monitor so a behind-taskbar backdrop never jumps above widgets
/// even when Windows temporarily elevates `Shell_TrayWnd` to a higher Z-order band (e.g. Start Menu).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn pick_lowest_backdrop_anchor(
    taskbar_id: usize,
    taskbar_band: Option<u32>,
    backdrop_band: Option<u32>,
    taskbar_z_rank: Option<usize>,
    same_band_overlays_with_rank: &[(usize, usize)],
) -> usize {
    let mut best: Option<(usize, usize)> = None;
    let tb_same_band = match (taskbar_band, backdrop_band) {
        (Some(tb), Some(bg)) => tb == bg,
        _ => true,
    };
    if tb_same_band {
        if let Some(rank) = taskbar_z_rank {
            best = Some((taskbar_id, rank));
        }
    }
    for &(id, rank) in same_band_overlays_with_rank {
        if best.is_none_or(|(_, best_rank)| rank > best_rank) {
            best = Some((id, rank));
        }
    }
    best.map(|(id, _)| id).unwrap_or(taskbar_id)
}

#[cfg(windows)]
mod win {
    use crate::color::{derive_background_accent, Rgb};
    use std::cell::RefCell;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering as WinOrdering};
    use std::sync::{Mutex, OnceLock};
    use windows::core::{s, w, PCWSTR, PWSTR};
    use windows::UI::ViewManagement::{UIColorType, UISettings};
    use windows::Win32::Foundation::{CloseHandle, BOOL, HMODULE, HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXCLUDED_FROM_PEEK,
        DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
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
        EnumWindows, FindWindowW, GetAncestor, GetClassNameW, GetDesktopWindow,
        GetForegroundWindow, GetShellWindow, GetTopWindow, GetWindow, GetWindowLongW,
        GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        PostMessageW, SetWindowPos, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_FOCUS, EVENT_OBJECT_HIDE,
        EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW,
        EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_CAPTUREEND, EVENT_SYSTEM_CAPTURESTART,
        EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GA_ROOT,
        GWL_EXSTYLE, GW_HWNDNEXT, GW_HWNDPREV, GW_OWNER, HWND_TOPMOST, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
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

    /// Returns `true` when Windows "Transparency effects" (`EnableTransparency`) is enabled.
    /// Defaults to `true` if the registry value is absent.
    pub fn is_transparency_enabled() -> bool {
        read_reg_dword(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("EnableTransparency"),
        ) != Some(0)
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

    fn mediabar_shared_accent_slot() -> &'static Mutex<Option<(Rgb, Rgb)>> {
        static SHARED_ACCENT: OnceLock<Mutex<Option<(Rgb, Rgb)>>> = OnceLock::new();
        SHARED_ACCENT.get_or_init(|| Mutex::new(None))
    }

    /// Publishes MediaBar's active cover accent pair `(light, dark)` for Tray & MonitorBar synchronization.
    /// Pass `None` when MediaBar is stopped, auto-color is disabled, or no active cover exists.
    pub fn set_mediabar_shared_accent(accent: Option<(Rgb, Rgb)>) {
        if let Ok(mut guard) = mediabar_shared_accent_slot().lock() {
            *guard = accent;
        }
    }

    /// Returns MediaBar's currently published `(accent_light, accent_dark)`, if active.
    pub fn get_mediabar_shared_accent() -> Option<(Rgb, Rgb)> {
        mediabar_shared_accent_slot().lock().ok().and_then(|g| *g)
    }

    /// Resolves `(ui_accent, halo_dark_base)` across Tray, MediaBar, and MonitorBar:
    /// prefers MediaBar's published album accent when active; otherwise lifts backdrop lightness preserving hue.
    pub fn resolve_unified_accent(backdrop_bg: Rgb, light_glass: bool) -> (Rgb, Rgb) {
        if let Some((al, ad)) = get_mediabar_shared_accent() {
            return (if light_glass { al } else { ad }, ad);
        }
        (
            derive_background_accent(backdrop_bg, light_glass),
            derive_background_accent(backdrop_bg, false),
        )
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

    /// Returns `true` if `hwnd` belongs to Windows desktop shell or modern tray/start/search flyouts.
    pub fn is_shell_or_flyout_window(hwnd: HWND) -> bool {
        !hwnd.is_invalid() && (is_shell_or_flyout_class(hwnd) || is_shell_flyout_process(hwnd))
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

    type GetWindowBandFn = unsafe extern "system" fn(HWND, *mut u32) -> BOOL;

    fn load_get_window_band() -> Option<GetWindowBandFn> {
        static CACHE: OnceLock<Option<GetWindowBandFn>> = OnceLock::new();
        *CACHE.get_or_init(|| unsafe {
            let hmod = GetModuleHandleW(w!("user32.dll"))
                .ok()
                .or_else(|| LoadLibraryW(w!("user32.dll")).ok())?;
            let proc = GetProcAddress(hmod, s!("GetWindowBand"))?;
            Some(std::mem::transmute::<_, GetWindowBandFn>(proc))
        })
    }

    fn window_band(hwnd: HWND) -> Option<u32> {
        if hwnd.is_invalid() {
            return None;
        }
        let f = load_get_window_band()?;
        let mut band = 0u32;
        unsafe { f(hwnd, &mut band).as_bool().then_some(band) }
    }

    fn is_window_above(upper: HWND, lower: HWND) -> bool {
        if upper.is_invalid() || lower.is_invalid() || upper == lower {
            return false;
        }
        unsafe {
            let mut cur = lower;
            for _ in 0..512 {
                match GetWindow(cur, GW_HWNDPREV) {
                    Ok(prev) if !prev.is_invalid() => {
                        if prev == upper {
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

    fn is_taskbar_above_window(bar_hwnd: HWND, taskbar_hwnd: HWND) -> bool {
        is_window_above(taskbar_hwnd, bar_hwnd)
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
    /// falling back to `HWND_TOPMOST` when needed (and avoiding cross-band thrashing when the
    /// taskbar is temporarily elevated by shell UI such as the Start Menu).
    pub fn ensure_window_above_taskbar(bar_hwnd: HWND, taskbar_hwnd: HWND) {
        unsafe {
            if bar_hwnd.is_invalid()
                || taskbar_hwnd.is_invalid()
                || !IsWindowVisible(bar_hwnd).as_bool()
            {
                return;
            }

            let bar_band = window_band(bar_hwnd);
            let tb_band = window_band(taskbar_hwnd);
            if matches!((bar_band, tb_band), (Some(bb), Some(tb)) if tb > bb) {
                let is_topmost =
                    (GetWindowLongW(bar_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0;
                if is_topmost && !is_any_backdrop_above(bar_hwnd) {
                    return;
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
                return;
            }

            if !is_taskbar_above_window(bar_hwnd, taskbar_hwnd) {
                if !is_any_backdrop_above(bar_hwnd) {
                    return;
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
                return;
            }

            if let Ok(above_tb) = GetWindow(taskbar_hwnd, GW_HWNDPREV) {
                if !above_tb.is_invalid() && above_tb != bar_hwnd {
                    let same_band = match (bar_band, window_band(above_tb)) {
                        (Some(bb), Some(ab)) => bb == ab,
                        _ => true,
                    };
                    if same_band {
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
        backdrops: Vec<HWND>,
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
                backdrops: Vec::new(),
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

        let tb_band = window_band(tb_hwnd);
        let any_below = widgets
            .iter()
            .copied()
            .any(|w| IsWindowVisible(w).as_bool() && is_taskbar_above_window(w, tb_hwnd));
        if any_below && !is_fullscreen_occluding_taskbar(tb_hwnd) {
            for w in widgets {
                if !IsWindowVisible(w).as_bool() {
                    continue;
                }
                if let (Some(wb), Some(tb)) = (window_band(w), tb_band) {
                    if tb > wb {
                        let is_topmost =
                            (GetWindowLongW(w, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0;
                        if is_topmost && !is_any_backdrop_above(w) {
                            continue;
                        }
                    }
                }
                ensure_window_above_taskbar(w, tb_hwnd);
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

    fn is_any_backdrop_above(bar_hwnd: HWND) -> bool {
        if bar_hwnd.is_invalid() {
            return false;
        }
        let backdrops = OVERLAY_GUARD.with(|cell| {
            cell.try_borrow()
                .map(|g| g.backdrops.clone())
                .unwrap_or_default()
        });
        backdrops.into_iter().any(|bg| unsafe {
            !bg.is_invalid()
                && IsWindow(bg).as_bool()
                && IsWindowVisible(bg).as_bool()
                && is_window_above(bg, bar_hwnd)
        })
    }

    /// Registers a behind-taskbar backdrop HWND so Z-order coordination keeps it below widgets.
    pub fn register_taskbar_backdrop(hwnd: HWND) {
        if hwnd.is_invalid() {
            return;
        }
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard
                    .backdrops
                    .retain(|&w| !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
                if !guard.backdrops.contains(&hwnd) {
                    guard.backdrops.push(hwnd);
                }
            }
        });
    }

    /// Unregisters a behind-taskbar backdrop HWND before destruction.
    pub fn unregister_taskbar_backdrop(hwnd: HWND) {
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard
                    .backdrops
                    .retain(|&w| w != hwnd && !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
            }
        });
    }

    /// Resolves the Z-order anchor (`insert_after`) for a behind-taskbar backdrop window on the
    /// same monitor as `tb_hwnd`, placing it below all visible same-band taskbar widgets and the
    /// taskbar itself even if Windows elevates `tb_hwnd` to a higher band.
    pub fn taskbar_backdrop_anchor(tb_hwnd: HWND, backdrop_hwnd: HWND) -> HWND {
        if tb_hwnd.is_invalid() {
            return tb_hwnd;
        }
        unsafe {
            let tb_mon = MonitorFromWindow(tb_hwnd, MONITOR_DEFAULTTONEAREST);
            let tb_band = window_band(tb_hwnd);
            let bg_band = window_band(backdrop_hwnd);

            let candidates: Vec<HWND> = OVERLAY_GUARD.with(|cell| {
                let Ok(guard) = cell.try_borrow() else {
                    return Vec::new();
                };
                guard
                    .widgets
                    .iter()
                    .copied()
                    .filter(|&w| {
                        !w.is_invalid()
                            && w != backdrop_hwnd
                            && IsWindow(w).as_bool()
                            && IsWindowVisible(w).as_bool()
                            && MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST) == tb_mon
                            && match (bg_band, window_band(w)) {
                                (Some(bg), Some(wb)) => bg == wb,
                                _ => true,
                            }
                    })
                    .collect()
            });

            if candidates.is_empty() {
                return tb_hwnd;
            }

            let mut taskbar_z_rank: Option<usize> = None;
            let mut matched_overlays: Vec<(usize, usize)> = Vec::with_capacity(candidates.len());
            let mut rank = 0usize;
            let mut cur = GetTopWindow(None).unwrap_or_default();
            while !cur.is_invalid() && rank < 1024 {
                if cur == tb_hwnd && taskbar_z_rank.is_none() {
                    taskbar_z_rank = Some(rank);
                }
                if candidates.contains(&cur)
                    && !matched_overlays.iter().any(|&(id, _)| id == cur.0 as usize)
                {
                    matched_overlays.push((cur.0 as usize, rank));
                }
                if taskbar_z_rank.is_some() && matched_overlays.len() == candidates.len() {
                    break;
                }
                rank += 1;
                cur = GetWindow(cur, GW_HWNDNEXT).unwrap_or_default();
            }

            let anchor_id = super::pick_lowest_backdrop_anchor(
                tb_hwnd.0 as usize,
                tb_band,
                bg_band,
                taskbar_z_rank,
                &matched_overlays,
            );
            HWND(anchor_id as *mut core::ffi::c_void)
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

    /// Collects screen bounding rectangles (with a 16px DWM shadow margin) of visible transient
    /// shell flyouts or tooltips intersecting `capture_rect`, so popup backdrop captures can
    /// cleanly inpaint them out before they fade/dismiss on activation.
    pub fn visible_transient_overlay_rects(capture_rect: RECT) -> Vec<RECT> {
        struct CollectCtx {
            capture: RECT,
            rects: Vec<RECT>,
        }

        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut CollectCtx);
            if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let mut cls_buf = [0u16; 96];
            let len = GetClassNameW(hwnd, &mut cls_buf);
            if len <= 0 {
                return BOOL(1);
            }
            let class_name = String::from_utf16_lossy(&cls_buf[..len as usize]);
            if !crate::system::is_transient_backdrop_occluder_class(&class_name) {
                return BOOL(1);
            }

            let mut cloaked: u32 = 0;
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0
            {
                return BOOL(1);
            }

            let mut win_rect = RECT::default();
            if GetWindowRect(hwnd, &mut win_rect).is_err() {
                return BOOL(1);
            }
            let mut ext_rect = RECT::default();
            let bounds = if DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut ext_rect as *mut RECT).cast(),
                size_of::<RECT>() as u32,
            )
            .is_ok()
            {
                RECT {
                    left: win_rect.left.min(ext_rect.left),
                    top: win_rect.top.min(ext_rect.top),
                    right: win_rect.right.max(ext_rect.right),
                    bottom: win_rect.bottom.max(ext_rect.bottom),
                }
            } else {
                win_rect
            };

            let w = bounds.right - bounds.left;
            let h = bounds.bottom - bounds.top;
            if w <= 0 || h <= 0 || w > 960 || h > 720 {
                return BOOL(1);
            }

            const SHADOW_PAD_PX: i32 = 16;
            let inter = RECT {
                left: (bounds.left - SHADOW_PAD_PX).max(ctx.capture.left),
                top: (bounds.top - SHADOW_PAD_PX).max(ctx.capture.top),
                right: (bounds.right + SHADOW_PAD_PX).min(ctx.capture.right),
                bottom: (bounds.bottom + SHADOW_PAD_PX).min(ctx.capture.bottom),
            };
            if inter.right > inter.left && inter.bottom > inter.top {
                ctx.rects.push(inter);
            }
            BOOL(1)
        }

        if capture_rect.right <= capture_rect.left || capture_rect.bottom <= capture_rect.top {
            return Vec::new();
        }
        let mut ctx = CollectCtx {
            capture: capture_rect,
            rects: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM((&mut ctx as *mut CollectCtx) as isize));
        }
            ctx.rects
    }

    static OVERFLOW_NOTIFY_HWND: AtomicUsize = AtomicUsize::new(0);
    static OVERFLOW_NOTIFY_MSG: AtomicU32 = AtomicU32::new(0);
    static OVERFLOW_NOTIFY_PENDING: AtomicBool = AtomicBool::new(false);

    /// Clears the coalescing flag so subsequent overflow WinEvents can schedule another host sync.
    pub fn clear_overflow_notify_pending() {
        OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
    }

    unsafe extern "system" fn overflow_win_event_proc(
        _hwineventhook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        idobject: i32,
        _idchild: i32,
        _ideventthread: u32,
        _dwmseventtime: u32,
    ) {
        if hwnd.is_invalid() || idobject != 0 {
            return;
        }
        let target_raw = OVERFLOW_NOTIFY_HWND.load(WinOrdering::Acquire);
        let notify_msg = OVERFLOW_NOTIFY_MSG.load(WinOrdering::Acquire);
        if target_raw == 0 || notify_msg == 0 {
            return;
        }
        let mut cls_buf = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut cls_buf);
        if len <= 0 {
            return;
        }
        let cls = String::from_utf16_lossy(&cls_buf[..len as usize]);
        if matches!(
            cls.as_str(),
            "TopLevelWindowForOverflowXamlIsland" | "NotifyIconOverflowWindow"
        ) && !OVERFLOW_NOTIFY_PENDING.swap(true, WinOrdering::AcqRel)
        {
            if PostMessageW(
                HWND(target_raw as *mut core::ffi::c_void),
                notify_msg,
                WPARAM(0),
                LPARAM(0),
            )
            .is_err()
            {
                OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
            }
        }
    }

    /// Resolves the Z-order anchor (`insert_after`) for the overflow flyout backdrop window.
    /// Same-band overlays insert right after `ov_hwnd`; higher-band shell flyouts sit above
    /// normal topmost windows via `HWND_TOPMOST` while staying underneath the flyout's own band.
    pub fn overflow_backdrop_anchor(ov_hwnd: HWND, backdrop_hwnd: HWND) -> HWND {
        if ov_hwnd.is_invalid() {
            return HWND_TOPMOST;
        }
        match (window_band(ov_hwnd), window_band(backdrop_hwnd)) {
            (Some(ob), Some(bb)) if ob == bb => ov_hwnd,
            _ => HWND_TOPMOST,
        }
    }

    /// RAII process-scoped WinEvent hook notifying the tray host immediately when the Explorer
    /// system tray overflow flyout shows, moves/resizes, cloaks, or hides.
    pub struct OverflowEventHook {
        host_hwnd: HWND,
        notify_msg: u32,
        explorer_pid: u32,
        show_hide_hook: HWINEVENTHOOK,
        location_hook: HWINEVENTHOOK,
        cloak_hook: HWINEVENTHOOK,
    }

    impl OverflowEventHook {
        pub fn new(host_hwnd: HWND, notify_msg: u32) -> Self {
            OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
            OVERFLOW_NOTIFY_HWND.store(host_hwnd.0 as usize, WinOrdering::Release);
            OVERFLOW_NOTIFY_MSG.store(notify_msg, WinOrdering::Release);
            Self {
                host_hwnd,
                notify_msg,
                explorer_pid: 0,
                show_hide_hook: HWINEVENTHOOK::default(),
                location_hook: HWINEVENTHOOK::default(),
                cloak_hook: HWINEVENTHOOK::default(),
            }
        }

        pub fn refresh_for_explorer_pid(&mut self, pid: u32) {
            OVERFLOW_NOTIFY_HWND.store(self.host_hwnd.0 as usize, WinOrdering::Release);
            OVERFLOW_NOTIFY_MSG.store(self.notify_msg, WinOrdering::Release);
            if self.explorer_pid == pid
                && (!self.show_hide_hook.is_invalid() || pid == 0)
            {
                return;
            }
            self.unhook_all();
            self.explorer_pid = pid;
            if pid == 0 {
                return;
            }
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            unsafe {
                self.show_hide_hook = SetWinEventHook(
                    EVENT_OBJECT_SHOW,
                    EVENT_OBJECT_HIDE,
                    HMODULE::default(),
                    Some(overflow_win_event_proc),
                    pid,
                    0,
                    flags,
                );
                self.location_hook = SetWinEventHook(
                    EVENT_OBJECT_LOCATIONCHANGE,
                    EVENT_OBJECT_LOCATIONCHANGE,
                    HMODULE::default(),
                    Some(overflow_win_event_proc),
                    pid,
                    0,
                    flags,
                );
                self.cloak_hook = SetWinEventHook(
                    EVENT_OBJECT_CLOAKED,
                    EVENT_OBJECT_UNCLOAKED,
                    HMODULE::default(),
                    Some(overflow_win_event_proc),
                    pid,
                    0,
                    flags,
                );
            }
        }

        fn unhook_all(&mut self) {
            unsafe {
                for hook in [
                    &mut self.show_hide_hook,
                    &mut self.location_hook,
                    &mut self.cloak_hook,
                ] {
                    if !hook.is_invalid() {
                        let _ = UnhookWinEvent(*hook);
                        *hook = HWINEVENTHOOK::default();
                    }
                }
            }
            self.explorer_pid = 0;
        }
    }

    impl Drop for OverflowEventHook {
        fn drop(&mut self) {
            self.unhook_all();
            OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
            OVERFLOW_NOTIFY_HWND.store(0, WinOrdering::Release);
            OVERFLOW_NOTIFY_MSG.store(0, WinOrdering::Release);
        }
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

    #[test]
    fn backdrop_anchor_stays_below_widgets_when_taskbar_elevates_band() {
        const TB: usize = 0x1001;
        const MEDIA: usize = 0x2001;
        const MONITOR: usize = 0x2002;
        const SEC_TB: usize = 0x1002;

        // Normal same-band state: MediaBar (5) & MonitorBar (6) above Shell_TrayWnd (7) -> anchor = Shell_TrayWnd.
        assert_eq!(
            pick_lowest_backdrop_anchor(TB, Some(1), Some(1), Some(7), &[(MEDIA, 5), (MONITOR, 6)]),
            TB
        );

        // Start Menu open: Shell_TrayWnd elevated to band 16 while widgets & backdrop remain in band 1
        // -> must anchor below the lowest visible widget (MonitorBar), NEVER Shell_TrayWnd.
        assert_eq!(
            pick_lowest_backdrop_anchor(TB, Some(16), Some(1), Some(1), &[(MEDIA, 5), (MONITOR, 6)]),
            MONITOR
        );

        // Transient same-band reorder where Shell_TrayWnd (4) sits above widgets (5, 6)
        // -> still picks lowest widget (MonitorBar) so backdrop stays beneath all widgets.
        assert_eq!(
            pick_lowest_backdrop_anchor(TB, Some(1), Some(1), Some(4), &[(MEDIA, 5), (MONITOR, 6)]),
            MONITOR
        );

        // Secondary monitor with no widgets falls back to its own Shell_SecondaryTrayWnd.
        assert_eq!(
            pick_lowest_backdrop_anchor(SEC_TB, Some(16), Some(1), Some(2), &[]),
            SEC_TB
        );
    }

    #[test]
    fn transient_backdrop_occluder_class_filter_matches_expected_windows() {
        for cls in [
            "TopLevelWindowForOverflowXamlIsland",
            "NotifyIconOverflowWindow",
            "XamlExplorerHostIslandWindow",
            "ControlCenterWindow",
            "tooltips_class32",
            "Xaml_WindowedPopupClass",
            "TaskListThumbnailWnd",
        ] {
            assert!(
                is_transient_backdrop_occluder_class(cls),
                "expected transient occluder match for {cls}"
            );
        }

        for keep in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Chrome_WidgetWin_1",
            "CabinetWClass",
            "PrettyWindowsTrayMenuWnd",
        ] {
            assert!(
                !is_transient_backdrop_occluder_class(keep),
                "should NOT mark {keep} as transient occluder"
            );
        }
    }
}
