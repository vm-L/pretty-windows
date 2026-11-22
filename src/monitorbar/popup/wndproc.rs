use super::{HoverTarget, MonitorPopupInner, SubHoverTarget, SubmenuKind, MAX_BRIGHTNESS_ROWS, MAX_LIST_ROWS, TIMER_POPUP_FLOW_ID, WM_MOUSELEAVE_MSG};
use super::super::{WM_APP_MONITOR_CLOSE_SELF, WM_APP_MONITOR_CONFIG_CHANGED, WM_APP_MONITOR_PICK_CUSTOM_APP, WM_APP_MONITOR_POPUP_CLOSED, WM_APP_MONITOR_RESET_POS, WM_APP_MONITOR_TOGGLE_LOCK};
use super::super::actions::open_shell_target;
use super::super::config::{save_config, LeftClickAction};
use super::super::wireless::{BtDeviceKind, WifiNetworkItem};
use crate::ui::{accumulate_wheel_scroll, PopupAnimPhase};
use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, GetWindowLongPtrW, IsWindowVisible, KillTimer, PostMessageW, SetCursor, SetWindowLongPtrW, ShowWindow, GWLP_USERDATA, HTTRANSPARENT, SW_HIDE, WA_INACTIVE, WM_ACTIVATE, WM_CAPTURECHANGED, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_NCHITTEST, WM_SETCURSOR, WM_TIMER};

