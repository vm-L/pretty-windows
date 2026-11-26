//! Peer rectangle registry, elastic neighbor-chain layout sync, auto-fit sizing, and bounds persistence.

use super::{
    BoxInner, PeerBoxEntry, HOST_CMD_PERSIST_CONFIG, WM_APP_DESKBOX_HOST_CMD,
};
use crate::deskbox::config::{BoxKind, BoxViewMode};
use crate::deskbox::desktop::monitor_info_for_window;
use crate::deskbox::layout::{
    clamp_box_bounds_to_work_area, compact_list_min_width_for_label_dip, compute_auto_fit_size_dip,
    estimate_title_width_dip, header_min_width_for_title_dip, BOX_HEADER_H_DIP, BOX_MIN_HEIGHT_DIP,
    BOX_MIN_WIDTH_DIP,
};
use crate::deskbox::snap::{
    resolve_elastic_auto_fit_resize, BoxRectPx, ChainBoxNode,
};
use crate::ui::glass::measure_text_width;
use crate::ui::{Spring1D, SHADOW_PAD_DIP};
use std::cell::RefCell;
use std::rc::Weak;
use windows::Win32::Foundation::{LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSENDCHANGING, SWP_NOSIZE,
    SWP_NOZORDER,
};

impl BoxInner {
    pub(super) fn resting_card_rect_px_inner(&self) -> BoxRectPx {
        let s = (self.dpi.max(96) as f32) / 96.0;
        let pad = (SHADOW_PAD_DIP * s).round() as i32;
        let card_left = self.win_x + pad;
        let card_top = self.win_y + pad;
        let card_w = (self.win_w - pad * 2).max(1);
        // Always use the resting footprint (collapsed header bar when `spec.collapsed` is true)
        // so temporary hover-expand overlays never push away or disturb neighboring boxes.
        let card_h = if self.spec.collapsed {
            (BOX_HEADER_H_DIP * s).round() as i32
        } else {
            (self.spec.h_dip.max(BOX_MIN_HEIGHT_DIP) * s).round() as i32
        };
        BoxRectPx::new(
            self.spec.id,
            card_left,
            card_top,
            card_left + card_w,
            card_top + card_h,
        )
    }

    pub(super) fn peer_card_rects(&self) -> Vec<BoxRectPx> {
        self.peer_rects
            .try_borrow()
            .map(|m| m.values().map(|e| e.rect).collect())
            .unwrap_or_default()
    }

    pub(super) fn snapshot_monitor_chain_nodes(&self, monitor_id: &str) -> Vec<ChainBoxNode> {
        let mut nodes: Vec<ChainBoxNode> = self
            .peer_rects
            .try_borrow()
            .map(|m| {
                m.values()
                    .filter(|e| {
                        e.rect.id != self.spec.id
                            && (monitor_id.is_empty()
                                || e.monitor_id.is_empty()
                                || e.monitor_id == monitor_id)
                    })
                    .map(|e| ChainBoxNode {
                        rect: e.rect,
                        locked: e.locked,
                    })
                    .collect()
            })
            .unwrap_or_default();
        nodes.push(ChainBoxNode {
            rect: self.resting_card_rect_px_inner(),
            locked: self.spec.locked,
        });
        nodes
    }

    /// Synchronously translates a sibling box moved by an elastic neighbor-chain resize/reflow.
    pub(super) fn apply_chain_position_rect(&mut self, rect: BoxRectPx, work: RECT, dpi: u32) {
        if self.cleaned_up || self.spec.edge_auto_hide || self.spec.locked {
            return;
        }
        let s = (dpi.max(96) as f32) / 96.0;
        let pad = (SHADOW_PAD_DIP * s).round() as i32;
        let margin = (6.0 * s).round() as i32;
        let new_wx = rect.left - pad;
        let new_wy = rect.top - pad;
        if new_wx == self.win_x && new_wy == self.win_y {
            return;
        }
        self.win_x = new_wx;
        self.win_y = new_wy;
        self.spec.x_dip = ((rect.left - work.left - margin) as f32 / s).max(0.0);
        self.spec.y_dip = ((rect.top - work.top - margin) as f32 / s).max(0.0);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                new_wx,
                new_wy,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
        self.sync_blur_layer();
        self.register_peer_rect();
    }

