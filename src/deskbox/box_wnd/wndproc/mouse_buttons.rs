//! Left/right mouse button down, up, double-click, and native Shell context menu invocation.

use super::inline_edit::{finish_inline_edit, start_inline_edit};
use super::super::{BoxInner, BoxMenuContext, BoxMenuKind, DragAction, HoverTarget, InlineEditMode};
use super::{client_rect_to_screen, schedule_hover_collapse_check};
use crate::deskbox::config::BoxKind;
use crate::deskbox::desktop::{monitor_info_for_window, open_item_in_shell};
use crate::deskbox::layout::BoxHitZone;
use crate::deskbox::shell_menu::{
    prepare_native_file_context_menu, track_prepared_shell_menu, ActiveShellMenu,
    PreparedShellMenu, ShellMenuOutcome,
};
use crate::deskbox::snap::detect_dock_edge;
use crate::ui::SHADOW_PAD_DIP;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
};

pub(super) unsafe fn handle_lbutton_down(
    hwnd: HWND,
    rc: &Rc<RefCell<BoxInner>>,
    pending_shell_menu: &Rc<RefCell<Option<PreparedShellMenu>>>,
    lparam: LPARAM,
) -> LRESULT {
    if let Ok(mut pending) = pending_shell_menu.try_borrow_mut() {
        *pending = None;
    }
    let cx = ((lparam.0 & 0xFFFF) as i16) as i32;
    let cy = (((lparam.0 >> 16) & 0xFFFF) as i16) as i32;

    // If user clicks the search clear "×" button while search is active (even if EDIT proxy
    // is still open), clear search immediately and consume the click so it can never fall
    // through onto the revealed DeleteButton beneath it.
    let clicked_search_clear = if let Ok(mut st) = rc.try_borrow_mut() {
        st.hide_tooltip();
        st.cancel_pending_hover_menu();
        st.suppress_hover_menu_btn = None;
        if st.search_active
            && st
                .current_layout()
                .search_clear_btn_rect
                .contains(cx as f32, cy as f32)
        {
            st.clear_search();
            true
        } else {
            false
        }
    } else {
        false
    };
    if clicked_search_clear {
        return LRESULT(0);
    }

    let mut open_menu_info: Option<(RECT, bool, BoxMenuKind, BoxMenuContext)> = None;
    let mut dismiss_open_menu = false;
    let mut want_search = false;
    let mut commit_edit_first = false;
    if let Ok(st) = rc.try_borrow() {
        if !st.inline_edit_hwnd.is_invalid() {
            commit_edit_first = true;
        }
    }
    if commit_edit_first {
        finish_inline_edit(rc, true);
    }

    let mut want_focus = false;
    if let Ok(mut st) = rc.try_borrow_mut() {
        if st.is_edge_tucked() {
            st.set_edge_drawer_open(true);
            return LRESULT(0);
        }
        want_focus = true;
    }
    if want_focus {
        // Call SetFocus outside any RefCell borrow so synchronous WM_SETFOCUS/WM_KILLFOCUS
        // re-entry cannot collide with a live mutable borrow.
        let _ = SetFocus(hwnd);
    }

    if let Ok(mut st) = rc.try_borrow_mut() {
        if st.cleaned_up {
            return LRESULT(0);
        }
        let mut pt_screen = POINT { x: cx, y: cy };
        let _ = ClientToScreen(hwnd, &mut pt_screen);

        let layout = st.current_layout();
        let zone = layout.hit_test(cx as f32, cy as f32);
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;

        if !matches!(
            zone,
            BoxHitZone::NewButton
                | BoxHitZone::SortButton
                | BoxHitZone::ViewButton
                | BoxHitZone::DeleteButton
        ) && st.open_menu_kind_for_self().is_some()
        {
            dismiss_open_menu = true;
        }

        match zone {
            BoxHitZone::Resize(edge) => {
                st.settle_collapse_animation_now();
                let mon = monitor_info_for_window(st.hwnd);
                st.dpi = mon.dpi;
                st.spec.monitor_id = mon.monitor_id.clone();
                st.resize_chain_work = mon.work;
                st.resize_chain_dpi = mon.dpi;
                st.resize_chain_snapshot = if st.spec.edge_auto_hide {
                    Vec::new()
                } else {
                    st.snapshot_monitor_chain_nodes(&mon.monitor_id)
                };
                let s = (st.dpi.max(96) as f32) / 96.0;
                let pad = (SHADOW_PAD_DIP * s).round() as i32;
                let orig_cl = st.win_x + pad;
                let orig_ct = st.win_y + pad;
                let orig_cr = st.win_x + st.win_w - pad;
                let orig_cb = st.win_y + st.win_h - pad;
                st.drag = Some(DragAction::Resize {
                    edge,
                    start_screen: pt_screen,
                    orig_card_left: orig_cl,
                    orig_card_top: orig_ct,
                    orig_card_right: orig_cr,
                    orig_card_bottom: orig_cb,
                });
                let _ = SetCapture(hwnd);
            }
            BoxHitZone::BackButton => {
                if st.nav_stack.pop().is_some() {
                    st.all_items.clear();
                    st.items.clear();
                    st.selected.clear();
                    st.selection_anchor = None;
                    st.set_scroll_target(0.0, false);
                    st.reload_items();
                    st.restart_watcher();
                    let _ = st.render();
                }
            }
            BoxHitZone::SearchClearButton => {
                st.clear_search();
            }
            BoxHitZone::NewButton => {
                open_menu_info = Some((
                    client_rect_to_screen(hwnd, layout.new_btn_rect),
                    st.light_glass,
                    BoxMenuKind::NewBox,
                    st.menu_context(),
                ));
            }
            BoxHitZone::SortButton => {
                open_menu_info = Some((
                    client_rect_to_screen(hwnd, layout.sort_btn_rect),
                    st.light_glass,
                    BoxMenuKind::Sort,
                    st.menu_context(),
                ));
            }
            BoxHitZone::ViewButton => {
                open_menu_info = Some((
                    client_rect_to_screen(hwnd, layout.view_btn_rect),
                    st.light_glass,
                    BoxMenuKind::ViewAndTools,
                    st.menu_context(),
                ));
            }
            BoxHitZone::LockButton => {
                st.spec.locked = !st.spec.locked;
                st.register_peer_rect();
                st.persist_to_host();
                let _ = st.render();
            }
            BoxHitZone::CollapseButton => {
                st.toggle_collapsed();
            }
            BoxHitZone::DeleteButton => {
                // Guard against accidental delete when an inline search/rename pill just
                // dismissed on WM_KILLFOCUS immediately prior to this WM_LBUTTONDOWN.
                let just_closed_edit = commit_edit_first
                    || st.last_inline_edit_closed_at.is_some_and(|t| {
                        t.elapsed() < Duration::from_millis(250)
                    });
                if !just_closed_edit {
                    open_menu_info = Some((
                        client_rect_to_screen(hwnd, layout.delete_btn_rect),
                        st.light_glass,
                        BoxMenuKind::ConfirmDelete,
                        st.menu_context(),
                    ));
                }
            }
            BoxHitZone::HeaderDrag => {
                if st.search_active {
                    want_search = true;
                } else if !st.spec.locked {
                    st.settle_collapse_animation_now();
                    st.drag = Some(DragAction::MoveBox {
                        start_screen: pt_screen,
                        orig_win_x: st.win_x,
                        orig_win_y: st.win_y,
                    });
                    let _ = SetCapture(hwnd);
                }
            }
            BoxHitZone::ScrollbarThumb => {
                let grab_offset_y = cy as f32 - layout.scrollbar_thumb_rect.top;
                st.drag = Some(DragAction::ScrollbarThumb { grab_offset_y });
                let _ = SetCapture(hwnd);
            }
            BoxHitZone::ScrollbarTrack => {
                let track = layout.scrollbar_track_rect;
                let ratio = ((cy as f32 - track.top) / track.height().max(1.0)).clamp(0.0, 1.0);
                st.set_scroll_target(ratio * layout.max_scroll, false);
                let _ = st.render();
            }
            BoxHitZone::ItemRemove(idx) => {
                st.drag = Some(DragAction::PendingRemoveClick { item_idx: idx });
                let _ = SetCapture(hwnd);
            }
            BoxHitZone::Item(idx) => {
                if shift {
                    let anchor = st.selection_anchor.unwrap_or(idx);
                    let lo = anchor.min(idx);
                    let hi = anchor.max(idx);
                    if !ctrl {
                        st.selected.clear();
                    }
                    for i in lo..=hi {
                        st.selected.insert(i);
                    }
                } else if ctrl {
                    if !st.selected.remove(&idx) {
                        st.selected.insert(idx);
                    }
                    st.selection_anchor = Some(idx);
                } else {
                    if !st.selected.contains(&idx) {
                        st.selected.clear();
                        st.selected.insert(idx);
                    }
                    st.selection_anchor = Some(idx);
                }
                st.drag = Some(DragAction::PendingItemDrag {
                    item_idx: idx,
                    start_client: POINT { x: cx, y: cy },
                });
                let _ = SetCapture(hwnd);
                let _ = st.render();
            }
            BoxHitZone::ContentBlank => {
                if !ctrl {
                    st.selected.clear();
                    st.selection_anchor = None;
                }
                st.drag = Some(DragAction::Marquee {
                    anchor_client: POINT { x: cx, y: cy },
                    current_client: POINT { x: cx, y: cy },
                });
                let _ = SetCapture(hwnd);
                let _ = st.render();
            }
            BoxHitZone::None => {}
        }
    }

    if dismiss_open_menu {
        let menu_rc = rc.try_borrow().ok().map(|s| Rc::clone(&s.box_menu));
        if let Some(menu) = menu_rc {
            menu.hide();
        }
    }

    if let Some((trigger_rect, light_glass, menu_kind, ctx)) = open_menu_info {
        let menu_rc = rc.try_borrow().ok().map(|s| Rc::clone(&s.box_menu));
        if let Some(menu) = menu_rc {
            if menu.open_kind_for(hwnd) == Some(menu_kind) {
                menu.promote_hover_menu_focus(hwnd, menu_kind);
            } else if !menu.was_just_closed_for(hwnd, menu_kind) {
                menu.show_for_trigger(
                    hwnd,
                    trigger_rect,
                    light_glass,
                    menu_kind,
                    ctx,
                    false,
                );
            }
        }
        if let Ok(mut st) = rc.try_borrow_mut() {
            let _ = st.render();
        }
    } else if want_search {
        start_inline_edit(rc, InlineEditMode::Search, None);
    }
    LRESULT(0)
}

