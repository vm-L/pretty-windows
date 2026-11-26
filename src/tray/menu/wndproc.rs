use super::{
    TrayMenuInner, SHELL_DEACTIVATE_GRACE_MS, TIMER_MENU_FLOW_ID, WM_APP_TRAY_DESKBOX_ACTION,
    WM_APP_TRAY_EXIT, WM_APP_TRAY_MENU_CLOSED, WM_APP_TRAY_SET_TASKBAR_STYLE,
    WM_APP_TRAY_TOGGLE_AUTOSTART, WM_APP_TRAY_TOGGLE_MEDIABAR, WM_APP_TRAY_TOGGLE_MONITORBAR,
    WM_MOUSELEAVE_MSG,
};
use super::super::layout::{TrayMenuHover, TrayMenuItem, TraySubmenuKind};
use crate::system::is_shell_or_flyout_window;
use crate::ui::PopupAnimPhase;
use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, GetWindowLongPtrW, IsWindowVisible, PostMessageW, SetCursor, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, GWLP_USERDATA, HTTRANSPARENT, SW_HIDE, WA_INACTIVE, WM_ACTIVATE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST, WM_SETCURSOR, WM_TIMER};

pub(super) unsafe fn finish_close_tray_menu(hwnd: HWND, rc: &Rc<RefCell<TrayMenuInner>>) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    let host_hwnd = if let Ok(mut state) = rc.try_borrow_mut() {
        state.reset_transient_state();
        Some(state.host_hwnd)
    } else {
        None
    };
    if let Some(host_hwnd) = host_hwnd {
        unsafe {
            let _ = PostMessageW(host_hwnd, WM_APP_TRAY_MENU_CLOSED, WPARAM(0), LPARAM(0));
        }
    }
}

