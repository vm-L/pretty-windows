//! Elastic neighbor-chain push/pull engine for manual drag-resize and auto-fit reflow.

use super::avoidance::{clamp_box_to_work, overlaps_any_peer, resolve_non_overlapping_move_rect};
use super::{BoxRectPx, SNAP_PEER_GAP_DIP, SNAP_THRESHOLD_DIP, WORK_EDGE_MARGIN_DIP};
use crate::deskbox::layout::ResizeEdge;

/// Snapshot node of a DeskBox card participating in elastic neighbor-chain layout resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct ChainBoxNode {
    pub rect: BoxRectPx,
    pub locked: bool,
}

/// Output of an elastic neighbor-chain resize or reflow operation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct ChainResizeResult {
    pub active_rect: BoxRectPx,
    pub sibling_rects: Vec<BoxRectPx>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChainAxisDir {
    PosX,
    NegX,
    PosY,
    NegY,
}

impl ChainAxisDir {
    #[inline]
    const fn opposite(self) -> Self {
        match self {
            Self::PosX => Self::NegX,
            Self::NegX => Self::PosX,
            Self::PosY => Self::NegY,
            Self::NegY => Self::PosY,
        }
    }

    #[inline]
    const fn front(self, r: BoxRectPx) -> i32 {
        match self {
            Self::PosX => r.right,
            Self::NegX => -r.left,
            Self::PosY => r.bottom,
            Self::NegY => -r.top,
        }
    }

    #[inline]
    const fn back(self, r: BoxRectPx) -> i32 {
        match self {
            Self::PosX => r.left,
            Self::NegX => -r.right,
            Self::PosY => r.top,
            Self::NegY => -r.bottom,
        }
    }

    #[inline]
    const fn perp_span(self, r: BoxRectPx) -> (i32, i32) {
        match self {
            Self::PosX | Self::NegX => (r.top, r.bottom),
            Self::PosY | Self::NegY => (r.left, r.right),
        }
    }

    #[inline]
    fn perp_overlap(self, a: BoxRectPx, b: BoxRectPx) -> i32 {
        let (a0, a1) = self.perp_span(a);
        let (b0, b1) = self.perp_span(b);
        (a1.min(b1) - a0.max(b0)).max(0)
    }

    #[inline]
    fn work_front_limit(self, work: BoxRectPx, margin: i32) -> i32 {
        match self {
            Self::PosX => work.right - margin,
            Self::NegX => -(work.left + margin),
            Self::PosY => work.bottom - margin,
            Self::NegY => -(work.top + margin),
        }
    }

    #[inline]
    fn work_back_limit(self, work: BoxRectPx, margin: i32) -> i32 {
        match self {
            Self::PosX => work.left + margin,
            Self::NegX => -(work.right - margin),
            Self::PosY => work.top + margin,
            Self::NegY => -(work.bottom - margin),
        }
    }

    #[inline]
    fn translate(self, r: BoxRectPx, delta: i32) -> BoxRectPx {
        match self {
            Self::PosX => BoxRectPx::new(r.id, r.left + delta, r.top, r.right + delta, r.bottom),
            Self::NegX => BoxRectPx::new(r.id, r.left - delta, r.top, r.right - delta, r.bottom),
            Self::PosY => BoxRectPx::new(r.id, r.left, r.top + delta, r.right, r.bottom + delta),
            Self::NegY => BoxRectPx::new(r.id, r.left, r.top - delta, r.right, r.bottom - delta),
        }
    }

    #[inline]
    fn extend_front(self, r: BoxRectPx, delta: i32) -> BoxRectPx {
        match self {
            Self::PosX => BoxRectPx::new(r.id, r.left, r.top, r.right + delta, r.bottom),
            Self::NegX => BoxRectPx::new(r.id, r.left - delta, r.top, r.right, r.bottom),
            Self::PosY => BoxRectPx::new(r.id, r.left, r.top, r.right, r.bottom + delta),
            Self::NegY => BoxRectPx::new(r.id, r.left, r.top - delta, r.right, r.bottom),
        }
    }
}

#[inline]
fn chain_metrics(dpi: u32) -> (i32, i32, i32, i32) {
    let s = (dpi.max(96) as f32) / 96.0;
    let margin = (WORK_EDGE_MARGIN_DIP * s).round() as i32;
    let gap = (SNAP_PEER_GAP_DIP * s).round().max(2.0) as i32;
    let adj_tol = ((SNAP_PEER_GAP_DIP + SNAP_THRESHOLD_DIP) * s)
        .round()
        .max(8.0) as i32;
    let min_perp = (12.0 * s).round().max(4.0) as i32;
    (margin, gap, adj_tol, min_perp)
}

