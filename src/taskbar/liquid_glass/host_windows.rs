use super::super::swca::inset_overflow_card_rect;
use std::cell::Cell;
use std::mem::size_of;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, GW_OWNER,
};

fn inspect_visible_explorer_window(
    hwnd: HWND,
    target_pid: u32,
    min_w: i32,
    min_h: i32,
) -> Option<(RECT, u32, u32)> {
    unsafe {
        if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        let mut pid = 0u32;
        let tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != target_pid {
            return None;
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
            return None;
        }
        let mut rect = RECT::default();
        let got_ext = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
        .is_ok();
        if !got_ext && GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w < min_w || h < min_h {
            return None;
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        Some((rect, dpi, tid))
    }
}

/// Locates the active uncloaked host window belonging to `explorer_pid` for the `Alt + Tab` switcher,
/// scanning across all ZBID bands via `FindWindowExW`, preferring `preferred_tid`, and isolating
/// taskbar / hover-flyout / OSD threads.
pub fn find_visible_alttab_host_window(
    explorer_pid: u32,
    preferred_tid: Option<u32>,
    excluded_tids: &[u32],
    taskbar_tids: &[u32],
) -> Option<(HWND, RECT, u32, u32)> {
    if explorer_pid == 0 {
        return None;
    }
    let mut found: Option<(HWND, RECT, u32, u32)> = None;
    for cls in [
        w!("XamlExplorerHostIslandWindow"),
        w!("XamlExplorerHostIslandWindow_WASDK"),
    ] {
        let mut prev = HWND::default();
        for _ in 0..256 {
            let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, cls, None) }) else {
                break;
            };
            if hwnd.is_invalid() || hwnd == prev {
                break;
            }
            prev = hwnd;
            let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, explorer_pid, 300, 200)
            else {
                continue;
            };
            if excluded_tids.contains(&tid) {
                continue;
            }
            if preferred_tid != Some(tid) && taskbar_tids.contains(&tid) {
                continue;
            }
            let matched = preferred_tid == Some(tid);
            if matched || found.is_none() {
                found = Some((hwnd, rect, dpi, tid));
            }
            if matched {
                return found;
            }
        }
    }
    found
}

/// Locates the active uncloaked host window belonging to `explorer_pid` for `Taskbar.FlyoutFrame`
/// (hover thumbnail previews & taskbar overflow), enumerating across all ZBID bands via `FindWindowExW`
/// (including `ZBID_IMMERSIVE_INACTIVEMOBILITY`), preferring `XamlExplorerHostIslandWindow*` and known
/// taskbar UI threads while excluding `Alt + Tab` switcher and OSD hosts.
pub fn find_visible_hover_flyout_host_window(
    explorer_pid: u32,
    preferred_tid: Option<u32>,
    excluded_tids: &[u32],
    taskbar_tids: &[u32],
) -> Option<(HWND, RECT, u32, u32)> {
    if explorer_pid == 0 {
        return None;
    }
    let mut best: Option<(u8, i64, HWND, RECT, u32, u32)> = None;
    for (cls, class_bonus, require_taskbar_or_pref) in [
        (w!("XamlExplorerHostIslandWindow"), 4u8, false),
        (w!("XamlExplorerHostIslandWindow_WASDK"), 4u8, false),
        (w!("Xaml_WindowedPopupClass"), 0u8, true),
    ] {
        let mut prev = HWND::default();
        for _ in 0..256 {
            let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, cls, None) }) else {
                break;
            };
            if hwnd.is_invalid() || hwnd == prev {
                break;
            }
            prev = hwnd;
            let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, explorer_pid, 48, 36)
            else {
                continue;
            };
            if excluded_tids.contains(&tid) {
                continue;
            }
            if require_taskbar_or_pref && preferred_tid != Some(tid) && !taskbar_tids.contains(&tid) {
                continue;
            }
            let thread_bonus = if preferred_tid == Some(tid) {
                2u8
            } else if taskbar_tids.contains(&tid) {
                1u8
            } else {
                0u8
            };
            let priority = class_bonus + thread_bonus;
            let w = rect.right - rect.left;
            let h = rect.bottom - rect.top;
            let area = i64::from(w) * i64::from(h);
            let take = match best {
                None => true,
                Some((best_prio, best_area, _, _, _, _)) => {
                    priority > best_prio || (priority == best_prio && area > best_area)
                }
            };
            if take {
                best = Some((priority, area, hwnd, rect, dpi, tid));
            }
        }
    }
    best.map(|(_, _, h, r, d, t)| (h, r, d, t))
}

