use super::*;

#[test]
fn snap_move_aligns_to_work_area_and_peer_edges_with_guides() {
    let work = BoxRectPx::new(0, 0, 0, 1920, 1032);
    let peer = BoxRectPx::new(1, 100, 80, 440, 380);

    // Near work-area left margin (6px) & peer top (80px)
    let moving = BoxRectPx::new(2, 10, 84, 350, 384);
    let res = snap_move_rect(moving, &[peer], work, 96);
    assert_eq!(res.rect.left, 6);
    assert_eq!(res.rect.top, 80);
    assert_eq!(res.guides.len(), 2);
    assert!(res.guides.iter().any(|g| g.is_vertical && g.pos == 6));
    assert!(res.guides.iter().any(|g| !g.is_vertical && g.pos == 80));

    // Near peer right + 10px gutter (450px)
    let beside = BoxRectPx::new(2, 447, 120, 787, 420);
    let res2 = snap_move_rect(beside, &[peer], work, 96);
    assert_eq!(res2.rect.left, 450);
    assert_eq!(res2.rect.top, 120);
    assert_eq!(res2.guides.len(), 1);
    assert!(res2.guides[0].is_vertical);
}

#[test]
fn snap_resize_respects_min_size_and_snaps_active_edges() {
    let work = BoxRectPx::new(0, 0, 0, 1920, 1032);
    let peer = BoxRectPx::new(1, 500, 80, 840, 400);

    let resizing = BoxRectPx::new(2, 100, 80, 487, 396);
    let res = snap_resize_rect(resizing, ResizeEdge::BottomRight, &[peer], work, 96, 220, 180);
    // Right snaps to peer.left - 10 = 490; bottom snaps to peer.bottom = 400.
    assert_eq!(res.rect.right, 490);
    assert_eq!(res.rect.bottom, 400);
    assert_eq!(res.guides.len(), 2);
}

#[test]
fn detect_dock_edge_and_drawer_bounds_handle_all_edges() {
    assert_eq!(detect_dock_edge(12.0, 200.0, 340.0, 1920.0), DockEdge::Left);
    assert_eq!(detect_dock_edge(1560.0, 200.0, 340.0, 1920.0), DockEdge::Right);
    assert_eq!(detect_dock_edge(600.0, 10.0, 340.0, 1920.0), DockEdge::Top);

    let pad = SHADOW_PAD_DIP as i32;
    let (tx, _ty, tw, th) = compute_edge_drawer_window_bounds(
        DockEdge::Right,
        1560.0,
        200.0,
        340.0,
        300.0,
        0,
        0,
        1920,
        1032,
        96,
        0.0,
    );
    assert_eq!(tx + pad + (tw - pad * 2), 1920);
    assert_eq!(tw - pad * 2, EDGE_TAB_THICKNESS_DIP as i32);
    assert!(th > pad * 2);

    let (ox, _oy, ow, oh) = compute_edge_drawer_window_bounds(
        DockEdge::Right,
        1560.0,
        200.0,
        340.0,
        300.0,
        0,
        0,
        1920,
        1032,
        96,
        1.0,
    );
    assert_eq!(ow - pad * 2, 340);
    assert_eq!(oh - pad * 2, 300);
    assert_eq!(ox + pad + 340, 1920 - WORK_EDGE_MARGIN_DIP as i32);
}

#[test]
fn negative_origin_monitor_snap_and_edge_drawer_stay_in_bounds() {
    // Secondary monitor positioned left of primary: [-1920, 0) x [0, 1040) @ 144 DPI
    let work = BoxRectPx::new(0, -1920, 0, 0, 1040);
    let peer = BoxRectPx::new(1, -1800, 120, -1400, 460);
    let moving = BoxRectPx::new(2, -1386, 123, -1046, 463);
    let res = snap_move_rect(moving, &[peer], work, 144);
    // 10 DIP gap at 144 DPI = 15px -> peer.right + 15 = -1385, peer.top = 120
    assert_eq!(res.rect.left, -1385);
    assert_eq!(res.rect.top, 120);
    assert_eq!(res.guides.len(), 2);

    let pad = (SHADOW_PAD_DIP * 1.5).round() as i32;
    let (tx, _ty, tw, _th) = compute_edge_drawer_window_bounds(
        DockEdge::Left,
        10.0,
        100.0,
        340.0,
        300.0,
        -1920,
        0,
        0,
        1040,
        144,
        0.0,
    );
    assert_eq!(tx + pad, -1920);
    assert_eq!(tw - pad * 2, (EDGE_TAB_THICKNESS_DIP * 1.5).round() as i32);

    let (rx, _ry, rw, _rh) = compute_edge_drawer_window_bounds(
        DockEdge::Right,
        900.0,
        100.0,
        340.0,
        300.0,
        -1920,
        0,
        0,
        1040,
        144,
        1.0,
    );
    let margin = (WORK_EDGE_MARGIN_DIP * 1.5).round() as i32;
    assert_eq!(rx + rw - pad, -margin);
}