pub(super) unsafe fn handle_lbutton_up(
    hwnd: HWND,
    rc: &Rc<RefCell<BoxInner>>,
    lparam: LPARAM,
) -> LRESULT {
    let mut commit_swp: Option<(i32, i32, i32, i32)> = None;
    let mut want_hover_check = false;
    if let Ok(mut st) = rc.try_borrow_mut() {
        if let Some(drag) = st.drag.take() {
            let _ = ReleaseCapture();
            match drag {
                DragAction::MoveBox { .. } | DragAction::Resize { .. } => {
                    st.resize_chain_snapshot.clear();
                    if let Ok(mut overlay) = st.snap_overlay.try_borrow_mut() {
                        overlay.hide();
                    }
                    let s = (st.dpi.max(96) as f32) / 96.0;
                    let pad = (SHADOW_PAD_DIP * s).round() as i32;
                    let cl = st.win_x + pad;
                    let ct = st.win_y + pad;
                    let cw = (st.win_w - pad * 2).max(1);
                    let ch = (st.win_h - pad * 2).max(1);
                    st.update_spec_from_current_bounds(cl, ct, cw, ch, true);
                    if st.spec.edge_auto_hide {
                        let mon = monitor_info_for_window(st.hwnd);
                        let ms = (mon.dpi.max(96) as f32) / 96.0;
                        let work_w_dip = (mon.work.right - mon.work.left).max(260) as f32 / ms;
                        st.edge_dock = detect_dock_edge(
                            st.spec.x_dip,
                            st.spec.y_dip,
                            st.spec.w_dip,
                            work_w_dip,
                        );
                        st.apply_edge_drawer_bounds(1.0);
                    } else {
                        commit_swp = Some((st.win_x, st.win_y, st.win_w, st.win_h));
                    }
                    st.persist_to_host();
                    if commit_swp.is_none() {
                        let _ = st.render();
                    }
                }
                DragAction::ReorderItem {
                    from_idx,
                    target_idx,
                } => {
                    if target_idx != from_idx {
                        st.reorder_item_in_place(from_idx, target_idx);
                    }
                    let _ = st.render();
                }
                DragAction::PendingItemDrag { item_idx, .. } => {
                    let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                    let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                    if !ctrl && !shift && !st.suppress_next_dblclk_select {
                        st.selected.clear();
                        st.selected.insert(item_idx);
                        st.selection_anchor = Some(item_idx);
                        let _ = st.render();
                    }
                }
                DragAction::PendingRemoveClick { item_idx } => {
                    let cx = ((lparam.0 & 0xFFFF) as i16) as f32;
                    let cy = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
                    let layout = st.current_layout();
                    if layout.hit_test(cx, cy) == BoxHitZone::ItemRemove(item_idx)
                        && st.remove_collection_indices(&[item_idx])
                    {
                        let new_layout = st.current_layout();
                        let next_hover = match new_layout.hit_test(cx, cy) {
                            BoxHitZone::BackButton => Some(HoverTarget::BackButton),
                            BoxHitZone::SearchClearButton => {
                                Some(HoverTarget::SearchClearButton)
                            }
                            BoxHitZone::NewButton => Some(HoverTarget::NewButton),
                            BoxHitZone::SortButton => Some(HoverTarget::SortButton),
                            BoxHitZone::ViewButton => Some(HoverTarget::ViewButton),
                            BoxHitZone::LockButton => Some(HoverTarget::LockButton),
                            BoxHitZone::CollapseButton => {
                                Some(HoverTarget::CollapseButton)
                            }
                            BoxHitZone::DeleteButton => Some(HoverTarget::DeleteButton),
                            BoxHitZone::Item(idx) => Some(HoverTarget::Item(idx)),
                            BoxHitZone::ItemRemove(idx) => Some(HoverTarget::ItemRemove(idx)),
                            _ => None,
                        };
                        if next_hover != st.hover {
                            st.hover = next_hover;
                            let _ = st.render();
                        }
                    }
                }
                DragAction::Marquee { .. } => {
                    let _ = st.render();
                }
                DragAction::ScrollbarThumb { .. } => {}
            }
            let mut pt = POINT::default();
            let inside = GetCursorPos(&mut pt).is_ok()
                && ScreenToClient(hwnd, &mut pt).as_bool()
                && st.current_layout().card_rect.contains(pt.x as f32, pt.y as f32);
            if !inside {
                st.hover = None;
                st.header_hovered = false;
                st.scrollbar_hovered = false;
                st.sync_header_opacity_target();
                if commit_swp.is_none() {
                    let _ = st.render();
                }
            }
            want_hover_check = (st.spec.collapsed && st.hover_expanded)
                || (st.spec.edge_auto_hide && st.edge_drawer_open);
        }
        st.suppress_next_dblclk_select = false;
    }
    if let Some((wx, wy, ww, wh)) = commit_swp {
        let _ = SetWindowPos(hwnd, None, wx, wy, ww, wh, SWP_NOZORDER | SWP_NOACTIVATE);
        if let Ok(mut st) = rc.try_borrow_mut() {
            st.sync_blur_layer();
            st.register_peer_rect();
            let _ = st.render();
        }
    }
    if want_hover_check {
        schedule_hover_collapse_check(hwnd);
    }
    LRESULT(0)
}