/// Locates the active uncloaked `HWConfirmatorUI` host window belonging to `explorer_pid`,
/// excluding taskbar, `Alt + Tab`, and hover-flyout threads and bounding unlearned hosts to compact OSD dimensions.
pub fn find_visible_osd_host_window(
    explorer_pid: u32,
    preferred_tid: Option<u32>,
    excluded_tids: &[u32],
    taskbar_tids: &[u32],
) -> Option<(HWND, RECT, u32, u32)> {
    if explorer_pid == 0 {
        return None;
    }
    let mut found: Option<(HWND, RECT, u32, u32)> = None;
    for cls in [
        w!("XamlExplorerHostIslandWindow"),
        w!("XamlExplorerHostIslandWindow_WASDK"),
    ] {
        let mut prev = HWND::default();
        for _ in 0..256 {
            let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, cls, None) }) else {
                break;
            };
            if hwnd.is_invalid() || hwnd == prev {
                break;
            }
            prev = hwnd;
            let Some((rect, dpi, tid)) =
                inspect_visible_explorer_window(hwnd, explorer_pid, 80, 36)
            else {
                continue;
            };
            if excluded_tids.contains(&tid) {
                continue;
            }
            if preferred_tid != Some(tid) && taskbar_tids.contains(&tid) {
                continue;
            }
            let matched = preferred_tid == Some(tid);
            if !matched {
                let scale = (dpi.max(96) as f32) / 96.0;
                let w = rect.right - rect.left;
                let h = rect.bottom - rect.top;
                let max_w = (560.0 * scale).round() as i32;
                let max_h = (220.0 * scale).round() as i32;
                if w > max_w || h > max_h {
                    continue;
                }
            }
            if matched || found.is_none() {
                found = Some((hwnd, rect, dpi, tid));
            }
            if matched {
                return found;
            }
        }
    }
    found
}

/// Enumerates visible uncloaked `Xaml_WindowedPopupClass` host windows belonging to
/// `explorer_pid` whose dimensions match `MenuFlyout` / `JumpList` surfaces, returning
/// `(hwnd, rect, dpi, thread_id)` sorted by thread preference.
pub fn find_visible_menu_flyout_host_windows(
    explorer_pid: u32,
    preferred_tids: &[u32],
    taskbar_tids: &[u32],
    exclude_hwnds: &[HWND],
) -> Vec<(HWND, RECT, u32, u32)> {
    if explorer_pid == 0 {
        return Vec::new();
    }
    let mut out: Vec<(u8, HWND, RECT, u32, u32)> = Vec::new();
    let mut prev = HWND::default();
    for _ in 0..256 {
        let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, w!("Xaml_WindowedPopupClass"), None) })
        else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev {
            break;
        }
        prev = hwnd;
        if exclude_hwnds.contains(&hwnd) {
            continue;
        }
        let Some((rect, dpi, tid)) =
            inspect_visible_explorer_window(hwnd, explorer_pid, 64, 28)
        else {
            continue;
        };
        let scale = (dpi.max(96) as f32) / 96.0;
        let w_dip = ((rect.right - rect.left) as f32) / scale;
        let h_dip = ((rect.bottom - rect.top) as f32) / scale;
        if !(64.0..=760.0).contains(&w_dip) || !(28.0..=1080.0).contains(&h_dip) {
            continue;
        }
        let prio = if preferred_tids.contains(&tid) {
            2u8
        } else if taskbar_tids.contains(&tid) {
            1u8
        } else {
            0u8
        };
        out.push((prio, hwnd, rect, dpi, tid));
    }
    out.sort_by_key(|item| std::cmp::Reverse(item.0));
    out.into_iter().map(|(_, h, r, d, t)| (h, r, d, t)).collect()
}