#[test]
fn move_avoidance_pushes_box_cleanly_around_peers_without_overlap() {
    let work = BoxRectPx::new(0, 0, 0, 1920, 1032);
    let peer = BoxRectPx::new(1, 400, 200, 740, 500);

    // Moving box overlaps peer's left side -> stops flush at peer.left - gap
    let desired = BoxRectPx::new(2, 160, 220, 500, 520);
    let fallback = BoxRectPx::new(2, 20, 220, 360, 520);
    let resolved = resolve_non_overlapping_move_rect(desired, fallback, &[peer], work, 96);
    assert!(!rects_overlap_strict(resolved, peer));
    assert_eq!(resolved.right, peer.left - SNAP_PEER_GAP_DIP as i32);
    assert_eq!(resolved.width(), desired.width());
    assert_eq!(resolved.height(), desired.height());

    // Moving box dragged past peer's center -> hops to peer's right side
    let desired_r = BoxRectPx::new(2, 600, 220, 940, 520);
    let resolved_r = resolve_non_overlapping_move_rect(desired_r, fallback, &[peer], work, 96);
    assert!(!rects_overlap_strict(resolved_r, peer));
    assert_eq!(resolved_r.left, peer.right + SNAP_PEER_GAP_DIP as i32);

    // Non-overlapping desired position is kept verbatim
    let clear = BoxRectPx::new(2, 800, 200, 1140, 500);
    assert_eq!(
        resolve_non_overlapping_move_rect(clear, fallback, &[peer], work, 96),
        clear
    );
}

#[test]
fn resize_avoidance_clamps_active_edge_at_neighbor_boundary() {
    let work = BoxRectPx::new(0, 0, 0, 1920, 1032);
    let peer_r = BoxRectPx::new(1, 500, 100, 840, 420);
    let peer_b = BoxRectPx::new(3, 100, 460, 440, 760);

    let orig = BoxRectPx::new(2, 100, 100, 440, 400);
    let raw_drag = BoxRectPx::new(2, 100, 100, 620, 560);
    let clamped = clamp_resize_against_peers(
        raw_drag,
        orig,
        ResizeEdge::BottomRight,
        &[peer_r, peer_b],
        work,
        96,
        60,
        48,
    );
    assert!(!rects_overlap_strict(clamped, peer_r));
    assert!(!rects_overlap_strict(clamped, peer_b));
    assert_eq!(clamped.right, peer_r.left - SNAP_PEER_GAP_DIP as i32);
    assert_eq!(clamped.bottom, peer_b.top - SNAP_PEER_GAP_DIP as i32);
}

#[test]
fn batch_avoidance_separates_overlapping_boxes_cleanly() {
    let work = BoxRectPx::new(0, 0, 0, 1920, 1032);
    let mut rects = vec![
        BoxRectPx::new(1, 100, 100, 440, 400),
        BoxRectPx::new(2, 200, 150, 540, 450),
        BoxRectPx::new(3, 250, 200, 590, 500),
    ];
    resolve_all_boxes_avoidance(&mut rects, work, 96);
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            assert!(
                !rects_overlap_strict(rects[i], rects[j]),
                "boxes {} and {} still overlap: {:?} vs {:?}",
                rects[i].id,
                rects[j].id,
                rects[i],
                rects[j]
            );
        }
    }
}

