use super::super::tree_match::{pipe_name_for_pid, CMD_ALTTAB_RECT, CMD_HIDE, CMD_HOVER_FLYOUT_RECT, CMD_INPUT_SWITCHER_RECT, CMD_MENU_FLYOUT_RECTS, CMD_OSD_RECT, CMD_PING, CMD_RESTORE, CMD_SET_ALTTAB_ITEM_STYLE, CMD_SET_INPUT_SWITCHER_FALLBACK_STYLE, CMD_SET_INPUT_SWITCHER_SUPPRESS, CMD_SET_MENU_FLYOUT_FALLBACK_STYLE, CMD_SET_MENU_FLYOUT_SUPPRESS, CMD_SET_OSD_FALLBACK_STYLE, CMD_SET_OSD_SUPPRESS, CMD_SET_START_MODAL_STYLE, CMD_SET_TOOLTIP_FALLBACK_STYLE, CMD_SET_TOOLTIP_SUPPRESS, CMD_SET_VIRTUAL_DESKTOP_BAR_STYLE, CMD_SHELL_CARD_RECTS, CMD_SHUTDOWN, CMD_STATUS, CMD_TOOLTIP_RECT, CMD_VERSION, MAX_MENU_FLYOUT_RECTS, MAX_SHELL_CARD_RECTS, TAP_PROTOCOL_VERSION};
use std::ffi::c_void;
use std::sync::atomic::{Ordering};
use std::sync::{Arc};
use windows::core::{PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, S_FALSE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE,
    FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_ANONYMOUS,
    SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcessId, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE};
use super::raw_com::*;
use super::TapRuntime;

// ---------- Named Pipe IPC + Host Crash Watchdog Thread ----------

unsafe fn read_pipe_exact_with_stop(
    pipe: HANDLE,
    event: HANDLE,
    stop_event: Option<HANDLE>,
    buf: &mut [u8],
    timeout_ms: u32,
) -> bool {
    let mut off = 0usize;
    while off < buf.len() {
        let _ = ResetEvent(event);
        let mut ov = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };
        let res = ReadFile(pipe, Some(&mut buf[off..]), None, Some(&mut ov));
        let got = if res.is_ok() {
            let mut tr = 0u32;
            if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
                return false;
            }
            tr as usize
        } else if GetLastError() == ERROR_IO_PENDING {
            let signaled_ok = match stop_event {
                Some(se) if !se.is_invalid() => {
                    WaitForMultipleObjects(&[event, se], false, timeout_ms) == WAIT_OBJECT_0
                }
                _ => WaitForSingleObject(event, timeout_ms) == WAIT_OBJECT_0,
            };
            if !signaled_ok {
                let _ = CancelIo(pipe);
                let mut tr = 0u32;
                let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                return false;
            }
            let mut tr = 0u32;
            if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
                return false;
            }
            tr as usize
        } else {
            return false;
        };
        if got == 0 {
            return false;
        }
        off += got;
    }
    true
}

unsafe fn write_pipe_exact_with_stop(
    pipe: HANDLE,
    event: HANDLE,
    stop_event: Option<HANDLE>,
    buf: &[u8],
    timeout_ms: u32,
) -> bool {
    let mut off = 0usize;
    while off < buf.len() {
        let _ = ResetEvent(event);
        let mut ov = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };
        let res = WriteFile(pipe, Some(&buf[off..]), None, Some(&mut ov));
        let put = if res.is_ok() {
            let mut tr = 0u32;
            if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
                return false;
            }
            tr as usize
        } else if GetLastError() == ERROR_IO_PENDING {
            let signaled_ok = match stop_event {
                Some(se) if !se.is_invalid() => {
                    WaitForMultipleObjects(&[event, se], false, timeout_ms) == WAIT_OBJECT_0
                }
                _ => WaitForSingleObject(event, timeout_ms) == WAIT_OBJECT_0,
            };
            if !signaled_ok {
                let _ = CancelIo(pipe);
                let mut tr = 0u32;
                let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                return false;
            }
            let mut tr = 0u32;
            if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
                return false;
            }
            tr as usize
        } else {
            return false;
        };
        if put == 0 {
            return false;
        }
        off += put;
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PipeAction {
    Continue,
    Shutdown,
    Disconnect,
}