/// Locates the active uncloaked `Xaml_WindowedPopupClass` host window belonging to `explorer_pid`
/// for a system/taskbar/overflow `ToolTip`, preferring matching dimensions and known/taskbar UI threads.
pub fn find_visible_tooltip_host_window(
    explorer_pid: u32,
    preferred_tid: Option<u32>,
    taskbar_tids: &[u32],
    expected_size_dip: Option<(f32, f32)>,
    exclude_hwnds: &[HWND],
) -> Option<(HWND, RECT, u32, u32)> {
    if explorer_pid == 0 {
        return None;
    }
    let mut best: Option<(u8, f32, HWND, RECT, u32, u32)> = None;
    let mut prev = HWND::default();
    for _ in 0..256 {
        let Ok(hwnd) =
            (unsafe { FindWindowExW(None, prev, w!("Xaml_WindowedPopupClass"), None) })
        else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev {
            break;
        }
        prev = hwnd;
        if exclude_hwnds.contains(&hwnd) {
            continue;
        }
        let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, explorer_pid, 20, 16)
        else {
            continue;
        };
        let scale = (dpi.max(96) as f32) / 96.0;
        let w_dip = ((rect.right - rect.left) as f32) / scale;
        let h_dip = ((rect.bottom - rect.top) as f32) / scale;
        if !(20.0..=680.0).contains(&w_dip) || !(16.0..=420.0).contains(&h_dip) {
            continue;
        }

        let (thread_prio, metric) = if let Some((ew, eh)) = expected_size_dip {
            if w_dip < ew - 4.0
                || h_dip < eh - 4.0
                || w_dip > ew + 64.0
                || h_dip > eh + 64.0
            {
                continue;
            }
            let prio = if preferred_tid == Some(tid) {
                2u8
            } else if taskbar_tids.contains(&tid) {
                1u8
            } else {
                0u8
            };
            (prio, (w_dip - ew).abs() + (h_dip - eh).abs())
        } else {
            if preferred_tid != Some(tid) && !taskbar_tids.contains(&tid) {
                continue;
            }
            if w_dip > 560.0 || h_dip > 240.0 {
                continue;
            }
            let prio = if preferred_tid == Some(tid) { 2u8 } else { 1u8 };
            (prio, w_dip * h_dip)
        };

        let take = match best {
            None => true,
            Some((best_prio, best_metric, _, _, _, _)) => {
                thread_prio > best_prio || (thread_prio == best_prio && metric < best_metric)
            }
        };
        if take {
            best = Some((thread_prio, metric, hwnd, rect, dpi, tid));
        }
    }
    best.map(|(_, _, h, r, d, t)| (h, r, d, t))
}

unsafe fn find_descendant_xaml_bridge(root: HWND) -> Option<(RECT, u32)> {
    if root.is_invalid() {
        return None;
    }
    if let Ok(bridge) = FindWindowExW(
        root,
        None,
        w!("Windows.UI.Composition.DesktopWindowContentBridge"),
        None,
    ) {
        if !bridge.is_invalid() {
            let mut rect = RECT::default();
            if GetWindowRect(bridge, &mut rect).is_ok()
                && rect.right > rect.left
                && rect.bottom > rect.top
            {
                let tid = GetWindowThreadProcessId(bridge, None);
                return Some((rect, tid));
            }
        }
    }
    let mut child = HWND::default();
    for _ in 0..16 {
        let Ok(c) = FindWindowExW(root, child, None, None) else {
            break;
        };
        if c.is_invalid() || c == child {
            break;
        }
        child = c;
        if let Ok(bridge) = FindWindowExW(
            child,
            None,
            w!("Windows.UI.Composition.DesktopWindowContentBridge"),
            None,
        ) {
            if !bridge.is_invalid() {
                let mut rect = RECT::default();
                if GetWindowRect(bridge, &mut rect).is_ok()
                    && rect.right > rect.left
                    && rect.bottom > rect.top
                {
                    let tid = GetWindowThreadProcessId(bridge, None);
                    return Some((rect, tid));
                }
            }
        }
    }
    None
}

