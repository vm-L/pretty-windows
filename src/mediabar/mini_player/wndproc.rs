use super::{MiniPlayerInner, MINI_ACTION_ACTIVATE, MINI_ACTION_NEXT, MINI_ACTION_OPEN_MENU, MINI_ACTION_PLAY_PAUSE, MINI_ACTION_PREV, MINI_ACTION_TOGGLE_MUTE, TIMER_MINI_TICK_ID, WM_APP_MINI_PLAYER_CLOSED, WM_APP_MINI_PLAYER_WHEEL, WM_MOUSELEAVE_MSG};
use super::super::layout::MiniPlayerHitZone;
use super::super::volume::slider_ratio_from_x;
use crate::ui::PopupAnimPhase;
use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, GetWindowLongPtrW, IsWindowVisible, PostMessageW, SetCursor, SetWindowLongPtrW, ShowWindow, GWLP_USERDATA, HTCLIENT, HTTRANSPARENT, SW_HIDE, WA_INACTIVE, WM_ACTIVATE, WM_CAPTURECHANGED, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_NCHITTEST, WM_SETCURSOR, WM_TIMER};

pub(super) unsafe fn begin_close_mini_player(hwnd: HWND, rc: &Rc<RefCell<MiniPlayerInner>>) {
    if !IsWindowVisible(hwnd).as_bool() {
        return;
    }
    let had_capture = if let Ok(mut state) = rc.try_borrow_mut() {
        if state.anim_phase == PopupAnimPhase::Closing || state.suppress_deactivate_close {
            return;
        }
        let had = state.dragging_seek_ratio.take().is_some()
            || state.pressed_zone != MiniPlayerHitZone::None;
        state.pressed_zone = MiniPlayerHitZone::None;
        had
    } else {
        return;
    };
    if had_capture {
        let _ = ReleaseCapture();
    }
    if let Ok(mut state) = rc.try_borrow_mut() {
        if state.anim_phase == PopupAnimPhase::Closing {
            return;
        }
        state.anim_phase = PopupAnimPhase::Closing;
        state.close_progress = 0.0;
        state.close_started_at = Some(Instant::now());
        state.last_flow_tick = Instant::now();
        state.sync_timer_rate();
    }
}

pub(super) unsafe fn finish_close_mini_player(hwnd: HWND, rc: &Rc<RefCell<MiniPlayerInner>>) {
    let _ = ShowWindow(hwnd, SW_HIDE);
    let main_hwnd = if let Ok(mut state) = rc.try_borrow_mut() {
        state.reset_transient_state();
        Some(state.main_hwnd)
    } else {
        None
    };
    if let Some(main_hwnd) = main_hwnd {
        let _ = PostMessageW(main_hwnd, WM_APP_MINI_PLAYER_CLOSED, WPARAM(0), LPARAM(0));
    }
}