#[test]
fn elastic_drag_grow_and_shrink_preserves_exact_neighbor_chain_spacing() {
    let work = BoxRectPx::new(0, 0, 0, 1920, 1032);
    // Three boxes in a row with custom 12px and 10px adjacent gaps
    let snap = vec![
        ChainBoxNode {
            rect: BoxRectPx::new(1, 100, 100, 300, 360),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(2, 312, 100, 512, 360),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(3, 522, 100, 722, 360),
            locked: false,
        },
    ];

    // Grow box 1 right edge by +80px -> pushes 2 and 3 right by +80px preserving exact 12px & 10px gaps
    let desired_grow = BoxRectPx::new(1, 100, 100, 380, 360);
    let grown = resolve_elastic_drag_resize(
        &snap,
        1,
        desired_grow,
        ResizeEdge::Right,
        work,
        96,
        60,
        48,
    );
    assert_eq!(grown.active_rect, desired_grow);
    assert_eq!(grown.sibling_rects[0], BoxRectPx::new(2, 392, 100, 592, 360));
    assert_eq!(grown.sibling_rects[1], BoxRectPx::new(3, 602, 100, 802, 360));

    // Shrink box 1 right edge by -60px -> pulls 2 and 3 left by -60px preserving exact 12px & 10px gaps
    let desired_shrink = BoxRectPx::new(1, 100, 100, 240, 360);
    let shrunk = resolve_elastic_drag_resize(
        &snap,
        1,
        desired_shrink,
        ResizeEdge::Right,
        work,
        96,
        60,
        48,
    );
    assert_eq!(shrunk.active_rect, desired_shrink);
    assert_eq!(shrunk.sibling_rects[0], BoxRectPx::new(2, 252, 100, 452, 360));
    assert_eq!(shrunk.sibling_rects[1], BoxRectPx::new(3, 462, 100, 662, 360));

    // Dragging back to initial rect restores exact original pixels with zero drift
    let restored = resolve_elastic_drag_resize(
        &snap,
        1,
        snap[0].rect,
        ResizeEdge::Right,
        work,
        96,
        60,
        48,
    );
    assert_eq!(restored.active_rect, snap[0].rect);
    assert_eq!(restored.sibling_rects[0], snap[1].rect);
    assert_eq!(restored.sibling_rects[1], snap[2].rect);
}

#[test]
fn elastic_drag_clamps_cleanly_at_locked_wall_and_screen_edge() {
    let work = BoxRectPx::new(0, 0, 0, 1000, 800);
    // Box 1 -> unlocked Box 2 (10px gap) -> locked Box 3 (30px gap)
    let snap = vec![
        ChainBoxNode {
            rect: BoxRectPx::new(1, 100, 100, 300, 360),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(2, 310, 100, 510, 360),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(3, 540, 100, 740, 360),
            locked: true,
        },
    ];

    // Box 2 has 30 - 10 = 20px slack before hitting locked Box 3's 10px margin.
    // Dragging Box 1 right by +120px must clamp at +20px (320), pushing Box 2 to 330..530, leaving Box 3 unmoved.
    let desired = BoxRectPx::new(1, 100, 100, 420, 360);
    let res =
        resolve_elastic_drag_resize(&snap, 1, desired, ResizeEdge::Right, work, 96, 60, 48);
    assert_eq!(res.active_rect, BoxRectPx::new(1, 100, 100, 320, 360));
    assert_eq!(res.sibling_rects[0], BoxRectPx::new(2, 330, 100, 530, 360));
    assert_eq!(res.sibling_rects[1], snap[2].rect);

    // Locked Box 2 directly next to Box 1 must neither push nor pull
    let locked_next = vec![
        ChainBoxNode {
            rect: BoxRectPx::new(1, 100, 100, 300, 360),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(2, 310, 100, 510, 360),
            locked: true,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(3, 520, 100, 720, 360),
            locked: false,
        },
    ];
    let pulled = resolve_elastic_drag_resize(
        &locked_next,
        1,
        BoxRectPx::new(1, 100, 100, 220, 360),
        ResizeEdge::Right,
        work,
        96,
        60,
        48,
    );
    assert_eq!(pulled.active_rect, BoxRectPx::new(1, 100, 100, 220, 360));
    assert_eq!(pulled.sibling_rects[0], locked_next[1].rect);
    assert_eq!(pulled.sibling_rects[1], locked_next[2].rect);
}

#[test]
fn elastic_pull_never_overlaps_stationary_sibling_behind_tall_neighbor() {
    let work = BoxRectPx::new(0, 0, 0, 1920, 1032);
    // Box 1 (top-left) and Box 4 (bottom-left) sit left of tall Box 2.
    // Shrinking only Box 1 must NOT pull tall Box 2 into stationary Box 4!
    let snap = vec![
        ChainBoxNode {
            rect: BoxRectPx::new(1, 100, 100, 300, 240),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(4, 100, 250, 300, 400),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(2, 310, 100, 510, 400),
            locked: false,
        },
    ];
    let res = resolve_elastic_drag_resize(
        &snap,
        1,
        BoxRectPx::new(1, 100, 100, 200, 240),
        ResizeEdge::Right,
        work,
        96,
        60,
        48,
    );
    assert_eq!(res.active_rect, BoxRectPx::new(1, 100, 100, 200, 240));
    assert_eq!(res.sibling_rects[1], snap[2].rect);
    assert!(!rects_overlap_strict(res.sibling_rects[0], res.sibling_rects[1]));
}

