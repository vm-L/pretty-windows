use super::*;

fn test_params(
    item_count: usize,
    view_mode: BoxViewMode,
    collapsed: bool,
    show_header_actions: bool,
) -> DeskBoxLayoutParams {
    DeskBoxLayoutParams {
        width_dip: 340.0,
        height_dip: 300.0,
        dpi: 96,
        item_count,
        view_mode,
        scroll_y: 0.0,
        can_go_back: false,
        show_item_remove: true,
        collapsed,
        show_header_actions,
        locked: false,
        auto_hide_header: false,
        header_visible: true,
        search_active: false,
        card_height_override_px: None,
    }
}

#[test]
fn deskbox_layout_handles_all_view_modes_scroll_and_hit_zones() {
    for mode in BoxViewMode::ALL {
        let mut p = test_params(32, mode, false, true);
        p.scroll_y = 24.0;
        p.can_go_back = true;
        let layout = DeskBoxLayout::compute(p);
        assert!(layout.header_rect.bottom < layout.content_rect.top);
        assert!(layout.back_btn_rect.right < layout.title_rect.left);
        assert!(layout.title_rect.right < layout.new_btn_rect.left);
        assert!(layout.new_btn_rect.right < layout.sort_btn_rect.left);
        assert!(layout.sort_btn_rect.right < layout.view_btn_rect.left);
        assert!(layout.view_btn_rect.right < layout.lock_btn_rect.left);
        assert!(layout.lock_btn_rect.right < layout.collapse_btn_rect.left);
        assert!(layout.collapse_btn_rect.right < layout.delete_btn_rect.left);
        assert!(layout.delete_btn_rect.right < layout.card_rect.right);
        assert!(!layout.visible_cells.is_empty());
        assert!(layout.max_scroll > 0.0);
        assert!(layout.scrollbar_thumb_rect.height() >= 22.0);

        for pair in layout.visible_cells.windows(2) {
            assert!(pair[0].index < pair[1].index);
        }
        for cell in &layout.visible_cells {
            assert!(cell.rect.left >= layout.content_rect.left - 0.5);
            assert!(cell.rect.right <= layout.scrollbar_track_rect.left + 0.5);
            assert!(cell.icon_rect.width() > 0.0);
            assert!(cell.label_rect.width() > 0.0);
            assert!(cell.remove_btn_rect.width() > 1.0);
            assert!(cell.remove_btn_rect.right <= cell.rect.right + 0.5);
        }

        let first = &layout.visible_cells[0];
        let vis_top = first.rect.top.max(layout.content_rect.top);
        let vis_bot = first.rect.bottom.min(layout.content_rect.bottom);
        assert!(vis_bot > vis_top);
        let icx = (first.rect.left + first.rect.right) * 0.5;
        let icy = (vis_top + vis_bot) * 0.5;
        assert_eq!(layout.hit_test(icx, icy), BoxHitZone::Item(first.index));
    }

    // Hover active on header reveals all 6 buttons and keeps their top pixels clickable
    let mut ph = test_params(3, BoxViewMode::MediumIcons, false, true);
    ph.dpi = 144;
    let hovered = DeskBoxLayout::compute(ph);
    assert_eq!(hovered.max_scroll, 0.0);
    for (rect, expected) in [
        (hovered.new_btn_rect, BoxHitZone::NewButton),
        (hovered.sort_btn_rect, BoxHitZone::SortButton),
        (hovered.view_btn_rect, BoxHitZone::ViewButton),
        (hovered.lock_btn_rect, BoxHitZone::LockButton),
        (hovered.collapse_btn_rect, BoxHitZone::CollapseButton),
        (hovered.delete_btn_rect, BoxHitZone::DeleteButton),
    ] {
        let cx = (rect.left + rect.right) * 0.5;
        let cy = (rect.top + rect.bottom) * 0.5;
        assert_eq!(hovered.hit_test(cx, cy), expected);
        assert_eq!(hovered.hit_test(cx, rect.top + 0.5), expected);
    }

    // Resting header hides action buttons and gives title full width
    let mut pr = test_params(3, BoxViewMode::MediumIcons, false, false);
    pr.dpi = 144;
    let resting = DeskBoxLayout::compute(pr);
    assert_eq!(resting.new_btn_rect.width(), 0.0);
    assert_eq!(resting.delete_btn_rect.width(), 0.0);
    assert!(resting.title_rect.right > hovered.title_rect.right + 40.0);
    let resting_right_title_x = resting.card_rect.right - 20.0;
    let hdr_cy = (resting.header_rect.top + resting.header_rect.bottom) * 0.5;
    assert_eq!(
        resting.hit_test(resting_right_title_x, hdr_cy),
        BoxHitZone::HeaderDrag
    );

    let hdr_cx = (hovered.title_rect.left + hovered.title_rect.right) * 0.5;
    assert_eq!(hovered.hit_test(hdr_cx, hdr_cy), BoxHitZone::HeaderDrag);

    assert_eq!(
        hovered.hit_test(hovered.card_rect.left + 2.0, hovered.card_rect.top + 2.0),
        BoxHitZone::Resize(ResizeEdge::TopLeft)
    );
    assert_eq!(
        hovered.hit_test(hovered.card_rect.right - 2.0, hovered.card_rect.bottom - 2.0),
        BoxHitZone::Resize(ResizeEdge::BottomRight)
    );
    assert_eq!(hovered.hit_test(1.0, 1.0), BoxHitZone::None);
}