/// Locates the active uncloaked Windows 11 Input Switcher (`键盘布局 ⊞ + 空格键`) host window
/// belonging to `explorer_pid`, returning `(host_hwnd, host_rect, island_rect, dpi, host_tid, bridge_tid)`.
pub fn find_visible_input_switcher_host_window(
    explorer_pid: u32,
    preferred_tid: Option<u32>,
    excluded_tids: &[u32],
    taskbar_tids: &[u32],
) -> Option<(HWND, RECT, RECT, u32, u32, Option<u32>)> {
    if explorer_pid == 0 {
        return None;
    }
    let mut prev_top = HWND::default();
    for _ in 0..32 {
        let Ok(hwnd) = (unsafe {
            FindWindowExW(None, prev_top, w!("Shell_InputSwitchTopLevelWindow"), None)
        }) else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev_top {
            break;
        }
        prev_top = hwnd;
        let Some((host_rect, dpi, host_tid)) =
            inspect_visible_explorer_window(hwnd, explorer_pid, 120, 60)
        else {
            continue;
        };
        let (island_rect, bridge_tid) = unsafe { find_descendant_xaml_bridge(hwnd) }
            .map(|(r, tid)| (r, Some(tid)))
            .unwrap_or((host_rect, None));
        return Some((hwnd, host_rect, island_rect, dpi, host_tid, bridge_tid));
    }

    let mut fallback: Option<(HWND, RECT, RECT, u32, u32, Option<u32>)> = None;
    for cls in [
        w!("XamlExplorerHostIslandWindow"),
        w!("XamlExplorerHostIslandWindow_WASDK"),
    ] {
        let mut prev = HWND::default();
        for _ in 0..256 {
            let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, cls, None) }) else {
                break;
            };
            if hwnd.is_invalid() || hwnd == prev {
                break;
            }
            prev = hwnd;
            let Some((host_rect, dpi, host_tid)) =
                inspect_visible_explorer_window(hwnd, explorer_pid, 120, 60)
            else {
                continue;
            };
            if excluded_tids.contains(&host_tid) {
                continue;
            }
            if preferred_tid != Some(host_tid) && taskbar_tids.contains(&host_tid) {
                continue;
            }
            let bridge_info = unsafe { find_descendant_xaml_bridge(hwnd) };
            let matched = preferred_tid == Some(host_tid)
                || bridge_info.is_some_and(|(_, btid)| preferred_tid == Some(btid));
            if !matched {
                let scale = (dpi.max(96) as f32) / 96.0;
                let w_dip = ((host_rect.right - host_rect.left) as f32) / scale;
                let h_dip = ((host_rect.bottom - host_rect.top) as f32) / scale;
                if !(160.0..=640.0).contains(&w_dip) || !(90.0..=820.0).contains(&h_dip) {
                    continue;
                }
            }
            let (island_rect, bridge_tid) = bridge_info
                .map(|(r, tid)| (r, Some(tid)))
                .unwrap_or((host_rect, None));
            let candidate = (hwnd, host_rect, island_rect, dpi, host_tid, bridge_tid);
            if matched {
                return Some(candidate);
            }
            if fallback.is_none() {
                fallback = Some(candidate);
            }
        }
    }
    fallback
}