#[test]
fn elastic_auto_fit_reverse_shifts_chain_when_screen_edge_blocks_primary_push() {
    let work = BoxRectPx::new(0, 0, 0, 1000, 800); // usable X: [6, 994]
    let snap = vec![
        ChainBoxNode {
            rect: BoxRectPx::new(1, 500, 100, 700, 300),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(2, 710, 100, 910, 300),
            locked: false,
        },
    ];
    // Box 1 auto-fits to width 350 (+150px). Box 2 only has 84px right slack ->
    // Box 2 hits 994, then Box 1 shifts left by 66px so both fit cleanly with 10px gap!
    let res = resolve_elastic_auto_fit_resize(&snap, 1, 350, 200, work, 96);
    assert_eq!(res.active_rect.width(), 350);
    assert_eq!(res.active_rect.left, 434);
    assert_eq!(res.active_rect.right, 784);
    assert_eq!(res.sibling_rects[0], BoxRectPx::new(2, 794, 100, 994, 300));
    assert!(!rects_overlap_strict(res.active_rect, res.sibling_rects[0]));
}

#[test]
fn elastic_auto_fit_between_locked_walls_relocates_without_moving_locked_boxes() {
    let work = BoxRectPx::new(0, 0, 0, 1000, 800);
    let snap = vec![
        ChainBoxNode {
            rect: BoxRectPx::new(1, 10, 100, 210, 300),
            locked: true,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(2, 220, 100, 420, 300),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(3, 430, 100, 630, 300),
            locked: true,
        },
    ];
    // Box 2 wants to grow to width 360, which cannot fit between locked Box 1 (right=210)
    // and locked Box 3 (left=430). Locked boxes must stay fixed; Box 2 relocates cleanly.
    let res = resolve_elastic_auto_fit_resize(&snap, 2, 360, 200, work, 96);
    assert_eq!(res.active_rect.width(), 360);
    assert_eq!(res.sibling_rects[0], snap[0].rect);
    assert_eq!(res.sibling_rects[1], snap[2].rect);
    assert!(!rects_overlap_strict(res.active_rect, res.sibling_rects[0]));
    assert!(!rects_overlap_strict(res.active_rect, res.sibling_rects[1]));
}

#[test]
fn batch_auto_fit_chain_pulls_shrunk_row_together_and_keeps_alignment() {
    let work = BoxRectPx::new(0, 0, 0, 1920, 1032);
    let nodes = vec![
        ChainBoxNode {
            rect: BoxRectPx::new(1, 100, 100, 420, 400),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(2, 430, 100, 750, 400),
            locked: false,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(3, 100, 410, 420, 710),
            locked: false,
        },
    ];
    let targets = vec![(1, 240, 200), (2, 260, 200), (3, 240, 220)];
    let out = reflow_batch_auto_fit_chain(&nodes, &targets, work, 96);
    assert_eq!(out[0], BoxRectPx::new(1, 100, 100, 340, 300));
    // Box 2 pulled left seamlessly to stay 10px right of Box 1
    assert_eq!(out[1], BoxRectPx::new(2, 350, 100, 610, 300));
    // Box 3 pulled up seamlessly to stay 10px below Box 1
    assert_eq!(out[2], BoxRectPx::new(3, 100, 310, 340, 530));
}

#[test]
fn elastic_auto_fit_locked_box_never_shifts_or_overlaps_on_shortfall() {
    let work = BoxRectPx::new(0, 0, 0, 1000, 800);
    // Locked Box 1 sits directly left of Locked Box 2 with only 10px gap.
    // Asking Box 1 to auto-fit wider (400px) hits shortfall because Box 2 is locked.
    // Box 1 is ALSO locked, so it must NEVER relocate/shift its origin or overlap Box 2.
    let snap = vec![
        ChainBoxNode {
            rect: BoxRectPx::new(1, 200, 100, 400, 300),
            locked: true,
        },
        ChainBoxNode {
            rect: BoxRectPx::new(2, 410, 100, 610, 300),
            locked: true,
        },
    ];
    let res = resolve_elastic_auto_fit_resize(&snap, 1, 400, 200, work, 96);
    assert_eq!(res.active_rect.left, 200);
    assert_eq!(res.active_rect.top, 100);
    assert_eq!(res.sibling_rects[0], snap[1].rect);
    assert!(!rects_overlap_strict(res.active_rect, res.sibling_rects[0]));
    assert!(res.active_rect.right <= snap[1].rect.left - 10);
}