/// Pushes `active_idx` (expanding its leading edge if `!shift_active_whole`, or translating the
/// whole active card if `shift_active_whole`) along `dir` by up to `delta > 0`, cascading through
/// unlocked neighbors while stopping cleanly at monitor margins and locked boxes.
/// Returns the actual applied distance `0..=delta`.
#[allow(clippy::too_many_arguments)]
fn apply_axis_push(
    nodes: &mut [ChainBoxNode],
    active_idx: usize,
    dir: ChainAxisDir,
    delta: i32,
    shift_active_whole: bool,
    work: BoxRectPx,
    margin: i32,
    gap: i32,
    adj_tol: i32,
) -> i32 {
    if delta <= 0 || active_idx >= nodes.len() {
        return 0;
    }
    if shift_active_whole && nodes[active_idx].locked {
        return 0;
    }

    let n = nodes.len();
    let work_limit = dir.work_front_limit(work, margin);

    // Strictly topological order: since every box has front > back, back(j) >= front(k) implies
    // front(j) > front(k). Processing descending `front` evaluates every downstream box `j` first.
    let mut desc_order: Vec<usize> = (0..n).collect();
    desc_order.sort_by(|&a, &b| {
        dir.front(nodes[b].rect)
            .cmp(&dir.front(nodes[a].rect))
            .then_with(|| dir.back(nodes[b].rect).cmp(&dir.back(nodes[a].rect)))
    });

    let mut max_shift = vec![0i32; n];
    for &k in &desc_order {
        if k != active_idx && nodes[k].locked {
            max_shift[k] = 0;
            continue;
        }
        let fk = dir.front(nodes[k].rect);
        let mut limit = (work_limit - fk).max(0);
        for j in 0..n {
            if j == k || dir.perp_overlap(nodes[k].rect, nodes[j].rect) <= 0 {
                continue;
            }
            let g = dir.back(nodes[j].rect) - fk;
            if g < 0 {
                continue;
            }
            let keep_gap = if nodes[j].locked {
                gap.min(g)
            } else if g <= adj_tol {
                g
            } else {
                gap.min(g)
            };
            let free_gap = (g - keep_gap).max(0);
            limit = limit.min(free_gap.saturating_add(max_shift[j]));
        }
        max_shift[k] = limit.max(0);
    }

    let applied = delta.min(max_shift[active_idx]).max(0);
    if applied == 0 {
        return 0;
    }

    // Forward propagation in strictly ascending `front` order.
    let mut shifts = vec![0i32; n];
    shifts[active_idx] = applied;
    for &k in desc_order.iter().rev() {
        let sk = shifts[k];
        if sk <= 0 {
            continue;
        }
        let fk = dir.front(nodes[k].rect);
        for j in 0..n {
            if j == k || j == active_idx || nodes[j].locked {
                continue;
            }
            if dir.perp_overlap(nodes[k].rect, nodes[j].rect) <= 0 {
                continue;
            }
            let g = dir.back(nodes[j].rect) - fk;
            if g < 0 {
                continue;
            }
            let keep_gap = if g <= adj_tol { g } else { gap.min(g) };
            let free_gap = (g - keep_gap).max(0);
            let req = (sk - free_gap).max(0);
            if req > shifts[j] {
                shifts[j] = req;
            }
        }
    }

    if shift_active_whole {
        nodes[active_idx].rect = dir.translate(nodes[active_idx].rect, applied);
    } else {
        nodes[active_idx].rect = dir.extend_front(nodes[active_idx].rect, applied);
    }
    for j in 0..n {
        if j != active_idx && shifts[j] > 0 {
            nodes[j].rect = dir.translate(nodes[j].rect, shifts[j]);
        }
    }
    applied
}