/// Enumerates visible uncloaked `CoreWindow` (`BAND` 4 or 6) and `Xaml_WindowedPopupClass`
/// windows owned by `shell_exp_pid` (Notification Center, Calendar, Floating Toasts, JumpLists,
/// Legacy Quick Settings), excluding `1x1` dormant host windows. Returns `(hwnd, rect, dpi, tid, is_popup)`.
pub fn find_visible_shell_experience_host_windows(
    shell_exp_pid: u32,
) -> Vec<(HWND, RECT, u32, u32, bool)> {
    if shell_exp_pid == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut prev = HWND::default();
    for _ in 0..128 {
        let Ok(hwnd) =
            (unsafe { FindWindowExW(None, prev, w!("Windows.UI.Core.CoreWindow"), None) })
        else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev {
            break;
        }
        prev = hwnd;
        let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, shell_exp_pid, 64, 28)
        else {
            continue;
        };
        if !matches!(crate::system::window_band(hwnd), Some(4 | 6)) {
            continue;
        }
        out.push((hwnd, rect, dpi, tid, false));
    }
    let mut prev_popup = HWND::default();
    for _ in 0..128 {
        let Ok(hwnd) =
            (unsafe { FindWindowExW(None, prev_popup, w!("Xaml_WindowedPopupClass"), None) })
        else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev_popup {
            break;
        }
        prev_popup = hwnd;
        if let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, shell_exp_pid, 64, 28)
        {
            out.push((hwnd, rect, dpi, tid, true));
        }
    }
    out
}

/// Enumerates visible uncloaked `ControlCenterWindow` and secondary `Xaml_WindowedPopupClass`
/// windows owned by `shell_host_pid` (Win11 24H2/26H2 Quick Settings & Media Card).
/// Returns `(hwnd, rect, dpi, tid, is_popup)`.
pub fn find_visible_shell_host_windows(
    shell_host_pid: u32,
) -> Vec<(HWND, RECT, u32, u32, bool)> {
    if shell_host_pid == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut prev = HWND::default();
    for _ in 0..64 {
        let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, w!("ControlCenterWindow"), None) })
        else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev {
            break;
        }
        prev = hwnd;
        if let Some((rect, dpi, tid)) =
            inspect_visible_explorer_window(hwnd, shell_host_pid, 64, 28)
        {
            out.push((hwnd, rect, dpi, tid, false));
        }
    }
    let mut prev_popup = HWND::default();
    for _ in 0..128 {
        let Ok(hwnd) =
            (unsafe { FindWindowExW(None, prev_popup, w!("Xaml_WindowedPopupClass"), None) })
        else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev_popup {
            break;
        }
        prev_popup = hwnd;
        if let Some((rect, dpi, tid)) =
            inspect_visible_explorer_window(hwnd, shell_host_pid, 64, 28)
        {
            out.push((hwnd, rect, dpi, tid, true));
        }
    }
    out
}

fn find_visible_core_and_popup_windows(pid: u32) -> Vec<(HWND, RECT, u32, u32, bool)> {
    if pid == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut prev = HWND::default();
    for _ in 0..64 {
        let Ok(hwnd) =
            (unsafe { FindWindowExW(None, prev, w!("Windows.UI.Core.CoreWindow"), None) })
        else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev {
            break;
        }
        prev = hwnd;
        if let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, pid, 64, 28) {
            out.push((hwnd, rect, dpi, tid, false));
        }
    }
    let mut prev_popup = HWND::default();
    for _ in 0..128 {
        let Ok(hwnd) =
            (unsafe { FindWindowExW(None, prev_popup, w!("Xaml_WindowedPopupClass"), None) })
        else {
            break;
        };
        if hwnd.is_invalid() || hwnd == prev_popup {
            break;
        }
        prev_popup = hwnd;
        if let Some((rect, dpi, tid)) = inspect_visible_explorer_window(hwnd, pid, 64, 28) {
            out.push((hwnd, rect, dpi, tid, true));
        }
    }
    out
}

