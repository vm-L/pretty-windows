use super::overlay_guard::is_any_backdrop_above;
use std::mem::size_of;
use std::sync::OnceLock;
use windows::core::{s, w, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXCLUDED_FROM_PEEK, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetTopWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetWindowPos, GA_ROOT, GWL_EXSTYLE, GW_HWNDNEXT, GW_HWNDPREV, GW_OWNER, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT};

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
            .or_else(|| {
                LoadLibraryExW(w!("user32.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()
            })?;
        let proc = GetProcAddress(hmod, s!("GetWindowBand"))?;
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            GetWindowBandFn,
        >(proc))
    })
}

pub fn window_band(hwnd: HWND) -> Option<u32> {
    if hwnd.is_invalid() {
        return None;
    }
    let f = load_get_window_band()?;
    let mut band = 0u32;
    unsafe { f(hwnd, &mut band).as_bool().then_some(band) }
}

pub(super) fn is_window_above(upper: HWND, lower: HWND) -> bool {
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

pub(super) fn is_taskbar_above_window(bar_hwnd: HWND, taskbar_hwnd: HWND) -> bool {
    is_window_above(taskbar_hwnd, bar_hwnd)
}

unsafe fn is_window_above_or_equal_check(target: HWND, taskbar_hwnd: HWND) -> bool {
    is_window_above(target, taskbar_hwnd)
}

enum FullscreenActive {
    None,
    GlobalNotification,
    Window(HWND),
}

unsafe fn detect_fullscreen_on_monitor(anchor_hwnd: HWND) -> FullscreenActive {
    if let Ok(state) = SHQueryUserNotificationState() {
        if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
            let fg = GetForegroundWindow();
            if !fg.is_invalid() {
                let root = GetAncestor(fg, GA_ROOT);
                let fg_root = if root.is_invalid() { fg } else { root };
                let fg_mon = MonitorFromWindow(fg_root, MONITOR_DEFAULTTONEAREST);
                let anchor_mon = MonitorFromWindow(anchor_hwnd, MONITOR_DEFAULTTONEAREST);
                if fg_mon != anchor_mon {
                    return FullscreenActive::None;
                }
            }
            return FullscreenActive::GlobalNotification;
        }
    }

    let fg = GetForegroundWindow();
    if fg.is_invalid() {
        return FullscreenActive::None;
    }
    let root = GetAncestor(fg, GA_ROOT);
    let target = if root.is_invalid() { fg } else { root };

    if target == anchor_hwnd
        || target == GetShellWindow()
        || target == GetDesktopWindow()
        || !IsWindowVisible(target).as_bool()
        || IsIconic(target).as_bool()
    {
        return FullscreenActive::None;
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
        return FullscreenActive::None;
    }

    if is_shell_or_flyout_class(target) || is_shell_flyout_process(target) {
        return FullscreenActive::None;
    }

    let ex_style = GetWindowLongW(target, GWL_EXSTYLE) as u32;
    if (ex_style & (WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0)) != 0 {
        return FullscreenActive::None;
    }

    if let Ok(owner) = GetWindow(target, GW_OWNER) {
        if !owner.is_invalid()
            && (is_shell_or_flyout_class(owner) || is_shell_flyout_process(owner))
        {
            return FullscreenActive::None;
        }
    }

    let anchor_mon = MonitorFromWindow(anchor_hwnd, MONITOR_DEFAULTTONEAREST);
    let fg_mon = MonitorFromWindow(target, MONITOR_DEFAULTTONEAREST);
    if anchor_mon != fg_mon {
        return FullscreenActive::None;
    }

    let mut mi = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !GetMonitorInfoW(anchor_mon, &mut mi).as_bool() {
        return FullscreenActive::None;
    }

    let mut fg_rect = RECT::default();
    if GetWindowRect(target, &mut fg_rect).is_err() {
        return FullscreenActive::None;
    }
    let covers_monitor = fg_rect.left <= mi.rcMonitor.left
        && fg_rect.top <= mi.rcMonitor.top
        && fg_rect.right >= mi.rcMonitor.right
        && fg_rect.bottom >= mi.rcMonitor.bottom;
    if covers_monitor {
        FullscreenActive::Window(target)
    } else {
        FullscreenActive::None
    }
}

