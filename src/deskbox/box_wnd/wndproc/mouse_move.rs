//! Mouse move, hover/tooltip/drawer reveal, drag-move/resize/marquee/DnD-out, and capture loss.

use super::super::{
    BoxInner, BoxMenuContext, BoxMenuKind, DragAction, HoverTarget, TIMER_HOVER_MENU_ID,
    TIMER_TOOLTIP_ID,
};
use super::{
    cancel_hover_collapse_check, client_rect_to_screen, schedule_hover_collapse_check,
    HEADER_REVEAL_GRACE_MS, HOVER_MENU_DELAY_MS, TOOLTIP_HOVER_DELAY_MS,
};
use crate::color::Rgb;
use crate::deskbox::config::BoxKind;
use crate::deskbox::desktop::{monitor_info_at_point, monitor_info_for_window};
use crate::deskbox::dnd::start_file_drag_out;
use crate::deskbox::layout::{
    BoxHitZone, ResizeEdge, BOX_HEADER_H_DIP, BOX_MIN_HEIGHT_DIP, BOX_MIN_WIDTH_DIP,
};
use crate::deskbox::snap::{
    detect_dock_edge, elastic_drag_snap_peers, resolve_elastic_drag_resize,
    resolve_non_overlapping_move_rect, snap_move_rect, snap_resize_rect, BoxRectPx,
};
use crate::ui::SHADOW_PAD_DIP;
use std::cell::RefCell;
use std::mem::size_of;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_CONTROL, VK_MENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, KillTimer, SetTimer, SetWindowPos, SM_CXDRAG, SM_CYDRAG,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
};