#[test]
fn collapsed_box_shows_header_only_and_disables_vertical_resize() {
    let mut p = test_params(12, BoxViewMode::MediumIcons, true, true);
    p.scroll_y = 40.0;
    let collapsed = DeskBoxLayout::compute(p);
    assert!(collapsed.collapsed);
    assert_eq!(collapsed.card_rect.height(), BOX_HEADER_H_DIP);
    assert_eq!(collapsed.win_h, (BOX_HEADER_H_DIP + SHADOW_PAD_DIP * 2.0) as i32);
    assert!(collapsed.visible_cells.is_empty());
    assert_eq!(collapsed.max_scroll, 0.0);

    let mid_y = (collapsed.card_rect.top + collapsed.card_rect.bottom) * 0.5;
    assert_eq!(
        collapsed.hit_test(collapsed.card_rect.left + 2.0, mid_y),
        BoxHitZone::Resize(ResizeEdge::Left)
    );
    assert_eq!(
        collapsed.hit_test(collapsed.card_rect.right - 2.0, mid_y),
        BoxHitZone::Resize(ResizeEdge::Right)
    );
    let title_mid_x = (collapsed.title_rect.left + collapsed.title_rect.right) * 0.5;
    assert_eq!(
        collapsed.hit_test(title_mid_x, collapsed.card_rect.top + 1.5),
        BoxHitZone::HeaderDrag
    );
    assert_eq!(
        collapsed.hit_test(title_mid_x, collapsed.card_rect.bottom - 1.5),
        BoxHitZone::HeaderDrag
    );
    let cbx = (collapsed.collapse_btn_rect.left + collapsed.collapse_btn_rect.right) * 0.5;
    assert_eq!(
        collapsed.hit_test(cbx, collapsed.collapse_btn_rect.top + 1.0),
        BoxHitZone::CollapseButton
    );

    // Regression test: sweeping card_height_override_px across every height during
    // collapse/expand spring animation with a scrollbar must never panic, and collapsed=true
    // must immediately hide all items.
    for dpi in [96u32, 144, 192] {
        let s = (dpi as f32) / 96.0;
        let min_h = (BOX_HEADER_H_DIP * s).round() as i32;
        let max_h = (300.0 * s).round() as i32;
        for h in (min_h..=max_h).step_by(3) {
            let mut pc = test_params(120, BoxViewMode::MediumIcons, true, true);
            pc.dpi = dpi;
            pc.card_height_override_px = Some(h as f32);
            let lc = DeskBoxLayout::compute(pc);
            assert!(lc.collapsed);
            assert!(lc.visible_cells.is_empty());

            let mut pe = test_params(120, BoxViewMode::MediumIcons, false, true);
            pe.dpi = dpi;
            pe.card_height_override_px = Some(h as f32);
            let le = DeskBoxLayout::compute(pe);
            if le.scrollbar_track_rect.height() > 0.0 {
                assert!(le.scrollbar_thumb_rect.height() > 0.0);
                assert!(le.scrollbar_thumb_rect.bottom <= le.scrollbar_track_rect.bottom + 1.0);
            }
        }
    }
}

