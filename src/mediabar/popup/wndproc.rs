use super::{HoverTarget, PopupInner, SliderTarget, SETTING_CLOSE, SETTING_RESET_POS, SETTING_TOGGLE_AUTO_COLOR, SETTING_TOGGLE_LOCK, TIMER_POPUP_FLOW_ID, WM_MOUSELEAVE_MSG, WM_APP_AUDIO_OUTPUT_CHANGED, WM_APP_POPUP_CLOSED, WM_APP_SETTING_ACTION, WM_APP_SOURCE_SELECTED};
use super::super::arbitration::SelectionMode;
use super::super::audio_devices::{enumerate_audio_endpoints, set_default_audio_endpoint};
use crate::ui::PopupAnimPhase;
use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, GetWindowLongPtrW, IsWindowVisible, PostMessageW, SetCursor, SetWindowLongPtrW, ShowWindow, GWLP_USERDATA, HTCLIENT, HTTRANSPARENT, SW_HIDE, WA_INACTIVE, WM_ACTIVATE, WM_CAPTURECHANGED, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_NCHITTEST, WM_SETCURSOR, WM_TIMER};

pub(super) unsafe fn begin_close_source_popup(hwnd: HWND, rc: &Rc<RefCell<PopupInner>>) {
    if !IsWindowVisible(hwnd).as_bool() {
        return;
    }
    let had_capture = if let Ok(mut state) = rc.try_borrow_mut() {
        if state.anim_phase == PopupAnimPhase::Closing {
            return;
        }
        state.dragging_slider.take().is_some()
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
        state.hover_target = HoverTarget::None;
        state.anim_phase = PopupAnimPhase::Closing;
        state.close_progress = 0.0;
        state.close_started_at = Some(Instant::now());
        state.last_flow_tick = Instant::now();
        state.sync_timer_rate();
        let _ = state.render();
    }
}

pub(super) unsafe fn finish_close_source_popup(hwnd: HWND, rc: &Rc<RefCell<PopupInner>>) {
    let _ = ShowWindow(hwnd, SW_HIDE);
    let main_hwnd = if let Ok(mut state) = rc.try_borrow_mut() {
        state.reset_transient_state();
        state.close_started_at = Some(Instant::now());
        Some(state.main_hwnd)
    } else {
        None
    };
    if let Some(main_hwnd) = main_hwnd {
        let _ = PostMessageW(main_hwnd, WM_APP_POPUP_CLOSED, WPARAM(0), LPARAM(0));
    }
}