enum SubClickOutcome {
    None,
    BeginClose,
    OpenSettingsAndClose(&'static str),
    HideImmediatelyAndPostOwner(u32),
}

pub(super) unsafe fn begin_close_monitor_popup(hwnd: HWND, rc: &Rc<RefCell<MonitorPopupInner>>) {
    if !IsWindowVisible(hwnd).as_bool() {
        return;
    }
    let had_capture = if let Ok(mut state) = rc.try_borrow_mut() {
        if state.anim_phase == PopupAnimPhase::Closing {
            return;
        }
        state.dragging_brightness.take().is_some()
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
        state.hover = None;
        state.last_flow_tick = Instant::now();
        state.sync_timer_rate();
        let _ = state.render();
    }
}

pub(super) unsafe fn finish_close_monitor_popup(hwnd: HWND, rc: &Rc<RefCell<MonitorPopupInner>>) {
    let _ = KillTimer(hwnd, TIMER_POPUP_FLOW_ID);
    let _ = ShowWindow(hwnd, SW_HIDE);
    let owner_hwnd = if let Ok(mut state) = rc.try_borrow_mut() {
        state.reset_transient_state();
        Some(state.owner_hwnd)
    } else {
        None
    };
    if let Some(owner_hwnd) = owner_hwnd {
        let _ = PostMessageW(owner_hwnd, WM_APP_MONITOR_POPUP_CLOSED, WPARAM(0), LPARAM(0));
    }
}

pub(super) unsafe fn hide_immediately_monitor_popup(
    hwnd: HWND,
    rc: &Rc<RefCell<MonitorPopupInner>>,
) -> Option<HWND> {
    let had_capture = rc
        .try_borrow_mut()
        .is_ok_and(|mut state| state.dragging_brightness.take().is_some());
    if had_capture {
        let _ = ReleaseCapture();
    }
    let _ = KillTimer(hwnd, TIMER_POPUP_FLOW_ID);
    let _ = ShowWindow(hwnd, SW_HIDE);
    let owner_hwnd = if let Ok(mut state) = rc.try_borrow_mut() {
        state.reset_transient_state();
        Some(state.owner_hwnd)
    } else {
        None
    };
    if let Some(owner_hwnd) = owner_hwnd {
        let _ = PostMessageW(owner_hwnd, WM_APP_MONITOR_POPUP_CLOSED, WPARAM(0), LPARAM(0));
    }
    owner_hwnd
}

pub(super) unsafe extern "system" fn monitor_popup_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        let raw =
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *const RefCell<MonitorPopupInner>;
        if !raw.is_null() {
            drop(Rc::from_raw(raw));
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let raw_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<MonitorPopupInner>;
    if raw_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    Rc::increment_strong_count(raw_ptr);
    let rc = Rc::from_raw(raw_ptr);

    match msg {
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                begin_close_monitor_popup(hwnd, &rc);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                begin_close_monitor_popup(hwnd, &rc);
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if let Ok(state) = rc.try_borrow() {
                let cursor = state.arrow_cursor;
                drop(state);
                let _ = SetCursor(cursor);
            }
            LRESULT(1)
        }
        WM_NCHITTEST => {
            let mut pt = POINT {
                x: i32::from((lparam.0 & 0xFFFF) as i16),
                y: i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let inside = if let Ok(state) = rc.try_borrow() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    false
                } else {
                    state.point_inside_cards(pt.x as f32, pt.y as f32)
                }
            } else {
                false
            };
            if inside {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
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
                    finish_close_monitor_popup(hwnd, &rc);
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
                if let Some(drag_id) = state.dragging_brightness.clone() {
                    if state.apply_brightness_drag(&drag_id, x) {
                        let _ = state.render();
                    }
                    return LRESULT(0);
                }
                let next_hover = state.hit_test(x, y);
                let sub_changed = match next_hover {
                    Some(HoverTarget::Primary(item)) => state.set_active_submenu(item.submenu()),
                    Some(HoverTarget::Sub(_)) | None => false,
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
                if state.anim_phase != PopupAnimPhase::Closing
                    && state.dragging_brightness.is_none()
                    && state.hover.is_some()
                {
                    state.hover = None;
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if state.dragging_brightness.take().is_some() {
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                let delta = i32::from(((wparam.0 >> 16) & 0xFFFF) as i16);
                if state.active_sub == Some(SubmenuKind::Brightness) && delta != 0 {
                    let mut pt = POINT {
                        x: i32::from((lparam.0 & 0xFFFF) as i16),
                        y: i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
                    };
                    let _ = ScreenToClient(hwnd, &mut pt);
                    let wheel_hover = state.hit_test(pt.x as f32, pt.y as f32);
                    if let Some(HoverTarget::Sub(
                        SubHoverTarget::BrightnessRow(idx) | SubHoverTarget::BrightnessSlider(idx),
                    )) = wheel_hover
                    {
                        if state.step_brightness_wheel(idx, delta as i16) {
                            let _ = state.render();
                        }
                        return LRESULT(0);
                    }
                }
                let steps = accumulate_wheel_scroll(&mut state.wheel_accum, delta);
                if steps != 0 {
                    let max_wifi = state
                        .wireless_snap
                        .wifi_networks
                        .len()
                        .saturating_sub(MAX_LIST_ROWS);
                    let max_bt = state
                        .wireless_snap
                        .bt_devices
                        .len()
                        .saturating_sub(MAX_LIST_ROWS);
                    let max_bright = state
                        .brightness_snap
                        .displays
                        .len()
                        .saturating_sub(MAX_BRIGHTNESS_ROWS);
                    let mut changed = false;
                    match state.active_sub {
                        Some(SubmenuKind::WiFi) if max_wifi > 0 => {
                            let next = (state.wifi_scroll as isize + steps as isize)
                                .clamp(0, max_wifi as isize) as usize;
                            if next != state.wifi_scroll {
                                state.wifi_scroll = next;
                                changed = true;
                            }
                        }
                        Some(SubmenuKind::Bluetooth) if max_bt > 0 => {
                            let next = (state.bt_scroll as isize + steps as isize)
                                .clamp(0, max_bt as isize) as usize;
                            if next != state.bt_scroll {
                                state.bt_scroll = next;
                                changed = true;
                            }
                        }
                        Some(SubmenuKind::Brightness) if max_bright > 0 => {
                            let next = (state.brightness_scroll as isize + steps as isize)
                                .clamp(0, max_bright as isize) as usize;
                            if next != state.brightness_scroll {
                                state.brightness_scroll = next;
                                changed = true;
                            }
                        }
                        _ => {}
                    }
                    if changed {
                        let _ = state.render();
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let (outside, drag_idx) = if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                if !state.point_inside_cards(x, y) {
                    (true, None)
                } else {
                    let next_hover = state.hit_test(x, y);
                    state.hover = next_hover;
                    let drag_id_opt = match next_hover {
                        Some(HoverTarget::Sub(SubHoverTarget::BrightnessSlider(idx))) => state
                            .brightness_snap
                            .displays
                            .get(idx)
                            .map(|d| d.id.clone()),
                        _ => None,
                    };
                    state.dragging_brightness = drag_id_opt.clone();
                    (false, drag_id_opt)
                }
            } else {
                return LRESULT(0);
            };
            if outside {
                begin_close_monitor_popup(hwnd, &rc);
                return LRESULT(0);
            }
            if let Some(drag_id) = drag_idx {
                let _ = SetCapture(hwnd);
                if let Ok(mut state) = rc.try_borrow_mut() {
                    let _ = state.apply_brightness_drag(&drag_id, x);
                    let _ = state.render();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = ((lparam.0 & 0xFFFF) as i16) as f32;
            let y = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
            let (ended_drag, outcome) = if let Ok(mut state) = rc.try_borrow_mut() {
                if state.anim_phase == PopupAnimPhase::Closing {
                    return LRESULT(0);
                }
                if state.dragging_brightness.take().is_some() {
                    (true, SubClickOutcome::None)
                } else {
                    let out = match state.hit_test(x, y) {
                        Some(HoverTarget::Primary(item)) => {
                            if state.set_active_submenu(item.submenu()) {
                                let _ = state.render();
                            }
                            SubClickOutcome::None
                        }
                        Some(HoverTarget::Sub(target)) => handle_sub_click(&mut state, target),
                        None => {
                            if !state.point_inside_cards(x, y) {
                                SubClickOutcome::BeginClose
                            } else {
                                SubClickOutcome::None
                            }
                        }
                    };
                    (false, out)
                }
            } else {
                return LRESULT(0);
            };

            if ended_drag {
                let _ = ReleaseCapture();
                if let Ok(mut state) = rc.try_borrow_mut() {
                    state.hover = state.hit_test(x, y);
                    let _ = state.render();
                }
                return LRESULT(0);
            }

            match outcome {
                SubClickOutcome::None => {}
                SubClickOutcome::BeginClose => {
                    begin_close_monitor_popup(hwnd, &rc);
                }
                SubClickOutcome::OpenSettingsAndClose(uri) => {
                    open_shell_target(uri);
                    begin_close_monitor_popup(hwnd, &rc);
                }
                SubClickOutcome::HideImmediatelyAndPostOwner(msg_id) => {
                    if let Some(owner_hwnd) = hide_immediately_monitor_popup(hwnd, &rc) {
                        let _ = PostMessageW(owner_hwnd, msg_id, WPARAM(0), LPARAM(0));
                    }
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn handle_sub_click(state: &mut MonitorPopupInner, target: SubHoverTarget) -> SubClickOutcome {
    match target {
        SubHoverTarget::WifiToggle => {
            state.wireless.toggle_wifi_radio();
            SubClickOutcome::None
        }
        SubHoverTarget::WifiNetwork(idx) => {
            let item_opt: Option<WifiNetworkItem> =
                state.wireless_snap.wifi_networks.get(idx).cloned();
            if let Some(item) = item_opt {
                let will_open_settings = !item.has_profile;
                state.wireless.activate_wifi_network(item);
                if will_open_settings {
                    return SubClickOutcome::BeginClose;
                }
            }
            SubClickOutcome::None
        }
        SubHoverTarget::WifiMoreSettings => {
            SubClickOutcome::OpenSettingsAndClose("ms-settings:network-wifi")
        }
        SubHoverTarget::BtToggle => {
            state.wireless.toggle_bluetooth_radio();
            SubClickOutcome::None
        }
        SubHoverTarget::BtDevice(idx) => {
            if let Some(dev) = state.wireless_snap.bt_devices.get(idx).cloned() {
                let will_open_settings =
                    dev.kind != BtDeviceKind::Classic || dev.address == 0 || dev.is_connected;
                if will_open_settings {
                    return SubClickOutcome::OpenSettingsAndClose("ms-settings:bluetooth");
                }
                state.wireless.activate_bluetooth_device(dev);
            }
            SubClickOutcome::None
        }
        SubHoverTarget::BtMoreSettings => {
            SubClickOutcome::OpenSettingsAndClose("ms-settings:bluetooth")
        }
        SubHoverTarget::BrightnessRow(_) | SubHoverTarget::BrightnessSlider(_) => {
            SubClickOutcome::None
        }
        SubHoverTarget::BrightnessMoreSettings => {
            SubClickOutcome::OpenSettingsAndClose("ms-settings:display")
        }
        SubHoverTarget::LeftClickPreset(action) => {
            if action == LeftClickAction::CustomApp {
                SubClickOutcome::HideImmediatelyAndPostOwner(WM_APP_MONITOR_PICK_CUSTOM_APP)
            } else {
                if let Ok(mut guard) = state.cfg.lock() {
                    guard.left_click = action;
                    save_config(&state.config_path, &guard);
                }
                unsafe {
                    let _ = PostMessageW(
                        state.owner_hwnd,
                        WM_APP_MONITOR_CONFIG_CHANGED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
                SubClickOutcome::BeginClose
            }
        }
        SubHoverTarget::DisplayMetric(item) => {
            let changed = if let Ok(mut guard) = state.cfg.lock() {
                if guard.toggle_metric(item) {
                    save_config(&state.config_path, &guard);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if changed {
                unsafe {
                    let _ = PostMessageW(
                        state.owner_hwnd,
                        WM_APP_MONITOR_CONFIG_CHANGED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
                let _ = state.render();
            }
            SubClickOutcome::None
        }
        SubHoverTarget::SettingLock => {
            unsafe {
                let _ = PostMessageW(
                    state.owner_hwnd,
                    WM_APP_MONITOR_TOGGLE_LOCK,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            SubClickOutcome::BeginClose
        }
        SubHoverTarget::SettingReset => {
            unsafe {
                let _ = PostMessageW(
                    state.owner_hwnd,
                    WM_APP_MONITOR_RESET_POS,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            SubClickOutcome::BeginClose
        }
        SubHoverTarget::SettingClose => {
            SubClickOutcome::HideImmediatelyAndPostOwner(WM_APP_MONITOR_CLOSE_SELF)
        }
    }
}