pub(super) unsafe fn handle_capture_changed(hwnd: HWND, rc: &Rc<RefCell<BoxInner>>) -> LRESULT {
    let mut commit_swp: Option<(i32, i32, i32, i32)> = None;
    let mut want_hover_check = false;
    if let Ok(mut st) = rc.try_borrow_mut() {
        st.resize_chain_snapshot.clear();
        if let Some(drag) = st.drag.take() {
            if let Ok(mut overlay) = st.snap_overlay.try_borrow_mut() {
                overlay.hide();
            }
            if matches!(drag, DragAction::MoveBox { .. } | DragAction::Resize { .. }) {
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
                    let work_w_dip =
                        (mon.work.right - mon.work.left).max(260) as f32 / ms;
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
            }
            want_hover_check = (st.spec.collapsed && st.hover_expanded)
                || (st.spec.edge_auto_hide && st.edge_drawer_open);
            let _ = st.render();
        }
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

pub(super) unsafe fn handle_mouse_move(
    hwnd: HWND,
    rc: &Rc<RefCell<BoxInner>>,
    lparam: LPARAM,
) -> LRESULT {
    let mut drag_out_paths: Option<(Vec<PathBuf>, bool)> = None;
    let mut hover_menu_req: Option<(RECT, bool, BoxMenuKind, BoxMenuContext)> = None;
    let mut pending_drag_swp: Option<(i32, i32, i32, i32, bool)> = None;
    let mut pending_chain_siblings: Option<(Vec<BoxRectPx>, RECT, u32)> = None;
    if let Ok(mut st) = rc.try_borrow_mut() {
        if !st.tracking_leave {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            if TrackMouseEvent(&mut tme).is_ok() {
                st.tracking_leave = true;
            }
        }
        let cx = ((lparam.0 & 0xFFFF) as i16) as i32;
        let cy = (((lparam.0 >> 16) & 0xFFFF) as i16) as i32;
        let cur_pt = POINT { x: cx, y: cy };
        let mut pt_screen = cur_pt;
        let _ = ClientToScreen(hwnd, &mut pt_screen);

        match st.drag {
            Some(DragAction::MoveBox {
                start_screen,
                orig_win_x,
                orig_win_y,
            }) => {
                let s = (st.dpi.max(96) as f32) / 96.0;
                let pad = (SHADOW_PAD_DIP * s).round() as i32;
                let dx = pt_screen.x - start_screen.x;
                let dy = pt_screen.y - start_screen.y;
                let raw_wx = orig_win_x + dx;
                let raw_wy = orig_win_y + dy;
                let card_w = (st.win_w - pad * 2).max(1);
                let rest_card_h = if st.spec.collapsed {
                    (BOX_HEADER_H_DIP * s).round() as i32
                } else {
                    (st.spec.h_dip.max(BOX_MIN_HEIGHT_DIP) * s).round() as i32
                };
                let alt_down = GetKeyState(VK_MENU.0 as i32) < 0;

                let (new_wx, new_wy) = if !st.spec.edge_auto_hide {
                    let mon = monitor_info_at_point(pt_screen);
                    let work = BoxRectPx::new(
                        0,
                        mon.work.left,
                        mon.work.top,
                        mon.work.right,
                        mon.work.bottom,
                    );
                    let peers = st.peer_card_rects();
                    let raw_card = BoxRectPx::new(
                        st.spec.id,
                        raw_wx + pad,
                        raw_wy + pad,
                        raw_wx + pad + card_w,
                        raw_wy + pad + rest_card_h,
                    );
                    let prev_card = BoxRectPx::new(
                        st.spec.id,
                        st.win_x + pad,
                        st.win_y + pad,
                        st.win_x + pad + card_w,
                        st.win_y + pad + rest_card_h,
                    );
                    let (cand_card, snap_guides) = if !alt_down {
                        let snapped = snap_move_rect(raw_card, &peers, work, mon.dpi);
                        (snapped.rect, snapped.guides)
                    } else {
                        (raw_card, Vec::new())
                    };
                    let avoided = resolve_non_overlapping_move_rect(
                        cand_card, prev_card, &peers, work, mon.dpi,
                    );
                    if !snap_guides.is_empty() && avoided == cand_card {
                        let accent = Rgb::new(
                            (st.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
                            (st.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
                            (st.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
                        );
                        if let Ok(mut overlay) = st.snap_overlay.try_borrow_mut() {
                            overlay.show_guides(
                                mon.work,
                                mon.dpi,
                                &snap_guides,
                                accent,
                                st.light_glass,
                            );
                        }
                    } else if let Ok(mut overlay) = st.snap_overlay.try_borrow_mut() {
                        overlay.hide();
                    }
                    (avoided.left - pad, avoided.top - pad)
                } else {
                    if let Ok(mut overlay) = st.snap_overlay.try_borrow_mut() {
                        overlay.hide();
                    }
                    (raw_wx, raw_wy)
                };

                if new_wx != st.win_x || new_wy != st.win_y {
                    st.win_x = new_wx;
                    st.win_y = new_wy;
                    pending_drag_swp = Some((new_wx, new_wy, 0, 0, false));
                }
            }
            Some(DragAction::Resize {
                edge,
                start_screen,
                orig_card_left,
                orig_card_top,
                orig_card_right,
                orig_card_bottom,
            }) => {
                let s = (st.dpi.max(96) as f32) / 96.0;
                let pad = (SHADOW_PAD_DIP * s).round() as i32;
                let min_w = (BOX_MIN_WIDTH_DIP * s).round() as i32;
                let min_h = (BOX_MIN_HEIGHT_DIP * s).round() as i32;
                let dx = pt_screen.x - start_screen.x;
                let dy = pt_screen.y - start_screen.y;

                let mut cl = orig_card_left;
                let mut ct = orig_card_top;
                let mut cr = orig_card_right;
                let mut cb = orig_card_bottom;

                match edge {
                    ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft => {
                        cl = (orig_card_left + dx).min(orig_card_right - min_w);
                    }
                    ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight => {
                        cr = (orig_card_right + dx).max(orig_card_left + min_w);
                    }
                    ResizeEdge::Top | ResizeEdge::Bottom => {}
                }
                match edge {
                    ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight => {
                        ct = (orig_card_top + dy).min(orig_card_bottom - min_h);
                    }
                    ResizeEdge::Bottom
                    | ResizeEdge::BottomLeft
                    | ResizeEdge::BottomRight => {
                        cb = (orig_card_bottom + dy).max(orig_card_top + min_h);
                    }
                    ResizeEdge::Left | ResizeEdge::Right => {}
                }

                let alt_down = GetKeyState(VK_MENU.0 as i32) < 0;
                let mut chain_siblings: Vec<BoxRectPx> = Vec::new();
                let chain_work = st.resize_chain_work;
                let chain_dpi = st.resize_chain_dpi.max(96);
                if !st.spec.edge_auto_hide {
                    let work = BoxRectPx::new(
                        0,
                        chain_work.left,
                        chain_work.top,
                        chain_work.right,
                        chain_work.bottom,
                    );
                    let mut snap_guides = Vec::new();
                    if !alt_down {
                        let snap_peers = elastic_drag_snap_peers(
                            &st.resize_chain_snapshot,
                            st.spec.id,
                            edge,
                            chain_dpi,
                        );
                        let snapped = snap_resize_rect(
                            BoxRectPx::new(st.spec.id, cl, ct, cr, cb),
                            edge,
                            &snap_peers,
                            work,
                            chain_dpi,
                            min_w,
                            min_h,
                        );
                        cl = snapped.rect.left;
                        ct = snapped.rect.top;
                        cr = snapped.rect.right;
                        cb = snapped.rect.bottom;
                        snap_guides = snapped.guides;
                    }
                    let rest_cb = if st.spec.collapsed {
                        ct + (BOX_HEADER_H_DIP * s).round() as i32
                    } else {
                        cb
                    };
                    let chain_edge = if st.spec.collapsed {
                        match edge {
                            ResizeEdge::BottomLeft => Some(ResizeEdge::Left),
                            ResizeEdge::BottomRight => Some(ResizeEdge::Right),
                            ResizeEdge::Bottom => None,
                            other => Some(other),
                        }
                    } else {
                        Some(edge)
                    };
                    let pre_chain = BoxRectPx::new(st.spec.id, cl, ct, cr, rest_cb);
                    let resolved_active = if let Some(ce) = chain_edge {
                        let res = resolve_elastic_drag_resize(
                            &st.resize_chain_snapshot,
                            st.spec.id,
                            pre_chain,
                            ce,
                            work,
                            chain_dpi,
                            min_w,
                            if st.spec.collapsed {
                                (BOX_HEADER_H_DIP * s).round() as i32
                            } else {
                                min_h
                            },
                        );
                        chain_siblings = res.sibling_rects;
                        res.active_rect
                    } else {
                        pre_chain
                    };
                    cl = resolved_active.left;
                    ct = resolved_active.top;
                    cr = resolved_active.right;
                    if !st.spec.collapsed {
                        cb = resolved_active.bottom;
                    }
                    if !snap_guides.is_empty() && resolved_active == pre_chain {
                        let accent = Rgb::new(
                            (st.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
                            (st.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
                            (st.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
                        );
                        if let Ok(mut overlay) = st.snap_overlay.try_borrow_mut() {
                            overlay.show_guides(
                                chain_work,
                                chain_dpi,
                                &snap_guides,
                                accent,
                                st.light_glass,
                            );
                        }
                    } else if let Ok(mut overlay) = st.snap_overlay.try_borrow_mut() {
                        overlay.hide();
                    }
                } else if let Ok(mut overlay) = st.snap_overlay.try_borrow_mut() {
                    overlay.hide();
                }

                let new_wx = cl - pad;
                let new_wy = ct - pad;
                let new_ww = (cr - cl) + pad * 2;
                let new_wh = if st.is_visually_collapsed() {
                    st.win_h
                } else {
                    (cb - ct) + pad * 2
                };
                st.win_x = new_wx;
                st.win_y = new_wy;
                st.win_w = new_ww;
                st.win_h = new_wh;
                st.spec.w_dip = ((cr - cl) as f32 / s).max(BOX_MIN_WIDTH_DIP);
                if !st.is_visually_collapsed() {
                    st.spec.h_dip = ((cb - ct) as f32 / s).max(BOX_MIN_HEIGHT_DIP);
                }
                pending_drag_swp = Some((new_wx, new_wy, new_ww, new_wh, true));
                if !chain_siblings.is_empty() {
                    pending_chain_siblings = Some((chain_siblings, chain_work, chain_dpi));
                }
            }
            Some(DragAction::ScrollbarThumb { grab_offset_y }) => {
                let layout = st.current_layout();
                let track = layout.scrollbar_track_rect;
                let thumb = layout.scrollbar_thumb_rect;
                let movable = (track.height() - thumb.height()).max(1.0);
                let desired_top = (cy as f32 - grab_offset_y - track.top).clamp(0.0, movable);
                st.set_scroll_target((desired_top / movable) * layout.max_scroll, false);
                let _ = st.render();
            }
            Some(DragAction::PendingItemDrag {
                item_idx,
                start_client,
            }) => {
                let thr_x = GetSystemMetrics(SM_CXDRAG).max(5);
                let thr_y = GetSystemMetrics(SM_CYDRAG).max(5);
                if (cx - start_client.x).abs() >= thr_x
                    || (cy - start_client.y).abs() >= thr_y
                {
                    if !st.selected.contains(&item_idx) {
                        st.selected.clear();
                        st.selected.insert(item_idx);
                        st.selection_anchor = Some(item_idx);
                    }
                    let can_reorder = st.selected.len() <= 1
                        && !st.search_active
                        && st.items.len() > 1;
                    if can_reorder {
                        let layout = st.current_layout();
                        let target_idx = layout
                            .drop_target_index_at(cx as f32, cy as f32, st.items.len())
                            .unwrap_or(item_idx);
                        st.drag = Some(DragAction::ReorderItem {
                            from_idx: item_idx,
                            target_idx,
                        });
                        let _ = st.render();
                    } else {
                        st.drag = None;
                        let _ = ReleaseCapture();
                        let _ = st.render();
                        let mut indices: Vec<usize> = st.selected.iter().copied().collect();
                        indices.sort_unstable();
                        let paths: Vec<PathBuf> = indices
                            .into_iter()
                            .filter_map(|i| st.items.get(i))
                            .filter(|it| it.exists)
                            .map(|it| it.path.clone())
                            .collect();
                        if !paths.is_empty() {
                            drag_out_paths =
                                Some((paths, st.spec.kind == BoxKind::FolderPortal));
                        }
                    }
                }
            }
            Some(DragAction::ReorderItem {
                from_idx,
                target_idx,
            }) => {
                let layout = st.current_layout();
                if !layout.card_rect.contains(cx as f32, cy as f32) {
                    // Dragged outside the box card -> hand off to OLE drag-out
                    let allow_move = st.spec.kind == BoxKind::FolderPortal;
                    st.drag = None;
                    let _ = ReleaseCapture();
                    let _ = st.render();
                    if let Some(item) = st.items.get(from_idx) {
                        if item.exists {
                            drag_out_paths = Some((vec![item.path.clone()], allow_move));
                        }
                    }
                } else if let Some(new_target) =
                    layout.drop_target_index_at(cx as f32, cy as f32, st.items.len())
                {
                    if new_target != target_idx {
                        st.drag = Some(DragAction::ReorderItem {
                            from_idx,
                            target_idx: new_target,
                        });
                        let _ = st.render();
                    }
                }
            }
            Some(DragAction::Marquee { anchor_client, .. }) => {
                st.drag = Some(DragAction::Marquee {
                    anchor_client,
                    current_client: cur_pt,
                });
                let layout = st.current_layout();
                if let Some(mrect) = st.marquee_rect_in_content(layout.content_rect) {
                    let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                    if !ctrl {
                        st.selected.clear();
                    }
                    for cell in &layout.visible_cells {
                        if rects_intersect(cell.rect, mrect) {
                            st.selected.insert(cell.index);
                        }
                    }
                }
                let _ = st.render();
            }
            Some(DragAction::PendingRemoveClick { item_idx }) => {
                let layout = st.current_layout();
                let next_hover = if layout.hit_test(cx as f32, cy as f32)
                    == BoxHitZone::ItemRemove(item_idx)
                {
                    Some(HoverTarget::ItemRemove(item_idx))
                } else {
                    None
                };
                if next_hover != st.hover {
                    st.hover = next_hover;
                    let _ = st.render();
                }
            }
            None => {
                cancel_hover_collapse_check(hwnd);
                let was_header_hovered = st.header_hovered;
                let was_hover_expanded = st.hover_expanded;
                let was_edge_open = st.edge_drawer_open;
                if st.spec.edge_auto_hide {
                    if !st.edge_drawer_open && !st.suppress_edge_open_until_leave {
                        st.set_edge_drawer_open(true);
                    }
                    if st.is_edge_tucked() {
                        return LRESULT(0);
                    }
                }
                let mut layout = st.current_layout();
                let mut hover_expand_started = false;
                if !st.spec.edge_auto_hide
                    && st.spec.collapsed
                    && !st.hover_expanded
                    && !st.suppress_hover_expand_until_leave
                    && layout.card_rect.contains(cx as f32, cy as f32)
                {
                    hover_expand_started = st.set_hover_expanded_ex(true, false);
                    layout = st.current_layout();
                }
                let just_expanded = (!was_hover_expanded && st.hover_expanded)
                    || (!was_edge_open && st.edge_drawer_open);
                let in_header = layout.header_rect.contains(cx as f32, cy as f32);
                let header_changed = st.header_hovered != in_header;
                let header_just_revealed = !was_header_hovered && in_header;
                st.header_hovered = in_header;
                if header_just_revealed || just_expanded {
                    st.header_revealed_at = Some(Instant::now());
                } else if !in_header {
                    st.header_revealed_at = None;
                }
                if header_changed {
                    layout = st.current_layout();
                }
                let prev_hover = st.hover;
                let hit = layout.hit_test(cx as f32, cy as f32);
                let next_sb_hover =
                    matches!(hit, BoxHitZone::ScrollbarThumb | BoxHitZone::ScrollbarTrack);
                let sb_changed = st.scrollbar_hovered != next_sb_hover;
                st.scrollbar_hovered = next_sb_hover;
                let next_hover = match hit {
                    BoxHitZone::BackButton => Some(HoverTarget::BackButton),
                    BoxHitZone::SearchClearButton => Some(HoverTarget::SearchClearButton),
                    BoxHitZone::NewButton => Some(HoverTarget::NewButton),
                    BoxHitZone::SortButton => Some(HoverTarget::SortButton),
                    BoxHitZone::ViewButton => Some(HoverTarget::ViewButton),
                    BoxHitZone::LockButton => Some(HoverTarget::LockButton),
                    BoxHitZone::CollapseButton => Some(HoverTarget::CollapseButton),
                    BoxHitZone::DeleteButton => Some(HoverTarget::DeleteButton),
                    BoxHitZone::Item(idx) => Some(HoverTarget::Item(idx)),
                    BoxHitZone::ItemRemove(idx) => Some(HoverTarget::ItemRemove(idx)),
                    _ => None,
                };
                if st
                    .suppress_hover_menu_btn
                    .is_some_and(|sup| next_hover != Some(sup))
                {
                    st.suppress_hover_menu_btn = None;
                }
                let hover_menu_target =
                    next_hover.and_then(|h| BoxInner::hover_menu_spec(h, &layout));
                let in_reveal_grace = header_just_revealed
                    || just_expanded
                    || st.header_revealed_at.is_some_and(|t| {
                        t.elapsed() < Duration::from_millis(HEADER_REVEAL_GRACE_MS)
                    });
                if hover_menu_target.is_some()
                    && next_hover != prev_hover
                    && in_reveal_grace
                {
                    st.suppress_hover_menu_btn = next_hover;
                }
                if st.inline_edit_hwnd.is_invalid() && next_hover != prev_hover {
                    st.cancel_pending_hover_menu();
                    if let Some((menu_kind, btn_rect)) = hover_menu_target {
                        st.hide_tooltip();
                        let open_kind = st.open_menu_kind_for_self();
                        let header_menu_open = matches!(
                            open_kind,
                            Some(
                                BoxMenuKind::NewBox
                                    | BoxMenuKind::Sort
                                    | BoxMenuKind::ViewAndTools
                            )
                        );
                        let suppressed = st.suppress_hover_menu_btn == next_hover;
                        if !suppressed
                            && open_kind != Some(menu_kind)
                            && !matches!(
                                open_kind,
                                Some(BoxMenuKind::Full | BoxMenuKind::ConfirmDelete)
                            )
                        {
                            if header_menu_open {
                                hover_menu_req = Some((
                                    client_rect_to_screen(hwnd, btn_rect),
                                    st.light_glass,
                                    menu_kind,
                                    st.menu_context(),
                                ));
                            } else {
                                st.pending_hover_menu = next_hover;
                                SetTimer(
                                    hwnd,
                                    TIMER_HOVER_MENU_ID,
                                    HOVER_MENU_DELAY_MS,
                                    None,
                                );
                            }
                        }
                    } else if let Some((tip_rect, tip_text)) = next_hover
                        .and_then(|h| st.tooltip_spec_for_hover(h, &layout))
                    {
                        if st.open_menu_kind_for_self().is_none() {
                            let already_visible = st
                                .box_tooltip
                                .try_borrow()
                                .is_ok_and(|t| t.is_visible());
                            if already_visible {
                                let _ = KillTimer(hwnd, TIMER_TOOLTIP_ID);
                                let screen_r = client_rect_to_screen(hwnd, tip_rect);
                                if let Ok(mut tip) = st.box_tooltip.try_borrow_mut() {
                                    tip.show(hwnd, screen_r, tip_text, st.light_glass);
                                }
                            } else {
                                SetTimer(hwnd, TIMER_TOOLTIP_ID, TOOLTIP_HOVER_DELAY_MS, None);
                            }
                        } else {
                            st.hide_tooltip();
                        }
                    } else {
                        st.hide_tooltip();
                    }
                } else if next_hover != prev_hover {
                    st.cancel_pending_hover_menu();
                    if let Some((tip_rect, tip_text)) = next_hover
                        .and_then(|h| st.tooltip_spec_for_hover(h, &layout))
                    {
                        let already_visible = st
                            .box_tooltip
                            .try_borrow()
                            .is_ok_and(|t| t.is_visible());
                        if already_visible {
                            let _ = KillTimer(hwnd, TIMER_TOOLTIP_ID);
                            let screen_r = client_rect_to_screen(hwnd, tip_rect);
                            if let Ok(mut tip) = st.box_tooltip.try_borrow_mut() {
                                tip.show(hwnd, screen_r, tip_text, st.light_glass);
                            }
                        } else {
                            SetTimer(hwnd, TIMER_TOOLTIP_ID, TOOLTIP_HOVER_DELAY_MS, None);
                        }
                    } else {
                        st.hide_tooltip();
                    }
                }
                if header_changed {
                    st.sync_header_opacity_target();
                }
                if next_hover != st.hover || header_changed || sb_changed || hover_expand_started {
                    st.hover = next_hover;
                    // While a collapse/expand spring is actively ticking at 120 FPS, avoid
                    // redundant synchronous redraws on high-polling-rate mouse jitter; frame 0
                    // (`hover_expand_started`) still renders immediately with zero latency.
                    if hover_expand_started || !st.collapse_animating {
                        let _ = st.render();
                    }
                }
            }
        }
    }

    if let Some((wx, wy, ww, wh, is_resize)) = pending_drag_swp {
        // Execute SetWindowPos outside `rc.try_borrow_mut()` so synchronous cross-DPI
        // `WM_DPICHANGED` notifications can borrow `BoxInner` cleanly.
        let swp_flags = if is_resize {
            SWP_NOZORDER | SWP_NOACTIVATE
        } else {
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
        };
        let _ = SetWindowPos(hwnd, None, wx, wy, ww, wh, swp_flags);
        if let Ok(mut st) = rc.try_borrow_mut() {
            st.sync_blur_layer();
            st.register_peer_rect();
            if let Some((siblings, chain_work, chain_dpi)) = pending_chain_siblings {
                st.apply_sibling_chain_rects(&siblings, chain_work, chain_dpi);
            }
            if is_resize {
                let _ = st.render();
            }
        }
    }

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

    if let Some((paths, allow_move)) = drag_out_paths {
        let (_eff, dropped) = start_file_drag_out(&paths, allow_move);
        if dropped {
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
    LRESULT(0)
}

pub(super) unsafe fn handle_mouse_leave(hwnd: HWND, rc: &Rc<RefCell<BoxInner>>) -> LRESULT {
    if let Ok(mut st) = rc.try_borrow_mut() {
        st.hide_tooltip();
        st.cancel_pending_hover_menu();
        st.suppress_hover_menu_btn = None;
        st.header_revealed_at = None;
        st.tracking_leave = false;
        st.suppress_hover_expand_until_leave = false;
        st.suppress_edge_open_until_leave = false;
        let changed = st.hover.is_some() || st.header_hovered || st.scrollbar_hovered;
        st.hover = None;
        st.header_hovered = false;
        st.scrollbar_hovered = false;
        st.sync_header_opacity_target();
        if (st.spec.collapsed && st.hover_expanded)
            || (st.spec.edge_auto_hide && st.edge_drawer_open)
        {
            schedule_hover_collapse_check(hwnd);
        }
        if changed {
            let _ = st.render();
        }
    }
    LRESULT(0)
}

fn rects_intersect(a: crate::ui::RectF, b: crate::ui::RectF) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}