/// Shrinks `active_idx`'s leading edge inward along `dir` by `delta > 0` and pulls back the
/// contiguous unlocked adjacent neighbor chain to preserve original spacing without creating
/// overlaps against stationary or locked boxes.
#[allow(clippy::too_many_arguments)]
fn apply_axis_pull(
    nodes: &mut [ChainBoxNode],
    active_idx: usize,
    dir: ChainAxisDir,
    delta: i32,
    work: BoxRectPx,
    margin: i32,
    gap: i32,
    adj_tol: i32,
    min_perp: i32,
) {
    if delta <= 0 || active_idx >= nodes.len() {
        return;
    }

    let n = nodes.len();
    let work_back_limit = dir.work_back_limit(work, margin);

    // Ascending `front` order is strictly topological for upstream relations `front(u) <= back(j)`.
    let mut asc_order: Vec<usize> = (0..n).collect();
    asc_order.sort_by(|&a, &b| {
        dir.front(nodes[a].rect)
            .cmp(&dir.front(nodes[b].rect))
            .then_with(|| dir.back(nodes[a].rect).cmp(&dir.back(nodes[b].rect)))
    });

    // Step 1: Identify contiguous unlocked chain originally adjacent to `active_idx`.
    let mut in_chain = vec![false; n];
    in_chain[active_idx] = true;
    for &j in &asc_order {
        if j == active_idx || nodes[j].locked {
            continue;
        }
        let bj = dir.back(nodes[j].rect);
        let (j0, j1) = dir.perp_span(nodes[j].rect);
        let j_span = (j1 - j0).max(1);
        let attached = (0..n).any(|u| {
            if !in_chain[u] {
                return false;
            }
            let g = bj - dir.front(nodes[u].rect);
            if g < 0 || g > adj_tol {
                return false;
            }
            let (u0, u1) = dir.perp_span(nodes[u].rect);
            let req_ov = min_perp.min(j_span.min((u1 - u0).max(1)));
            dir.perp_overlap(nodes[u].rect, nodes[j].rect) >= req_ov
        });
        if attached {
            in_chain[j] = true;
        }
    }

    // Step 2: Forward pull-distance DP respecting every upstream box (in-chain and stationary).
    let mut pull = vec![0i32; n];
    pull[active_idx] = delta;
    for &j in &asc_order {
        if j == active_idx || !in_chain[j] {
            continue;
        }
        let bj = dir.back(nodes[j].rect);
        let mut best_pull = delta.min((bj - work_back_limit).max(0));
        for u in 0..n {
            if u == j || dir.perp_overlap(nodes[u].rect, nodes[j].rect) <= 0 {
                continue;
            }
            let g = bj - dir.front(nodes[u].rect);
            if g < 0 {
                continue;
            }
            let keep_gap = if g <= adj_tol { g } else { gap.min(g) };
            let avail = pull[u].saturating_add((g - keep_gap).max(0));
            best_pull = best_pull.min(avail.max(0));
        }
        pull[j] = best_pull.max(0);
    }

    nodes[active_idx].rect = dir.extend_front(nodes[active_idx].rect, -delta);
    for j in 0..n {
        if j != active_idx && pull[j] > 0 {
            nodes[j].rect = dir.translate(nodes[j].rect, -pull[j]);
        }
    }
}

/// Filters `snapshot` peers for magnetic snap during elastic drag-resize: keeps all peers except
/// unlocked peers that are already immediately adjacent in the actively dragged direction, so
/// pushing an adjacent neighbor starts smoothly from the very first pixel without sticky snap-back.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn elastic_drag_snap_peers(
    snapshot: &[ChainBoxNode],
    active_id: u64,
    edge: ResizeEdge,
    dpi: u32,
) -> Vec<BoxRectPx> {
    let (_, _, adj_tol, _) = chain_metrics(dpi);
    let Some(active) = snapshot.iter().find(|n| n.rect.id == active_id) else {
        return snapshot.iter().map(|n| n.rect).collect();
    };
    let a = active.rect;
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

    snapshot
        .iter()
        .filter(|n| {
            if n.rect.id == active_id || n.locked {
                return true;
            }
            let p = n.rect;
            let adj_r = moves_right
                && ChainAxisDir::PosX.perp_overlap(a, p) > 0
                && p.left >= a.right
                && p.left - a.right <= adj_tol;
            let adj_l = moves_left
                && ChainAxisDir::NegX.perp_overlap(a, p) > 0
                && a.left >= p.right
                && a.left - p.right <= adj_tol;
            let adj_b = moves_bottom
                && ChainAxisDir::PosY.perp_overlap(a, p) > 0
                && p.top >= a.bottom
                && p.top - a.bottom <= adj_tol;
            let adj_t = moves_top
                && ChainAxisDir::NegY.perp_overlap(a, p) > 0
                && a.top >= p.bottom
                && a.top - p.bottom <= adj_tol;
            !(adj_r || adj_l || adj_b || adj_t)
        })
        .map(|n| n.rect)
        .collect()
}

