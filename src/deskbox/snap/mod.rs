//! Cross-platform magnetic snap alignment, peer guide geometry, and edge auto-hide drawer math.

mod avoidance;
mod elastic;
#[cfg(test)]
mod tests;

pub use avoidance::*;
pub use elastic::*;

use super::layout::{ResizeEdge, BOX_HEADER_H_DIP, BOX_MIN_HEIGHT_DIP, BOX_MIN_WIDTH_DIP};
use crate::ui::SHADOW_PAD_DIP;

/// Magnetic snap activation distance (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const SNAP_THRESHOLD_DIP: f32 = 8.0;
/// Clean spacing gutter when snapping two DeskBox cards side-by-side or stacked (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const SNAP_PEER_GAP_DIP: f32 = 10.0;
/// Outer monitor work-area breathing margin (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const WORK_EDGE_MARGIN_DIP: f32 = 6.0;
/// Exposed thickness of a tucked edge auto-hide drawer handle (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const EDGE_TAB_THICKNESS_DIP: f32 = 8.0;
/// Maximum visible length of a tucked edge sensor handle (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const EDGE_TAB_MAX_LEN_DIP: f32 = 96.0;

/// Physical-pixel screen-space rectangle of a DeskBox card (excluding drop-shadow padding).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct BoxRectPx {
    pub id: u64,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl BoxRectPx {
    pub const fn new(id: u64, left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            id,
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn width(self) -> i32 {
        self.right - self.left
    }

    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub const fn center_x(self) -> i32 {
        (self.left + self.right) / 2
    }

    pub const fn center_y(self) -> i32 {
        (self.top + self.bottom) / 2
    }
}

/// A single screen-space alignment guide segment to draw during drag or resize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct SnapGuideLine {
    pub is_vertical: bool,
    pub pos: i32,
    pub start: i32,
    pub end: i32,
}

/// Snapped box rectangle together with active visual alignment guide lines.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct SnapResult {
    pub rect: BoxRectPx,
    pub guides: Vec<SnapGuideLine>,
}

/// Monitor screen edge to which an auto-hide DeskBox drawer docks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum DockEdge {
    #[default]
    Left,
    Right,
    Top,
}