#[test]
fn locked_auto_hide_header_and_search_clear_layout_rules() {
    let mut p = test_params(4, BoxViewMode::MediumIcons, false, false);
    p.locked = true;
    p.auto_hide_header = true;
    p.header_visible = false;
    let locked_hidden = DeskBoxLayout::compute(p);
    // Resize borders are disabled when locked.
    assert_ne!(
        locked_hidden.hit_test(
            locked_hidden.card_rect.left + 2.0,
            locked_hidden.card_rect.top + 2.0
        ),
        BoxHitZone::Resize(ResizeEdge::TopLeft)
    );
    // Content starts near top without header gap when auto_hide_header is enabled.
    assert!(locked_hidden.content_rect.top < locked_hidden.header_rect.bottom);

    p.header_visible = true;
    p.search_active = true;
    let search_layout = DeskBoxLayout::compute(p);
    let scx = (search_layout.search_clear_btn_rect.left + search_layout.search_clear_btn_rect.right) * 0.5;
    let scy = (search_layout.search_clear_btn_rect.top + search_layout.search_clear_btn_rect.bottom) * 0.5;
    assert_eq!(
        search_layout.hit_test(scx, scy),
        BoxHitZone::SearchClearButton
    );
}

#[test]
fn clamp_box_bounds_keeps_window_inside_work_area_and_preserves_expanded_height() {
    let (x, y, w, h, wx, wy, ww, wh) =
        clamp_box_bounds_to_work_area(2500.0, 1800.0, 400.0, 320.0, false, 0, 0, 1920, 1032, 96);
    let pad = SHADOW_PAD_DIP as i32;
    assert!(x >= 0.0 && y >= 0.0);
    assert_eq!(w, 400.0);
    assert_eq!(h, 320.0);
    assert!(wx + ww - pad <= 1920);
    assert!(wy + wh - pad <= 1032);

    let (_, _, _, ch_collapsed, _, _, _, wh_collapsed) =
        clamp_box_bounds_to_work_area(100.0, 100.0, 340.0, 320.0, true, 0, 0, 1920, 1032, 96);
    assert_eq!(ch_collapsed, 320.0);
    assert_eq!(wh_collapsed, (BOX_HEADER_H_DIP as i32) + pad * 2);
}

#[test]
fn item_remove_button_hit_test_matches_kind() {
    for mode in [BoxViewMode::MediumIcons, BoxViewMode::CompactList] {
        let collection = DeskBoxLayout::compute(test_params(4, mode, false, false));
        let c0 = &collection.visible_cells[0];
        let rcx = (c0.remove_btn_rect.left + c0.remove_btn_rect.right) * 0.5;
        let rcy = (c0.remove_btn_rect.top + c0.remove_btn_rect.bottom) * 0.5;
        assert_eq!(collection.hit_test(rcx, rcy), BoxHitZone::ItemRemove(0));

        let mut pp = test_params(4, mode, false, false);
        pp.show_item_remove = false;
        let portal = DeskBoxLayout::compute(pp);
        let p0 = &portal.visible_cells[0];
        assert_eq!(p0.remove_btn_rect.width(), 0.0);
        assert_eq!(portal.hit_test(rcx, rcy), BoxHitZone::Item(0));
    }
}

