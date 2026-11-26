//! Win32 message dispatch, mouse/keyboard/selection/DnD/context-menu/inline-rename interactions.

mod inline_edit;
mod mouse_buttons;
mod mouse_move;

use inline_edit::{finish_inline_edit, read_edit_text, start_inline_edit};
use mouse_buttons::{
    handle_lbutton_dblclk, handle_lbutton_down, handle_lbutton_up, handle_rbutton_down,
    handle_rbutton_up,
};
use mouse_move::{handle_capture_changed, handle_mouse_leave, handle_mouse_move};

use super::{
    BoxInner, BoxMenuAction, BoxMenuContext, BoxMenuKind, BoxWndUserData, InlineEditMode,
    BOX_MENU_CLOSED_CODE, HOST_CMD_DELETE_BOX, HOST_CMD_NEW_COLLECTION, HOST_CMD_NEW_FOLDER_PORTAL,
    TIMER_BOX_ANIM_ID, TIMER_HOVER_COLLAPSE_ID, TIMER_HOVER_MENU_ID, TIMER_TOOLTIP_ID,
    WM_APP_DESKBOX_DIR_CHANGED, WM_APP_DESKBOX_DIR_LOADED, WM_APP_DESKBOX_DROP,
    WM_APP_DESKBOX_HOST_CMD, WM_APP_DESKBOX_ICON_READY, WM_APP_DESKBOX_MENU_ACTION,
    WM_APP_DESKBOX_ZORDER_SYNC,
};
use crate::deskbox::config::{normalize_path_key, BoxKind};
use crate::deskbox::desktop::{
    monitor_info_at_point, open_folder_in_explorer, shell_move_or_copy_files,
};
use crate::deskbox::dnd::{DROP_EVENT_ENTER, DROP_EVENT_FILES, DROP_EVENT_LEAVE};
use crate::deskbox::layout::clamp_box_bounds_to_work_area;
use crate::deskbox::shell_menu::ActiveShellMenu;
use crate::ui::SHADOW_PAD_DIP;
use std::path::PathBuf;
use std::rc::Rc;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, ClientToScreen, EndPaint, ScreenToClient, PAINTSTRUCT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_MENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetCursorPos, GetWindowLongPtrW, KillTimer, PostMessageW, SetCursor, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, EN_CHANGE, EN_KILLFOCUS, GWLP_USERDATA, HTCLIENT,
    HTTRANSPARENT, SC_CLOSE, SC_MINIMIZE, SIZE_MINIMIZED, SWP_HIDEWINDOW, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE, WA_INACTIVE, WINDOWPOS, WM_ACTIVATE,
    WM_ACTIVATEAPP, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_DPICHANGED,
    WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SETCURSOR, WM_SHOWWINDOW, WM_SIZE, WM_SYSCOMMAND, WM_TIMER,
    WM_WINDOWPOSCHANGING,
};

const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
pub(super) const DLGC_WANTALLKEYS_VAL: isize = 0x0004;
pub(super) const INLINE_EDIT_CARET_TIMER_ID: usize = 1;
pub(super) const INLINE_EDIT_CARET_BLINK_MS: u32 = 530;
const HOVER_COLLAPSE_DELAY_MS: u32 = 120;
pub(super) const TOOLTIP_HOVER_DELAY_MS: u32 = 240;
pub(super) const HOVER_MENU_DELAY_MS: u32 = 200;
pub(super) const HEADER_REVEAL_GRACE_MS: u64 = 120;

pub(super) unsafe fn schedule_hover_collapse_check(hwnd: HWND) {
    if !hwnd.is_invalid() {
        SetTimer(hwnd, TIMER_HOVER_COLLAPSE_ID, HOVER_COLLAPSE_DELAY_MS, None);
    }
}

pub(super) unsafe fn cancel_hover_collapse_check(hwnd: HWND) {
    if !hwnd.is_invalid() {
        let _ = KillTimer(hwnd, TIMER_HOVER_COLLAPSE_ID);
    }
}