/// Resolves live manual drag-resize against a monitor-wide snapshot captured at `WM_LBUTTONDOWN`.
///
/// - Expanding an edge pushes unlocked neighbor chains along the expansion direction while
///   preserving original adjacent spacing (or `SNAP_PEER_GAP_DIP` upon contacting distant boxes).
/// - Shrinking an edge pulls back the contiguous unlocked adjacent chain to keep spacing intact.
/// - Locked boxes and monitor work-area margins act as hard walls, clamping the dragged edge
///   smoothly with zero overlap and zero drift when dragging back and forth.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(windows), allow(dead_code))]
pub fn resolve_elastic_drag_resize(
    snapshot: &[ChainBoxNode],
    active_id: u64,
    desired_active: BoxRectPx,
    edge: ResizeEdge,
    work: BoxRectPx,
    dpi: u32,
    min_w: i32,
    min_h: i32,
) -> ChainResizeResult {
    let (margin, gap, adj_tol, min_perp) = chain_metrics(dpi);
    let mut nodes: Vec<ChainBoxNode> = snapshot.to_vec();
    let Some(active_idx) = nodes.iter().position(|n| n.rect.id == active_id) else {
        return ChainResizeResult {
            active_rect: clamp_box_to_work(desired_active, work, margin),
            sibling_rects: Vec::new(),
        };
    };

    let orig = nodes[active_idx].rect;
    let h_op = match edge {
        ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight => {
            let clamped_r = desired_active.right.max(orig.left + min_w);
            Some((ChainAxisDir::PosX, clamped_r - orig.right))
        }
        ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft => {
            let clamped_l = desired_active.left.min(orig.right - min_w);
            Some((ChainAxisDir::NegX, orig.left - clamped_l))
        }
        ResizeEdge::Top | ResizeEdge::Bottom => None,
    };
    let v_op = match edge {
        ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight => {
            let clamped_b = desired_active.bottom.max(orig.top + min_h);
            Some((ChainAxisDir::PosY, clamped_b - orig.bottom))
        }
        ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight => {
            let clamped_t = desired_active.top.min(orig.bottom - min_h);
            Some((ChainAxisDir::NegY, orig.top - clamped_t))
        }
        ResizeEdge::Left | ResizeEdge::Right => None,
    };

    // Process shrinking axis before growing axis so freed clearance is immediately available.
    let mut ops: Vec<(ChainAxisDir, i32)> = [h_op, v_op].into_iter().flatten().collect();
    ops.sort_by_key(|&(_, delta)| if delta < 0 { 0 } else { 1 });

    for (dir, delta) in ops {
        if delta > 0 {
            let _ = apply_axis_push(
                &mut nodes,
                active_idx,
                dir,
                delta,
                false,
                work,
                margin,
                gap,
                adj_tol,
            );
        } else if delta < 0 {
            apply_axis_pull(
                &mut nodes,
                active_idx,
                dir,
                -delta,
                work,
                margin,
                gap,
                adj_tol,
                min_perp,
            );
        }
    }

    let active_rect = nodes[active_idx].rect;
    let sibling_rects = nodes
        .into_iter()
        .enumerate()
        .filter(|&(i, _)| i != active_idx)
        .map(|(_, n)| n.rect)
        .collect();

    ChainResizeResult {
        active_rect,
        sibling_rects,
    }
}