/// Enumerates visible uncloaked `CoreWindow` and secondary `Xaml_WindowedPopupClass` windows
/// owned by `start_menu_pid` (`StartMenuExperienceHost.exe`, Start Menu + Companion + Power/Context menus).
/// Returns `(hwnd, rect, dpi, tid, is_popup)`.
pub fn find_visible_start_menu_windows(
    start_menu_pid: u32,
) -> Vec<(HWND, RECT, u32, u32, bool)> {
    find_visible_core_and_popup_windows(start_menu_pid)
}

/// Enumerates visible uncloaked `CoreWindow` and secondary `Xaml_WindowedPopupClass` windows
/// owned by `search_host_pid` (`SearchHost.exe`, Win+S / taskbar search panel & context menus).
/// Returns `(hwnd, rect, dpi, tid, is_popup)`.
pub fn find_visible_search_host_windows(
    search_host_pid: u32,
) -> Vec<(HWND, RECT, u32, u32, bool)> {
    find_visible_core_and_popup_windows(search_host_pid)
}

unsafe fn xaml_island_screen_origin(hwnd: HWND) -> Option<[i32; 2]> {
    if hwnd.is_invalid() {
        return None;
    }
    if let Ok(bridge) = FindWindowExW(
        hwnd,
        None,
        w!("Windows.UI.Composition.DesktopWindowContentBridge"),
        None,
    ) {
        if !bridge.is_invalid() {
            let mut rect = RECT::default();
            if GetWindowRect(bridge, &mut rect).is_ok()
                && rect.right > rect.left
                && rect.bottom > rect.top
            {
                return Some([rect.left, rect.top]);
            }
        }
    }
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_ok()
        && rect.right > rect.left
        && rect.bottom > rect.top
    {
        Some([rect.left, rect.top])
    } else {
        None
    }
}

/// Collects candidate screen origins `[x, y]` for the XAML Island hosting `host_hwnd`
/// (owner window, un-inset overflow island, and primary/secondary taskbars) so island-relative
/// `TransformToVisual(null)` coordinates map without symmetric-centering bias.
pub fn collect_tooltip_island_origins(
    host_hwnd: HWND,
    explorer_pid: u32,
    host_tid: u32,
) -> Vec<[i32; 2]> {
    if explorer_pid == 0 {
        return Vec::new();
    }
    let mut origins = Vec::new();
    let push_unique = |list: &mut Vec<[i32; 2]>, pt: [i32; 2]| {
        if !list.contains(&pt) {
            list.push(pt);
        }
    };

    unsafe {
        if !host_hwnd.is_invalid() {
            if let Ok(owner) = GetWindow(host_hwnd, GW_OWNER) {
                if !owner.is_invalid() {
                    let mut pid = 0u32;
                    let _ = GetWindowThreadProcessId(owner, Some(&mut pid));
                    if pid == explorer_pid {
                        if let Some(pt) = xaml_island_screen_origin(owner) {
                            push_unique(&mut origins, pt);
                        }
                    }
                }
            }
        }

        let mut prev_ov = HWND::default();
        for _ in 0..16 {
            let Ok(hwnd) =
                FindWindowExW(None, prev_ov, w!("TopLevelWindowForOverflowXamlIsland"), None)
            else {
                break;
            };
            if hwnd.is_invalid() || hwnd == prev_ov {
                break;
            }
            prev_ov = hwnd;
            if !IsWindowVisible(hwnd).as_bool() {
                continue;
            }
            let mut pid = 0u32;
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != explorer_pid {
                continue;
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
                continue;
            }
            if let Some(pt) = xaml_island_screen_origin(hwnd) {
                push_unique(&mut origins, pt);
            }
        }

        let mut other_taskbars = Vec::new();
        for cls in [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")] {
            let mut prev_tb = HWND::default();
            for _ in 0..32 {
                let Ok(hwnd) = FindWindowExW(None, prev_tb, cls, None) else {
                    break;
                };
                if hwnd.is_invalid() || hwnd == prev_tb {
                    break;
                }
                prev_tb = hwnd;
                if !IsWindowVisible(hwnd).as_bool() {
                    continue;
                }
                let mut pid = 0u32;
                let tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != explorer_pid {
                    continue;
                }
                if let Some(pt) = xaml_island_screen_origin(hwnd) {
                    if tid == host_tid {
                        push_unique(&mut origins, pt);
                    } else if !other_taskbars.contains(&pt) {
                        other_taskbars.push(pt);
                    }
                }
            }
        }
        for pt in other_taskbars {
            push_unique(&mut origins, pt);
        }
    }

    origins
}

