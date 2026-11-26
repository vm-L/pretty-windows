//! Collision detection and static non-overlapping placement/resize clamping for DeskBox.

use super::{BoxRectPx, SNAP_PEER_GAP_DIP, WORK_EDGE_MARGIN_DIP};
use crate::deskbox::layout::ResizeEdge;

/// Returns `true` when two physical-pixel card rectangles strictly overlap (touching edges do not overlap).
#[cfg_attr(not(windows), allow(dead_code))]
#[inline]
pub const fn rects_overlap_strict(a: BoxRectPx, b: BoxRectPx) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

pub(super) fn clamp_box_to_work(rect: BoxRectPx, work: BoxRectPx, margin: i32) -> BoxRectPx {
    let w = rect.width().max(1);
    let h = rect.height().max(1);
    let min_x = work.left + margin;
    let max_x = (work.right - margin - w).max(min_x);
    let min_y = work.top + margin;
    let max_y = (work.bottom - margin - h).max(min_y);
    let left = rect.left.clamp(min_x, max_x);
    let top = rect.top.clamp(min_y, max_y);
    BoxRectPx::new(rect.id, left, top, left + w, top + h)
}

pub(super) fn overlaps_any_peer(cand: BoxRectPx, peers: &[BoxRectPx]) -> bool {
    peers
        .iter()
        .any(|&p| p.id != cand.id && rects_overlap_strict(cand, p))
}

/// Resolves a moved/placed DeskBox rectangle so it never overlaps any other peer box in `peers`.
///
/// - If `desired` (clamped to `work`) already has zero overlap, returns it unchanged.
/// - Otherwise finds the nearest non-overlapping position within `work` (preferring a clean
///   `SNAP_PEER_GAP_DIP` gutter, falling back to `0px` flush and then a coarse work-area scan).
/// - If the monitor has no free space anywhere for this size, falls back to `fallback` (when valid).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn resolve_non_overlapping_move_rect(
    desired: BoxRectPx,
    fallback: BoxRectPx,
    peers: &[BoxRectPx],
    work: BoxRectPx,
    dpi: u32,
) -> BoxRectPx {
    let s = (dpi.max(96) as f32) / 96.0;
    let margin = (WORK_EDGE_MARGIN_DIP * s).round() as i32;
    let gap = (SNAP_PEER_GAP_DIP * s).round().max(2.0) as i32;

    let clamped = clamp_box_to_work(desired, work, margin);
    if !overlaps_any_peer(clamped, peers) {
        return clamped;
    }

    let w = clamped.width().max(1);
    let h = clamped.height().max(1);
    let min_x = work.left + margin;
    let max_x = (work.right - margin - w).max(min_x);
    let min_y = work.top + margin;
    let max_y = (work.bottom - margin - h).max(min_y);

    let active_peers: Vec<BoxRectPx> = peers
        .iter()
        .copied()
        .filter(|p| p.id != clamped.id && rects_overlap_strict(*p, work))
        .collect();
    if active_peers.is_empty() {
        return clamped;
    }

    for gap_try in [gap, 0] {
        let mut xs: Vec<i32> = Vec::with_capacity(active_peers.len() * 4 + 3);
        let mut ys: Vec<i32> = Vec::with_capacity(active_peers.len() * 4 + 3);
        xs.push(clamped.left);
        xs.push(min_x);
        xs.push(max_x);
        ys.push(clamped.top);
        ys.push(min_y);
        ys.push(max_y);
        for &p in &active_peers {
            xs.push((p.left - gap_try - w).clamp(min_x, max_x));
            xs.push((p.right + gap_try).clamp(min_x, max_x));
            xs.push(p.left.clamp(min_x, max_x));
            xs.push((p.right - w).clamp(min_x, max_x));
            ys.push((p.top - gap_try - h).clamp(min_y, max_y));
            ys.push((p.bottom + gap_try).clamp(min_y, max_y));
            ys.push(p.top.clamp(min_y, max_y));
            ys.push((p.bottom - h).clamp(min_y, max_y));
        }
        xs.sort_unstable();
        xs.dedup();
        ys.sort_unstable();
        ys.dedup();

        let mut best: Option<(i64, BoxRectPx)> = None;
        for &cx in &xs {
            for &cy in &ys {
                let cand = BoxRectPx::new(clamped.id, cx, cy, cx + w, cy + h);
                if overlaps_any_peer(cand, &active_peers) {
                    continue;
                }
                let dx = i64::from(cx - clamped.left);
                let dy = i64::from(cy - clamped.top);
                let dist2 = dx * dx + dy * dy;
                if best.is_none_or(|(b_d2, _)| dist2 < b_d2) {
                    best = Some((dist2, cand));
                }
            }
        }
        if let Some((_, rect)) = best {
            return rect;
        }
    }

    // Coarse grid sweep fallback for heavily packed desktops.
    let step = (24.0 * s).round().max(12.0) as i32;
    let mut best_grid: Option<(i64, BoxRectPx)> = None;
    let mut gy = min_y;
    loop {
        let mut gx = min_x;
        loop {
            let cand = BoxRectPx::new(clamped.id, gx, gy, gx + w, gy + h);
            if !overlaps_any_peer(cand, &active_peers) {
                let dx = i64::from(gx - clamped.left);
                let dy = i64::from(gy - clamped.top);
                let dist2 = dx * dx + dy * dy;
                if best_grid.is_none_or(|(b_d2, _)| dist2 < b_d2) {
                    best_grid = Some((dist2, cand));
                }
            }
            if gx >= max_x {
                break;
            }
            gx = (gx + step).min(max_x);
        }
        if gy >= max_y {
            break;
        }
        gy = (gy + step).min(max_y);
    }
    if let Some((_, rect)) = best_grid {
        return rect;
    }

    let fb = clamp_box_to_work(fallback, work, margin);
    if !overlaps_any_peer(fb, &active_peers) {
        fb
    } else {
        clamped
    }
}