    /// Applies chain-resolved sibling positions in the same frame, dropping the shared registry
    /// borrow before mutating siblings so sibling `register_peer_rect()` calls never collide.
    pub(super) fn apply_sibling_chain_rects(
        &self,
        sibling_rects: &[BoxRectPx],
        work: RECT,
        dpi: u32,
    ) {
        let updates: Vec<(Weak<RefCell<BoxInner>>, BoxRectPx)> = {
            let Ok(peers) = self.peer_rects.try_borrow() else {
                return;
            };
            sibling_rects
                .iter()
                .filter_map(|&r| {
                    if r.id == self.spec.id {
                        return None;
                    }
                    let entry = peers.get(&r.id)?;
                    if entry.locked || entry.rect == r {
                        return None;
                    }
                    Some((entry.inner.clone(), r))
                })
                .collect()
        };

        for (weak, target_rect) in updates {
            if let Some(sib_rc) = weak.upgrade() {
                if let Ok(mut sib) = sib_rc.try_borrow_mut() {
                    sib.apply_chain_position_rect(target_rect, work, dpi);
                }
            }
        }
    }

    pub(super) fn register_peer_rect(&self) {
        if let Ok(mut peers) = self.peer_rects.try_borrow_mut() {
            if self.spec.edge_auto_hide {
                peers.remove(&self.spec.id);
                return;
            }
            let rect = self.resting_card_rect_px_inner();
            peers.insert(
                self.spec.id,
                PeerBoxEntry {
                    rect,
                    locked: self.spec.locked,
                    monitor_id: self.spec.monitor_id.clone(),
                    inner: self.self_weak.clone(),
                },
            );
        }
    }

    pub(super) fn unregister_peer_rect(&self) {
        if let Ok(mut peers) = self.peer_rects.try_borrow_mut() {
            peers.remove(&self.spec.id);
        }
    }

    /// Measures the minimum card width (DIP) required to fit the full header title and all action buttons.
    pub(super) fn measure_header_min_width_dip(&mut self) -> f32 {
        let s = (self.dpi.max(96) as f32) / 96.0;
        let title = self.header_display_title();
        let title_w_dip = if let Ok((title_fmt, _, _, _, _)) = self.ensure_formats(s) {
            unsafe { measure_text_width(&self.dwrite_factory, &title, &title_fmt, 24.0 * s) }
                .map(|px| px / s)
                .unwrap_or_else(|| estimate_title_width_dip(&title))
        } else {
            estimate_title_width_dip(&title)
        };
        header_min_width_for_title_dip(title_w_dip, self.can_go_back())
    }

    /// Measures the preferred card width (DIP) for `CompactList` so item filenames fit cleanly
    /// without truncation (sampling the longest names when directories contain hundreds of files).
    pub(super) fn measure_compact_list_width_dip(&mut self) -> f32 {
        if self.spec.view_mode != BoxViewMode::CompactList || self.items.is_empty() {
            return 0.0;
        }
        let s = (self.dpi.max(96) as f32) / 96.0;
        let show_remove = self.spec.kind == BoxKind::Collection;
        let mut indices: Vec<usize> = (0..self.items.len()).collect();
        if indices.len() > 48 {
            indices.select_nth_unstable_by(48, |&a, &b| {
                self.items[b].name.len().cmp(&self.items[a].name.len())
            });
            indices.truncate(48);
        }
        let max_label_dip = if let Ok((_, _, _, list_fmt, _)) = self.ensure_formats(s) {
            let mut max_w = 0.0f32;
            for idx in indices {
                let name = &self.items[idx].name;
                let w = unsafe { measure_text_width(&self.dwrite_factory, name, &list_fmt, 27.0 * s) }
                    .map(|px| px / s)
                    .unwrap_or_else(|| estimate_title_width_dip(name));
                if w > max_w {
                    max_w = w;
                }
            }
            max_w
        } else {
            indices
                .into_iter()
                .map(|idx| estimate_title_width_dip(&self.items[idx].name))
                .fold(0.0f32, f32::max)
        };
        compact_list_min_width_for_label_dip(max_label_dip, show_remove)
    }