pub(super) unsafe fn handle_lbutton_dblclk(
    _hwnd: HWND,
    rc: &Rc<RefCell<BoxInner>>,
    lparam: LPARAM,
) -> LRESULT {
    enum DblAction {
        None,
        OpenFile(PathBuf),
        StartRename,
    }
    let action = if let Ok(mut st) = rc.try_borrow_mut() {
        let cx = ((lparam.0 & 0xFFFF) as i16) as f32;
        let cy = (((lparam.0 >> 16) & 0xFFFF) as i16) as f32;
        let layout = st.current_layout();
        match layout.hit_test(cx, cy) {
            BoxHitZone::Item(idx) => {
                st.suppress_next_dblclk_select = true;
                st.selected.clear();
                st.selected.insert(idx);
                if let Some(item) = st.items.get(idx).cloned() {
                    if st.spec.kind == BoxKind::FolderPortal && item.is_dir && item.exists {
                        st.nav_stack.push(item.path);
                        st.all_items.clear();
                        st.items.clear();
                        st.selected.clear();
                        st.selection_anchor = None;
                        st.set_scroll_target(0.0, false);
                        st.reload_items();
                        st.restart_watcher();
                        let _ = st.render();
                        DblAction::None
                    } else if item.exists {
                        DblAction::OpenFile(item.path)
                    } else {
                        DblAction::None
                    }
                } else {
                    DblAction::None
                }
            }
            BoxHitZone::HeaderDrag
                if !st.search_active && st.spec.kind != BoxKind::FolderPortal =>
            {
                DblAction::StartRename
            }
            _ => DblAction::None,
        }
    } else {
        DblAction::None
    };

    match action {
        DblAction::OpenFile(p) => {
            let _ = open_item_in_shell(&p);
        }
        DblAction::StartRename => {
            start_inline_edit(rc, InlineEditMode::Rename, None);
        }
        DblAction::None => {}
    }
    LRESULT(0)
}

