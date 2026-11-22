use super::super::inject::{decode_menu_flyout_rects_payload, invoke_xaml_diagnostics_ex_status, pipe_name_wide, prepare_tap_dll_for_injection, verify_pipe_server_pid, XamlDiagInjectStatus, CMD_MENU_FLYOUT_RECTS, CMD_PING, CMD_SET_ALTTAB_ITEM_STYLE, CMD_SET_INPUT_SWITCHER_FALLBACK_STYLE, CMD_SET_INPUT_SWITCHER_SUPPRESS, CMD_SET_MENU_FLYOUT_FALLBACK_STYLE, CMD_SET_MENU_FLYOUT_SUPPRESS, CMD_SET_OSD_FALLBACK_STYLE, CMD_SET_OSD_SUPPRESS, CMD_SET_TOOLTIP_FALLBACK_STYLE, CMD_SET_TOOLTIP_SUPPRESS, CMD_SET_VIRTUAL_DESKTOP_BAR_STYLE, MAX_MENU_FLYOUT_RECTS};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING, SECURITY_ANONYMOUS, SECURITY_SQOS_PRESENT};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::System::Threading::GetCurrentProcessId;

pub(super) fn inject_and_wait_pipe(explorer_pid: u32) -> XamlDiagInjectStatus {
    let Some(dll_path) = prepare_tap_dll_for_injection(false) else {
        return XamlDiagInjectStatus::Failed;
    };
    let status = invoke_xaml_diagnostics_ex_status(explorer_pid, &dll_path);
    if status != XamlDiagInjectStatus::Connected && !send_command(explorer_pid, CMD_PING, 40) {
        return status;
    }

    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        if send_command(explorer_pid, CMD_PING, 50) {
            return XamlDiagInjectStatus::Connected;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    XamlDiagInjectStatus::Failed
}


fn open_pipe(explorer_pid: u32, wait_ms: u32) -> Option<HANDLE> {
    let pipe_wide = pipe_name_wide(explorer_pid);
    let flags = FILE_ATTRIBUTE_NORMAL | SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS;
    unsafe {
        match CreateFileW(
            PCWSTR(pipe_wide.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        ) {
            Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE => {
                if verify_pipe_server_pid(h, explorer_pid) {
                    Some(h)
                } else {
                    let _ = CloseHandle(h);
                    None
                }
            }
            _ => {
                if wait_ms == 0 || !WaitNamedPipeW(PCWSTR(pipe_wide.as_ptr()), wait_ms).as_bool()
                {
                    return None;
                }
                match CreateFileW(
                    PCWSTR(pipe_wide.as_ptr()),
                    GENERIC_READ.0 | GENERIC_WRITE.0,
                    FILE_SHARE_NONE,
                    None,
                    OPEN_EXISTING,
                    flags,
                    None,
                ) {
                    Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE => {
                        if verify_pipe_server_pid(h, explorer_pid) {
                            Some(h)
                        } else {
                            let _ = CloseHandle(h);
                            None
                        }
                    }
                    _ => None,
                }
            }
        }
    }
}

pub(super) fn query_card_rect(
    explorer_pid: u32,
    cmd: u8,
    wait_ms: u32,
) -> Result<Option<(u32, [f32; 4])>, ()> {
    let handle = open_pipe(explorer_pid, wait_ms).ok_or(())?;
    unsafe {
        let host_pid = GetCurrentProcessId().to_le_bytes();
        let req = [cmd, host_pid[0], host_pid[1], host_pid[2], host_pid[3]];
        let mut written = 0u32;
        let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
            && written == req.len() as u32;
        if !write_ok {
            let _ = CloseHandle(handle);
            return Err(());
        }

        let mut tag = [0u8; 1];
        let mut tag_read = 0u32;
        if ReadFile(handle, Some(&mut tag), Some(&mut tag_read), None).is_err() || tag_read != 1
        {
            let _ = CloseHandle(handle);
            return Err(());
        }
        if tag[0] == 0xFF {
            let _ = CloseHandle(handle);
            return Ok(None);
        }
        if tag[0] != cmd {
            let _ = CloseHandle(handle);
            return Err(());
        }

        let mut payload = [0u8; 21];
        let mut off = 0usize;
        while off < payload.len() {
            let mut r = 0u32;
            if ReadFile(handle, Some(&mut payload[off..]), Some(&mut r), None).is_err()
                || r == 0
            {
                let _ = CloseHandle(handle);
                return Err(());
            }
            off += r as usize;
        }
        let _ = CloseHandle(handle);

        if payload[0] == 0 {
            return Ok(None);
        }
        let tid = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
        let l = f32::from_le_bytes([payload[5], payload[6], payload[7], payload[8]]);
        let t = f32::from_le_bytes([payload[9], payload[10], payload[11], payload[12]]);
        let r = f32::from_le_bytes([payload[13], payload[14], payload[15], payload[16]]);
        let b = f32::from_le_bytes([payload[17], payload[18], payload[19], payload[20]]);
        Ok(Some((tid, [l, t, r, b])))
    }
}

fn send_style_colors_cmd(
    explorer_pid: u32,
    cmd: u8,
    bg_argb: u32,
    border_argb: u32,
    wait_ms: u32,
) -> bool {
    let Some(handle) = open_pipe(explorer_pid, wait_ms) else {
        return false;
    };
    unsafe {
        let host_pid = GetCurrentProcessId().to_le_bytes();
        let bg = bg_argb.to_le_bytes();
        let bd = border_argb.to_le_bytes();
        let req = [
            cmd,
            host_pid[0],
            host_pid[1],
            host_pid[2],
            host_pid[3],
            bg[0],
            bg[1],
            bg[2],
            bg[3],
            bd[0],
            bd[1],
            bd[2],
            bd[3],
        ];
        let mut written = 0u32;
        let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
            && written == req.len() as u32;

        let mut ack = [0u8; 1];
        let mut read = 0u32;
        let read_ok = write_ok
            && ReadFile(handle, Some(&mut ack), Some(&mut read), None).is_ok()
            && read == 1
            && (ack[0] == cmd || ack[0] == 0xFF);

        let _ = CloseHandle(handle);
        read_ok
    }
}

pub(super) fn send_alt_tab_item_style(
    explorer_pid: u32,
    bg_argb: u32,
    border_argb: u32,
    wait_ms: u32,
) -> bool {
    send_style_colors_cmd(
        explorer_pid,
        CMD_SET_ALTTAB_ITEM_STYLE,
        bg_argb,
        border_argb,
        wait_ms,
    )
}

pub(super) fn send_osd_fallback_style(
    explorer_pid: u32,
    bg_argb: u32,
    border_argb: u32,
    wait_ms: u32,
) -> bool {
    send_style_colors_cmd(
        explorer_pid,
        CMD_SET_OSD_FALLBACK_STYLE,
        bg_argb,
        border_argb,
        wait_ms,
    )
}

pub(super) fn send_tooltip_fallback_style(
    explorer_pid: u32,
    bg_argb: u32,
    border_argb: u32,
    wait_ms: u32,
) -> bool {
    send_style_colors_cmd(
        explorer_pid,
        CMD_SET_TOOLTIP_FALLBACK_STYLE,
        bg_argb,
        border_argb,
        wait_ms,
    )
}

pub(super) fn send_menu_flyout_fallback_style(
    explorer_pid: u32,
    bg_argb: u32,
    border_argb: u32,
    wait_ms: u32,
) -> bool {
    send_style_colors_cmd(
        explorer_pid,
        CMD_SET_MENU_FLYOUT_FALLBACK_STYLE,
        bg_argb,
        border_argb,
        wait_ms,
    )
}

pub(super) fn send_virtual_desktop_bar_style(
    explorer_pid: u32,
    bg_argb: u32,
    border_argb: u32,
    wait_ms: u32,
) -> bool {
    send_style_colors_cmd(
        explorer_pid,
        CMD_SET_VIRTUAL_DESKTOP_BAR_STYLE,
        bg_argb,
        border_argb,
        wait_ms,
    )
}


pub(super) fn query_menu_flyout_rects(
    explorer_pid: u32,
    wait_ms: u32,
) -> Result<Vec<(u32, [f32; 4])>, ()> {
    let handle = open_pipe(explorer_pid, wait_ms).ok_or(())?;
    unsafe {
        let host_pid = GetCurrentProcessId().to_le_bytes();
        let req = [
            CMD_MENU_FLYOUT_RECTS,
            host_pid[0],
            host_pid[1],
            host_pid[2],
            host_pid[3],
        ];
        let mut written = 0u32;
        let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
            && written == req.len() as u32;
        if !write_ok {
            let _ = CloseHandle(handle);
            return Err(());
        }

        let mut tag = [0u8; 1];
        let mut tag_read = 0u32;
        if ReadFile(handle, Some(&mut tag), Some(&mut tag_read), None).is_err() || tag_read != 1
        {
            let _ = CloseHandle(handle);
            return Err(());
        }
        if tag[0] == 0xFF {
            let _ = CloseHandle(handle);
            return Ok(Vec::new());
        }
        if tag[0] != CMD_MENU_FLYOUT_RECTS {
            let _ = CloseHandle(handle);
            return Err(());
        }

        let mut payload = [0u8; 1 + MAX_MENU_FLYOUT_RECTS * 20];
        let mut off = 0usize;
        while off < payload.len() {
            let mut r = 0u32;
            if ReadFile(handle, Some(&mut payload[off..]), Some(&mut r), None).is_err()
                || r == 0
            {
                let _ = CloseHandle(handle);
                return Err(());
            }
            off += r as usize;
        }
        let _ = CloseHandle(handle);
        Ok(decode_menu_flyout_rects_payload(&payload))
    }
}

fn send_bool_flag_cmd(explorer_pid: u32, cmd: u8, flag: bool, wait_ms: u32) -> bool {
    let Some(handle) = open_pipe(explorer_pid, wait_ms) else {
        return false;
    };
    unsafe {
        let host_pid = GetCurrentProcessId().to_le_bytes();
        let req = [
            cmd,
            host_pid[0],
            host_pid[1],
            host_pid[2],
            host_pid[3],
            u8::from(flag),
        ];
        let mut written = 0u32;
        let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
            && written == req.len() as u32;

        let mut ack = [0u8; 1];
        let mut read = 0u32;
        let read_ok = write_ok
            && ReadFile(handle, Some(&mut ack), Some(&mut read), None).is_ok()
            && read == 1
            && (ack[0] == cmd || ack[0] == 0xFF);

        let _ = CloseHandle(handle);
        read_ok
    }
}

pub(super) fn send_input_switcher_fallback_style(
    explorer_pid: u32,
    bg_argb: u32,
    border_argb: u32,
    wait_ms: u32,
) -> bool {
    send_style_colors_cmd(
        explorer_pid,
        CMD_SET_INPUT_SWITCHER_FALLBACK_STYLE,
        bg_argb,
        border_argb,
        wait_ms,
    )
}

pub(super) fn send_osd_suppress(explorer_pid: u32, suppressed: bool, wait_ms: u32) -> bool {
    send_bool_flag_cmd(explorer_pid, CMD_SET_OSD_SUPPRESS, suppressed, wait_ms)
}

pub(super) fn send_tooltip_suppress(explorer_pid: u32, suppressed: bool, wait_ms: u32) -> bool {
    send_bool_flag_cmd(explorer_pid, CMD_SET_TOOLTIP_SUPPRESS, suppressed, wait_ms)
}

pub(super) fn send_input_switcher_suppress(explorer_pid: u32, suppressed: bool, wait_ms: u32) -> bool {
    send_bool_flag_cmd(
        explorer_pid,
        CMD_SET_INPUT_SWITCHER_SUPPRESS,
        suppressed,
        wait_ms,
    )
}

pub(super) fn send_menu_flyout_suppress(explorer_pid: u32, suppressed: bool, wait_ms: u32) -> bool {
    send_bool_flag_cmd(explorer_pid, CMD_SET_MENU_FLYOUT_SUPPRESS, suppressed, wait_ms)
}

pub(super) fn send_command(explorer_pid: u32, cmd: u8, wait_ms: u32) -> bool {
    let Some(handle) = open_pipe(explorer_pid, wait_ms) else {
        return false;
    };
    unsafe {
        let host_pid = GetCurrentProcessId().to_le_bytes();
        let req = [cmd, host_pid[0], host_pid[1], host_pid[2], host_pid[3]];
        let mut written = 0u32;
        let write_ok = WriteFile(handle, Some(&req), Some(&mut written), None).is_ok()
            && written == req.len() as u32;

        let mut ack = [0u8; 1];
        let mut read = 0u32;
        let read_ok = write_ok
            && ReadFile(handle, Some(&mut ack), Some(&mut read), None).is_ok()
            && read == 1
            && ack[0] == cmd;

        let _ = CloseHandle(handle);
        read_ok
    }
}