#[test]
fn multimonitor_hotplug_fallback_resolution_shrink_and_negative_origin() {
    let monitors = vec![
        MonitorWorkRect {
            monitor_id: r"\\.\DISPLAY1".to_string(),
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1032,
            dpi: 120,
            is_primary: true,
        },
        MonitorWorkRect {
            monitor_id: r"\\.\DISPLAY2".to_string(),
            left: -2560,
            top: 0,
            right: 0,
            bottom: 1400,
            dpi: 144,
            is_primary: false,
        },
    ];

    // 1. Box on negative-origin secondary monitor resolves strictly inside [-2560, 0).
    let r1 = resolve_monitor_and_clamp_bounds(
        &monitors,
        r"\\.\display2",
        120.0,
        80.0,
        340.0,
        300.0,
        false,
    )
    .unwrap();
    assert_eq!(r1.monitor_idx, 1);
    let pad_144 = (SHADOW_PAD_DIP * 1.5).round() as i32;
    assert!(r1.win_x + pad_144 >= -2560);
    assert!(r1.win_x + r1.win_w - pad_144 <= 0);
    assert!(r1.win_y + pad_144 >= 0);
    assert!(r1.win_y + r1.win_h - pad_144 <= 1400);
    assert!((r1.w_dip - 340.0).abs() < 0.01 && (r1.h_dip - 300.0).abs() < 0.01);
    assert!(r1.x_dip >= 0.0 && r1.y_dip >= 0.0);

    // 2. Unplugging DISPLAY2 falls back cleanly to primary DISPLAY1 without off-screen coords.
    let single_primary = vec![monitors[0].clone()];
    let r2 = resolve_monitor_and_clamp_bounds(
        &single_primary,
        r"\\.\DISPLAY2",
        1800.0,
        950.0,
        400.0,
        360.0,
        false,
    )
    .unwrap();
    assert_eq!(r2.monitor_idx, 0);
    let pad_120 = (SHADOW_PAD_DIP * 1.25).round() as i32;
    assert!(r2.win_x + pad_120 >= 0);
    assert!(r2.win_x + r2.win_w - pad_120 <= 1920);
    assert!(r2.win_y + pad_120 >= 0);
    assert!(r2.win_y + r2.win_h - pad_120 <= 1032);

    // 3. Resolution shrink + high DPI (1280x720 @ 192 DPI = 640x360 logical) clamps oversized box cleanly.
    let tiny_hi_dpi = vec![MonitorWorkRect {
        monitor_id: r"\\.\DISPLAY1".to_string(),
        left: 0,
        top: 40,
        right: 1280,
        bottom: 720,
        dpi: 192,
        is_primary: true,
    }];
    let r3 = resolve_monitor_and_clamp_bounds(
        &tiny_hi_dpi,
        r"\\.\DISPLAY1",
        900.0,
        600.0,
        800.0,
        500.0,
        false,
    )
    .unwrap();
    let pad_192 = (SHADOW_PAD_DIP * 2.0).round() as i32;
    assert!((BOX_MIN_WIDTH_DIP..=640.0).contains(&r3.w_dip));
    assert!((BOX_MIN_HEIGHT_DIP..=340.0).contains(&r3.h_dip));
    assert!(r3.win_x + pad_192 >= 0);
    assert!(r3.win_x + r3.win_w - pad_192 <= 1280);
    assert!(r3.win_y + pad_192 >= 40);
    assert!(r3.win_y + r3.win_h - pad_192 <= 720);
}

#[test]
fn high_load_directory_virtualization_keeps_visible_cells_bounded() {
    const TOTAL_ITEMS: usize = 2_400;
    for mode in BoxViewMode::ALL {
        let mut p = test_params(TOTAL_ITEMS, mode, false, false);
        p.width_dip = 360.0;
        p.height_dip = 320.0;
        p.dpi = 144;
        let top_layout = DeskBoxLayout::compute(p);
        assert!(top_layout.max_scroll > 5_000.0);
        assert!(top_layout.visible_cells.len() <= 40);
        assert_eq!(top_layout.visible_cells.first().map(|c| c.index), Some(0));

        p.scroll_y = top_layout.max_scroll * 0.55;
        let mid_layout = DeskBoxLayout::compute(p);
        assert!(mid_layout.visible_cells.len() <= 40);
        assert!(mid_layout.visible_cells[0].index > 800);
        let sample = mid_layout.visible_cells[mid_layout.visible_cells.len() / 2];
        let sx = (sample.rect.left + sample.rect.right) * 0.5;
        let sy = (sample.rect.top.max(mid_layout.content_rect.top)
            + sample.rect.bottom.min(mid_layout.content_rect.bottom))
            * 0.5;
        assert_eq!(
            mid_layout.drop_target_index_at(sx, sy, TOTAL_ITEMS),
            Some(sample.index)
        );

        p.scroll_y = top_layout.max_scroll + 9999.0;
        let end_layout = DeskBoxLayout::compute(p);
        assert!((end_layout.scroll_y - end_layout.max_scroll).abs() < 0.5);
        assert_eq!(
            end_layout.visible_cells.last().map(|c| c.index),
            Some(TOTAL_ITEMS - 1)
        );
    }
}