    pub(super) fn apply_resolved_auto_fit_rect(
        &mut self,
        resolved_rest: BoxRectPx,
        fit_w_dip: f32,
        fit_h_dip: f32,
        work: RECT,
        dpi: u32,
    ) {
        self.dpi = dpi;
        let s = (dpi.max(96) as f32) / 96.0;
        let pad = (SHADOW_PAD_DIP * s).round() as i32;
        let margin = (6.0 * s).round() as i32;

        let cw = (resolved_rest.width() as f32 / s).max(BOX_MIN_WIDTH_DIP);
        let ch = if self.spec.collapsed {
            fit_h_dip.max(BOX_MIN_HEIGHT_DIP)
        } else {
            (resolved_rest.height() as f32 / s).max(BOX_MIN_HEIGHT_DIP)
        };
        let wx = resolved_rest.left - pad;
        let wy = resolved_rest.top - pad;
        let ww = resolved_rest.width().max((BOX_MIN_WIDTH_DIP * s).round() as i32) + pad * 2;
        let vis_card_h = if self.is_visually_collapsed() {
            (BOX_HEADER_H_DIP * s).round() as i32
        } else {
            (ch * s).round() as i32
        };
        let wh = vis_card_h + pad * 2;

        self.spec.x_dip = ((resolved_rest.left - work.left - margin) as f32 / s).max(0.0);
        self.spec.y_dip = ((resolved_rest.top - work.top - margin) as f32 / s).max(0.0);
        self.spec.w_dip = cw.max(fit_w_dip.min(cw));
        self.spec.h_dip = ch;
        self.scroll_y = 0.0;
        self.scroll_target_y = 0.0;
        self.scroll_spring = Spring1D::still(0.0);
        self.scroll_animating = false;
        self.win_x = wx;
        self.win_y = wy;
        self.win_w = ww;
        self.win_h = wh;
        let cur_card_h = vis_card_h.max(1) as f32;
        self.collapse_height_spring = Spring1D::still(cur_card_h);
        self.collapse_anim_to_h = cur_card_h;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                wx,
                wy,
                ww,
                wh,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING,
            );
        }
        self.sync_blur_layer();
        self.register_peer_rect();
    }

    /// Resizes the box cleanly to display all items without scrollbars (bounded to monitor work area),
    /// reserving enough width for the header title + action buttons, preserving collapsed/hover-expand state,
    /// and elastically pushing/pulling unlocked neighbors so relative layout spacing stays intact.
    pub(super) fn auto_fit_size(&mut self) {
        if self.spec.locked {
            return;
        }
        self.cancel_inline_rename();
        self.settle_collapse_animation_now();
        let mon = monitor_info_for_window(self.hwnd);
        self.dpi = mon.dpi;
        let s = (self.dpi.max(96) as f32) / 96.0;
        let margin = (6.0 * s).round();
        let work_w_dip = ((mon.work.right - mon.work.left).max(260) as f32 - margin * 2.0) / s;
        let work_h_dip = ((mon.work.bottom - mon.work.top).max(220) as f32 - margin * 2.0) / s;
        let min_header_w_dip = self.measure_header_min_width_dip();
        let preferred_list_w_dip = self.measure_compact_list_width_dip();

        let (fit_w, fit_h) = compute_auto_fit_size_dip(
            self.items.len(),
            self.spec.view_mode,
            self.spec.auto_hide_header,
            self.spec.w_dip,
            min_header_w_dip,
            preferred_list_w_dip,
            work_w_dip,
            work_h_dip,
        );
        self.spec.monitor_id = mon.monitor_id.clone();

        if self.spec.edge_auto_hide {
            self.spec.w_dip = fit_w;
            self.spec.h_dip = fit_h;
            self.scroll_y = 0.0;
            self.scroll_target_y = 0.0;
            self.scroll_spring = Spring1D::still(0.0);
            self.scroll_animating = false;
            let slide_t = if self.edge_drawer_open { 1.0 } else { 0.0 };
            self.edge_animating = false;
            self.edge_slide_spring = Spring1D::still(slide_t);
            self.apply_edge_drawer_bounds(slide_t);
        } else {
            let snapshot = self.snapshot_monitor_chain_nodes(&mon.monitor_id);
            let target_rest_w = (fit_w.max(BOX_MIN_WIDTH_DIP) * s).round() as i32;
            let target_rest_h = if self.spec.collapsed {
                (BOX_HEADER_H_DIP * s).round() as i32
            } else {
                (fit_h.max(BOX_MIN_HEIGHT_DIP) * s).round() as i32
            };
            let work =
                BoxRectPx::new(0, mon.work.left, mon.work.top, mon.work.right, mon.work.bottom);
            let res = resolve_elastic_auto_fit_resize(
                &snapshot,
                self.spec.id,
                target_rest_w,
                target_rest_h,
                work,
                self.dpi,
            );
            self.apply_resolved_auto_fit_rect(res.active_rect, fit_w, fit_h, mon.work, self.dpi);
            self.apply_sibling_chain_rects(&res.sibling_rects, mon.work, self.dpi);
        }
        self.persist_to_host();
        let _ = self.render();
    }

    pub(super) fn persist_to_host(&self) {
        unsafe {
            if !self.host_hwnd.is_invalid() {
                let _ = PostMessageW(
                    self.host_hwnd,
                    WM_APP_DESKBOX_HOST_CMD,
                    WPARAM(HOST_CMD_PERSIST_CONFIG),
                    LPARAM(self.spec.id as isize),
                );
            }
        }
    }

    pub(super) fn update_spec_from_current_bounds(
        &mut self,
        card_left: i32,
        card_top: i32,
        card_w: i32,
        card_h: i32,
        preserve_dip_size: bool,
    ) {
        let old_s = (self.dpi.max(96) as f32) / 96.0;
        let mon = monitor_info_for_window(self.hwnd);
        self.dpi = mon.dpi;
        let s = (self.dpi.max(96) as f32) / 96.0;
        let margin = (6.0 * s).round();
        let raw_x_dip = (card_left - mon.work.left) as f32 - margin;
        let raw_y_dip = (card_top - mon.work.top) as f32 - margin;
        let x_dip = raw_x_dip / s;
        let y_dip = raw_y_dip / s;
        let w_dip = if preserve_dip_size {
            self.spec.w_dip.max(BOX_MIN_WIDTH_DIP)
        } else {
            (card_w as f32 / old_s).max(BOX_MIN_WIDTH_DIP)
        };
        let h_dip = if preserve_dip_size || self.is_visually_collapsed() {
            self.spec.h_dip.max(BOX_MIN_HEIGHT_DIP)
        } else {
            (card_h as f32 / old_s).max(BOX_MIN_HEIGHT_DIP)
        };

        let (cx, cy, cw, ch, wx, wy, ww, wh) = clamp_box_bounds_to_work_area(
            x_dip,
            y_dip,
            w_dip,
            h_dip,
            self.is_visually_collapsed(),
            mon.work.left,
            mon.work.top,
            mon.work.right,
            mon.work.bottom,
            self.dpi,
        );
        self.spec.monitor_id = mon.monitor_id;
        self.spec.x_dip = cx;
        self.spec.y_dip = cy;
        self.spec.w_dip = cw;
        self.spec.h_dip = ch;
        self.win_x = wx;
        self.win_y = wy;
        self.win_w = ww;
        self.win_h = wh;
        self.register_peer_rect();
    }
}