pub(super) unsafe extern "system" fn tray_menu_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        let raw = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *const RefCell<TrayMenuInner>;
        if !raw.is_null() {
            drop(Rc::from_raw(raw));
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let raw_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<TrayMenuInner>;
    if raw_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    Rc::increment_strong_count(raw_ptr);
    let rc = Rc::from_raw(raw_ptr);

    match msg {
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                let other_hwnd = HWND(lparam.0 as *mut core::ffi::c_void);
                let reclaim_focus = if let Ok(mut state) = rc.try_borrow_mut() {
                    let just_shown = state.shown_at.is_some_and(|t| {
                        t.elapsed() < Duration::from_millis(SHELL_DEACTIVATE_GRACE_MS)
                    });
                    if just_shown
                        && !state.shell_focus_reclaimed
                        && (other_hwnd.is_invalid() || is_shell_or_flyout_window(other_hwnd))
                    {
                        state.shell_focus_reclaimed = true;
                        true
                    } else {
                        state.begin_close_from_deactivate();
                        false
                    }
                } else {
                    false
                };
                if reclaim_focus {
                    let _ = SetForegroundWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.begin_close();
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_MENU_FLOW_ID && IsWindowVisible(hwnd).as_bool() {
                let finished = if let Ok(mut state) = rc.try_borrow_mut() {
                    let now = Instant::now();
                    let raw_dt = now.duration_since(state.last_flow_tick).as_secs_f32();
                    state.last_flow_tick = now;
                    let dt_flow = raw_dt.clamp(0.001, 0.1);
                    let dt_anim = raw_dt.clamp(0.001, 0.028);
                    state.flow_phase += f64::from(dt_flow);
                    if state.advance_animations(dt_anim) {
                        true
                    } else {
                        state.sync_timer_rate();
                        let _ = state.render();
                        false
                    }
                } else {
                    false
                };
                if finished {
                    finish_close_tray_menu(hwnd, &rc);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                if !state.tracking_leave {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    if TrackMouseEvent(&mut tme).is_ok() {
                        state.tracking_leave = true;
                    }
                }
                let x = ((lparam.0 & 0xFFFF) as i16) as f32;
                let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
                let layout = state.current_layout();
                let next_hover = layout.hit_test(x, y, state.active_submenu);
                let sub_changed = match next_hover {
                    Some(TrayMenuHover::Primary(TrayMenuItem::DeskBox)) => {
                        state.open_submenu(TraySubmenuKind::DeskBox)
                    }
                    Some(TrayMenuHover::Primary(TrayMenuItem::TaskbarStyle)) => {
                        state.open_submenu(TraySubmenuKind::TaskbarStyle)
                    }
                    Some(TrayMenuHover::Primary(_)) => state.close_submenu(),
                    Some(TrayMenuHover::SubDeskBox(_))
                    | Some(TrayMenuHover::SubTaskbarStyle(_))
                    | None => false,
                };
                if next_hover != state.hover || sub_changed {
                    state.hover = next_hover;
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.tracking_leave = false;
                if state.anim_phase != PopupAnimPhase::Closing && state.hover.is_some() {
                    state.hover = None;
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                let x = ((lparam.0 & 0xFFFF) as i16) as f32;
                let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
                let layout = state.current_layout();
                if !layout.visible_contains(x, y, state.active_submenu) {
                    state.begin_close();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let exit_host_hwnd = if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                let x = ((lparam.0 & 0xFFFF) as i16) as f32;
                let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
                let layout = state.current_layout();
                match layout.hit_test(x, y, state.active_submenu) {
                    Some(TrayMenuHover::Primary(TrayMenuItem::MediaBar)) => {
                        state.mediabar_visible = !state.mediabar_visible;
                        let _ = PostMessageW(
                            state.host_hwnd,
                            WM_APP_TRAY_TOGGLE_MEDIABAR,
                            WPARAM(0),
                            LPARAM(0),
                        );
                        state.begin_close();
                        None
                    }
                    Some(TrayMenuHover::Primary(TrayMenuItem::MonitorBar)) => {
                        state.monitorbar_visible = !state.monitorbar_visible;
                        let _ = PostMessageW(
                            state.host_hwnd,
                            WM_APP_TRAY_TOGGLE_MONITORBAR,
                            WPARAM(0),
                            LPARAM(0),
                        );
                        state.begin_close();
                        None
                    }
                    Some(TrayMenuHover::Primary(TrayMenuItem::DeskBox)) => {
                        if state.open_submenu(TraySubmenuKind::DeskBox) {
                            let _ = state.render();
                        }
                        None
                    }
                    Some(TrayMenuHover::Primary(TrayMenuItem::TaskbarStyle)) => {
                        if state.open_submenu(TraySubmenuKind::TaskbarStyle) {
                            let _ = state.render();
                        }
                        None
                    }
                    Some(TrayMenuHover::SubDeskBox(action)) => {
                        let _ = PostMessageW(
                            state.host_hwnd,
                            WM_APP_TRAY_DESKBOX_ACTION,
                            WPARAM(action.to_u8() as usize),
                            LPARAM(0),
                        );
                        state.begin_close();
                        None
                    }
                    Some(TrayMenuHover::SubTaskbarStyle(style)) => {
                        state.taskbar_style = style;
                        state.sync_timer_rate();
                        let _ = PostMessageW(
                            state.host_hwnd,
                            WM_APP_TRAY_SET_TASKBAR_STYLE,
                            WPARAM(style.to_u8() as usize),
                            LPARAM(0),
                        );
                        state.begin_close();
                        None
                    }
                    Some(TrayMenuHover::Primary(TrayMenuItem::Autostart)) => {
                        state.autostart = !state.autostart;
                        let _ = PostMessageW(
                            state.host_hwnd,
                            WM_APP_TRAY_TOGGLE_AUTOSTART,
                            WPARAM(0),
                            LPARAM(0),
                        );
                        state.begin_close();
                        None
                    }
                    Some(TrayMenuHover::Primary(TrayMenuItem::Exit)) => Some(state.host_hwnd),
                    None => {
                        if !layout.visible_contains(x, y, state.active_submenu) {
                            state.begin_close();
                        }
                        None
                    }
                }
            } else {
                None
            };
            if let Some(host_hwnd) = exit_host_hwnd {
                let _ = ShowWindow(hwnd, SW_HIDE);
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.reset_transient_state();
                }
                let _ = PostMessageW(host_hwnd, WM_APP_TRAY_EXIT, WPARAM(0), LPARAM(0));
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == windows::Win32::UI::WindowsAndMessaging::HTCLIENT {
                if let Ok(state) = rc.try_borrow() {
                    SetCursor(state.arrow_cursor);
                    return LRESULT(1);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCHITTEST => {
            let mut pt = POINT {
                x: i32::from((lparam.0 & 0xFFFF) as i16),
                y: i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let decision = if let Ok(state) = rc.try_borrow() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    Some(false)
                } else {
                    let layout = state.current_layout();
                    Some(layout.visible_contains(pt.x as f32, pt.y as f32, state.active_submenu))
                }
            } else {
                None
            };
            match decision {
                Some(false) => LRESULT(HTTRANSPARENT as isize),
                Some(true) | None => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