pub(super) unsafe fn client_rect_to_screen(hwnd: HWND, rect: crate::ui::RectF) -> RECT {
    let mut tl = POINT {
        x: rect.left.round() as i32,
        y: rect.top.round() as i32,
    };
    let mut br = POINT {
        x: rect.right.round() as i32,
        y: rect.bottom.round() as i32,
    };
    let _ = ClientToScreen(hwnd, &mut tl);
    let _ = ClientToScreen(hwnd, &mut br);
    RECT {
        left: tl.x,
        top: tl.y,
        right: br.x,
        bottom: br.y,
    }
}

pub(in super::super) unsafe extern "system" fn deskbox_blur_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub(in super::super) unsafe extern "system" fn deskbox_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        let raw = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut BoxWndUserData;
        if !raw.is_null() {
            drop(Box::from_raw(raw));
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const BoxWndUserData;
    if raw.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ud = &*raw;
    let rc = Rc::clone(&ud.inner);
    let active_shell_menu = Rc::clone(&ud.active_shell_menu);
    let pending_shell_menu = Rc::clone(&ud.pending_shell_menu);

    if let Some(shell_res) =
        ActiveShellMenu::try_handle_wnd_msg(&active_shell_menu, msg, wparam, lparam)
    {
        return shell_res;
    }

    match msg {
        WM_CLOSE => LRESULT(0),
        WM_SYSCOMMAND => {
            let cmd = wparam.0 & 0xFFF0;
            if cmd == (SC_MINIMIZE as usize) || cmd == (SC_CLOSE as usize) {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_WINDOWPOSCHANGING => {
            if let Ok(st) = rc.try_borrow() {
                if lparam.0 != 0 {
                    let wp = &mut *(lparam.0 as *mut WINDOWPOS);
                    if !st.cleaned_up && !st.fullscreen_suppressed {
                        wp.flags &= !SWP_HIDEWINDOW;
                        // Reject Win32 iconic/off-screen relocation (e.g. `-32000, -32000` during Win+D).
                        if (wp.flags & SWP_NOMOVE).0 == 0 && (wp.x <= -16000 || wp.y <= -16000) {
                            wp.flags |= SWP_NOMOVE | SWP_NOSIZE;
                        }
                    }
                    if !st.allow_zorder_change {
                        wp.flags |= SWP_NOZORDER;
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SHOWWINDOW => {
            if wparam.0 == 0 {
                if let Ok(st) = rc.try_borrow() {
                    if !st.cleaned_up && !st.fullscreen_suppressed && !st.host_hwnd.is_invalid() {
                        let _ = PostMessageW(
                            st.host_hwnd,
                            WM_APP_DESKBOX_ZORDER_SYNC,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_ACTIVATEAPP => {
            if wparam.0 != 0 {
                if let Ok(st) = rc.try_borrow() {
                    if !st.cleaned_up && !st.host_hwnd.is_invalid() {
                        let _ = PostMessageW(
                            st.host_hwnd,
                            WM_APP_DESKBOX_ZORDER_SYNC,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SIZE => {
            if wparam.0 == SIZE_MINIMIZED as usize {
                if let Ok(mut st) = rc.try_borrow_mut() {
                    if !st.cleaned_up {
                        st.allow_zorder_change = true;
                        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        st.allow_zorder_change = false;
                        st.sync_desktop_zorder(true);
                        let _ = st.render();
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                let mut pt = POINT::default();
                if GetCursorPos(&mut pt).is_ok() && ScreenToClient(hwnd, &mut pt).as_bool() {
                    if let Ok(st) = rc.try_borrow() {
                        let cursor = st.cursor_for_point(pt.x as f32, pt.y as f32);
                        SetCursor(cursor);
                        return LRESULT(1);
                    }
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
            let inside = rc.try_borrow().is_ok_and(|st| {
                if st.is_edge_tucked() {
                    let s = (st.dpi.max(96) as f32) / 96.0;
                    let pad = SHADOW_PAD_DIP * s;
                    let card = crate::ui::RectF::new(
                        pad,
                        pad,
                        (st.win_w as f32 - pad).max(pad + 4.0),
                        (st.win_h as f32 - pad).max(pad + 4.0),
                    );
                    card.contains(pt.x as f32, pt.y as f32)
                } else {
                    st.current_layout().card_rect.contains(pt.x as f32, pt.y as f32)
                }
            });
            if inside {
                LRESULT(HTCLIENT as isize)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        WM_CAPTURECHANGED => handle_capture_changed(hwnd, &rc),
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                let shell_menu_open = active_shell_menu
                    .try_borrow()
                    .is_ok_and(|guard| guard.is_some());
                if !shell_menu_open {
                    let other_hwnd = HWND(lparam.0 as *mut core::ffi::c_void);
                    if let Ok(mut st) = rc.try_borrow_mut() {
                        st.hide_tooltip();
                        st.cancel_pending_hover_menu();
                        let own_child = (!st.inline_edit_hwnd.is_invalid()
                            && other_hwnd == st.inline_edit_hwnd)
                            || st.open_menu_kind_for_self().is_some();
                        if !own_child && !st.selected.is_empty() {
                            st.selected.clear();
                            st.selection_anchor = None;
                            let _ = st.render();
                        }
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_KILLFOCUS => {
            let shell_menu_open = active_shell_menu
                .try_borrow()
                .is_ok_and(|guard| guard.is_some());
            if !shell_menu_open {
                let new_focus = HWND(wparam.0 as *mut core::ffi::c_void);
                if let Ok(mut st) = rc.try_borrow_mut() {
                    st.hide_tooltip();
                    st.cancel_pending_hover_menu();
                    let own_child = (!st.inline_edit_hwnd.is_invalid()
                        && new_focus == st.inline_edit_hwnd)
                        || st.open_menu_kind_for_self().is_some();
                    if !own_child && !st.selected.is_empty() {
                        st.selected.clear();
                        st.selection_anchor = None;
                        let _ = st.render();
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => handle_mouse_move(hwnd, &rc, lparam),
        WM_MOUSELEAVE_MSG => handle_mouse_leave(hwnd, &rc),
        WM_TIMER => {
            if wparam.0 == TIMER_BOX_ANIM_ID {
                if let Ok(mut st) = rc.try_borrow_mut() {
                    st.step_box_animations();
                }
            } else if wparam.0 == TIMER_TOOLTIP_ID {
                let _ = KillTimer(hwnd, TIMER_TOOLTIP_ID);
                if let Ok(st) = rc.try_borrow() {
                    if st.drag.is_none()
                        && !st.is_edge_tucked()
                        && st.open_menu_kind_for_self().is_none()
                    {
                        let layout = st.current_layout();
                        let mut pt = POINT::default();
                        let cursor_ok = GetCursorPos(&mut pt).is_ok()
                            && ScreenToClient(hwnd, &mut pt).as_bool();
                        if cursor_ok {
                            if let Some((tip_rect, tip_text)) = st
                                .hover
                                .and_then(|h| st.tooltip_spec_for_hover(h, &layout))
                            {
                                if tip_rect.contains(pt.x as f32, pt.y as f32) {
                                    let screen_r = client_rect_to_screen(hwnd, tip_rect);
                                    if let Ok(mut tip) = st.box_tooltip.try_borrow_mut() {
                                        tip.show(hwnd, screen_r, tip_text, st.light_glass);
                                    }
                                }
                            }
                        }
                    }
                }
            } else if wparam.0 == TIMER_HOVER_MENU_ID {
                let _ = KillTimer(hwnd, TIMER_HOVER_MENU_ID);
                let hover_menu_req: Option<(RECT, bool, BoxMenuKind, BoxMenuContext)> =
                    if let Ok(mut st) = rc.try_borrow_mut() {
                        let pending = st.pending_hover_menu.take();
                        if !st.cleaned_up
                            && st.drag.is_none()
                            && st.inline_edit_hwnd.is_invalid()
                            && !st.is_edge_tucked()
                            && !st.collapse_animating
                            && !st.edge_animating
                            && st.header_hovered
                        {
                            if let Some(target) = pending {
                                if st.hover == Some(target)
                                    && st.suppress_hover_menu_btn != Some(target)
                                {
                                    let layout = st.current_layout();
                                    if let Some((menu_kind, btn_rect)) =
                                        BoxInner::hover_menu_spec(target, &layout)
                                    {
                                        let open_kind = st.open_menu_kind_for_self();
                                        let mut pt = POINT::default();
                                        let cursor_on_btn = GetCursorPos(&mut pt).is_ok()
                                            && ScreenToClient(hwnd, &mut pt).as_bool()
                                            && btn_rect.contains(pt.x as f32, pt.y as f32);
                                        if cursor_on_btn
                                            && open_kind != Some(menu_kind)
                                            && !matches!(
                                                open_kind,
                                                Some(BoxMenuKind::Full | BoxMenuKind::ConfirmDelete)
                                            )
                                        {
                                            st.hide_tooltip();
                                            Some((
                                                client_rect_to_screen(hwnd, btn_rect),
                                                st.light_glass,
                                                menu_kind,
                                                st.menu_context(),
                                            ))
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                if let Some((trigger_rect, light_glass, menu_kind, ctx)) = hover_menu_req {
                    let menu_rc = rc.try_borrow().ok().map(|s| Rc::clone(&s.box_menu));
                    if let Some(menu) = menu_rc {
                        if !menu.was_just_closed_for(hwnd, menu_kind) {
                            menu.show_for_trigger(
                                hwnd,
                                trigger_rect,
                                light_glass,
                                menu_kind,
                                ctx,
                                true,
                            );
                        }
                    }
                    if let Ok(mut st) = rc.try_borrow_mut() {
                        let _ = st.render();
                    }
                }
            } else if wparam.0 == TIMER_HOVER_COLLAPSE_ID {
                cancel_hover_collapse_check(hwnd);
                let shell_menu_open = active_shell_menu
                    .try_borrow()
                    .is_ok_and(|guard| guard.is_some());
                if !shell_menu_open {
                    if let Ok(mut st) = rc.try_borrow_mut() {
                        let wants_retract = (st.spec.collapsed && st.hover_expanded)
                            || (st.spec.edge_auto_hide && st.edge_drawer_open);
                        if wants_retract {
                            let mut pt = POINT::default();
                            let cursor_inside_box = GetCursorPos(&mut pt).is_ok()
                                && ScreenToClient(hwnd, &mut pt).as_bool()
                                && st.current_layout().card_rect.contains(pt.x as f32, pt.y as f32);
                            let menu_active = st.open_menu_kind_for_self().is_some()
                                || st.box_menu.is_cursor_inside_menu_or_trigger(hwnd);
                            let busy = st.drag.is_some()
                                || st.is_drop_hovered
                                || !st.inline_edit_hwnd.is_invalid()
                                || cursor_inside_box
                                || menu_active;
                            if !busy {
                                if st.spec.edge_auto_hide && st.edge_drawer_open {
                                    st.set_edge_drawer_open(false);
                                } else if st.spec.collapsed && st.hover_expanded {
                                    st.set_hover_expanded(false);
                                }
                            }
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => handle_lbutton_down(hwnd, &rc, &pending_shell_menu, lparam),
        WM_LBUTTONUP => handle_lbutton_up(hwnd, &rc, lparam),
        WM_LBUTTONDBLCLK => handle_lbutton_dblclk(hwnd, &rc, lparam),
        WM_RBUTTONDOWN => handle_rbutton_down(hwnd, &rc, &pending_shell_menu, lparam),
        WM_RBUTTONUP => handle_rbutton_up(hwnd, &rc, &active_shell_menu, &pending_shell_menu, lparam),
        WM_MOUSEWHEEL => {
            if let Ok(mut st) = rc.try_borrow_mut() {
                st.hide_tooltip();
                st.cancel_pending_hover_menu();
                if st.is_visually_collapsed() || st.is_edge_tucked() {
                    return LRESULT(0);
                }
                let delta = ((wparam.0 >> 16) as i16) as f32;
                let s = (st.dpi.max(96) as f32) / 96.0;
                let step = -(delta / 120.0) * 78.0 * s;
                let was_animating = st.scroll_animating;
                let next_target = st.scroll_target_y + step;
                st.set_scroll_target(next_target, true);
                if !was_animating && st.scroll_animating {
                    // Render an immediate initial response frame instead of waiting for the first timer tick.
                    let max_scroll = st.max_scroll_px();
                    let diff = st.scroll_target_y - st.scroll_y;
                    st.scroll_y = (st.scroll_y + diff * 0.28).clamp(0.0, max_scroll);
                    let _ = st.render();
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            let mut want_search = false;
            if let Ok(mut st) = rc.try_borrow_mut() {
                if st.is_edge_tucked() {
                    return LRESULT(0);
                }
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                if ctrl && (vk == b'A' as u16) {
                    st.selected = (0..st.items.len()).collect();
                    let _ = st.render();
                } else if ctrl && (vk == b'F' as u16) {
                    want_search = true;
                } else if vk == VK_DELETE.0
                    && st.spec.kind == BoxKind::Collection
                    && !st.selected.is_empty()
                {
                    let indices: Vec<usize> = st.selected.iter().copied().collect();
                    st.remove_collection_indices(&indices);
                } else if vk == VK_ESCAPE.0 {
                    if st.search_active {
                        st.clear_search();
                    } else if !st.selected.is_empty() {
                        st.selected.clear();
                        st.selection_anchor = None;
                        let _ = st.render();
                    }
                }
            }
            if want_search {
                start_inline_edit(&rc, InlineEditMode::Search, None);
            }
            LRESULT(0)
        }
        WM_CHAR => {
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let alt = GetKeyState(VK_MENU.0 as i32) < 0;
            if !ctrl && !alt {
                if let Some(ch) = char::from_u32(wparam.0 as u32) {
                    if !ch.is_control() && ch != '\x7f' {
                        let can_type = rc.try_borrow().is_ok_and(|st| {
                            st.inline_edit_hwnd.is_invalid() && !st.is_edge_tucked()
                        });
                        if can_type {
                            start_inline_edit(&rc, InlineEditMode::Search, Some(ch));
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_DESKBOX_ICON_READY => {
            if let Ok(mut st) = rc.try_borrow_mut() {
                let updated = st
                    .icon_service
                    .try_borrow_mut()
                    .is_ok_and(|mut svc| svc.drain_completed());
                if updated {
                    st.bump_content_rev();
                    // While collapse or scroll animation is already ticking at high FPS, skip
                    // extra coalesced repaints so icon decode bursts never double-render on top.
                    if !st.scroll_animating && !st.collapse_animating {
                        let _ = st.render();
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_DESKBOX_DIR_CHANGED => {
            if let Ok(mut st) = rc.try_borrow_mut() {
                if st.spec.kind == BoxKind::FolderPortal && wparam.0 == st.watch_gen {
                    st.reload_items();
                }
            }
            LRESULT(0)
        }
        WM_APP_DESKBOX_DIR_LOADED => {
            if let Ok(mut st) = rc.try_borrow_mut() {
                let payload = st
                    .dir_scan_result
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.take());
                if let Some(payload) = payload {
                    st.apply_dir_scan_result(payload);
                }
            }
            LRESULT(0)
        }
        WM_APP_DESKBOX_DROP => {
            let kind = wparam.0;
            if kind == DROP_EVENT_ENTER || kind == DROP_EVENT_LEAVE {
                if let Ok(mut st) = rc.try_borrow_mut() {
                    let hovered = kind == DROP_EVENT_ENTER;
                    if hovered {
                        cancel_hover_collapse_check(hwnd);
                        if st.spec.edge_auto_hide && !st.edge_drawer_open {
                            st.set_edge_drawer_open(true);
                        } else if st.spec.collapsed && !st.hover_expanded {
                            st.set_hover_expanded(true);
                        }
                    } else if (st.spec.collapsed && st.hover_expanded)
                        || (st.spec.edge_auto_hide && st.edge_drawer_open)
                    {
                        schedule_hover_collapse_check(hwnd);
                    }
                    if st.is_drop_hovered != hovered {
                        st.is_drop_hovered = hovered;
                        let _ = st.render();
                    }
                }
                return LRESULT(0);
            }
            if kind == DROP_EVENT_FILES {
                let payloads = rc
                    .try_borrow()
                    .ok()
                    .map(|st| st.drop_shared.take_drops())
                    .unwrap_or_default();
                for payload in payloads {
                    let mut portal_copy_op: Option<(Vec<PathBuf>, PathBuf, bool)> = None;
                    if let Ok(mut st) = rc.try_borrow_mut() {
                        if st.cleaned_up {
                            break;
                        }
                        st.is_drop_hovered = false;
                        match st.spec.kind {
                            BoxKind::Collection => {
                                st.deduplicate_and_add_collection(&payload.paths);
                            }
                            BoxKind::FolderPortal => {
                                let dest_dir = st.current_portal_dir();
                                let dest_key = normalize_path_key(&dest_dir.to_string_lossy());
                                let external: Vec<PathBuf> = payload
                                    .paths
                                    .into_iter()
                                    .filter(|src| {
                                        src.parent().is_none_or(|p| {
                                            normalize_path_key(&p.to_string_lossy()) != dest_key
                                        })
                                    })
                                    .collect();
                                if !external.is_empty() {
                                    portal_copy_op = Some((external, dest_dir, payload.copy));
                                }
                            }
                        }
                    }
                    if let Some((srcs, dest_dir, copy)) = portal_copy_op {
                        if shell_move_or_copy_files(hwnd, &srcs, &dest_dir, copy) {
                            if let Ok(mut st) = rc.try_borrow_mut() {
                                if !st.cleaned_up {
                                    st.reload_items();
                                    let _ = st.render();
                                }
                            }
                        }
                    }
                }
                if rc.try_borrow().is_ok_and(|st| {
                    !st.cleaned_up
                        && ((st.spec.collapsed && st.hover_expanded)
                            || (st.spec.edge_auto_hide && st.edge_drawer_open))
                }) {
                    schedule_hover_collapse_check(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_APP_DESKBOX_MENU_ACTION => {
            if wparam.0 == BOX_MENU_CLOSED_CODE {
                if let Ok(mut st) = rc.try_borrow_mut() {
                    st.refresh_header_hover_from_cursor();
                    if (st.spec.collapsed && st.hover_expanded)
                        || (st.spec.edge_auto_hide && st.edge_drawer_open)
                    {
                        schedule_hover_collapse_check(hwnd);
                    }
                }
                return LRESULT(0);
            }
            let Some(action) = BoxMenuAction::from_usize(wparam.0) else {
                return LRESULT(0);
            };
            let mut open_explorer_dir: Option<PathBuf> = None;
            let mut host_cmd: Option<(HWND, usize, u64)> = None;

            if let Ok(mut st) = rc.try_borrow_mut() {
                match action {
                    BoxMenuAction::NewCollection => {
                        host_cmd = Some((st.host_hwnd, HOST_CMD_NEW_COLLECTION, st.spec.id));
                    }
                    BoxMenuAction::NewFolderPortal => {
                        host_cmd = Some((st.host_hwnd, HOST_CMD_NEW_FOLDER_PORTAL, st.spec.id));
                    }
                    BoxMenuAction::SetView(view) => {
                        if st.spec.view_mode != view {
                            st.spec.view_mode = view;
                            let layout = st.current_layout();
                            st.set_scroll_target(layout.scroll_y, false);
                            st.persist_to_host();
                            let _ = st.render();
                        }
                    }
                    BoxMenuAction::SetSort(sort) => {
                        st.apply_one_shot_sort(sort);
                    }
                    BoxMenuAction::AutoFitSize => {
                        st.auto_fit_size();
                    }
                    BoxMenuAction::ToggleEdgeAutoHide => {
                        st.toggle_edge_auto_hide();
                        if st.spec.edge_auto_hide && st.edge_drawer_open {
                            schedule_hover_collapse_check(hwnd);
                        }
                    }
                    BoxMenuAction::OpenInExplorer => {
                        if st.spec.kind == BoxKind::FolderPortal {
                            open_explorer_dir = Some(st.current_portal_dir());
                        }
                    }
                    BoxMenuAction::CleanMissing => {
                        st.clean_missing_collection_items();
                    }
                    BoxMenuAction::ConfirmDeleteBox => {
                        host_cmd = Some((st.host_hwnd, HOST_CMD_DELETE_BOX, st.spec.id));
                    }
                    BoxMenuAction::CancelDeleteBox => {}
                }
            }

            if let Some((h_hwnd, cmd, box_id)) = host_cmd {
                if !h_hwnd.is_invalid() {
                    let _ = PostMessageW(
                        h_hwnd,
                        WM_APP_DESKBOX_HOST_CMD,
                        WPARAM(cmd),
                        LPARAM(box_id as isize),
                    );
                }
            }
            if let Some(dir) = open_explorer_dir {
                let _ = open_folder_in_explorer(&dir);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let notif = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let ctrl_hwnd = HWND(lparam.0 as *mut core::ffi::c_void);
            let is_our_edit = rc
                .try_borrow()
                .is_ok_and(|st| !st.inline_edit_hwnd.is_invalid() && st.inline_edit_hwnd == ctrl_hwnd);
            if is_our_edit {
                if notif == EN_KILLFOCUS {
                    finish_inline_edit(&rc, true);
                } else if notif == EN_CHANGE {
                    let live_text = read_edit_text(ctrl_hwnd);
                    if let Ok(mut st) = rc.try_borrow_mut() {
                        st.inline_edit_caret_visible = true;
                        if st.inline_edit_mode == Some(InlineEditMode::Search) {
                            st.search_query = live_text;
                            st.apply_search_filter();
                        }
                        let _ = st.render();
                    }
                }
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam.0 & 0xFFFF) as u32;
            if let Ok(mut st) = rc.try_borrow_mut() {
                if st.drag.is_some() {
                    // During an active cross-DPI drag, record the updated DPI cleanly without
                    // snapping window coordinates away from the cursor; `WM_LBUTTONUP` settles
                    // the final bounds preserving logical DIP dimensions.
                    st.dpi = new_dpi.max(96);
                    st.last_blur_key = None;
                } else if lparam.0 != 0 {
                    let suggested = &*(lparam.0 as *const RECT);
                    st.dpi = new_dpi.max(96);
                    st.collapse_animating = false;
                    st.edge_animating = false;
                    st.last_blur_key = None;
                    if st.spec.edge_auto_hide {
                        let slide_t = if st.edge_drawer_open { 1.0 } else { 0.0 };
                        st.edge_slide_spring = crate::ui::Spring1D::still(slide_t);
                        st.apply_edge_drawer_bounds(slide_t);
                    } else {
                        let mon = monitor_info_at_point(POINT {
                            x: (suggested.left + suggested.right) / 2,
                            y: (suggested.top + suggested.bottom) / 2,
                        });
                        let (cx, cy, cw, ch, wx, wy, ww, wh) = clamp_box_bounds_to_work_area(
                            st.spec.x_dip,
                            st.spec.y_dip,
                            st.spec.w_dip,
                            st.spec.h_dip,
                            st.is_visually_collapsed(),
                            mon.work.left,
                            mon.work.top,
                            mon.work.right,
                            mon.work.bottom,
                            st.dpi,
                        );
                        let _ = (cx, cy, cw, ch);
                        st.win_x = wx;
                        st.win_y = wy;
                        st.win_w = ww;
                        st.win_h = wh;
                        let _ = SetWindowPos(
                            st.hwnd,
                            None,
                            wx,
                            wy,
                            ww,
                            wh,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        st.sync_blur_layer();
                        st.register_peer_rect();
                    }
                    let _ = st.render();
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