pub(super) unsafe fn handle_rbutton_down(
    hwnd: HWND,
    rc: &Rc<RefCell<BoxInner>>,
    pending_shell_menu: &Rc<RefCell<Option<PreparedShellMenu>>>,
    lparam: LPARAM,
) -> LRESULT {
    if let Ok(mut pending) = pending_shell_menu.try_borrow_mut() {
        *pending = None;
    }
    let prep_paths: Option<Vec<PathBuf>> = if let Ok(mut st) = rc.try_borrow_mut() {
        st.hide_tooltip();
        st.cancel_pending_hover_menu();
        let cx = ((lparam.0 & 0xFFFF) as i16) as i32;
        let cy = (((lparam.0 >> 16) & 0xFFFF) as i16) as i32;
        let layout = st.current_layout();
        match layout.hit_test(cx as f32, cy as f32) {
            BoxHitZone::Item(idx) => {
                if !st.selected.contains(&idx) {
                    st.selected.clear();
                    st.selected.insert(idx);
                    st.selection_anchor = Some(idx);
                    let _ = st.render();
                }
                let mut indices: Vec<usize> = st.selected.iter().copied().collect();
                indices.sort_unstable();
                let paths: Vec<PathBuf> = indices
                    .iter()
                    .filter_map(|&i| st.items.get(i))
                    .map(|it| it.path.clone())
                    .collect();
                (!paths.is_empty()).then_some(paths)
            }
            _ => None,
        }
    } else {
        None
    };
    if let Some(paths) = prep_paths {
        if let Some(prepared) = prepare_native_file_context_menu(hwnd, &paths) {
            if let Ok(mut pending) = pending_shell_menu.try_borrow_mut() {
                *pending = Some(prepared);
            }
        }
    }
    LRESULT(0)
}