/// Computes magnetic snap adjustments when moving a box card across a monitor work area.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn snap_move_rect(
    moving: BoxRectPx,
    peers: &[BoxRectPx],
    work: BoxRectPx,
    dpi: u32,
) -> SnapResult {
    let s = (dpi.max(96) as f32) / 96.0;
    let thr = (SNAP_THRESHOLD_DIP * s).round().max(4.0) as i32;
    let margin = (WORK_EDGE_MARGIN_DIP * s).round() as i32;
    let gap = (SNAP_PEER_GAP_DIP * s).round() as i32;

    let w = moving.width();
    let h = moving.height();

    // (abs_delta, signed_dx, guide_x, span_min_y, span_max_y)
    let mut best_x: Option<(i32, i32, i32, i32, i32)> = None;
    let mut try_x = |dx: i32, guide_x: i32, y0: i32, y1: i32| {
        let d = dx.abs();
        if d <= thr && best_x.is_none_or(|b| d < b.0) {
            best_x = Some((d, dx, guide_x, y0, y1));
        }
    };

    let work_left = work.left + margin;
    let work_right = work.right - margin;
    let work_cx = work.center_x();
    try_x(work_left - moving.left, work_left, work.top + margin, work.bottom - margin);
    try_x(work_right - moving.right, work_right, work.top + margin, work.bottom - margin);
    try_x(work_cx - moving.center_x(), work_cx, work.top + margin, work.bottom - margin);

    for &p in peers {
        if p.id == moving.id {
            continue;
        }
        let y0 = moving.top.min(p.top);
        let y1 = moving.bottom.max(p.bottom);
        try_x(p.left - moving.left, p.left, y0, y1);
        try_x(p.right - moving.right, p.right, y0, y1);
        try_x(p.center_x() - moving.center_x(), p.center_x(), y0, y1);
        try_x((p.right + gap) - moving.left, p.right + gap / 2, y0, y1);
        try_x((p.left - gap) - moving.right, p.left - gap / 2, y0, y1);
    }

    let mut best_y: Option<(i32, i32, i32, i32, i32)> = None;
    let mut try_y = |dy: i32, guide_y: i32, x0: i32, x1: i32| {
        let d = dy.abs();
        if d <= thr && best_y.is_none_or(|b| d < b.0) {
            best_y = Some((d, dy, guide_y, x0, x1));
        }
    };

    let work_top = work.top + margin;
    let work_bottom = work.bottom - margin;
    let work_cy = work.center_y();
    try_y(work_top - moving.top, work_top, work.left + margin, work.right - margin);
    try_y(work_bottom - moving.bottom, work_bottom, work.left + margin, work.right - margin);
    try_y(work_cy - moving.center_y(), work_cy, work.left + margin, work.right - margin);

    let snapped_left = moving.left + best_x.map_or(0, |b| b.1);
    let snapped_right = snapped_left + w;

    for &p in peers {
        if p.id == moving.id {
            continue;
        }
        let x0 = snapped_left.min(p.left);
        let x1 = snapped_right.max(p.right);
        try_y(p.top - moving.top, p.top, x0, x1);
        try_y(p.bottom - moving.bottom, p.bottom, x0, x1);
        try_y(p.center_y() - moving.center_y(), p.center_y(), x0, x1);
        try_y((p.bottom + gap) - moving.top, p.bottom + gap / 2, x0, x1);
        try_y((p.top - gap) - moving.bottom, p.top - gap / 2, x0, x1);
    }

    let snapped_top = moving.top + best_y.map_or(0, |b| b.1);
    let snapped_bottom = snapped_top + h;

    let mut guides = Vec::with_capacity(2);
    if let Some((_, _, gx, y0, y1)) = best_x {
        guides.push(SnapGuideLine {
            is_vertical: true,
            pos: gx,
            start: y0.min(snapped_top),
            end: y1.max(snapped_bottom),
        });
    }
    if let Some((_, _, gy, x0, x1)) = best_y {
        guides.push(SnapGuideLine {
            is_vertical: false,
            pos: gy,
            start: x0.min(snapped_left),
            end: x1.max(snapped_right),
        });
    }

    SnapResult {
        rect: BoxRectPx::new(
            moving.id,
            snapped_left,
            snapped_top,
            snapped_right,
            snapped_bottom,
        ),
        guides,
    }
}