pub(super) unsafe extern "system" fn mini_player_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        let raw =
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *const RefCell<MiniPlayerInner>;
        if !raw.is_null() {
            drop(Rc::from_raw(raw));
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let raw_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<MiniPlayerInner>;
    if raw_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    Rc::increment_strong_count(raw_ptr);
    let rc = Rc::from_raw(raw_ptr);

    match msg {
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                begin_close_mini_player(hwnd, &rc);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == VK_ESCAPE.0 as usize {
                begin_close_mini_player(hwnd, &rc);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCHITTEST => {
            let sx = ((lparam.0 & 0xFFFF) as i16) as i32;
            let sy = (((lparam.0 >> 16) & 0xFFFF) as i16) as i32;
            let mut pt = POINT { x: sx, y: sy };
            let _ = ScreenToClient(hwnd, &mut pt);
            let inside = rc.try_borrow().is_ok_and(|state| {
                if state.anim_phase == PopupAnimPhase::Closing {
                    false
                } else {
                    let layout = state.current_layout();
                    layout.card_rect.contains(pt.x as f32, pt.y as f32)
                }
            });
            if inside {
                LRESULT(HTCLIENT as isize)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                if let Ok(state) = rc.try_borrow() {
                    SetCursor(state.arrow_cursor);
                    return LRESULT(1);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_MINI_TICK_ID {
                let finished = rc
                    .try_borrow_mut()
                    .is_ok_and(|mut state| state.tick());
                if finished {
                    finish_close_mini_player(hwnd, &rc);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let wheel_delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
            if wheel_delta != 0 {
                if let Ok(state) = rc.try_borrow() {
                    if state.anim_phase != PopupAnimPhase::Closing {
                        let _ = PostMessageW(
                            state.main_hwnd,
                            WM_APP_MINI_PLAYER_WHEEL,
                            WPARAM(wheel_delta as isize as usize),
                            LPARAM(0),
                        );
                    }
                }
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if state.dragging_seek_ratio.is_some()
                    || state.pressed_zone != MiniPlayerHitZone::None
                {
                    state.dragging_seek_ratio = None;
                    state.pressed_zone = MiniPlayerHitZone::None;
                    let _ = state.render();
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

                if state.dragging_seek_ratio.is_some() {
                    let r = slider_ratio_from_x(
                        layout.progress_track_rect.left,
                        layout.progress_track_rect.right,
                        x,
                    );
                    state.dragging_seek_ratio = Some(r);
                    let _ = state.render();
                    return LRESULT(0);
                }

                let next_zone = layout.hit_test(x, y, state.can_seek());
                if state.hover_zone != next_zone {
                    state.hover_zone = next_zone;
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.tracking_leave = false;
                if state.dragging_seek_ratio.is_none()
                    && state.hover_zone != MiniPlayerHitZone::None
                {
                    state.hover_zone = MiniPlayerHitZone::None;
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let need_capture = if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                let x = ((lparam.0 & 0xFFFF) as i16) as f32;
                let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
                let layout = state.current_layout();
                let zone = layout.hit_test(x, y, state.can_seek());
                state.pressed_zone = zone;
                if zone == MiniPlayerHitZone::ProgressSlider {
                    let r = slider_ratio_from_x(
                        layout.progress_track_rect.left,
                        layout.progress_track_rect.right,
                        x,
                    );
                    state.dragging_seek_ratio = Some(r);
                    true
                } else {
                    zone != MiniPlayerHitZone::None
                }
            } else {
                false
            };
            if need_capture {
                let _ = SetCapture(hwnd);
            }
            if let Ok(mut state) = rc.try_borrow_mut() {
                let _ = state.render();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let (need_release, seek_ratio, action, need_close) =
                if let Ok(mut state) = rc.try_borrow_mut() {
                    if state.anim_phase == PopupAnimPhase::Closing {
                        return LRESULT(0);
                    }
                    let layout = state.current_layout();
                    if state.dragging_seek_ratio.is_some() {
                        let final_r = slider_ratio_from_x(
                            layout.progress_track_rect.left,
                            layout.progress_track_rect.right,
                            x,
                        );
                        state.dragging_seek_ratio = None;
                        state.pressed_zone = MiniPlayerHitZone::None;
                        state.hover_zone = layout.hit_test(x, y, state.can_seek());
                        (true, Some(final_r), None, false)
                    } else {
                        let pressed = state.pressed_zone;
                        let had_capture = pressed != MiniPlayerHitZone::None;
                        state.pressed_zone = MiniPlayerHitZone::None;
                        let zone = layout.hit_test(x, y, state.can_seek());
                        state.hover_zone = zone;
                        let (action, close) = if zone == pressed {
                            match zone {
                                MiniPlayerHitZone::PrevButton => (Some(MINI_ACTION_PREV), false),
                                MiniPlayerHitZone::PlayPauseButton => {
                                    (Some(MINI_ACTION_PLAY_PAUSE), false)
                                }
                                MiniPlayerHitZone::NextButton => (Some(MINI_ACTION_NEXT), false),
                                MiniPlayerHitZone::MuteButton => {
                                    (Some(MINI_ACTION_TOGGLE_MUTE), false)
                                }
                                MiniPlayerHitZone::CoverOrInfo => {
                                    (Some(MINI_ACTION_ACTIVATE), true)
                                }
                                MiniPlayerHitZone::MoreMenuButton => {
                                    (Some(MINI_ACTION_OPEN_MENU), false)
                                }
                                MiniPlayerHitZone::ProgressSlider | MiniPlayerHitZone::None => {
                                    (None, false)
                                }
                            }
                        } else {
                            (None, false)
                        };
                        (had_capture, None, action, close)
                    }
                } else {
                    return LRESULT(0);
                };

            if need_release {
                let _ = ReleaseCapture();
            }
            if let Ok(mut state) = rc.try_borrow_mut() {
                if let Some(final_r) = seek_ratio {
                    state.commit_seek_ratio(final_r);
                }
                if let Some(act) = action {
                    state.post_action(act);
                }
                let _ = state.render();
            }
            if need_close {
                begin_close_mini_player(hwnd, &rc);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