/// Returns `true` when the active session is in exclusive D3D fullscreen or presentation mode
/// (`QUNS_RUNNING_D3D_FULL_SCREEN` / `QUNS_PRESENTATION_MODE`).
///
/// Regular borderless fullscreen windows and desktop video players intentionally return `false`
/// so custom translucent overlays (such as the hardware OSD capsule backdrop) can remain active
/// unless occluded by an external topmost window.
pub fn is_exclusive_fullscreen_or_presentation() -> bool {
    unsafe {
        SHQueryUserNotificationState().is_ok_and(|state| {
            state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE
        })
    }
}

/// Detects whether a genuine fullscreen/presentation window is currently occluding `taskbar_hwnd`.
///
/// Desktop surfaces (`Progman`/`WorkerW`), system shell flyouts (Quick Settings, Start Menu,
/// Search, TextInputHost, `Windows.UI.Core.CoreWindow`), DWM-cloaked/minimized windows, and
/// windows sitting below `taskbar_hwnd` in Z-order (such as after `Win+D`) do not count.
pub fn is_fullscreen_occluding_taskbar(taskbar_hwnd: HWND) -> bool {
    unsafe {
        match detect_fullscreen_on_monitor(taskbar_hwnd) {
            FullscreenActive::None => false,
            FullscreenActive::GlobalNotification => true,
            // If `taskbar_hwnd` sits above `target` in Z-order (e.g. `Win+D` or desktop companion),
            // the taskbar is visible and not occluded.
            FullscreenActive::Window(target) => {
                is_window_above_or_equal_check(target, taskbar_hwnd)
            }
        }
    }
}

unsafe fn is_external_topmost_window_intersecting(
    cur: HWND,
    my_pid: u32,
    card_arr: [i32; 4],
) -> bool {
    if !IsWindowVisible(cur).as_bool() {
        return false;
    }
    let mut cloaked: u32 = 0;
    let is_cloaked = DwmGetWindowAttribute(
        cur,
        DWMWA_CLOAKED,
        (&mut cloaked as *mut u32).cast(),
        size_of::<u32>() as u32,
    )
    .is_ok()
        && cloaked != 0;
    if is_cloaked {
        return false;
    }
    let ex = GetWindowLongW(cur, GWL_EXSTYLE) as u32;
    if (ex & WS_EX_NOREDIRECTIONBITMAP.0) != 0 && window_band(cur).is_some_and(|b| b > 2) {
        return false;
    }
    let mut pid = 0u32;
    let _ = GetWindowThreadProcessId(cur, Some(&mut pid));
    if pid == 0
        || pid == my_pid
        || is_shell_or_flyout_class(cur)
        || is_shell_flyout_process(cur)
    {
        return false;
    }
    let mut r = RECT::default();
    let got_rect = DwmGetWindowAttribute(
        cur,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        (&mut r as *mut RECT).cast(),
        size_of::<RECT>() as u32,
    )
    .is_ok()
        || GetWindowRect(cur, &mut r).is_ok();
    got_rect && super::rects_intersect([r.left, r.top, r.right, r.bottom], card_arr)
}