/// Computes magnetic snap adjustments for the actively dragged edge(s) during a resize.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn snap_resize_rect(
    resizing: BoxRectPx,
    edge: ResizeEdge,
    peers: &[BoxRectPx],
    work: BoxRectPx,
    dpi: u32,
    min_w: i32,
    min_h: i32,
) -> SnapResult {
    let s = (dpi.max(96) as f32) / 96.0;
    let thr = (SNAP_THRESHOLD_DIP * s).round().max(4.0) as i32;
    let margin = (WORK_EDGE_MARGIN_DIP * s).round() as i32;
    let gap = (SNAP_PEER_GAP_DIP * s).round() as i32;

    let mut left = resizing.left;
    let mut top = resizing.top;
    let mut right = resizing.right;
    let mut bottom = resizing.bottom;
    let mut guides = Vec::with_capacity(2);

    let moves_left = matches!(
        edge,
        ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
    );
    let moves_right = matches!(
        edge,
        ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight
    );
    let moves_top = matches!(
        edge,
        ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
    );
    let moves_bottom = matches!(
        edge,
        ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight
    );

    if moves_left || moves_right {
        let mut best: Option<(i32, i32, i32, i32, i32)> = None;
        let mut consider = |target: i32, guide_x: i32, y0: i32, y1: i32| {
            let cur = if moves_left { left } else { right };
            let d = (target - cur).abs();
            let valid = if moves_left {
                right - target >= min_w
            } else {
                target - left >= min_w
            };
            if valid && d <= thr && best.is_none_or(|b| d < b.0) {
                best = Some((d, target, guide_x, y0, y1));
            }
        };

        if moves_left {
            consider(work.left + margin, work.left + margin, work.top + margin, work.bottom - margin);
        } else {
            consider(work.right - margin, work.right - margin, work.top + margin, work.bottom - margin);
        }

        for &p in peers {
            if p.id == resizing.id {
                continue;
            }
            let y0 = top.min(p.top);
            let y1 = bottom.max(p.bottom);
            if moves_left {
                consider(p.left, p.left, y0, y1);
                consider(p.right + gap, p.right + gap / 2, y0, y1);
            } else {
                consider(p.right, p.right, y0, y1);
                consider(p.left - gap, p.left - gap / 2, y0, y1);
            }
        }

        if let Some((_, target, gx, y0, y1)) = best {
            if moves_left {
                left = target;
            } else {
                right = target;
            }
            guides.push(SnapGuideLine {
                is_vertical: true,
                pos: gx,
                start: y0.min(top),
                end: y1.max(bottom),
            });
        }
    }

    if moves_top || moves_bottom {
        let mut best: Option<(i32, i32, i32, i32, i32)> = None;
        let mut consider = |target: i32, guide_y: i32, x0: i32, x1: i32| {
            let cur = if moves_top { top } else { bottom };
            let d = (target - cur).abs();
            let valid = if moves_top {
                bottom - target >= min_h
            } else {
                target - top >= min_h
            };
            if valid && d <= thr && best.is_none_or(|b| d < b.0) {
                best = Some((d, target, guide_y, x0, x1));
            }
        };

        if moves_top {
            consider(work.top + margin, work.top + margin, work.left + margin, work.right - margin);
        } else {
            consider(work.bottom - margin, work.bottom - margin, work.left + margin, work.right - margin);
        }

        for &p in peers {
            if p.id == resizing.id {
                continue;
            }
            let x0 = left.min(p.left);
            let x1 = right.max(p.right);
            if moves_top {
                consider(p.top, p.top, x0, x1);
                consider(p.bottom + gap, p.bottom + gap / 2, x0, x1);
            } else {
                consider(p.bottom, p.bottom, x0, x1);
                consider(p.top - gap, p.top - gap / 2, x0, x1);
            }
        }

        if let Some((_, target, gy, x0, x1)) = best {
            if moves_top {
                top = target;
            } else {
                bottom = target;
            }
            guides.push(SnapGuideLine {
                is_vertical: false,
                pos: gy,
                start: x0.min(left),
                end: x1.max(right),
            });
        }
    }

    SnapResult {
        rect: BoxRectPx::new(resizing.id, left, top, right, bottom),
        guides,
    }
}

/// Detects the closest dockable screen edge (`Left`, `Right`, or `Top`) for an auto-hide drawer.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn detect_dock_edge(x_dip: f32, y_dip: f32, w_dip: f32, work_w_dip: f32) -> DockEdge {
    let dist_left = x_dip.max(0.0);
    let dist_right = (work_w_dip - (x_dip + w_dip)).max(0.0);
    let dist_top = y_dip.max(0.0);

    if dist_top <= 24.0 && dist_top <= dist_left && dist_top <= dist_right {
        DockEdge::Top
    } else if dist_left <= dist_right && dist_left <= dist_top {
        DockEdge::Left
    } else if dist_right <= dist_top {
        DockEdge::Right
    } else {
        DockEdge::Top
    }
}