#[test]
fn hover_expand_animation_keeps_icon_positions_invariant() {
    for &count in &[4usize, 10, 28] {
        let mut full_p = test_params(count, BoxViewMode::MediumIcons, false, false);
        full_p.width_dip = 340.0;
        full_p.height_dip = 300.0;
        full_p.dpi = 120;
        let full = DeskBoxLayout::compute(full_p);
        assert!(!full.visible_cells.is_empty());

        for anim_h in [80.0f32, 135.0, 210.0, 320.0] {
            let mut anim_p = full_p;
            anim_p.card_height_override_px = Some(anim_h);
            let anim = DeskBoxLayout::compute(anim_p);
            for cell in &anim.visible_cells {
                let target = full
                    .visible_cells
                    .iter()
                    .find(|c| c.index == cell.index)
                    .expect("animated visible cell must exist in full layout");
                assert!(
                    (cell.rect.left - target.rect.left).abs() < 0.001
                        && (cell.rect.right - target.rect.right).abs() < 0.001
                        && (cell.rect.top - target.rect.top).abs() < 0.001
                        && (cell.icon_rect.left - target.icon_rect.left).abs() < 0.001
                        && (cell.icon_rect.top - target.icon_rect.top).abs() < 0.001,
                    "icon shifted during hover-expand (count={count}, anim_h={anim_h}, idx={})",
                    cell.index
                );
            }
        }
    }
}

#[test]
fn grid_resize_keeps_cell_width_and_column_positions_fixed() {
    for mode in [
        BoxViewMode::LargeIcons,
        BoxViewMode::MediumIcons,
        BoxViewMode::SmallIcons,
    ] {
        let mut base_p = test_params(12, mode, false, false);
        base_p.width_dip = 220.0;
        base_p.height_dip = 300.0;
        base_p.dpi = 120;
        let base = DeskBoxLayout::compute(base_p);
        let base_c0 = base.visible_cells[0];
        let expected_w = base_c0.rect.width();
        let expected_h = base_c0.rect.height();
        assert!(
            expected_w > expected_h,
            "grid cell should stay a horizontal rectangle (mode={mode:?}, {expected_w}x{expected_h})"
        );
        let expected_gap = base.visible_cells[1].rect.left - base_c0.rect.right;

        for w_step in (220..=520).step_by(7) {
            let mut p = base_p;
            p.width_dip = w_step as f32;
            let cur = DeskBoxLayout::compute(p);
            let c0 = cur.visible_cells[0];
            let c1 = cur.visible_cells[1];
            assert!(
                (c0.rect.width() - expected_w).abs() < 0.001
                    && (c0.rect.left - base_c0.rect.left).abs() < 0.001
                    && (c0.icon_rect.left - base_c0.icon_rect.left).abs() < 0.001,
                "grid cell width or col-0 shifted during resize (mode={mode:?}, w={w_step})"
            );
            assert!(
                ((c1.rect.left - c0.rect.right) - expected_gap).abs() < 0.001,
                "inter-column gap drifted during resize (mode={mode:?}, w={w_step})"
            );
        }
    }
}

