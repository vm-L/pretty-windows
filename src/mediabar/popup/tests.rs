use super::{
    compute_cover_halo_pair, resolve_dynamic_menu_accent, SETTING_ROW_H_DIP, SUB_WIDTH_DAMPING,
    SUB_WIDTH_STIFFNESS,
};
use crate::color::{derive_background_accent, rgb_to_hsl, Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::mediabar::arbitration::PlaybackState;
use crate::mediabar::layout::RectF;
use crate::mediabar::smtc::{MediaCover, SourceItem};
use crate::ui::{
    animated_closing_sub_rect, animated_primary_rect, clamp_sub_spring_rect,
    compute_submenu_vertical_span, ease_out_cubic, ease_retract, row_stagger_dy,
    sample_liquid_silk_field, sub_bud_start_rect, window_master_alpha, PopupAnimPhase, Spring1D,
    DIVIDER_H_DIP, INNER_PAD_DIP, OPEN_SPRING_DAMPING, OPEN_SPRING_STIFFNESS, ROW_GAP_DIP,
};

#[test]
fn cover_halo_pair_preserves_clean_accent_for_dark_and_light_themes() {
    let teal_cover = MediaCover {
        hash: 1,
        thumb_w: 2,
        thumb_h: 2,
        thumb_pbgra: vec![255; 16],
        art_w: 2,
        art_h: 2,
        art_pbgra: vec![255; 16],
        accent_dark: Rgb::new(72, 195, 220),
        accent_light: Rgb::new(40, 128, 172),
    };
    let warm_cover = MediaCover {
        hash: 2,
        thumb_w: 2,
        thumb_h: 2,
        thumb_pbgra: vec![255; 16],
        art_w: 2,
        art_h: 2,
        art_pbgra: vec![255; 16],
        accent_dark: Rgb::new(235, 192, 86),
        accent_light: Rgb::new(155, 118, 30),
    };
    let gray_cover = MediaCover {
        hash: 3,
        thumb_w: 2,
        thumb_h: 2,
        thumb_pbgra: vec![255; 16],
        art_w: 2,
        art_h: 2,
        art_pbgra: vec![255; 16],
        accent_dark: Rgb::new(128, 128, 128),
        accent_light: Rgb::new(128, 128, 128),
    };

    for light_theme in [false, true] {
        for cov in [Some(&teal_cover), Some(&warm_cover), None] {
            let pair = compute_cover_halo_pair(
                cov,
                derive_background_accent(Rgb::new(22, 64, 42), false),
                light_theme,
            );
            for c in pair {
                let (_, s, l) = rgb_to_hsl(c);
                assert!(
                    (0.30..=0.74).contains(&s),
                    "expected restrained halo saturation in [0.30, 0.74], got {s} ({c:?})"
                );
                assert!(
                    (0.52..=0.80).contains(&l),
                    "expected controlled halo luminance in [0.52, 0.80], got {l} ({c:?})"
                );
            }
        }

        let gray_pair = compute_cover_halo_pair(
            Some(&gray_cover),
            derive_background_accent(NEUTRAL_BACKDROP_FALLBACK, false),
            light_theme,
        );
        for c in gray_pair {
            let (_, s, l) = rgb_to_hsl(c);
            assert!(
                s <= 0.12,
                "grayscale cover halo must stay neutral pearl gray, got s={s} ({c:?})"
            );
            assert!(
                (0.55..=0.85).contains(&l),
                "expected soft grayscale halo luminance in [0.55, 0.85], got {l} ({c:?})"
            );
        }
    }
}

#[test]
fn liquid_silk_field_stays_sparse_smooth_and_restrained() {
    let colors = [(0.25, 0.65, 0.80), (0.55, 0.45, 0.85)];
    for light_glass in [false, true] {
        let cap = if light_glass { 0.141 } else { 0.181 };
        let mut max_alpha = 0.0f32;
        let mut quiet_samples = 0usize;
        let mut total_samples = 0usize;
        let mut max_step = 0.0f32;
        let nx = 64usize;
        let ny = 48usize;
        for iy in 0..ny {
            let y = (iy as f32) / (ny as f32) * 1.4;
            let mut prev_a = 0.0f32;
            for ix in 0..nx {
                let x = (ix as f32) / (nx as f32) * 2.0;
                let (r, g, b, a) = sample_liquid_silk_field(x, y, 1.25, &colors, light_glass);
                assert!((0.0..=1.0).contains(&r));
                assert!((0.0..=1.0).contains(&g));
                assert!((0.0..=1.0).contains(&b));
                assert!(a >= 0.0 && a <= cap, "alpha exceeded cap {cap}: {a}");
                max_alpha = max_alpha.max(a);
                if a < 0.036 {
                    quiet_samples += 1;
                }
                total_samples += 1;
                if ix > 0 {
                    max_step = max_step.max((a - prev_a).abs());
                }
                prev_a = a;
            }
        }
        let quiet_ratio = (quiet_samples as f32) / (total_samples as f32);
        assert!(
            quiet_ratio >= 0.35,
            "expected >= 35% clear negative space (alpha < 0.036), got {quiet_ratio}"
        );
        assert!(
            max_alpha >= 0.08,
            "expected visible silk crest >= 0.08, got {max_alpha}"
        );
        assert!(max_step < 0.045, "silk field step too steep: {max_step}");
    }
}

#[test]
fn dynamic_menu_accent_matches_active_cover_and_falls_back_cleanly() {
    use std::sync::Arc;

    let cover_a = Arc::new(MediaCover {
        hash: 10,
        thumb_w: 2,
        thumb_h: 2,
        thumb_pbgra: vec![255; 16],
        art_w: 2,
        art_h: 2,
        art_pbgra: vec![255; 16],
        accent_dark: Rgb::new(228, 186, 78),
        accent_light: Rgb::new(152, 114, 24),
    });
    let cover_b = Arc::new(MediaCover {
        hash: 20,
        thumb_w: 2,
        thumb_h: 2,
        thumb_pbgra: vec![255; 16],
        art_w: 2,
        art_h: 2,
        art_pbgra: vec![255; 16],
        accent_dark: Rgb::new(82, 205, 198),
        accent_light: Rgb::new(28, 138, 132),
    });
    let mk = |id: &str, cover: Option<Arc<MediaCover>>| SourceItem {
        id: id.to_string(),
        app_name: id.to_string(),
        title: id.to_string(),
        artist: String::new(),
        status: PlaybackState::Playing,
        pid: None,
        cover,
        timeline: None,
    };

    let sources = vec![
        mk("a", Some(cover_a.clone())),
        mk("b", Some(cover_b.clone())),
        mk("no_cover", None),
    ];
    let fallback = Rgb::new(0, 120, 212);

    // Matches active_id cover per theme
    assert_eq!(
        resolve_dynamic_menu_accent(&sources, Some("b"), fallback, false),
        cover_b.accent_dark
    );
    assert_eq!(
        resolve_dynamic_menu_accent(&sources, Some("a"), fallback, true),
        cover_a.accent_light
    );

    // Active source without cover falls back to first source cover
    assert_eq!(
        resolve_dynamic_menu_accent(&sources, Some("no_cover"), fallback, false),
        cover_a.accent_dark
    );

    // No covers anywhere falls back cleanly to the provided backdrop accent without alteration
    let plain_sources = vec![mk("no_cover", None)];
    assert_eq!(
        resolve_dynamic_menu_accent(&plain_sources, Some("no_cover"), fallback, false),
        fallback
    );
    assert_eq!(
        resolve_dynamic_menu_accent(&plain_sources, Some("no_cover"), fallback, true),
        fallback
    );
}

#[test]
fn dynamic_menu_accent_pushes_muted_cover_while_halo_stays_soft() {
    use std::sync::Arc;

    let muted_cover = Arc::new(MediaCover {
        hash: 30,
        thumb_w: 2,
        thumb_h: 2,
        thumb_pbgra: vec![255; 16],
        art_w: 2,
        art_h: 2,
        art_pbgra: vec![255; 16],
        accent_dark: Rgb::new(125, 167, 171),
        accent_light: Rgb::new(97, 145, 149),
    });
    let sources = vec![SourceItem {
        id: "muted".to_string(),
        app_name: "muted".to_string(),
        title: "muted".to_string(),
        artist: String::new(),
        status: PlaybackState::Playing,
        pid: None,
        cover: Some(muted_cover.clone()),
        timeline: None,
    }];
    let fallback = Rgb::new(0, 120, 212);
    let rgb_chroma =
        |c: Rgb| f32::from(c.r.max(c.g).max(c.b) - c.r.min(c.g).min(c.b)) / 255.0;

    for (light_theme, orig) in [
        (false, muted_cover.accent_dark),
        (true, muted_cover.accent_light),
    ] {
        let ui_accent =
            resolve_dynamic_menu_accent(&sources, Some("muted"), fallback, light_theme);
        assert_ne!(ui_accent, orig);
        assert!(
            rgb_chroma(ui_accent) > rgb_chroma(orig),
            "expected pushed chroma for muted cover (light={light_theme}): {orig:?} -> {ui_accent:?}"
        );
        let (orig_h, _, _) = rgb_to_hsl(orig);
        let (out_h, _, _) = rgb_to_hsl(ui_accent);
        let diff = (orig_h - out_h).abs().rem_euclid(360.0);
        assert!(diff.min(360.0 - diff) <= 4.0);

        let halo = compute_cover_halo_pair(Some(&muted_cover), fallback, light_theme);
        let (_, halo_s0, _) = rgb_to_hsl(halo[0]);
        let (_, halo_s1, _) = rgb_to_hsl(halo[1]);
        assert!(
            halo_s0 < 0.30,
            "halo ribbon 0 should remain soft, got s={halo_s0}"
        );
        assert!(
            halo_s1 < 0.30,
            "halo ribbon 1 should remain soft, got s={halo_s1}"
        );
    }
}

#[test]
fn submenu_vertical_span_prefers_downward_then_aligns_last_item_upward() {
    let pad = 8.0f32;
    let top_limit = 14.0f32;
    let bottom_limit = 316.0f32; // primary last row (`设置`, top=274, bottom=308) + pad

    // 1. Ample downward space: top aligns with parent row (`row_top - pad`)
    let (t1, b1) = compute_submenu_vertical_span(122.0, pad, 110.0, top_limit, bottom_limit);
    assert!((t1 - 114.0).abs() < 0.01);
    assert!((b1 - 224.0).abs() < 0.01);

    // 2. Middle row without enough downward space: last submenu item aligns with primary last item (`bottom_limit`)
    let (t2, b2) = compute_submenu_vertical_span(236.0, pad, 140.0, top_limit, bottom_limit);
    assert!((b2 - bottom_limit).abs() < 0.01);
    assert!((t2 - 176.0).abs() < 0.01);
    assert!(((b2 - pad) - 308.0).abs() < 0.01);

    // 3. Bottom-most Settings row (`关闭` 34px aligns flush with `设置` 34px at bottom_limit)
    let settings_h = INNER_PAD_DIP * 2.0 + SETTING_ROW_H_DIP * 3.0 + ROW_GAP_DIP + DIVIDER_H_DIP;
    let (t3, b3) = compute_submenu_vertical_span(274.0, pad, settings_h, top_limit, bottom_limit);
    assert!((b3 - bottom_limit).abs() < 0.01);
    assert!((t3 - 185.0).abs() < 0.01);
    let close_bottom = b3 - pad;
    let close_top = close_bottom - 34.0;
    assert!((close_bottom - 308.0).abs() < 0.01);
    assert!((close_top - 274.0).abs() < 0.01);

    // 4. Extremely tall submenu exceeding upward headroom clamps cleanly within [top_limit, bottom_limit]
    let (t4, b4) = compute_submenu_vertical_span(160.0, pad, 340.0, top_limit, bottom_limit);
    assert!((t4 - top_limit).abs() < 0.01);
    assert!((b4 - bottom_limit).abs() < 0.01);
}

#[test]
fn popup_anim_curves_and_morph_stay_bounded_and_safe() {
    // 1. Easing curves & window alpha boundaries
    assert!((ease_out_cubic(0.0) - 0.0).abs() < 1e-5);
    assert!((ease_out_cubic(1.0) - 1.0).abs() < 1e-5);
    assert!((ease_retract(0.0) - 0.0).abs() < 1e-5);
    assert!((ease_retract(1.0) - 1.0).abs() < 1e-5);
    assert!((window_master_alpha(PopupAnimPhase::Opening, 0.0, 0.0) - 0.24).abs() < 1e-5);
    assert!((window_master_alpha(PopupAnimPhase::Opening, 0.050, 0.0) - 1.0).abs() < 1e-5);
    assert!((window_master_alpha(PopupAnimPhase::Open, 1.0, 0.0) - 1.0).abs() < 1e-5);
    assert!((window_master_alpha(PopupAnimPhase::Closing, 1.0, 1.0) - 0.0).abs() < 1e-5);

    // 2. Open spring convergence, liquid surface-tension swell [1.02..=1.06], & frame-rate independence
    let mut s_10ms = Spring1D::new(0.14, 2.8);
    let mut s_25ms = Spring1D::new(0.14, 2.8);
    let mut peak_pos = s_10ms.pos;
    let mut max_dt_diff = 0.0f32;
    for _step in 1..=5 {
        for _ in 0..5 {
            s_10ms.step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, 0.010);
            peak_pos = peak_pos.max(s_10ms.pos);
        }
        for _ in 0..2 {
            s_25ms.step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, 0.025);
        }
        max_dt_diff = max_dt_diff.max((s_10ms.pos - s_25ms.pos).abs());
    }
    assert!(
        (1.02..=1.06).contains(&peak_pos),
        "expected liquid spring crest in [1.02, 1.06], got {peak_pos}"
    );
    assert!(
        max_dt_diff < 0.01,
        "expected sub-stepped spring frame-rate invariance (<0.01), got {max_dt_diff}"
    );
    assert!(
        s_10ms.is_settled(1.0, 0.008, 0.20),
        "open spring failed to approach settle within 250ms: {s_10ms:?}"
    );
    for _ in 0..15 {
        s_10ms.step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, 0.010);
    }
    assert!(
        s_10ms.is_settled(1.0, 0.0015, 0.015),
        "open spring failed full settle by 400ms: {s_10ms:?}"
    );

    // 3. Staggered bottom-to-top row micro-glide & clean zeroing at rest
    let dy_top = row_stagger_dy(0, 5, 0.025, 0.65, 1.0);
    let dy_bot = row_stagger_dy(4, 5, 0.025, 0.65, 1.0);
    assert!(dy_top > dy_bot, "top row should trail bottom row during rise");
    let dy_rest = row_stagger_dy(0, 5, 0.16, 1.0, 1.0);
    assert!(dy_rest.abs() < 1e-4, "row stagger did not settle to zero: {dy_rest}");

    // 4. Primary envelope stays strictly anchored to MediaBar bottom/sides with safe top >= 4 DIP & min height
    let target_primary = RectF::new(14.0, 14.0, 162.0, 220.0);
    for step in 0..=20 {
        let p = (step as f32) / 20.0;
        let op = animated_primary_rect(
            target_primary,
            PopupAnimPhase::Opening,
            0.08 + 0.98 * p,
            0.0,
            1.0,
        );
        assert!((op.left - target_primary.left).abs() < 1e-4);
        assert!((op.right - target_primary.right).abs() < 1e-4);
        assert!((op.bottom - target_primary.bottom).abs() < 1e-4);
        assert!(op.top >= 4.0);
        assert!(op.height() >= 42.0);

        let cl = animated_primary_rect(target_primary, PopupAnimPhase::Closing, 1.0, p, 1.0);
        assert!((cl.left - target_primary.left).abs() < 1e-4);
        assert!((cl.right - target_primary.right).abs() < 1e-4);
        assert!((cl.bottom - target_primary.bottom).abs() < 1e-4);
        assert!(cl.height() >= 42.0);
    }

    // 5. Submenu spring continuity on rapid target retargeting, lateral budding & closing bounds
    let mut morph_w = Spring1D::still(172.0);
    for _ in 0..5 {
        morph_w.step(280.0, SUB_WIDTH_STIFFNESS, SUB_WIDTH_DAMPING, 0.010);
    }
    let vel_before_retarget = morph_w.vel;
    assert!(vel_before_retarget > 100.0);
    morph_w.step(264.0, SUB_WIDTH_STIFFNESS, SUB_WIDTH_DAMPING, 0.010);
    assert!(
        morph_w.vel > 0.0,
        "spring velocity should remain continuous across retargeting"
    );

    let sub = RectF::new(162.0, 106.0, 438.0, 316.0);
    let row = RectF::new(22.0, 122.0, 154.0, 156.0);
    for open_left in [false, true] {
        let seam_x = if open_left { 300.0 } else { 162.0 };
        let target_s = if open_left {
            RectF::new(24.0, 106.0, 300.0, 316.0)
        } else {
            sub
        };
        let bud = sub_bud_start_rect(target_s, row, seam_x, open_left, 1.0);
        assert!(bud.width() >= 40.0, "bud width too narrow: {}", bud.width());
        assert!(bud.height() >= 44.0);
        if open_left {
            assert!((bud.right - seam_x).abs() < 1e-4);
        } else {
            assert!((bud.left - seam_x).abs() < 1e-4);
        }

        let clamped = clamp_sub_spring_rect(
            -10.0,
            400.0,
            295.0,
            target_s,
            seam_x,
            316.0,
            460.0,
            open_left,
            1.0,
        );
        assert!(clamped.width() >= 40.0);
        assert!(clamped.top >= 4.0);
        assert!(clamped.bottom <= 316.0);

        let close_sub = animated_closing_sub_rect(
            target_s,
            RectF::new(162.0, 190.0, 310.0, 320.0),
            seam_x,
            316.0,
            open_left,
            0.7,
            1.0,
        );
        assert!(close_sub.width() >= 40.0);
        assert!(close_sub.height() >= 42.0);
        assert!(close_sub.bottom <= 316.0);
    }
}
