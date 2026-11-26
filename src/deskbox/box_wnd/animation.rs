//! Collapse/hover spring physics, auto-hide header fade, edge drawer slide, and halo color animation.

use super::{
    BoxInner, HighResTimerGuard, HoverTarget, BOX_ANIM_INTERVAL_MS, COLLAPSE_EXPAND_DAMPING,
    COLLAPSE_EXPAND_STIFFNESS, COLLAPSE_RETRACT_DAMPING, COLLAPSE_RETRACT_STIFFNESS,
    EDGE_ANIM_DUR_S, TIMER_BOX_ANIM_ID, TIMER_HOVER_COLLAPSE_ID,
};
use crate::color::NEUTRAL_BACKDROP_FALLBACK;
use crate::deskbox::desktop::monitor_info_for_window;
use crate::deskbox::layout::{
    clamp_box_bounds_to_work_area, BoxHitZone, BOX_HEADER_H_DIP, BOX_MIN_HEIGHT_DIP,
    BOX_MIN_WIDTH_DIP,
};
use crate::deskbox::snap::{
    compute_edge_drawer_window_bounds, detect_dock_edge, resolve_elastic_auto_fit_resize, BoxRectPx,
};
use crate::system::resolve_unified_accent;
use crate::ui::{compute_halo_pair, ease_out_cubic, Spring1D, SHADOW_PAD_DIP};
use std::time::Instant;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, KillTimer, SetTimer, SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER,
    SWP_NOSENDCHANGING, SWP_NOSIZE, SWP_NOZORDER,
};

impl BoxInner {
    pub(super) fn sync_header_opacity_target(&mut self) {
        let target = if self.is_header_visible() { 1.0 } else { 0.0 };
        if !self.spec.auto_hide_header || self.is_visually_collapsed() {
            self.header_opacity_spring = Spring1D::still(1.0);
            self.header_opacity_animating = false;
        } else if (target - self.header_opacity_spring.pos).abs() > 0.01 {
            self.header_opacity_animating = true;
            self.start_anim_timer();
        }
    }

    pub(super) fn start_anim_timer(&mut self) {
        self.last_anim_tick = Instant::now();
        if !self.hwnd.is_invalid() {
            if self.high_res_timer.is_none() {
                self.high_res_timer = Some(HighResTimerGuard::acquire());
            }
            unsafe {
                SetTimer(self.hwnd, TIMER_BOX_ANIM_ID, BOX_ANIM_INTERVAL_MS, None);
            }
        }
    }

    fn step_collapse_spring_once(&mut self, dt: f32) -> bool {
        let s = (self.dpi.max(96) as f32) / 96.0;
        let pad = (SHADOW_PAD_DIP * s).round() as i32;
        let min_h_px = BOX_HEADER_H_DIP * s;
        let max_h_px = ((self.win_h - pad * 2).max(1) as f32).max(min_h_px);
        let target = self.collapse_anim_to_h.clamp(min_h_px, max_h_px);
        let expanding = target > self.collapse_height_spring.pos;
        let (stiffness, damping) = if expanding {
            (COLLAPSE_EXPAND_STIFFNESS, COLLAPSE_EXPAND_DAMPING)
        } else {
            (COLLAPSE_RETRACT_STIFFNESS, COLLAPSE_RETRACT_DAMPING)
        };
        self.collapse_height_spring
            .step(target, stiffness, damping, dt);
        self.collapse_height_spring.pos = self.collapse_height_spring.pos.clamp(min_h_px, max_h_px);
        if self.collapse_height_spring.is_settled(target, 0.55, 16.0) {
            self.collapse_height_spring = Spring1D::still(target);
            true
        } else {
            false
        }
    }