pub(super) unsafe fn handle_rbutton_up(
    hwnd: HWND,
    rc: &Rc<RefCell<BoxInner>>,
    active_shell_menu: &Rc<RefCell<Option<ActiveShellMenu>>>,
    pending_shell_menu: &Rc<RefCell<Option<PreparedShellMenu>>>,
    lparam: LPARAM,
) -> LRESULT {
    enum RClick {
        None,
        FileMenu {
            screen_pt: POINT,
            paths: Vec<PathBuf>,
        },
        BoxMenu {
            screen_pt: POINT,
            light_glass: bool,
            ctx: BoxMenuContext,
        },
    }

    let taken_pending = pending_shell_menu
        .try_borrow_mut()
        .ok()
        .and_then(|mut guard| guard.take());

    let action = if let Ok(mut st) = rc.try_borrow_mut() {
        if st.is_edge_tucked() {
            return LRESULT(0);
        }
        let cx = ((lparam.0 & 0xFFFF) as i16) as i32;
        let cy = (((lparam.0 >> 16) & 0xFFFF) as i16) as i32;
        let mut screen_pt = POINT { x: cx, y: cy };
        let _ = ClientToScreen(hwnd, &mut screen_pt);
        let layout = st.current_layout();
        match layout.hit_test(cx as f32, cy as f32) {
            BoxHitZone::Item(idx) => {
                if !st.selected.contains(&idx) {
                    st.selected.clear();
                    st.selected.insert(idx);
                    st.selection_anchor = Some(idx);
                    let _ = st.render();
                }
                let mut indices: Vec<usize> = st.selected.iter().copied().collect();
                indices.sort_unstable();
                let paths: Vec<PathBuf> = indices
                    .iter()
                    .filter_map(|&i| st.items.get(i))
                    .map(|it| it.path.clone())
                    .collect();
                if paths.is_empty() {
                    RClick::None
                } else {
                    RClick::FileMenu { screen_pt, paths }
                }
            }
            BoxHitZone::HeaderDrag
            | BoxHitZone::BackButton
            | BoxHitZone::SearchClearButton
            | BoxHitZone::NewButton
            | BoxHitZone::SortButton
            | BoxHitZone::ViewButton
            | BoxHitZone::LockButton
            | BoxHitZone::CollapseButton
            | BoxHitZone::DeleteButton
            | BoxHitZone::ContentBlank => RClick::BoxMenu {
                screen_pt,
                light_glass: st.light_glass,
                ctx: st.menu_context(),
            },
            _ => RClick::None,
        }
    } else {
        RClick::None
    };

    match action {
        RClick::FileMenu { screen_pt, paths } => {
            let prepared = taken_pending
                .filter(|p| p.matches_paths(&paths))
                .or_else(|| prepare_native_file_context_menu(hwnd, &paths));
            let outcome = match prepared {
                Some(menu) => {
                    track_prepared_shell_menu(hwnd, screen_pt, menu, active_shell_menu)
                }
                None => ShellMenuOutcome::Cancelled,
            };
            if outcome == ShellMenuOutcome::ShellInvoked {
                if let Ok(mut st) = rc.try_borrow_mut() {
                    if !st.cleaned_up && st.spec.kind == BoxKind::FolderPortal {
                        st.reload_items();
                        let _ = st.render();
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
        RClick::BoxMenu {
            screen_pt,
            light_glass,
            ctx,
        } => {
            let menu_rc = rc.try_borrow().ok().map(|s| Rc::clone(&s.box_menu));
            if let Some(menu) = menu_rc {
                menu.show(hwnd, screen_pt, light_glass, BoxMenuKind::Full, ctx);
            }
            if let Ok(mut st) = rc.try_borrow_mut() {
                let _ = st.render();
            }
        }
        RClick::None => {}
    }
    LRESULT(0)
}
