use crate::system::read_reg_dword;
use std::mem::size_of;
use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowRect, IsWindowVisible,
};

#[derive(Clone, Copy, Debug)]
pub struct TaskbarState {
    pub taskbar_hwnd: HWND,
    pub taskbar_rect: RECT,
    pub tray_rect: RECT,
    pub dpi: u32,
    pub auto_hide_collapsed: bool,
    pub widgets_enabled: bool,
}

/// Locates Windows 11 `Shell_TrayWnd` and `TrayNotifyWnd` geometry and DPI.
pub fn query_taskbar_state() -> Option<TaskbarState> {
    unsafe {
        let taskbar_hwnd = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        if taskbar_hwnd.is_invalid() || !IsWindowVisible(taskbar_hwnd).as_bool() {
            return None;
        }
        let tray_hwnd = FindWindowExW(taskbar_hwnd, None, w!("TrayNotifyWnd"), None).ok()?;
        if tray_hwnd.is_invalid() {
            return None;
        }

        let mut taskbar_rect = RECT::default();
        let mut tray_rect = RECT::default();
        GetWindowRect(taskbar_hwnd, &mut taskbar_rect).ok()?;
        GetWindowRect(tray_hwnd, &mut tray_rect).ok()?;

        let dpi = GetDpiForWindow(taskbar_hwnd).max(96);

        // Detect whether taskbar is auto-hide and currently slid off-screen.
        let mut abd = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: taskbar_hwnd,
            ..Default::default()
        };
        let state_bits = SHAppBarMessage(ABM_GETSTATE, &mut abd) as u32;
        let is_autohide = (state_bits & ABS_AUTOHIDE) != 0;
        let mut auto_hide_collapsed = false;
        if is_autohide {
            let mon = MonitorFromWindow(taskbar_hwnd, MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(mon, &mut mi).as_bool() && taskbar_rect.top >= mi.rcMonitor.bottom - 4
            {
                auto_hide_collapsed = true;
            }
        }

        Some(TaskbarState {
            taskbar_hwnd,
            taskbar_rect,
            tray_rect,
            dpi,
            auto_hide_collapsed,
            widgets_enabled: is_widgets_enabled(),
        })
    }
}

/// Computes MediaBar screen coordinates anchored on the left side of the taskbar
/// (automatically leaving room for the Win11 Widgets button when enabled).
/// Returns `(x, y, width, height, clamped_offset_x)`.
pub fn compute_bar_rect(
    tb: &TaskbarState,
    bar_width_px: i32,
    offset_x: i32,
) -> (i32, i32, i32, i32, i32) {
    let scale = (tb.dpi as f32) / 96.0;
    let left_anchor_margin = if tb.widgets_enabled {
        (158.0 * scale).round() as i32
    } else {
        (6.0 * scale).round() as i32
    };
    let default_x = tb.taskbar_rect.left + left_anchor_margin;

    let min_x = tb.taskbar_rect.left + (4.0 * scale).round() as i32;
    let max_x = (tb.tray_rect.left - bar_width_px - (4.0 * scale).round() as i32).max(min_x);

    let clamped_x = (default_x + offset_x).clamp(min_x, max_x);
    let clamped_offset = clamped_x - default_x;
    let height = (tb.taskbar_rect.bottom - tb.taskbar_rect.top).max(28);
    (
        clamped_x,
        tb.taskbar_rect.top,
        bar_width_px,
        height,
        clamped_offset,
    )
}

/// Checks whether a genuine fullscreen or presentation window is occluding the taskbar.
pub fn is_fullscreen_active(taskbar_hwnd: HWND) -> bool {
    crate::system::is_fullscreen_occluding_taskbar(taskbar_hwnd)
}

/// Ensures MediaBar stays above `Shell_TrayWnd` in Z-order without activating or stealing focus.
pub fn ensure_above_taskbar(bar_hwnd: HWND, taskbar_hwnd: HWND) {
    crate::system::ensure_window_above_taskbar(bar_hwnd, taskbar_hwnd);
}

/// Detects whether Windows 11 Taskbar Widgets button is enabled on the left side.
pub fn is_widgets_enabled() -> bool {
    if read_reg_dword(
        HKEY_LOCAL_MACHINE,
        w!("SOFTWARE\\Policies\\Microsoft\\Dsh"),
        w!("AllowNewsAndInterests"),
    ) == Some(0)
    {
        return false;
    }

    match read_reg_dword(
        HKEY_CURRENT_USER,
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"),
        w!("TaskbarDa"),
    ) {
        Some(v) => v != 0,
        None => true,
    }
}