/// Clamps the actively dragged edge(s) of a resizing box against peer boxes and monitor work bounds
/// so resizing never overlaps neighboring DeskBoxes.
#[allow(clippy::too_many_arguments, dead_code)]
pub fn clamp_resize_against_peers(
    resizing: BoxRectPx,
    orig: BoxRectPx,
    edge: ResizeEdge,
    peers: &[BoxRectPx],
    work: BoxRectPx,
    dpi: u32,
    min_w: i32,
    min_h: i32,
) -> BoxRectPx {
    let s = (dpi.max(96) as f32) / 96.0;
    let margin = (WORK_EDGE_MARGIN_DIP * s).round() as i32;
    let gap = (SNAP_PEER_GAP_DIP * s).round().max(2.0) as i32;

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

    let mut left = resizing.left;
    let mut top = resizing.top;
    let mut right = resizing.right;
    let mut bottom = resizing.bottom;

    if moves_left {
        left = left.max(work.left + margin).min(right - min_w);
    }
    if moves_right {
        right = right.min(work.right - margin).max(left + min_w);
    }
    if moves_top {
        top = top.max(work.top + margin).min(bottom - min_h);
    }
    if moves_bottom {
        bottom = bottom.min(work.bottom - margin).max(top + min_h);
    }

    // Pass 1: Clamp horizontal moving edge against peers intersecting the original vertical span.
    if moves_left || moves_right {
        for &p in peers {
            if p.id == resizing.id || !(orig.top < p.bottom && orig.bottom > p.top) {
                continue;
            }
            if moves_right && orig.right <= p.left {
                let limit = if orig.right <= p.left - gap {
                    p.left - gap
                } else {
                    p.left
                };
                if right > limit {
                    right = limit.max(left + min_w);
                }
            }
            if moves_left && orig.left >= p.right {
                let limit = if orig.left >= p.right + gap {
                    p.right + gap
                } else {
                    p.right
                };
                if left < limit {
                    left = limit.min(right - min_w);
                }
            }
        }
    }

    // Pass 2: Clamp vertical moving edge against peers intersecting the updated horizontal span.
    if moves_top || moves_bottom {
        for &p in peers {
            if p.id == resizing.id || !(left < p.right && right > p.left) {
                continue;
            }
            if moves_bottom && orig.bottom <= p.top {
                let limit = if orig.bottom <= p.top - gap {
                    p.top - gap
                } else {
                    p.top
                };
                if bottom > limit {
                    bottom = limit.max(top + min_h);
                }
            }
            if moves_top && orig.top >= p.bottom {
                let limit = if orig.top >= p.bottom + gap {
                    p.bottom + gap
                } else {
                    p.bottom
                };
                if top < limit {
                    top = limit.min(bottom - min_h);
                }
            }
        }
    }

    // Pass 3: Corner drag safety pass — if expanding both axes simultaneously still clips a diagonal peer,
    // clamp the horizontal axis as well.
    if (moves_left || moves_right) && (moves_top || moves_bottom) {
        for &p in peers {
            if p.id == resizing.id || !(top < p.bottom && bottom > p.top) {
                continue;
            }
            if moves_right && orig.right <= p.left {
                let limit = if orig.right <= p.left - gap {
                    p.left - gap
                } else {
                    p.left
                };
                if right > limit {
                    right = limit.max(left + min_w);
                }
            }
            if moves_left && orig.left >= p.right {
                let limit = if orig.left >= p.right + gap {
                    p.right + gap
                } else {
                    p.right
                };
                if left < limit {
                    left = limit.min(right - min_w);
                }
            }
        }
    }

    BoxRectPx::new(resizing.id, left, top, right, bottom)
}

/// Resolves mutual overlaps across a set of boxes on the same monitor in top-left reading order,
/// preserving relative positions whenever possible.
#[cfg_attr(not(windows), allow(dead_code))]
#[allow(dead_code)]
pub fn resolve_all_boxes_avoidance(rects: &mut [BoxRectPx], work: BoxRectPx, dpi: u32) {
    if rects.len() <= 1 {
        return;
    }
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by_key(|&i| (rects[i].top, rects[i].left, rects[i].id));

    let mut placed: Vec<BoxRectPx> = Vec::with_capacity(rects.len());
    for idx in order {
        let cur = rects[idx];
        let resolved = resolve_non_overlapping_move_rect(cur, cur, &placed, work, dpi);
        rects[idx] = resolved;
        placed.push(resolved);
    }
}

