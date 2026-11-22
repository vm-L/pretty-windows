//! Windows system actions, shell launchers, and native file picker for MonitorBar.

#[cfg(windows)]
use super::config::LeftClickAction;
#[cfg(windows)]
use std::mem::size_of;
#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use windows::core::{w, HSTRING, PCWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_NODEREFERENCELINKS,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VK_D, VK_LWIN, VIRTUAL_KEY,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::ShellExecuteW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Launches an executable, shortcut, document, or `ms-settings:` URI via ShellExecuteW.
#[cfg(windows)]
pub fn open_shell_target(target: &str) -> bool {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return false;
    }
    let wide_target = HSTRING::from(trimmed);
    let target_path = Path::new(trimmed);
    let wide_dir = if target_path.is_file() {
        target_path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(HSTRING::from)
    } else {
        None
    };
    let dir_ptr = wide_dir
        .as_ref()
        .map_or(PCWSTR::null(), |dir| PCWSTR(dir.as_ptr()));
    let ret = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide_target.as_ptr()),
            PCWSTR::null(),
            dir_ptr,
            SW_SHOWNORMAL,
        )
    };
    (ret.0 as isize) > 32
}

/// Sends Win+D via `SendInput` to toggle Show Desktop reliably.
#[cfg(windows)]
pub fn toggle_show_desktop() -> bool {
    let key_input = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [
        key_input(VK_LWIN, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_D, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_D, KEYEVENTF_KEYUP),
        key_input(VK_LWIN, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    sent == inputs.len() as u32
}

/// Opens the Windows native Open File modal dialog for choosing a custom application/script.
///
/// Callers must hide or dismiss any topmost popup windows before calling this function.
#[cfg(windows)]
pub fn pick_custom_app_file(owner: HWND, current_path: &str) -> Option<String> {
    let filter_utf16: Vec<u16> =
        "应用程序与快捷方式 (*.exe;*.lnk;*.bat;*.cmd)\0*.exe;*.lnk;*.bat;*.cmd\0所有文件 (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
    let title_utf16: Vec<u16> = "选择左键打开的程序\0".encode_utf16().collect();

    let mut file_buf = vec![0u16; 1024];
    let current_trimmed = current_path.trim();
    if !current_trimmed.is_empty() {
        let existing_dir = std::path::Path::new(current_trimmed);
        if existing_dir.exists() {
            let init_utf16: Vec<u16> = current_trimmed.encode_utf16().collect();
            let copy_len = init_utf16.len().min(file_buf.len() - 1);
            file_buf[..copy_len].copy_from_slice(&init_utf16[..copy_len]);
        }
    }

    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter_utf16.as_ptr()),
        nFilterIndex: 1,
        lpstrFile: windows::core::PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrTitle: PCWSTR(title_utf16.as_ptr()),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_NODEREFERENCELINKS,
        ..Default::default()
    };

    let ok = unsafe { GetOpenFileNameW(&mut ofn) };
    if !ok.as_bool() {
        return None;
    }

    let end = file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len());
    let chosen = String::from_utf16_lossy(&file_buf[..end]).trim().to_string();
    if chosen.is_empty() {
        None
    } else {
        Some(chosen)
    }
}

/// Executes non-menu left-click behavior; returns `false` if `CustomApp` has no configured path.
#[cfg(windows)]
pub fn execute_left_click_non_menu(action: LeftClickAction, custom_app_path: &str) -> bool {
    match action {
        LeftClickAction::Menu => false,
        LeftClickAction::TaskManager => open_shell_target("taskmgr.exe"),
        LeftClickAction::ShowDesktop => toggle_show_desktop(),
        LeftClickAction::Settings => open_shell_target("ms-settings:"),
        LeftClickAction::CustomApp => open_shell_target(custom_app_path),
    }
}