fn inspect_overflow_candidate(hwnd: HWND, is_xaml_island: bool) -> Option<(HWND, RECT, u32)> {
    unsafe {
        if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() {
            return None;
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
            return None;
        }
        let mut rect = RECT::default();
        let got_ext = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
        .is_ok();
        if !got_ext && GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = (dpi as f32) / 96.0;
        let max_w = (960.0 * scale).round() as i32;
        let max_h = (720.0 * scale).round() as i32;
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w <= 0 || h <= 0 || w > max_w || h > max_h {
            return None;
        }
        let [l, t, r, b] =
            inset_overflow_card_rect([rect.left, rect.top, rect.right, rect.bottom], dpi, is_xaml_island);
        rect = RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        };
        Some((hwnd, rect, dpi))
    }
}

pub(super) fn find_visible_overflow_window() -> Option<(HWND, RECT, u32)> {
    unsafe {
        for (cls, is_xaml_island) in [
            (w!("TopLevelWindowForOverflowXamlIsland"), true),
            (w!("NotifyIconOverflowWindow"), false),
        ] {
            let mut prev = HWND::default();
            for _ in 0..16 {
                let Ok(hwnd) = FindWindowExW(None, prev, cls, None) else {
                    break;
                };
                if hwnd.is_invalid() || hwnd == prev {
                    break;
                }
                prev = hwnd;
                if let Some(found) = inspect_overflow_candidate(hwnd, is_xaml_island) {
                    return Some(found);
                }
            }
        }
        None
    }
}

pub(super) fn should_hide_backdrop_for_taskbar(tb_hwnd: HWND, tb_rect: RECT) -> bool {
    unsafe {
        if !IsWindowVisible(tb_hwnd).as_bool() {
            return true;
        }
        if tb_rect.right <= tb_rect.left || tb_rect.bottom <= tb_rect.top {
            return true;
        }

        let mon_tb = MonitorFromWindow(tb_hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(mon_tb, &mut mi).as_bool() {
            thread_local! {
                static AUTOHIDE_CACHE: Cell<(Option<Instant>, bool)> = const { Cell::new((None, false)) };
            }
            let is_autohide = AUTOHIDE_CACHE.with(|cell| {
                let now = Instant::now();
                let (last_check, cached) = cell.get();
                if last_check
                    .is_some_and(|t| now.saturating_duration_since(t) < Duration::from_millis(500))
                {
                    return cached;
                }
                let mut abd = APPBARDATA {
                    cbSize: size_of::<APPBARDATA>() as u32,
                    hWnd: tb_hwnd,
                    ..Default::default()
                };
                let autohide =
                    (SHAppBarMessage(ABM_GETSTATE, &mut abd) as u32 & ABS_AUTOHIDE) != 0;
                cell.set((Some(now), autohide));
                autohide
            });
            let threshold = if is_autohide { 8 } else { 2 };
            if tb_rect.top >= mi.rcMonitor.bottom - threshold
                || tb_rect.bottom <= mi.rcMonitor.top + threshold
            {
                return true;
            }
        }
    }
    crate::system::is_fullscreen_occluding_taskbar(tb_hwnd)
}