/// Resolves a programmatic size change (`auto_fit_size` or permanent `toggle_collapsed`) on
/// `active_id` to `(target_w, target_h)` using elastic neighbor push/pull, bidirectional reverse
/// shifting when one side hits a screen edge or locked box, and lock-respecting fallback relocation
/// only when total bidirectional space is insufficient.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn resolve_elastic_auto_fit_resize(
    snapshot: &[ChainBoxNode],
    active_id: u64,
    target_w: i32,
    target_h: i32,
    work: BoxRectPx,
    dpi: u32,
) -> ChainResizeResult {
    let (margin, gap, adj_tol, min_perp) = chain_metrics(dpi);
    let max_w = (work.width() - margin * 2).max(1);
    let max_h = (work.height() - margin * 2).max(1);
    let tw = target_w.clamp(1, max_w);
    let th = target_h.clamp(1, max_h);

    let mut nodes: Vec<ChainBoxNode> = snapshot.to_vec();
    let Some(active_idx) = nodes.iter().position(|n| n.rect.id == active_id) else {
        let fallback = BoxRectPx::new(
            active_id,
            work.left + margin,
            work.top + margin,
            work.left + margin + tw,
            work.top + margin + th,
        );
        return ChainResizeResult {
            active_rect: fallback,
            sibling_rects: Vec::new(),
        };
    };

    let orig = nodes[active_idx].rect;
    let dw = tw - orig.width();
    let dh = th - orig.height();

    let mut ops = [(ChainAxisDir::PosX, dw), (ChainAxisDir::PosY, dh)];
    if dh < 0 && dw > 0 {
        ops.swap(0, 1);
    }

    let mut had_shortfall = false;
    for (dir, delta) in ops {
        if delta < 0 {
            apply_axis_pull(
                &mut nodes,
                active_idx,
                dir,
                -delta,
                work,
                margin,
                gap,
                adj_tol,
                min_perp,
            );
        } else if delta > 0 {
            let applied = apply_axis_push(
                &mut nodes,
                active_idx,
                dir,
                delta,
                false,
                work,
                margin,
                gap,
                adj_tol,
            );
            let rem = delta - applied;
            if rem > 0 {
                let shifted_back = if !nodes[active_idx].locked {
                    let sb = apply_axis_push(
                        &mut nodes,
                        active_idx,
                        dir.opposite(),
                        rem,
                        true,
                        work,
                        margin,
                        gap,
                        adj_tol,
                    );
                    if sb > 0 {
                        nodes[active_idx].rect = dir.extend_front(nodes[active_idx].rect, sb);
                    }
                    sb
                } else {
                    0
                };
                if applied + shifted_back < delta {
                    had_shortfall = true;
                }
            }
        }
    }

    // If bidirectional slack was insufficient (e.g. boxed between locked walls), apply the full
    // target size and relocate only the overflowing unlocked box(es) with zero overlap.
    // Never shift a locked box or force it to overlap another locked box.
    if had_shortfall && !nodes[active_idx].locked {
        let cur = nodes[active_idx].rect;
        let forced = clamp_box_to_work(
            BoxRectPx::new(cur.id, cur.left, cur.top, cur.left + tw, cur.top + th),
            work,
            margin,
        );
        nodes[active_idx].rect = forced;

        // Collect immovable locked anchors first.
        let mut placed: Vec<BoxRectPx> = nodes
            .iter()
            .enumerate()
            .filter(|&(i, n)| n.locked && i != active_idx)
            .map(|(_, n)| n.rect)
            .collect();

        if !nodes[active_idx].locked {
            let resolved_active = resolve_non_overlapping_move_rect(
                nodes[active_idx].rect,
                nodes[active_idx].rect,
                &placed,
                work,
                dpi,
            );
            nodes[active_idx].rect = resolved_active;
        }
        placed.push(nodes[active_idx].rect);

        // Keep non-overlapping unlocked siblings verbatim; relocate only those that overlap.
        let mut unlocked_indices: Vec<usize> = (0..nodes.len())
            .filter(|&i| i != active_idx && !nodes[i].locked)
            .collect();
        unlocked_indices.sort_by_key(|&i| {
            let overlaps = overlaps_any_peer(nodes[i].rect, &placed);
            (overlaps as u8, nodes[i].rect.top, nodes[i].rect.left, nodes[i].rect.id)
        });
        for i in unlocked_indices {
            let cur_sib = nodes[i].rect;
            let resolved = resolve_non_overlapping_move_rect(cur_sib, cur_sib, &placed, work, dpi);
            nodes[i].rect = resolved;
            placed.push(resolved);
        }
    }

    let active_rect = nodes[active_idx].rect;
    let sibling_rects = nodes
        .into_iter()
        .enumerate()
        .filter(|&(i, _)| i != active_idx)
        .map(|(_, n)| n.rect)
        .collect();

    ChainResizeResult {
        active_rect,
        sibling_rects,
    }
}

/// Reflows a monitor group of DeskBoxes during batch `AutoFitAll`, applying each box's target
/// resting `(width, height)` in top-left reading order with elastic push/pull chain preservation.
/// Returns the resolved `BoxRectPx` slice aligned 1:1 with `initial_nodes`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn reflow_batch_auto_fit_chain(
    initial_nodes: &[ChainBoxNode],
    target_sizes: &[(u64, i32, i32)],
    work: BoxRectPx,
    dpi: u32,
) -> Vec<BoxRectPx> {
    if initial_nodes.is_empty() {
        return Vec::new();
    }
    let mut current: Vec<ChainBoxNode> = initial_nodes.to_vec();
    let mut order: Vec<usize> = (0..current.len()).collect();
    order.sort_by_key(|&i| (current[i].rect.top, current[i].rect.left, current[i].rect.id));

    for idx in order {
        let id = current[idx].rect.id;
        let Some(&(_, tw, th)) = target_sizes.iter().find(|&&(tid, _, _)| tid == id) else {
            continue;
        };
        let res = resolve_elastic_auto_fit_resize(&current, id, tw, th, work, dpi);
        current[idx].rect = res.active_rect;
        for sr in res.sibling_rects {
            if let Some(node) = current.iter_mut().find(|n| n.rect.id == sr.id) {
                node.rect = sr;
            }
        }
    }

    current.into_iter().map(|n| n.rect).collect()
}