#[test]
fn auto_fit_size_displays_all_icons_without_scrollbar_across_dpis() {
    for mode in BoxViewMode::ALL {
        let counts: &[usize] = if mode == BoxViewMode::CompactList {
            &[1, 3, 9, 20]
        } else {
            &[1, 3, 9, 25]
        };
        for &count in counts {
            let header_min = header_min_width_for_title_dip(
                estimate_title_width_dip("桌面收纳"),
                false,
            );
            let list_pref = compact_list_min_width_for_label_dip(
                estimate_title_width_dip("示例文件.docx"),
                true,
            );
            let (w, h) = compute_auto_fit_size_dip(
                count,
                mode,
                false,
                340.0,
                header_min,
                list_pref,
                1280.0,
                720.0,
            );
            assert!(
                w >= header_min,
                "auto-fit width {w} must accommodate title + action buttons ({header_min})"
            );
            for dpi in [96u32, 120, 144, 168, 192] {
                let mut p = test_params(count, mode, false, true);
                p.width_dip = w;
                p.height_dip = h;
                p.dpi = dpi;
                let layout = DeskBoxLayout::compute(p);
                assert_eq!(
                    layout.max_scroll, 0.0,
                    "auto-fit left scrollbar (mode={mode:?}, count={count}, dpi={dpi}, {w}x{h})"
                );
                assert_eq!(
                    layout.visible_cells.len(),
                    count,
                    "auto-fit did not show all items (mode={mode:?}, count={count}, dpi={dpi})"
                );
                // All 6 action buttons must remain visible with room for the title.
                assert!(layout.new_btn_rect.width() > 1.0);
                assert!(layout.delete_btn_rect.width() > 1.0);
                assert!(layout.title_rect.width() >= estimate_title_width_dip("桌面收纳") * layout.scale - 2.0);
                assert_eq!(
                    compute_max_scroll_px(w, h, dpi, count, mode, false, false),
                    0.0
                );
            }
        }
    }

    // Ultra-compact narrow box stays cleanly bounded with no button overflow past card_rect.left.
    let mut tiny = test_params(1, BoxViewMode::SmallIcons, false, true);
    tiny.width_dip = 74.0;
    tiny.height_dip = 88.0;
    tiny.dpi = 120;
    let tiny_layout = DeskBoxLayout::compute(tiny);
    for btn in [
        tiny_layout.new_btn_rect,
        tiny_layout.sort_btn_rect,
        tiny_layout.view_btn_rect,
        tiny_layout.lock_btn_rect,
        tiny_layout.collapse_btn_rect,
        tiny_layout.delete_btn_rect,
    ] {
        if btn.width() > 1.0 {
            assert!(btn.left >= tiny_layout.card_rect.left);
            assert!(btn.right <= tiny_layout.card_rect.right);
        }
    }

    // CompactList auto-fit must shrink wide boxes with short names AND expand narrow boxes for long names.
    let hdr = header_min_width_for_title_dip(estimate_title_width_dip("收纳"), false);
    let short_pref = compact_list_min_width_for_label_dip(estimate_title_width_dip("GPT"), true);
    let (shrunk_w, shrunk_h) = compute_auto_fit_size_dip(
        2,
        BoxViewMode::CompactList,
        false,
        520.0,
        hdr,
        short_pref,
        1280.0,
        720.0,
    );
    assert!(shrunk_w < 300.0 && shrunk_w >= hdr);
    assert!(shrunk_h < 120.0);

    let long_pref = compact_list_min_width_for_label_dip(
        estimate_title_width_dip("2026年度桌面收纳架构设计说明文档.docx"),
        true,
    );
    let (expanded_w, _) = compute_auto_fit_size_dip(
        2,
        BoxViewMode::CompactList,
        false,
        200.0,
        hdr,
        long_pref,
        1280.0,
        720.0,
    );
    assert!(expanded_w >= long_pref);
}

#[test]
fn fractional_dpi_layout_and_clamp_share_exact_shadow_padding() {
    for dpi in [96u32, 120, 144, 168, 192] {
        let w_dip = 320.0;
        let h_dip = 240.0;
        let mut p = test_params(3, BoxViewMode::MediumIcons, false, false);
        p.width_dip = w_dip;
        p.height_dip = h_dip;
        p.dpi = dpi;
        let layout = DeskBoxLayout::compute(p);

        let (_, _, clamped_w, clamped_h, _, _, win_w, win_h) = clamp_box_bounds_to_work_area(
            100.0, 100.0, w_dip, h_dip, false, 0, 0, 1920, 1040, dpi,
        );
        assert_eq!(clamped_w, w_dip);
        assert_eq!(clamped_h, h_dip);
        assert_eq!(
            layout.win_w, win_w,
            "layout.win_w and clamp win_w diverged at dpi={dpi}"
        );
        assert_eq!(
            layout.win_h, win_h,
            "layout.win_h and clamp win_h diverged at dpi={dpi}"
        );
        let s = (dpi as f32) / 96.0;
        let expected_pad = (SHADOW_PAD_DIP * s).round();
        assert_eq!(layout.card_rect.left, expected_pad);
        assert_eq!(layout.card_rect.top, expected_pad);
    }
}
