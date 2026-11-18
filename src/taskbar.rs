use crate::color::Rgb;
use std::mem::size_of;
use windows::core::{w, PCWSTR};
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_DWORD, REG_SZ, REG_VALUE_TYPE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{
    SHAppBarMessage, SHQueryUserNotificationState, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA,
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowRect,
    IsWindowVisible, SetWindowPos, GW_HWNDPREV, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE,
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

/// Checks whether a fullscreen application or presentation mode is currently active.
pub fn is_fullscreen_active(taskbar_hwnd: HWND) -> bool {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_BUSY
                || state == QUNS_RUNNING_D3D_FULL_SCREEN
                || state == QUNS_PRESENTATION_MODE
            {
                return true;
            }
        }

        let fg = GetForegroundWindow();
        if fg.is_invalid() || fg == taskbar_hwnd || !IsWindowVisible(fg).as_bool() {
            return false;
        }

        let mut class_buf = [0u16; 64];
        let len = GetClassNameW(fg, &mut class_buf);
        if len > 0 {
            let class_name = String::from_utf16_lossy(&class_buf[..len as usize]);
            if matches!(
                class_name.as_str(),
                "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
            ) {
                return false;
            }
        }

        let mon_tb = MonitorFromWindow(taskbar_hwnd, MONITOR_DEFAULTTOPRIMARY);
        let mon_fg = MonitorFromWindow(fg, MONITOR_DEFAULTTOPRIMARY);
        if mon_tb != mon_fg {
            return false;
        }

        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mut fg_rect = RECT::default();
        if GetMonitorInfoW(mon_tb, &mut mi).as_bool() && GetWindowRect(fg, &mut fg_rect).is_ok() {
            return fg_rect.left <= mi.rcMonitor.left
                && fg_rect.top <= mi.rcMonitor.top
                && fg_rect.right >= mi.rcMonitor.right
                && fg_rect.bottom >= mi.rcMonitor.bottom;
        }
    }
    false
}

/// Ensures MediaBar stays above `Shell_TrayWnd` in Z-order without activating or stealing focus.
pub fn ensure_above_taskbar(bar_hwnd: HWND, taskbar_hwnd: HWND) {
    unsafe {
        // Walk upward from bar_hwnd toward top of Z-order; if taskbar_hwnd sits above bar_hwnd,
        // re-promote bar_hwnd to HWND_TOPMOST without activating.
        let mut cur = bar_hwnd;
        let mut taskbar_above = false;
        for _ in 0..512 {
            match GetWindow(cur, GW_HWNDPREV) {
                Ok(prev) if !prev.is_invalid() => {
                    if prev == taskbar_hwnd {
                        taskbar_above = true;
                        break;
                    }
                    cur = prev;
                }
                _ => break,
            }
        }
        if !taskbar_above {
            return;
        }

        let _ = SetWindowPos(
            bar_hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

fn read_reg_dword(root: HKEY, subkey: PCWSTR, value_name: PCWSTR) -> Option<u32> {
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

/// Reads Windows 11 taskbar theme (`true` = light taskbar, `false` = dark taskbar).
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

const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE_NAME: PCWSTR = w!("MediaBar");

/// Returns `true` if MediaBar is configured to launch at Windows startup.
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

/// Enables or disables MediaBar auto-start in HKCU Run registry key.
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
            RegSetValueExW(hkey, RUN_VALUE_NAME, 0, REG_SZ, Some(byte_slice)).0 == 0
        } else {
            let err = RegDeleteValueW(hkey, RUN_VALUE_NAME);
            err.0 == 0 || err.0 == 2 // ERROR_FILE_NOT_FOUND
        };
        let _ = RegCloseKey(hkey);
        ok
    }
}