    fn begin_collapse_height_spring(&mut self) {
        let mon = monitor_info_for_window(self.hwnd);
        self.dpi = mon.dpi;
        let s = (self.dpi.max(96) as f32) / 96.0;
        let pad = (SHADOW_PAD_DIP * s).round() as i32;
        let margin = (6.0 * s).round() as i32;
        let min_h_px = BOX_HEADER_H_DIP * s;
        let was_animating = self.collapse_animating;
        let cur_card_h = if was_animating {
            self.collapse_height_spring.pos
        } else {
            (self.win_h - pad * 2).max(1) as f32
        };

        let is_temp_hover = self.spec.collapsed;
        let (cx, cy, cw, ch, wx, wy, ww, _) = clamp_box_bounds_to_work_area(
            self.spec.x_dip,
            self.spec.y_dip,
            self.spec.w_dip,
            self.spec.h_dip,
            self.is_visually_collapsed(),
            mon.work.left,
            mon.work.top,
            mon.work.right,
            mon.work.bottom,
            self.dpi,
        );
        self.spec.monitor_id = mon.monitor_id;
        // Temporary hover-expand on a collapsed box must NEVER overwrite persisted resting
        // coordinates (and locked boxes keep their exact origin even temporarily).
        let mut full_card_h = ch.max(BOX_MIN_HEIGHT_DIP) * s;
        if !is_temp_hover {
            self.spec.x_dip = cx;
            self.spec.y_dip = cy;
            self.spec.w_dip = cw;
            self.spec.h_dip = ch;
            self.win_x = wx;
            self.win_y = wy;
            self.win_w = ww;
        } else if self.spec.locked {
            let (_, _, _, _, rx, ry, rw, _) = clamp_box_bounds_to_work_area(
                self.spec.x_dip,
                self.spec.y_dip,
                self.spec.w_dip,
                self.spec.h_dip,
                true,
                mon.work.left,
                mon.work.top,
                mon.work.right,
                mon.work.bottom,
                self.dpi,
            );
            self.win_x = rx;
            self.win_y = ry;
            self.win_w = rw;
            let avail_h = ((mon.work.bottom - margin) - (ry + pad)) as f32;
            if avail_h >= min_h_px {
                full_card_h = full_card_h.min(avail_h).max(min_h_px);
            }
        } else {
            self.win_x = wx;
            self.win_y = wy;
            self.win_w = ww;
        }

        let target_h = if self.is_visually_collapsed() {
            min_h_px
        } else {
            full_card_h
        };
        // Pre-allocate the full animation window height once so intermediate frames never trigger
        // `SetWindowPos` resize or `CreateDIBSection` churn; transparent pixels outside `card_rect`
        // remain click-through via `WM_NCHITTEST`.
        let anim_win_h = (full_card_h.round() as i32) + pad * 2;
        if self.win_h < anim_win_h {
            self.win_h = anim_win_h;
        }

        self.collapse_anim_to_h = target_h;
        if !was_animating {
            self.collapse_height_spring = Spring1D::still(cur_card_h);
        }
        self.collapse_animating = (target_h - cur_card_h).abs() > 0.55
            || self.collapse_height_spring.vel.abs() > 16.0;
        if self.collapse_animating {
            // Advance a small immediate frame-0 impulse and pre-size the live blur window envelope
            // once so motion starts on millisecond 0 without waiting for the first timer tick.
            let settled = self.step_collapse_spring_once(0.008);
            if settled {
                self.collapse_animating = false;
            }
        }
        if self.collapse_animating {
            self.sync_blur_layer();
            self.start_anim_timer();
        } else {
            // Shrink window & blur envelope back to settled resting geometry whenever no animation
            // remains active (including fast reversals that settle on frame 0).
            let layout = self.current_layout();
            self.win_w = layout.win_w;
            self.win_h = layout.win_h;
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    self.win_x,
                    self.win_y,
                    self.win_w,
                    self.win_h,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING,
                );
            }
            self.sync_blur_layer();
            self.register_peer_rect();
        }
    }

    pub(super) fn set_hover_expanded(&mut self, expanded: bool) {
        self.set_hover_expanded_ex(expanded, true);
    }

    pub(super) fn set_hover_expanded_ex(&mut self, expanded: bool, render_now: bool) -> bool {
        if !self.spec.collapsed || self.spec.edge_auto_hide {
            self.hover_expanded = false;
            return false;
        }
        if self.hover_expanded == expanded {
            return false;
        }
        self.hover_expanded = expanded;
        if !expanded
            && matches!(
                self.hover,
                Some(HoverTarget::Item(_) | HoverTarget::ItemRemove(_))
            )
        {
            self.hover = None;
        }
        self.begin_collapse_height_spring();
        self.sync_header_opacity_target();
        if render_now {
            let _ = self.render();
        }
        true
    }

    pub(super) fn refresh_header_hover_from_cursor(&mut self) {
        self.cancel_pending_hover_menu();
        let mut pt = POINT::default();
        let cursor_in_client = unsafe {
            GetCursorPos(&mut pt).is_ok()
                && ScreenToClient(self.hwnd, &mut pt).as_bool()
                && !self.is_edge_tucked()
        };
        let layout = self.current_layout();
        let in_header = cursor_in_client && layout.header_rect.contains(pt.x as f32, pt.y as f32);
        let prev_hdr = self.header_hovered;
        self.header_hovered = in_header;
        if !in_header {
            self.header_revealed_at = None;
            self.suppress_hover_menu_btn = None;
        }
        let next_hover = if !in_header
            && matches!(
                self.hover,
                Some(
                    HoverTarget::BackButton
                        | HoverTarget::SearchClearButton
                        | HoverTarget::NewButton
                        | HoverTarget::SortButton
                        | HoverTarget::ViewButton
                        | HoverTarget::LockButton
                        | HoverTarget::CollapseButton
                        | HoverTarget::DeleteButton
                )
            ) {
            None
        } else {
            self.hover
        };
        if in_header {
            let cur_menu_target = match layout.hit_test(pt.x as f32, pt.y as f32) {
                BoxHitZone::NewButton => Some(HoverTarget::NewButton),
                BoxHitZone::SortButton => Some(HoverTarget::SortButton),
                BoxHitZone::ViewButton => Some(HoverTarget::ViewButton),
                _ => None,
            };
            if cur_menu_target.is_some() {
                self.suppress_hover_menu_btn = cur_menu_target;
            }
        }
        let hover_changed = next_hover != self.hover;
        self.hover = next_hover;
        if prev_hdr != self.header_hovered || hover_changed {
            self.sync_header_opacity_target();
            let _ = self.render();
        }
    }

    pub(super) fn toggle_collapsed(&mut self) {
        self.cancel_inline_rename();
        let was_edge_auto_hide = self.spec.edge_auto_hide;
        let next_collapsed = !self.spec.collapsed;
        let pre_chain = if !was_edge_auto_hide {
            self.settle_collapse_animation_now();
            let mon = monitor_info_for_window(self.hwnd);
            self.dpi = mon.dpi;
            self.spec.monitor_id = mon.monitor_id.clone();
            let snap = self.snapshot_monitor_chain_nodes(&mon.monitor_id);
            Some((mon, snap))
        } else {
            None
        };

        self.spec.collapsed = next_collapsed;
        if next_collapsed && self.spec.edge_auto_hide {
            self.spec.edge_auto_hide = false;
            self.edge_drawer_open = false;
            self.edge_animating = false;
            self.edge_slide_spring = Spring1D::still(1.0);
        }
        self.hover_expanded = false;
        self.suppress_hover_expand_until_leave = next_collapsed;
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_HOVER_COLLAPSE_ID);
        }
        // Permanently collapsing or expanding updates resting footprint and elastically pushes/pulls
        // neighboring unlocked boxes (temporary hover-expand never triggers this path).
        if let Some((mon, snapshot)) = pre_chain {
            let s = (self.dpi.max(96) as f32) / 96.0;
            let pad = (SHADOW_PAD_DIP * s).round() as i32;
            let margin = (6.0 * s).round() as i32;
            let target_w = (self.spec.w_dip.max(BOX_MIN_WIDTH_DIP) * s).round() as i32;
            let target_h = if next_collapsed {
                (BOX_HEADER_H_DIP * s).round() as i32
            } else {
                (self.spec.h_dip.max(BOX_MIN_HEIGHT_DIP) * s).round() as i32
            };
            let work =
                BoxRectPx::new(0, mon.work.left, mon.work.top, mon.work.right, mon.work.bottom);
            let res = resolve_elastic_auto_fit_resize(
                &snapshot,
                self.spec.id,
                target_w,
                target_h,
                work,
                self.dpi,
            );
            let new_wx = res.active_rect.left - pad;
            let new_wy = res.active_rect.top - pad;
            if new_wx != self.win_x || new_wy != self.win_y {
                self.win_x = new_wx;
                self.win_y = new_wy;
                self.spec.x_dip = ((res.active_rect.left - mon.work.left - margin) as f32 / s).max(0.0);
                self.spec.y_dip = ((res.active_rect.top - mon.work.top - margin) as f32 / s).max(0.0);
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        self.win_x,
                        self.win_y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                    );
                }
            }
            self.apply_sibling_chain_rects(&res.sibling_rects, mon.work, self.dpi);
        }
        self.begin_collapse_height_spring();
        self.sync_header_opacity_target();
        self.register_peer_rect();
        self.sync_desktop_zorder(true);
        self.persist_to_host();
        let _ = self.render();
    }

    pub(super) fn apply_edge_drawer_bounds(&mut self, slide_t: f32) {
        let mon = monitor_info_for_window(self.hwnd);
        self.dpi = mon.dpi;
        let (wx, wy, ww, wh) = compute_edge_drawer_window_bounds(
            self.edge_dock,
            self.spec.x_dip,
            self.spec.y_dip,
            self.spec.w_dip,
            self.spec.h_dip,
            mon.work.left,
            mon.work.top,
            mon.work.right,
            mon.work.bottom,
            mon.dpi,
            slide_t,
        );
        self.win_x = wx;
        self.win_y = wy;
        self.win_w = ww;
        self.win_h = wh;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                wx,
                wy,
                ww,
                wh,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.sync_blur_layer();
    }

    pub(super) fn set_edge_drawer_open(&mut self, open: bool) {
        if !self.spec.edge_auto_hide || self.edge_drawer_open == open {
            return;
        }
        self.edge_drawer_open = open;
        if !open {
            self.hover = None;
            self.header_hovered = false;
        }
        self.edge_anim_from_t = self.edge_slide_spring.pos.clamp(0.0, 1.0);
        self.edge_anim_to_t = if open { 1.0 } else { 0.0 };
        self.edge_anim_elapsed = 0.0;
        self.edge_animating = true;
        self.start_anim_timer();
    }

    pub(super) fn toggle_edge_auto_hide(&mut self) {
        self.cancel_inline_rename();
        let next = !self.spec.edge_auto_hide;
        self.spec.edge_auto_hide = next;
        self.fullscreen_suppressed = false;
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_HOVER_COLLAPSE_ID);
        }
        if next {
            self.spec.collapsed = false;
            self.hover_expanded = false;
            self.collapse_animating = false;
            let mon = monitor_info_for_window(self.hwnd);
            self.dpi = mon.dpi;
            let s = (mon.dpi.max(96) as f32) / 96.0;
            let work_w_dip = (mon.work.right - mon.work.left).max(260) as f32 / s;
            self.edge_dock = detect_dock_edge(self.spec.x_dip, self.spec.y_dip, self.spec.w_dip, work_w_dip);
            self.edge_drawer_open = true;
            self.edge_slide_spring = Spring1D::still(1.0);
            self.edge_anim_from_t = 1.0;
            self.edge_anim_to_t = 1.0;
            self.edge_anim_elapsed = EDGE_ANIM_DUR_S;
            self.edge_animating = false;
            self.suppress_edge_open_until_leave = false;
            self.apply_edge_drawer_bounds(1.0);
            self.unregister_peer_rect();
            self.sync_desktop_zorder(true);
        } else {
            self.edge_drawer_open = false;
            self.edge_animating = false;
            self.edge_slide_spring = Spring1D::still(1.0);
            self.edge_anim_from_t = 1.0;
            self.edge_anim_to_t = 1.0;
            self.edge_anim_elapsed = EDGE_ANIM_DUR_S;
            let mon = monitor_info_for_window(self.hwnd);
            self.dpi = mon.dpi;
            let (cx, cy, cw, ch, wx, wy, ww, wh) = clamp_box_bounds_to_work_area(
                self.spec.x_dip,
                self.spec.y_dip,
                self.spec.w_dip,
                self.spec.h_dip,
                self.is_visually_collapsed(),
                mon.work.left,
                mon.work.top,
                mon.work.right,
                mon.work.bottom,
                mon.dpi,
            );
            self.spec.x_dip = cx;
            self.spec.y_dip = cy;
            self.spec.w_dip = cw;
            self.spec.h_dip = ch;
            self.win_x = wx;
            self.win_y = wy;
            self.win_w = ww;
            self.win_h = wh;
            self.sync_desktop_zorder(true);
            self.register_peer_rect();
        }
        self.persist_to_host();
        let _ = self.render();
    }

    /// Immediately snaps any in-flight collapse/expand height animation to its target so
    /// interactive move/resize always starts from settled card dimensions rather than an envelope.
    pub(super) fn settle_collapse_animation_now(&mut self) {
        if !self.collapse_animating {
            return;
        }
        self.collapse_height_spring = Spring1D::still(self.collapse_anim_to_h);
        self.collapse_animating = false;
        let layout = self.current_layout();
        self.win_w = layout.win_w;
        self.win_h = layout.win_h;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                self.win_x,
                self.win_y,
                self.win_w,
                self.win_h,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING,
            );
        }
        self.sync_blur_layer();
        self.register_peer_rect();
    }

    pub(super) fn step_box_animations(&mut self) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_anim_tick)
            .as_secs_f32()
            .clamp(0.001, 0.035);
        self.last_anim_tick = now;

        let mut need_render = false;

        if self.collapse_animating {
            let settled = self.step_collapse_spring_once(dt);
            if settled {
                self.collapse_animating = false;
                let layout = self.current_layout();
                self.win_w = layout.win_w;
                self.win_h = layout.win_h;
                // `need_render = true` immediately calls `self.render()` -> `UpdateLayeredWindow`,
                // which atomically commits the settled `(win_w, win_h)` without an extra synchronous `SetWindowPos`.
                self.register_peer_rect();
            }
            self.sync_blur_layer();
            need_render = true;
        }

        if self.edge_animating && self.spec.edge_auto_hide {
            let was_tucked = self.is_edge_tucked();
            let prev_size = (self.win_w, self.win_h);
            self.edge_anim_elapsed += dt;
            let progress = (self.edge_anim_elapsed / EDGE_ANIM_DUR_S).clamp(0.0, 1.0);
            let eased = ease_out_cubic(progress);
            let cur_t = (self.edge_anim_from_t
                + (self.edge_anim_to_t - self.edge_anim_from_t) * eased)
                .clamp(0.0, 1.0);
            self.edge_slide_spring = Spring1D::still(cur_t);
            if progress >= 1.0 {
                self.edge_slide_spring = Spring1D::still(self.edge_anim_to_t);
                self.edge_animating = false;
            }
            self.apply_edge_drawer_bounds(self.edge_slide_spring.pos);
            let now_tucked = self.is_edge_tucked();
            let size_changed = (self.win_w, self.win_h) != prev_size;
            // Intermediate slide frames are pure window translation via `SetWindowPos`; skip full
            // D2D redraws unless crossing the tucked-tab boundary, resizing, or settling on the end frame.
            if was_tucked != now_tucked || size_changed || !self.edge_animating {
                need_render = true;
            }
        } else {
            self.edge_animating = false;
        }

        if self.scroll_animating {
            let max_scroll = self.max_scroll_px();
            let target = self.scroll_target_y.clamp(0.0, max_scroll);
            self.scroll_target_y = target;
            let diff = target - self.scroll_y;
            if diff.abs() <= 0.45 {
                self.scroll_y = target;
                self.scroll_spring = Spring1D::still(target);
                self.scroll_animating = false;
            } else {
                // Snappy exponential ease: zero overshoot, immediate wheel response, settles in ~130ms.
                let k = 1.0 - (-24.0 * dt).exp();
                self.scroll_y = (self.scroll_y + diff * k).clamp(0.0, max_scroll);
                self.scroll_spring = Spring1D::still(self.scroll_y);
            }
            need_render = true;
        }

        if self.header_opacity_animating {
            let target = if !self.spec.auto_hide_header || self.is_header_visible() {
                1.0
            } else {
                0.0
            };
            self.header_opacity_spring.step(target, 460.0, 38.0, dt);
            self.header_opacity_spring.pos = self.header_opacity_spring.pos.clamp(0.0, 1.0);
            if self.header_opacity_spring.is_settled(target, 0.008, 0.04) {
                self.header_opacity_spring = Spring1D::still(target);
                self.header_opacity_animating = false;
            }
            need_render = true;
        }

        if !self.collapse_animating
            && !self.edge_animating
            && !self.scroll_animating
            && !self.header_opacity_animating
            && !self.hwnd.is_invalid()
        {
            self.high_res_timer = None;
            unsafe {
                let _ = KillTimer(self.hwnd, TIMER_BOX_ANIM_ID);
            }
        }
        if need_render {
            let _ = self.render();
        }
    }

    pub(super) fn update_halo_colors(&mut self) -> bool {
        let (ui_rgb, base_dark) = resolve_unified_accent(NEUTRAL_BACKDROP_FALLBACK, self.light_glass);
        let pair = compute_halo_pair(base_dark, self.light_glass);
        let next_halo = [pair[0].to_normalized(), pair[1].to_normalized()];
        let next_accent = ui_rgb.to_normalized();
        let changed = (next_accent.0 - self.ui_accent.0).abs() > 0.002
            || (next_accent.1 - self.ui_accent.1).abs() > 0.002
            || (next_accent.2 - self.ui_accent.2).abs() > 0.002
            || (next_halo[0].0 - self.halo_colors[0].0).abs() > 0.002
            || (next_halo[0].1 - self.halo_colors[0].1).abs() > 0.002
            || (next_halo[0].2 - self.halo_colors[0].2).abs() > 0.002;
        self.halo_colors = next_halo;
        self.ui_accent = next_accent;
        changed
    }
}