/// Physical-pixel window geometry `(win_x, win_y, win_w, win_h)` for an edge auto-hide drawer
/// at animation progress `slide_t` (`0.0` = tucked sensor tab, `1.0` = fully slid out).
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(windows), allow(dead_code))]
pub fn compute_edge_drawer_window_bounds(
    edge: DockEdge,
    x_dip: f32,
    y_dip: f32,
    w_dip: f32,
    h_dip: f32,
    work_left: i32,
    work_top: i32,
    work_right: i32,
    work_bottom: i32,
    dpi: u32,
    slide_t: f32,
) -> (i32, i32, i32, i32) {
    let s = (dpi.max(96) as f32) / 96.0;
    let pad = (SHADOW_PAD_DIP * s).round() as i32;
    let margin = (WORK_EDGE_MARGIN_DIP * s).round() as i32;
    let t = slide_t.clamp(0.0, 1.0);

    let full_w = (w_dip.max(BOX_MIN_WIDTH_DIP) * s).round() as i32;
    let full_h = (h_dip.max(BOX_MIN_HEIGHT_DIP) * s).round() as i32;
    let tab_thick = (EDGE_TAB_THICKNESS_DIP * s).round().max(6.0) as i32;
    let tab_max_len = (EDGE_TAB_MAX_LEN_DIP * s).round() as i32;

    let work_w = (work_right - work_left).max(full_w);
    let work_h = (work_bottom - work_top).max(full_h);

    let clamped_card_x = (work_left + margin + (x_dip * s).round() as i32)
        .clamp(work_left + margin, (work_right - margin - full_w).max(work_left + margin));
    let clamped_card_y = (work_top + margin + (y_dip * s).round() as i32)
        .clamp(work_top + margin, (work_bottom - margin - full_h).max(work_top + margin));

    if t <= 0.015 {
        // Compact edge sensor pill flush to the monitor edge.
        let (card_x, card_y, card_w, card_h) = match edge {
            DockEdge::Left => {
                let min_len = (BOX_HEADER_H_DIP * s).round() as i32;
                let max_len = tab_max_len.min(work_h).max(min_len);
                let len = (full_h * 45 / 100).clamp(min_len, max_len);
                let cy = clamped_card_y + full_h / 2;
                let min_top = work_top + margin;
                let top = (cy - len / 2).clamp(min_top, (work_bottom - margin - len).max(min_top));
                (work_left, top, tab_thick, len)
            }
            DockEdge::Right => {
                let min_len = (BOX_HEADER_H_DIP * s).round() as i32;
                let max_len = tab_max_len.min(work_h).max(min_len);
                let len = (full_h * 45 / 100).clamp(min_len, max_len);
                let cy = clamped_card_y + full_h / 2;
                let min_top = work_top + margin;
                let top = (cy - len / 2).clamp(min_top, (work_bottom - margin - len).max(min_top));
                (work_right - tab_thick, top, tab_thick, len)
            }
            DockEdge::Top => {
                let min_len = (BOX_HEADER_H_DIP * s).round() as i32;
                let max_len = tab_max_len.min(work_w).max(min_len);
                let len = (full_w * 45 / 100).clamp(min_len, max_len);
                let cx = clamped_card_x + full_w / 2;
                let min_left = work_left + margin;
                let left = (cx - len / 2).clamp(min_left, (work_right - margin - len).max(min_left));
                (left, work_top, len, tab_thick)
            }
        };
        return (card_x - pad, card_y - pad, card_w + pad * 2, card_h + pad * 2);
    }

    // Smoothly slide the full card out from behind the docked screen edge.
    let (card_x, card_y) = match edge {
        DockEdge::Left => {
            let start_x = work_left - full_w + tab_thick;
            let end_x = work_left + margin;
            let x = start_x + ((end_x - start_x) as f32 * t).round() as i32;
            (x, clamped_card_y)
        }
        DockEdge::Right => {
            let start_x = work_right - tab_thick;
            let end_x = work_right - margin - full_w;
            let x = start_x + ((end_x - start_x) as f32 * t).round() as i32;
            (x, clamped_card_y)
        }
        DockEdge::Top => {
            let start_y = work_top - full_h + tab_thick;
            let end_y = work_top + margin;
            let y = start_y + ((end_y - start_y) as f32 * t).round() as i32;
            (clamped_card_x, y)
        }
    };

    (card_x - pad, card_y - pad, full_w + pad * 2, full_h + pad * 2)
}