/// Returns `true` when the `band=1` `ShellCardGlassBackdrop` behind the OSD would fail to
/// composite cleanly above foreground content: either the system is in D3D exclusive fullscreen /
/// presentation mode, or an external `WS_EX_TOPMOST` window overlaps `card_rect`.
///
/// In those cases the OSD switches to calibrated inline XAML translucent glass (`band=18`)
/// instead of hiding its backdrop altogether.
pub fn should_suppress_osd_backdrop(osd_host_hwnd: HWND, card_rect: RECT) -> bool {
    if is_exclusive_fullscreen_or_presentation() {
        return true;
    }
    unsafe {
        let my_pid = GetCurrentProcessId();
        let card_arr = [
            card_rect.left,
            card_rect.top,
            card_rect.right,
            card_rect.bottom,
        ];
        let mut cur = GetTopWindow(None).unwrap_or_default();
        for _ in 0..64 {
            if cur.is_invalid() {
                break;
            }
            let ex = GetWindowLongW(cur, GWL_EXSTYLE) as u32;
            if (ex & WS_EX_TOPMOST.0) == 0 {
                break;
            }
            if cur != osd_host_hwnd
                && is_external_topmost_window_intersecting(cur, my_pid, card_arr)
            {
                return true;
            }
            cur = GetWindow(cur, GW_HWNDNEXT).unwrap_or_default();
        }
    }
    false
}

/// Returns `true` when the behind-ToolTip `ShellCardGlassBackdrop` would be occluded or fail to
/// composite cleanly: either the session is in D3D exclusive fullscreen / presentation mode, or an
/// external `WS_EX_TOPMOST` window strictly above `tooltip_host_hwnd` overlaps `card_rect`.
///
/// Windows ordered below `tooltip_host_hwnd` in the topmost band (such as the taskbar overflow
/// island or taskbar itself) do not occlude the backdrop anchored right behind `tooltip_host_hwnd`.
pub fn should_suppress_tooltip_backdrop(tooltip_host_hwnd: HWND, card_rect: RECT) -> bool {
    if is_exclusive_fullscreen_or_presentation() {
        return true;
    }
    unsafe {
        let my_pid = GetCurrentProcessId();
        let card_arr = [
            card_rect.left,
            card_rect.top,
            card_rect.right,
            card_rect.bottom,
        ];
        let mut cur = GetTopWindow(None).unwrap_or_default();
        for _ in 0..64 {
            if cur.is_invalid() || cur == tooltip_host_hwnd {
                break;
            }
            let ex = GetWindowLongW(cur, GWL_EXSTYLE) as u32;
            if (ex & WS_EX_TOPMOST.0) == 0 {
                break;
            }
            if is_external_topmost_window_intersecting(cur, my_pid, card_arr) {
                return true;
            }
            cur = GetWindow(cur, GW_HWNDNEXT).unwrap_or_default();
        }
    }
    false
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

/// Returns `true` when any visible, uncloaked external topmost window in the same ZBID band
/// overlaps `hwnd`.
pub fn is_occluded_by_same_band_topmost(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    let my_pid = unsafe { GetCurrentProcessId() };
    let my_band = window_band(hwnd);
    let mut target_rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut target_rect) }.is_err() {
        return false;
    }
    let mut cur = hwnd;
    for _ in 0..32 {
        let Ok(prev) = (unsafe { GetWindow(cur, GW_HWNDPREV) }) else {
            break;
        };
        if prev.is_invalid() || prev == cur {
            break;
        }
        cur = prev;
        if !unsafe { IsWindowVisible(cur).as_bool() } {
            continue;
        }
        let mut cloaked = 0u32;
        let is_cloaked = unsafe {
            DwmGetWindowAttribute(
                cur,
                DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        }
        .is_ok()
            && cloaked != 0;
        if is_cloaked {
            continue;
        }
        let ex = unsafe { GetWindowLongW(cur, GWL_EXSTYLE) as u32 };
        if (ex & WS_EX_TOPMOST.0) == 0 {
            continue;
        }
        if window_band(cur) != my_band {
            continue;
        }
        let mut cur_rect = RECT::default();
        let overlaps = unsafe { GetWindowRect(cur, &mut cur_rect) }.is_ok()
            && cur_rect.right > target_rect.left
            && cur_rect.left < target_rect.right
            && cur_rect.bottom > target_rect.top
            && cur_rect.top < target_rect.bottom;
        if !overlaps {
            continue;
        }
        let mut pid = 0u32;
        unsafe {
            let _ = GetWindowThreadProcessId(cur, Some(&mut pid));
        }
        if pid != 0 && pid != my_pid {
            return true;
        }
    }
    false
}