pub(super) unsafe extern "system" fn popup_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        let raw = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *const RefCell<PopupInner>;
        if !raw.is_null() {
            drop(Rc::from_raw(raw));
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let raw_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<PopupInner>;
    if raw_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    Rc::increment_strong_count(raw_ptr);
    let rc = Rc::from_raw(raw_ptr);

    match msg {
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                begin_close_source_popup(hwnd, &rc);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                begin_close_source_popup(hwnd, &rc);
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                let had_drag = state.dragging_slider.take().is_some();
                if had_drag && state.pending_session_reload {
                    state.pending_session_reload = false;
                    state.reload_app_sessions_only();
                }
                if had_drag
                    && IsWindowVisible(hwnd).as_bool()
                    && state.anim_phase != PopupAnimPhase::Closing
                {
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_POPUP_FLOW_ID && IsWindowVisible(hwnd).as_bool() {
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
                    finish_close_source_popup(hwnd, &rc);
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
                if let Some(target) = state.dragging_slider {
                    if state.apply_slider_drag(target, x) {
                        let _ = state.render();
                    }
                    return LRESULT(0);
                }
                if state.update_hover_at(x, y) {
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                state.tracking_leave = false;
                if state.anim_phase != PopupAnimPhase::Closing
                    && state.dragging_slider.is_none()
                    && state.hover_target != HoverTarget::None
                {
                    state.hover_target = HoverTarget::None;
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let (outside, drag_target) = if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                if !state.point_inside_cards(x, y) {
                    (true, None)
                } else {
                    let _ = state.update_hover_at(x, y);
                    let target = match state.hover_target {
                        HoverTarget::SubVolMasterSlider => {
                            state.dragging_slider = Some(SliderTarget::Master);
                            Some(SliderTarget::Master)
                        }
                        HoverTarget::SubVolAppSlider(idx) => {
                            state.dragging_slider = Some(SliderTarget::App(idx));
                            Some(SliderTarget::App(idx))
                        }
                        _ => None,
                    };
                    (false, target)
                }
            } else {
                return LRESULT(0);
            };
            if outside {
                begin_close_source_popup(hwnd, &rc);
                return LRESULT(0);
            }
            if let Some(target) = drag_target {
                let _ = SetCapture(hwnd);
                if let Ok(mut state) = rc.try_borrow_mut() {
                    let _ = state.apply_slider_drag(target, x);
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;

            enum PostUpAction {
                None,
                EndDrag,
                BeginClose,
                HideCloseApp(HWND),
            }

            let action = if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                if state.dragging_slider.take().is_some() {
                    PostUpAction::EndDrag
                } else {
                    let _ = state.update_hover_at(x, y);
                    match state.hover_target {
                        HoverTarget::SubAuto => {
                            state.mode = SelectionMode::Auto;
                            state.pending_selected_source_id = None;
                            let _ = PostMessageW(
                                state.main_hwnd,
                                WM_APP_SOURCE_SELECTED,
                                WPARAM(usize::MAX),
                                LPARAM(0),
                            );
                            PostUpAction::BeginClose
                        }
                        HoverTarget::SubSource(idx) => {
                            if let Some(src) = state.sources.get(idx) {
                                let src_id = src.id.clone();
                                state.mode = SelectionMode::Manual(src_id.clone());
                                state.active_id = Some(src_id.clone());
                                state.pending_selected_source_id = Some(src_id);
                                let _ = PostMessageW(
                                    state.main_hwnd,
                                    WM_APP_SOURCE_SELECTED,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                                PostUpAction::BeginClose
                            } else {
                                PostUpAction::None
                            }
                        }
                        HoverTarget::SubVolMasterMute => {
                            state.toggle_master_mute();
                            let _ = state.render();
                            PostUpAction::None
                        }
                        HoverTarget::SubVolAppMute(idx) => {
                            if let Some(app) = state.app_volumes.get_mut(idx) {
                                app.toggle_group_mute();
                                let _ = state.render();
                            }
                            PostUpAction::None
                        }
                        HoverTarget::SubPlayback(idx) => {
                            if let Some(dev_id) = state.playback_devices.get(idx).map(|d| d.id.clone())
                            {
                                if set_default_audio_endpoint(&dev_id) {
                                    state.playback_devices = enumerate_audio_endpoints(false);
                                    state.reload_volumes();
                                    let _ = PostMessageW(
                                        state.main_hwnd,
                                        WM_APP_AUDIO_OUTPUT_CHANGED,
                                        WPARAM(0),
                                        LPARAM(0),
                                    );
                                    PostUpAction::BeginClose
                                } else {
                                    PostUpAction::None
                                }
                            } else {
                                PostUpAction::None
                            }
                        }
                        HoverTarget::SubRecording(idx) => {
                            if let Some(dev_id) =
                                state.recording_devices.get(idx).map(|d| d.id.clone())
                            {
                                if set_default_audio_endpoint(&dev_id) {
                                    state.recording_devices = enumerate_audio_endpoints(true);
                                    PostUpAction::BeginClose
                                } else {
                                    PostUpAction::None
                                }
                            } else {
                                PostUpAction::None
                            }
                        }
                        HoverTarget::SubSettingAutoColor => {
                            state.auto_color = !state.auto_color;
                            state.halo_colors_ready = false;
                            state.last_silk_tick = None;
                            let _ = PostMessageW(
                                state.main_hwnd,
                                WM_APP_SETTING_ACTION,
                                WPARAM(SETTING_TOGGLE_AUTO_COLOR),
                                LPARAM(0),
                            );
                            let _ = state.render();
                            PostUpAction::None
                        }
                        HoverTarget::SubSettingLock => {
                            state.locked = !state.locked;
                            let _ = PostMessageW(
                                state.main_hwnd,
                                WM_APP_SETTING_ACTION,
                                WPARAM(SETTING_TOGGLE_LOCK),
                                LPARAM(0),
                            );
                            PostUpAction::BeginClose
                        }
                        HoverTarget::SubSettingResetPos => {
                            let _ = PostMessageW(
                                state.main_hwnd,
                                WM_APP_SETTING_ACTION,
                                WPARAM(SETTING_RESET_POS),
                                LPARAM(0),
                            );
                            PostUpAction::BeginClose
                        }
                        HoverTarget::SubSettingClose => PostUpAction::HideCloseApp(state.main_hwnd),
                        HoverTarget::Primary(_)
                        | HoverTarget::SubVolMasterSlider
                        | HoverTarget::SubVolMasterRow
                        | HoverTarget::SubVolAppSlider(_)
                        | HoverTarget::SubVolAppRow(_)
                        | HoverTarget::None => PostUpAction::None,
                    }
                }
            } else {
                PostUpAction::None
            };

            match action {
                PostUpAction::None => {}
                PostUpAction::EndDrag => {
                    let _ = ReleaseCapture();
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        if state.pending_session_reload {
                            state.pending_session_reload = false;
                            state.reload_app_sessions_only();
                        }
                        let _ = state.update_hover_at(x, y);
                        let _ = state.render();
                    }
                }
                PostUpAction::BeginClose => {
                    begin_close_source_popup(hwnd, &rc);
                }
                PostUpAction::HideCloseApp(main_hwnd) => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    if let Ok(mut state) = rc.try_borrow_mut() {
                        state.reset_transient_state();
                    }
                    let _ = PostMessageW(
                        main_hwnd,
                        WM_APP_SETTING_ACTION,
                        WPARAM(SETTING_CLOSE),
                        LPARAM(0),
                    );
                }
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                let wheel_delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
                if wheel_delta == 0 {
                    return LRESULT(0);
                }
                let mut pt = POINT {
                    x: i32::from((lparam.0 & 0xFFFF) as i16),
                    y: i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
                };
                let _ = ScreenToClient(hwnd, &mut pt);
                let _ = state.update_hover_at(pt.x as f32, pt.y as f32);

                let handled = match state.hover_target {
                    HoverTarget::SubVolMasterMute
                    | HoverTarget::SubVolMasterSlider
                    | HoverTarget::SubVolMasterRow => {
                        state.step_master_volume_wheel(wheel_delta);
                        true
                    }
                    HoverTarget::SubVolAppMute(idx)
                    | HoverTarget::SubVolAppSlider(idx)
                    | HoverTarget::SubVolAppRow(idx) => {
                        if let Some(app) = state.app_volumes.get_mut(idx) {
                            app.step_group_wheel(wheel_delta);
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if handled {
                    let _ = state.render();
                }
            }
            LRESULT(0)
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
        WM_NCHITTEST => {
            let closing = rc
                .try_borrow()
                .is_ok_and(|state| state.anim_phase == PopupAnimPhase::Closing);
            if closing {
                LRESULT(HTTRANSPARENT as isize)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

