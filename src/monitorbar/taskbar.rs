//! Windows 11 taskbar geometry, TrayNotifyWnd right-anchoring, fullscreen/auto-hide guard, and Z-order maintenance.

#[cfg(windows)]
use std::mem::size_of;
#[cfg(windows)]
use windows::core::w;
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, RECT};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::GetDpiForWindow;
#[cfg(windows)]
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowRect, IsWindowVisible,
};

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
pub struct TaskbarState {
    pub taskbar_hwnd: HWND,
    pub tray_rect: RECT,
    pub tray_notify_left: i32,
    pub dpi: u32,
    pub auto_hide_hidden: bool,
}

#[cfg(windows)]
pub fn query_taskbar_state() -> Option<TaskbarState> {
    unsafe {
        let taskbar_hwnd = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        if taskbar_hwnd.0.is_null() {
            return None;
        }
        let mut tray_rect = RECT::default();
        if GetWindowRect(taskbar_hwnd, &mut tray_rect).is_err() {
            return None;
        }
        let dpi = match GetDpiForWindow(taskbar_hwnd) {
            0 => 96,
            v => v,
        };
        let scale = (dpi as f32) / 96.0;

        let mut tray_notify_left = tray_rect.right - (12.0 * scale).round() as i32;
        if let Ok(notify_hwnd) = FindWindowExW(taskbar_hwnd, None, w!("TrayNotifyWnd"), None) {
            if !notify_hwnd.0.is_null() && IsWindowVisible(notify_hwnd).as_bool() {
                let mut notify_rect = RECT::default();
                if GetWindowRect(notify_hwnd, &mut notify_rect).is_ok()
                    && notify_rect.left > tray_rect.left
                    && notify_rect.left < tray_rect.right
                {
                    tray_notify_left = notify_rect.left;
                }
            }
        }

        let mut abd = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: taskbar_hwnd,
            ..Default::default()
        };
        let state_flags = SHAppBarMessage(ABM_GETSTATE, &mut abd) as u32;
        let auto_hide_enabled = (state_flags & ABS_AUTOHIDE) != 0;

        let mon = MonitorFromWindow(taskbar_hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let auto_hide_hidden = if auto_hide_enabled && GetMonitorInfoW(mon, &mut mi).as_bool() {
            tray_rect.top >= mi.rcMonitor.bottom - 2
        } else {
            false
        };

        Some(TaskbarState {
            taskbar_hwnd,
            tray_rect,
            tray_notify_left,
            dpi,
            auto_hide_hidden,
        })
    }
}

/// Computes `(x, y, width, height)` for MonitorBar anchored directly left of `TrayNotifyWnd`.
#[cfg(windows)]
pub fn compute_monitor_bar_rect(
    state: &TaskbarState,
    desired_width_dip: f32,
    offset_x: i32,
) -> (i32, i32, i32, i32) {
    let scale = (state.dpi as f32) / 96.0;
    let margin = (4.0 * scale).round() as i32;
    let bar_w = (desired_width_dip * scale).round() as i32;
    let bar_h = (state.tray_rect.bottom - state.tray_rect.top).max(28);
    let y = state.tray_rect.top;

    let min_x = state.tray_rect.left + margin;
    let max_x = (state.tray_notify_left - margin - bar_w).max(min_x);
    let default_x = max_x;
    let clamped_x = (default_x + offset_x).clamp(min_x, max_x);

    (clamped_x, y, bar_w, bar_h)
}

#[cfg(windows)]
pub fn compute_clamped_offset(
    state: &TaskbarState,
    desired_width_dip: f32,
    raw_offset_x: i32,
) -> i32 {
    let scale = (state.dpi as f32) / 96.0;
    let margin = (4.0 * scale).round() as i32;
    let bar_w = (desired_width_dip * scale).round() as i32;
    let min_x = state.tray_rect.left + margin;
    let max_x = (state.tray_notify_left - margin - bar_w).max(min_x);
    let default_x = max_x;
    (default_x + raw_offset_x).clamp(min_x, max_x) - default_x
}

/// Detects whether a genuine fullscreen or presentation window is occluding the taskbar.
#[cfg(windows)]
pub fn is_fullscreen_active(taskbar_hwnd: HWND) -> bool {
    crate::system::is_fullscreen_occluding_taskbar(taskbar_hwnd)
}

/// Ensures MonitorBar stays above `Shell_TrayWnd` only when `Shell_TrayWnd` actually sits above it.
#[cfg(windows)]
pub fn ensure_above_taskbar(bar_hwnd: HWND, taskbar_hwnd: HWND) {
    crate::system::ensure_window_above_taskbar(bar_hwnd, taskbar_hwnd);
}