/// Processes a single incoming IPC command.
unsafe fn handle_pipe_command(
    rt: &Arc<TapRuntime>,
    pipe: HANDLE,
    io_event: HANDLE,
    cmd: u8,
    host_pid: u32,
) -> PipeAction {
    let stop = Some(rt.stop_event_handle());
    match cmd {
        CMD_HIDE => {
            rt.set_hide(true, host_pid);
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &[cmd], 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_RESTORE => {
            rt.set_hide(false, host_pid);
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &[cmd], 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_PING => {
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &[cmd], 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_VERSION => {
            if !write_pipe_exact_with_stop(
                pipe,
                io_event,
                stop,
                &[CMD_VERSION, TAP_PROTOCOL_VERSION],
                1000,
            ) {
                return PipeAction::Disconnect;
            }
        }
        CMD_ALTTAB_RECT => {
            let bounds = if rt.desired_hide() {
                rt.query_alt_tab_bounds_dip(40)
            } else {
                None
            };
            let mut resp = [0u8; 22];
            resp[0] = CMD_ALTTAB_RECT;
            if let Some((tid, r)) = bounds {
                resp[1] = 1;
                resp[2..6].copy_from_slice(&tid.to_le_bytes());
                resp[6..10].copy_from_slice(&r[0].to_le_bytes());
                resp[10..14].copy_from_slice(&r[1].to_le_bytes());
                resp[14..18].copy_from_slice(&r[2].to_le_bytes());
                resp[18..22].copy_from_slice(&r[3].to_le_bytes());
            }
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &resp, 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_HOVER_FLYOUT_RECT => {
            let bounds = if rt.desired_hide() {
                rt.query_hover_flyout_bounds_dip(40)
            } else {
                None
            };
            let mut resp = [0u8; 22];
            resp[0] = CMD_HOVER_FLYOUT_RECT;
            if let Some((tid, r)) = bounds {
                resp[1] = 1;
                resp[2..6].copy_from_slice(&tid.to_le_bytes());
                resp[6..10].copy_from_slice(&r[0].to_le_bytes());
                resp[10..14].copy_from_slice(&r[1].to_le_bytes());
                resp[14..18].copy_from_slice(&r[2].to_le_bytes());
                resp[18..22].copy_from_slice(&r[3].to_le_bytes());
            }
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &resp, 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_OSD_RECT | CMD_TOOLTIP_RECT | CMD_INPUT_SWITCHER_RECT => {
            let bounds = match cmd {
                CMD_OSD_RECT => {
                    if rt.desired_hide() && !rt.is_osd_suppressed() {
                        rt.query_osd_bounds_dip(40)
                    } else {
                        None
                    }
                }
                CMD_TOOLTIP_RECT => {
                    if rt.desired_hide() && !rt.is_tooltip_suppressed() {
                        rt.query_tooltip_bounds_dip(40)
                    } else {
                        None
                    }
                }
                _ => {
                    if rt.desired_hide() && !rt.is_input_switcher_suppressed() {
                        rt.query_input_switcher_bounds_dip(40)
                    } else {
                        None
                    }
                }
            };
            let mut resp = [0u8; 22];
            resp[0] = cmd;
            if let Some((tid, r)) = bounds {
                resp[1] = 1;
                resp[2..6].copy_from_slice(&tid.to_le_bytes());
                resp[6..10].copy_from_slice(&r[0].to_le_bytes());
                resp[10..14].copy_from_slice(&r[1].to_le_bytes());
                resp[14..18].copy_from_slice(&r[2].to_le_bytes());
                resp[18..22].copy_from_slice(&r[3].to_le_bytes());
            }
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &resp, 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_MENU_FLYOUT_RECTS => {
            let rects = if rt.desired_hide() && !rt.is_menu_flyout_suppressed() {
                rt.query_menu_flyout_bounds_dip(40)
            } else {
                Vec::new()
            };
            let mut resp = [0u8; 2 + MAX_MENU_FLYOUT_RECTS * 20];
            resp[0] = CMD_MENU_FLYOUT_RECTS;
            let count = rects.len().min(MAX_MENU_FLYOUT_RECTS);
            resp[1] = count as u8;
            for (slot, &(tid, r)) in rects.iter().take(count).enumerate() {
                let base = 2 + slot * 20;
                resp[base..base + 4].copy_from_slice(&tid.to_le_bytes());
                resp[base + 4..base + 8].copy_from_slice(&r[0].to_le_bytes());
                resp[base + 8..base + 12].copy_from_slice(&r[1].to_le_bytes());
                resp[base + 12..base + 16].copy_from_slice(&r[2].to_le_bytes());
                resp[base + 16..base + 20].copy_from_slice(&r[3].to_le_bytes());
            }
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &resp, 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_SHELL_CARD_RECTS => {
            let rects = if rt.desired_hide() && !rt.is_menu_flyout_suppressed() {
                rt.query_shell_card_bounds_dip(40)
            } else {
                Vec::new()
            };
            let mut resp = [0u8; 2 + MAX_SHELL_CARD_RECTS * 20];
            resp[0] = CMD_SHELL_CARD_RECTS;
            let count = rects.len().min(MAX_SHELL_CARD_RECTS);
            resp[1] = count as u8;
            for (slot, &(tid, r)) in rects.iter().take(count).enumerate() {
                let base = 2 + slot * 20;
                resp[base..base + 4].copy_from_slice(&tid.to_le_bytes());
                resp[base + 4..base + 8].copy_from_slice(&r[0].to_le_bytes());
                resp[base + 8..base + 12].copy_from_slice(&r[1].to_le_bytes());
                resp[base + 12..base + 16].copy_from_slice(&r[2].to_le_bytes());
                resp[base + 16..base + 20].copy_from_slice(&r[3].to_le_bytes());
            }
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &resp, 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_SET_OSD_SUPPRESS
        | CMD_SET_TOOLTIP_SUPPRESS
        | CMD_SET_MENU_FLYOUT_SUPPRESS
        | CMD_SET_INPUT_SWITCHER_SUPPRESS => {
            let mut extra = [0u8; 1];
            if !read_pipe_exact_with_stop(pipe, io_event, stop, &mut extra, 1000) {
                return PipeAction::Disconnect;
            }
            match cmd {
                CMD_SET_OSD_SUPPRESS => {
                    rt.set_osd_suppressed(extra[0] != 0);
                }
                CMD_SET_TOOLTIP_SUPPRESS => {
                    rt.set_tooltip_suppressed(extra[0] != 0);
                }
                CMD_SET_INPUT_SWITCHER_SUPPRESS => {
                    rt.set_input_switcher_suppressed(extra[0] != 0);
                }
                _ => {
                    rt.set_menu_flyout_suppressed(extra[0] != 0);
                }
            }
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &[cmd], 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_SET_ALTTAB_ITEM_STYLE
        | CMD_SET_OSD_FALLBACK_STYLE
        | CMD_SET_TOOLTIP_FALLBACK_STYLE
        | CMD_SET_MENU_FLYOUT_FALLBACK_STYLE
        | CMD_SET_VIRTUAL_DESKTOP_BAR_STYLE
        | CMD_SET_INPUT_SWITCHER_FALLBACK_STYLE => {
            let mut extra = [0u8; 8];
            if !read_pipe_exact_with_stop(pipe, io_event, stop, &mut extra, 1000) {
                return PipeAction::Disconnect;
            }
            let bg_argb = u32::from_le_bytes([extra[0], extra[1], extra[2], extra[3]]);
            let border_argb = u32::from_le_bytes([extra[4], extra[5], extra[6], extra[7]]);
            match cmd {
                CMD_SET_ALTTAB_ITEM_STYLE => {
                    rt.set_alt_tab_item_colors(bg_argb, border_argb);
                }
                CMD_SET_OSD_FALLBACK_STYLE => {
                    rt.set_osd_fallback_colors(bg_argb, border_argb);
                }
                CMD_SET_TOOLTIP_FALLBACK_STYLE => {
                    rt.set_tooltip_fallback_colors(bg_argb, border_argb);
                }
                CMD_SET_VIRTUAL_DESKTOP_BAR_STYLE => {
                    rt.set_virtual_desktop_bar_colors(bg_argb, border_argb);
                }
                CMD_SET_INPUT_SWITCHER_FALLBACK_STYLE => {
                    rt.set_input_switcher_fallback_colors(bg_argb, border_argb);
                }
                _ => {
                    rt.set_menu_flyout_fallback_colors(bg_argb, border_argb);
                }
            }
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &[cmd], 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_SET_START_MODAL_STYLE => {
            let mut extra = [0u8; 10];
            if !read_pipe_exact_with_stop(pipe, io_event, stop, &mut extra, 1000) {
                return PipeAction::Disconnect;
            }
            let halo0_rgb = u32::from_le_bytes([extra[2], extra[3], extra[4], extra[5]]);
            let halo1_rgb = u32::from_le_bytes([extra[6], extra[7], extra[8], extra[9]]);
            rt.set_start_modal_style(extra[0], extra[1] != 0, halo0_rgb, halo1_rgb);
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &[cmd], 1000) {
                return PipeAction::Disconnect;
            }
        }
        CMD_SHUTDOWN => {
            rt.trigger_shutdown(600);
            let _ = write_pipe_exact_with_stop(pipe, io_event, stop, &[cmd], 1000);
            return PipeAction::Shutdown;
        }
        CMD_STATUS => {
            let desired = u8::from(rt.desired_hide());
            let advise_hr = rt.advise_hr.load(Ordering::Acquire).to_le_bytes();
            let node_cnt = rt
                .tree
                .lock()
                .map(|t| t.node_count().min(u32::MAX as usize) as u32)
                .unwrap_or(0)
                .to_le_bytes();
            let target_cnt = rt
                .state
                .lock()
                .map(|g| g.targets.len().min(u32::MAX as usize) as u32)
                .unwrap_or(0)
                .to_le_bytes();
            let op_hr = rt.last_put_opacity_hr.load(Ordering::Acquire).to_le_bytes();
            let vis_hr = rt
                .last_put_visibility_hr
                .load(Ordering::Acquire)
                .to_le_bytes();
            let flags = rt.diag_flags.load(Ordering::Acquire).to_le_bytes();
            let mut resp = [0u8; 26];
            resp[0] = CMD_STATUS;
            resp[1] = desired;
            resp[2..6].copy_from_slice(&advise_hr);
            resp[6..10].copy_from_slice(&node_cnt);
            resp[10..14].copy_from_slice(&target_cnt);
            resp[14..18].copy_from_slice(&op_hr);
            resp[18..22].copy_from_slice(&vis_hr);
            resp[22..26].copy_from_slice(&flags);
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &resp, 1000) {
                return PipeAction::Disconnect;
            }
        }
        _ => {
            if !write_pipe_exact_with_stop(pipe, io_event, stop, &[0xFF], 1000) {
                return PipeAction::Disconnect;
            }
        }
    }
    PipeAction::Continue
}

pub(super) fn run_reverse_pipe_client_loop(rt: Arc<TapRuntime>, pipe_name: String) {
    unsafe {
        let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

        let Ok(io_event) = CreateEventW(None, true, false, None) else {
            rt.stop_requested.store(true, Ordering::SeqCst);
            let _ = SetEvent(rt.stop_event_handle());
            if com_ok {
                CoUninitialize();
            }
            return;
        };

        // Wait briefly for AdviseVisualTreeChange to settle before connecting so a failed
        // duplicate TAP instance never steals the single-instance reverse Named Pipe.
        for _ in 0..80 {
            if rt.stop_requested.load(Ordering::SeqCst)
                || rt.advise_hr.load(Ordering::Acquire) != S_FALSE.0
            {
                break;
            }
            let _ = WaitForSingleObject(rt.stop_event_handle(), 15);
        }
        let adv_hr = rt.advise_hr.load(Ordering::Acquire);
        if adv_hr < 0 && (rt.diag_flags.load(Ordering::Acquire) & DIAG_WATCHER_CALLED) == 0 {
            let _ = CloseHandle(io_event);
            if com_ok {
                CoUninitialize();
            }
            return;
        }

        'outer: while !rt.stop_requested.load(Ordering::SeqCst) {
            let pipe = match CreateFileW(
                PCWSTR(wide_name.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS,
                None,
            ) {
                Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE => h,
                _ => {
                    if GetLastError() == ERROR_PIPE_BUSY {
                        let _ = WaitNamedPipeW(PCWSTR(wide_name.as_ptr()), 150);
                    } else {
                        let _ = WaitForSingleObject(rt.stop_event_handle(), 100);
                    }
                    continue;
                }
            };

            loop {
                if rt.stop_requested.load(Ordering::SeqCst) {
                    break;
                }
                let mut req = [0u8; 5];
                if !read_pipe_exact_with_stop(
                    pipe,
                    io_event,
                    Some(rt.stop_event_handle()),
                    &mut req,
                    INFINITE,
                ) {
                    break;
                }
                let cmd = req[0];
                let host_pid = u32::from_le_bytes([req[1], req[2], req[3], req[4]]);
                match handle_pipe_command(&rt, pipe, io_event, cmd, host_pid) {
                    PipeAction::Continue => {}
                    PipeAction::Disconnect => break,
                    PipeAction::Shutdown => {
                        let _ = FlushFileBuffers(pipe);
                        let _ = CloseHandle(pipe);
                        break 'outer;
                    }
                }
            }

            let _ = CloseHandle(pipe);
            if !rt.stop_requested.load(Ordering::SeqCst) {
                rt.on_host_exited();
            }
        }

        let _ = CloseHandle(io_event);
        if com_ok {
            CoUninitialize();
        }
    }
}

pub(super) fn run_pipe_server_loop(rt: Arc<TapRuntime>) {
    unsafe {
        let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let pid = GetCurrentProcessId();
        let wide_name: Vec<u16> = pipe_name_for_pid(pid)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let pipe = CreateNamedPipeW(
            PCWSTR(wide_name.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            512,
            512,
            0,
            None,
        );
        if pipe == INVALID_HANDLE_VALUE || pipe.is_invalid() {
            rt.stop_requested.store(true, Ordering::SeqCst);
            let _ = SetEvent(rt.stop_event_handle());
            if com_ok {
                CoUninitialize();
            }
            return;
        }

        let Ok(conn_event) = CreateEventW(None, true, false, None) else {
            let _ = CloseHandle(pipe);
            rt.stop_requested.store(true, Ordering::SeqCst);
            let _ = SetEvent(rt.stop_event_handle());
            if com_ok {
                CoUninitialize();
            }
            return;
        };
        let Ok(io_event) = CreateEventW(None, true, false, None) else {
            let _ = CloseHandle(conn_event);
            let _ = CloseHandle(pipe);
            rt.stop_requested.store(true, Ordering::SeqCst);
            let _ = SetEvent(rt.stop_event_handle());
            if com_ok {
                CoUninitialize();
            }
            return;
        };

        while !rt.stop_requested.load(Ordering::SeqCst) {
            let _ = ResetEvent(conn_event);
            let mut ov = OVERLAPPED {
                hEvent: conn_event,
                ..Default::default()
            };
            let conn_res = ConnectNamedPipe(pipe, Some(&mut ov));
            let connected = if conn_res.is_ok() {
                true
            } else {
                let err = GetLastError();
                if err == ERROR_PIPE_CONNECTED {
                    true
                } else if err == ERROR_IO_PENDING {
                    let mut wait_handles = [conn_event, rt.stop_event_handle(), HANDLE::default()];
                    let mut count = 2usize;
                    let mut host_h_copy = HANDLE::default();
                    if let Ok(hh) = rt.host_handle_raw.lock() {
                        if let Some((_, raw)) = *hh {
                            if raw != 0 {
                                let h = HANDLE(raw as *mut c_void);
                                host_h_copy = h;
                                wait_handles[2] = h;
                                count = 3;
                            }
                        }
                    }

                    loop {
                        if rt.stop_requested.load(Ordering::SeqCst) {
                            let _ = CancelIo(pipe);
                            let mut tr = 0u32;
                            let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                            break false;
                        }
                        let w = WaitForMultipleObjects(&wait_handles[..count], false, INFINITE);
                        if w == WAIT_OBJECT_0 {
                            let _ = &ov;
                            let mut tr = 0u32;
                            break GetOverlappedResult(pipe, &ov, &mut tr, false).is_ok();
                        } else if w == WAIT_EVENT_INDEX(1) || rt.stop_requested.load(Ordering::SeqCst) {
                            let _ = CancelIo(pipe);
                            let mut tr = 0u32;
                            let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                            break false;
                        } else if (count == 3 && w == WAIT_EVENT_INDEX(2))
                            || (!host_h_copy.is_invalid()
                                && WaitForSingleObject(host_h_copy, 0) == WAIT_OBJECT_0)
                        {
                            rt.on_host_exited();
                            count = 2;
                            host_h_copy = HANDLE::default();
                            wait_handles[2] = HANDLE::default();
                        } else {
                            let _ = CancelIo(pipe);
                            let mut tr = 0u32;
                            let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                            break false;
                        }
                    }
                } else {
                    false
                }
            };

            if !connected {
                if rt.stop_requested.load(Ordering::SeqCst) {
                    break;
                }
                continue;
            }

            let mut req = [0u8; 5];
            if read_pipe_exact_with_stop(
                pipe,
                io_event,
                Some(rt.stop_event_handle()),
                &mut req,
                1000,
            ) {
                let cmd = req[0];
                let host_pid = u32::from_le_bytes([req[1], req[2], req[3], req[4]]);
                match handle_pipe_command(&rt, pipe, io_event, cmd, host_pid) {
                    PipeAction::Continue => {
                        let _ = FlushFileBuffers(pipe);
                    }
                    PipeAction::Shutdown => {
                        let _ = FlushFileBuffers(pipe);
                        let _ = DisconnectNamedPipe(pipe);
                        break;
                    }
                    PipeAction::Disconnect => {}
                }
            }

            let _ = DisconnectNamedPipe(pipe);
        }

        let _ = CloseHandle(io_event);
        let _ = CloseHandle(conn_event);
        let _ = CloseHandle(pipe);
        if com_ok {
            CoUninitialize();
        }
    }
}

#[inline]
#[allow(non_snake_case)]
const fn WAIT_EVENT_INDEX(idx: u32) -> windows::Win32::Foundation::WAIT_EVENT {
    windows::Win32::Foundation::WAIT_EVENT(WAIT_OBJECT_0.0 + idx)
}
