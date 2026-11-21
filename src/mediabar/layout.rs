use super::dsp::NUM_BARS;
pub use crate::ui::RectF;

/// Base logical width of MediaBar in DIPs (at 96 DPI).
pub const BAR_WIDTH_DIP: f32 = 256.0;
/// Fixed logical width of the right action zone (shared identically by spectrum bars and control buttons).
pub const RIGHT_ZONE_WIDTH_DIP: f32 = 78.0;

/// Interactive hit-test zones inside MediaBar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitZone {
    MediaInfo,
    PrevButton,
    PlayPauseButton,
    NextButton,
    VolumeMuteButton,
    VolumeSlider,
    None,
}

/// Complete physical-pixel layout snapshot for a given height, DPI scale, and hover interpolation `t`.
#[derive(Clone, Debug)]
pub struct BarLayout {
    pub scale: f32,
    pub width_px: f32,
    pub height_px: f32,
    pub hover_t: f32,
    pub bg_rect: RectF,
    pub bg_radius: f32,
    pub cover_rect: RectF,
    pub cover_radius: f32,
    pub title_rect: RectF,
    pub artist_rect: RectF,
    pub media_info_hit_rect: RectF,
    pub spectrum_alpha: f32,
    pub spectrum_rects: [RectF; NUM_BARS],
    pub controls_alpha: f32,
    pub prev_btn_rect: RectF,
    pub play_btn_rect: RectF,
    pub next_btn_rect: RectF,
    pub vol_mute_btn_rect: RectF,
    pub vol_track_rect: RectF,
    pub vol_slider_hit_rect: RectF,
    pub vol_pct_rect: RectF,
    pub progress_track_rect: RectF,
}

impl BarLayout {
    pub fn compute(height_px: i32, dpi: u32, hover_t: f32, bar_levels: &[f32; NUM_BARS]) -> Self {
        let scale = (dpi.max(96) as f32) / 96.0;
        let t = hover_t.clamp(0.0, 1.0);
        let width_px = (BAR_WIDTH_DIP * scale).round();
        let h_px = (height_px.max(28)) as f32;

        // Pill background inset nicely inside taskbar height
        let bg_margin_x = 2.0 * scale;
        let bg_margin_y = (3.5 * scale).min((h_px - 26.0 * scale).max(2.0) * 0.5);
        let bg_rect = RectF::new(
            bg_margin_x,
            bg_margin_y,
            width_px - bg_margin_x,
            h_px - bg_margin_y,
        );
        let bg_radius = 7.5 * scale;

        // Cover art: 32x32 DIP (clamped if taskbar is ultra-compact)
        let cover_size = (32.0 * scale).min((h_px - 8.0 * scale).max(20.0));
        let left_pad = 8.0 * scale;
        let right_pad = 8.0 * scale;
        let gap_cover_text = 9.0 * scale;
        let gap_text_right = 8.0 * scale;
        let cover_top = (h_px - cover_size) * 0.5;
        let cover_rect = RectF::new(
            left_pad,
            cover_top,
            left_pad + cover_size,
            cover_top + cover_size,
        );
        let cover_radius = 6.0 * scale;

        // Right zone keeps a fixed 78 DIP width across both spectrum and control buttons
        // so media title/artist widths never jump or shrink on hover.
        let right_zone_w = RIGHT_ZONE_WIDTH_DIP * scale;
        let right_zone_right = width_px - right_pad;
        let right_zone_left = right_zone_right - right_zone_w;

        let text_left = cover_rect.right + gap_cover_text;
        let text_right = (right_zone_left - gap_text_right).max(text_left + 12.0 * scale);

        let text_center_y = h_px * 0.5;
        let title_h = 17.0 * scale;
        let artist_h = 15.0 * scale;
        let title_rect = RectF::new(
            text_left,
            text_center_y - title_h + 0.5 * scale,
            text_right,
            text_center_y + 0.5 * scale,
        );
        let artist_rect = RectF::new(
            text_left,
            text_center_y + 0.5 * scale,
            text_right,
            text_center_y + artist_h + 0.5 * scale,
        );

        let media_info_hit_rect = RectF::new(
            bg_rect.left,
            bg_rect.top,
            (right_zone_left - 2.0 * scale).max(cover_rect.right),
            bg_rect.bottom,
        );

        // 10 capsule spectrum bars fill the exact 78 DIP right zone [right_zone_left .. right_zone_right]
        let spectrum_alpha = (1.0 - t * 1.9).clamp(0.0, 1.0);
        let bar_w = 3.6 * scale;
        let bar_gap = (right_zone_w - bar_w * (NUM_BARS as f32)) / ((NUM_BARS - 1) as f32);
        let max_bar_h = (22.0 * scale).min(h_px - 12.0 * scale);
        let min_bar_h = 3.5 * scale;

        let mut spectrum_rects = [RectF::new(0.0, 0.0, 0.0, 0.0); NUM_BARS];
        for (i, rect) in spectrum_rects.iter_mut().enumerate() {
            let x = right_zone_left + (i as f32) * (bar_w + bar_gap);
            let level = bar_levels[i].clamp(0.0, 1.0);
            let bh = min_bar_h + level * (max_bar_h - min_bar_h);
            let top = (h_px - bh) * 0.5;
            *rect = RectF::new(x, top, x + bar_w, top + bh);
        }

        // 3 square control buttons (24x24 DIP + 3 DIP gaps) fill the exact same 78 DIP right zone
        let controls_alpha = ((t - 0.20) / 0.80).clamp(0.0, 1.0);
        let btn_w = 24.0 * scale;
        let btn_h = (24.0 * scale).min(h_px - 10.0 * scale);
        let btn_gap = (right_zone_w - btn_w * 3.0) * 0.5;
        let btn_top = (h_px - btn_h) * 0.5;

        let prev_btn_rect = RectF::new(right_zone_left, btn_top, right_zone_left + btn_w, btn_top + btn_h);
        let play_left = prev_btn_rect.right + btn_gap;
        let play_btn_rect = RectF::new(play_left, btn_top, play_left + btn_w, btn_top + btn_h);
        let next_left = play_btn_rect.right + btn_gap;
        let next_btn_rect = RectF::new(next_left, btn_top, next_left + btn_w, btn_top + btn_h);

        // Inline volume HUD (replaces text + spectrum/buttons when active via taskbar scroll)
        let vol_mute_left = cover_rect.right + 7.0 * scale;
        let vol_mute_btn_rect = RectF::new(vol_mute_left, btn_top, vol_mute_left + btn_w, btn_top + btn_h);
        let vol_pct_w = 34.0 * scale;
        let vol_pct_h = 20.0 * scale;
        let vol_pct_top = (h_px - vol_pct_h) * 0.5;
        let vol_pct_right = width_px - right_pad;
        let vol_pct_rect = RectF::new(
            vol_pct_right - vol_pct_w,
            vol_pct_top,
            vol_pct_right,
            vol_pct_top + vol_pct_h,
        );
        let track_h = 4.5 * scale;
        let track_top = (h_px - track_h) * 0.5;
        let track_left = vol_mute_btn_rect.right + 7.0 * scale;
        let track_right = (vol_pct_rect.left - 7.0 * scale).max(track_left + 20.0 * scale);
        let vol_track_rect = RectF::new(track_left, track_top, track_right, track_top + track_h);
        let vol_slider_hit_rect = RectF::new(
            vol_track_rect.left - 5.0 * scale,
            bg_rect.top,
            vol_track_rect.right + 5.0 * scale,
            bg_rect.bottom,
        );

        // Ultra-slim playback progress hairline inset cleanly along the bottom edge of the pill
        let prog_inset_x = 10.0 * scale;
        let prog_h = (2.0 * scale).max(1.5);
        let prog_bottom = (bg_rect.bottom - 1.0 * scale).max(bg_rect.top + prog_h);
        let progress_track_rect = RectF::new(
            bg_rect.left + prog_inset_x,
            prog_bottom - prog_h,
            (bg_rect.right - prog_inset_x).max(bg_rect.left + prog_inset_x + 12.0 * scale),
            prog_bottom,
        );

        Self {
            scale,
            width_px,
            height_px: h_px,
            hover_t: t,
            bg_rect,
            bg_radius,
            cover_rect,
            cover_radius,
            title_rect,
            artist_rect,
            media_info_hit_rect,
            spectrum_alpha,
            spectrum_rects,
            controls_alpha,
            prev_btn_rect,
            play_btn_rect,
            next_btn_rect,
            vol_mute_btn_rect,
            vol_track_rect,
            vol_slider_hit_rect,
            vol_pct_rect,
            progress_track_rect,
        }
    }
}

/// Performs interactive hit testing against the current bar layout.
#[allow(dead_code)]
pub fn hit_test(layout: &BarLayout, x_px: f32, y_px: f32, controls_visible: bool) -> HitZone {
    hit_test_mode(layout, x_px, y_px, controls_visible, false)
}

/// Mode-aware hit testing supporting both normal media playback mode and inline volume HUD mode.
pub fn hit_test_mode(
    layout: &BarLayout,
    x_px: f32,
    y_px: f32,
    controls_visible: bool,
    vol_hud_active: bool,
) -> HitZone {
    if x_px < 0.0 || y_px < 0.0 || x_px > layout.width_px || y_px > layout.height_px {
        return HitZone::None;
    }

    if vol_hud_active {
        if layout.vol_mute_btn_rect.contains(x_px, y_px) {
            return HitZone::VolumeMuteButton;
        }
        if layout.vol_slider_hit_rect.contains(x_px, y_px) {
            return HitZone::VolumeSlider;
        }
        if x_px >= layout.bg_rect.left
            && x_px <= layout.cover_rect.right + 4.0 * layout.scale
            && y_px >= layout.bg_rect.top
            && y_px <= layout.bg_rect.bottom
        {
            return HitZone::MediaInfo;
        }
        return HitZone::None;
    }

    if controls_visible {
        if layout.prev_btn_rect.contains(x_px, y_px) {
            return HitZone::PrevButton;
        }
        if layout.play_btn_rect.contains(x_px, y_px) {
            return HitZone::PlayPauseButton;
        }
        if layout.next_btn_rect.contains(x_px, y_px) {
            return HitZone::NextButton;
        }
    } else if layout.bg_rect.contains(x_px, y_px) {
        return HitZone::MediaInfo;
    }

    if layout.media_info_hit_rect.contains(x_px, y_px) {
        return HitZone::MediaInfo;
    }

    HitZone::None
}

/// Logical width of the floating Liquid Glass mini-player card in DIPs.
#[allow(dead_code)]
pub const MINI_PLAYER_WIDTH_DIP: f32 = 316.0;
/// Number of compact visualizer bars rendered in the mini-player header.
#[allow(dead_code)]
pub const MINI_SPECTRUM_BARS: usize = 6;

/// Interactive hit zones inside the Liquid Glass mini-player popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum MiniPlayerHitZone {
    None,
    CoverOrInfo,
    MoreMenuButton,
    ProgressSlider,
    PrevButton,
    PlayPauseButton,
    NextButton,
    MuteButton,
}

/// Physical-pixel layout for the floating Liquid Glass mini-player card.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct MiniPlayerLayout {
    pub scale: f32,
    pub win_w: i32,
    pub win_h: i32,
    pub card_rect: RectF,
    pub card_radius: f32,
    pub cover_rect: RectF,
    pub cover_radius: f32,
    pub info_hit_rect: RectF,
    pub source_badge_rect: RectF,
    pub mini_spectrum_rects: [RectF; MINI_SPECTRUM_BARS],
    pub title_rect: RectF,
    pub artist_rect: RectF,
    pub has_timeline: bool,
    pub progress_track_rect: RectF,
    pub progress_hit_rect: RectF,
    pub elapsed_rect: RectF,
    pub duration_rect: RectF,
    pub more_btn_rect: RectF,
    pub prev_btn_rect: RectF,
    pub play_btn_rect: RectF,
    pub next_btn_rect: RectF,
    pub mute_btn_rect: RectF,
}

impl MiniPlayerLayout {
    /// Computes physical-pixel card and child-control rectangles for the mini-player popup.
    #[allow(dead_code)]
    pub fn compute(dpi: u32, has_timeline: bool, bar_levels: &[f32; NUM_BARS]) -> Self {
        let s = (dpi.max(96) as f32) / 96.0;
        let pad_shadow = crate::ui::SHADOW_PAD_DIP * s;
        let card_w = (MINI_PLAYER_WIDTH_DIP * s).round();
        let card_h = ((if has_timeline { 174.0 } else { 136.0 }) * s).round();

        let win_w = (card_w + pad_shadow * 2.0).round() as i32;
        let win_h = (card_h + pad_shadow * 2.0).round() as i32;

        let card_rect = RectF::new(
            pad_shadow,
            pad_shadow,
            pad_shadow + card_w,
            pad_shadow + card_h,
        );
        let card_radius = crate::ui::PANEL_RADIUS_DIP * s;

        let inner_x = 14.0 * s;
        let inner_top = 14.0 * s;
        let cover_size = 72.0 * s;
        let cover_rect = RectF::new(
            card_rect.left + inner_x,
            card_rect.top + inner_top,
            card_rect.left + inner_x + cover_size,
            card_rect.top + inner_top + cover_size,
        );
        let cover_radius = 10.0 * s;

        let right_edge = card_rect.right - inner_x;
        let text_left = cover_rect.right + 12.0 * s;

        // Top meta row: source badge + right-aligned compact live spectrum
        let spec_bar_w = 2.6 * s;
        let spec_gap = 2.2 * s;
        let spec_total_w =
            spec_bar_w * (MINI_SPECTRUM_BARS as f32) + spec_gap * ((MINI_SPECTRUM_BARS - 1) as f32);
        let spec_right = right_edge;
        let spec_left = spec_right - spec_total_w;
        let spec_mid_y = cover_rect.top + 10.0 * s;
        let spec_min_h = 3.0 * s;
        let spec_max_h = 13.0 * s;
        let mut mini_spectrum_rects = [RectF::new(0.0, 0.0, 0.0, 0.0); MINI_SPECTRUM_BARS];
        for (i, rect) in mini_spectrum_rects.iter_mut().enumerate() {
            // Sample evenly across the 10-band FFT spectrum
            let src_idx = (i * (NUM_BARS - 1)) / (MINI_SPECTRUM_BARS - 1);
            let level = bar_levels[src_idx].clamp(0.0, 1.0);
            let bh = spec_min_h + level * (spec_max_h - spec_min_h);
            let x = spec_left + (i as f32) * (spec_bar_w + spec_gap);
            *rect = RectF::new(x, spec_mid_y - bh * 0.5, x + spec_bar_w, spec_mid_y + bh * 0.5);
        }

        let source_badge_rect = RectF::new(
            text_left,
            cover_rect.top + 1.0 * s,
            (spec_left - 8.0 * s).max(text_left + 32.0 * s),
            cover_rect.top + 19.0 * s,
        );

        let title_rect = RectF::new(
            text_left,
            cover_rect.top + 25.0 * s,
            right_edge,
            cover_rect.top + 47.0 * s,
        );
        let artist_rect = RectF::new(
            text_left,
            cover_rect.top + 49.0 * s,
            right_edge,
            cover_rect.top + 68.0 * s,
        );

        let info_hit_rect = RectF::new(
            cover_rect.left,
            cover_rect.top,
            right_edge,
            cover_rect.bottom,
        );

        // Middle timeline row (when a finite duration is available)
        let (progress_track_rect, progress_hit_rect, elapsed_rect, duration_rect, controls_top) =
            if has_timeline {
                let track_top = cover_rect.bottom + 14.0 * s;
                let track_h = 4.5 * s;
                let track_rect = RectF::new(
                    card_rect.left + inner_x,
                    track_top,
                    right_edge,
                    track_top + track_h,
                );
                let hit_rect = RectF::new(
                    track_rect.left - 4.0 * s,
                    track_rect.top - 8.0 * s,
                    track_rect.right + 4.0 * s,
                    track_rect.bottom + 8.0 * s,
                );
                let time_top = track_rect.bottom + 4.0 * s;
                let time_h = 14.0 * s;
                let time_w = 58.0 * s;
                let el_rect = RectF::new(
                    track_rect.left,
                    time_top,
                    track_rect.left + time_w,
                    time_top + time_h,
                );
                let dur_rect = RectF::new(
                    track_rect.right - time_w,
                    time_top,
                    track_rect.right,
                    time_top + time_h,
                );
                (track_rect, hit_rect, el_rect, dur_rect, el_rect.bottom + 6.0 * s)
            } else {
                let empty = RectF::new(0.0, 0.0, 0.0, 0.0);
                (empty, empty, empty, empty, cover_rect.bottom + 10.0 * s)
            };

        // Bottom transport & utility controls row
        let play_size = 36.0 * s;
        let skip_size = 30.0 * s;
        let side_size = 28.0 * s;
        let row_cy = (controls_top + play_size * 0.5).min(card_rect.bottom - 12.0 * s - play_size * 0.5);
        let card_cx = (card_rect.left + card_rect.right) * 0.5;
        let skip_gap = 12.0 * s;

        let play_btn_rect = RectF::new(
            card_cx - play_size * 0.5,
            row_cy - play_size * 0.5,
            card_cx + play_size * 0.5,
            row_cy + play_size * 0.5,
        );
        let prev_btn_rect = RectF::new(
            play_btn_rect.left - skip_gap - skip_size,
            row_cy - skip_size * 0.5,
            play_btn_rect.left - skip_gap,
            row_cy + skip_size * 0.5,
        );
        let next_btn_rect = RectF::new(
            play_btn_rect.right + skip_gap,
            row_cy - skip_size * 0.5,
            play_btn_rect.right + skip_gap + skip_size,
            row_cy + skip_size * 0.5,
        );

        let more_btn_rect = RectF::new(
            card_rect.left + inner_x,
            row_cy - side_size * 0.5,
            card_rect.left + inner_x + side_size,
            row_cy + side_size * 0.5,
        );
        let mute_btn_rect = RectF::new(
            right_edge - side_size,
            row_cy - side_size * 0.5,
            right_edge,
            row_cy + side_size * 0.5,
        );

        Self {
            scale: s,
            win_w,
            win_h,
            card_rect,
            card_radius,
            cover_rect,
            cover_radius,
            info_hit_rect,
            source_badge_rect,
            mini_spectrum_rects,
            title_rect,
            artist_rect,
            has_timeline,
            progress_track_rect,
            progress_hit_rect,
            elapsed_rect,
            duration_rect,
            more_btn_rect,
            prev_btn_rect,
            play_btn_rect,
            next_btn_rect,
            mute_btn_rect,
        }
    }

    /// Performs interactive hit testing against the mini-player layout.
    #[allow(dead_code)]
    pub fn hit_test(&self, x: f32, y: f32, can_seek: bool) -> MiniPlayerHitZone {
        if !self.card_rect.contains(x, y) {
            return MiniPlayerHitZone::None;
        }
        if self.has_timeline && can_seek && self.progress_hit_rect.contains(x, y) {
            return MiniPlayerHitZone::ProgressSlider;
        }
        if self.play_btn_rect.contains(x, y) {
            return MiniPlayerHitZone::PlayPauseButton;
        }
        if self.prev_btn_rect.contains(x, y) {
            return MiniPlayerHitZone::PrevButton;
        }
        if self.next_btn_rect.contains(x, y) {
            return MiniPlayerHitZone::NextButton;
        }
        if self.more_btn_rect.contains(x, y) {
            return MiniPlayerHitZone::MoreMenuButton;
        }
        if self.mute_btn_rect.contains(x, y) {
            return MiniPlayerHitZone::MuteButton;
        }
        if self.info_hit_rect.contains(x, y) {
            return MiniPlayerHitZone::CoverOrInfo;
        }
        MiniPlayerHitZone::None
    }
}

/// Computes the `(win_x, win_y)` screen origin for the mini-player popup centered cleanly above `anchor`,
/// clamped inside `[work_left, work_top, work_right, work_bottom]`.
#[allow(dead_code)]
pub fn compute_mini_player_window_origin(
    anchor_left: i32,
    anchor_top: i32,
    anchor_right: i32,
    win_w: i32,
    win_h: i32,
    dpi: u32,
    work_left: i32,
    work_top: i32,
    work_right: i32,
    work_bottom: i32,
) -> (i32, i32) {
    let s = (dpi.max(96) as f32) / 96.0;
    let shadow_pad = (crate::ui::SHADOW_PAD_DIP * s).round() as i32;
    let float_gap = (crate::ui::POPUP_FLOAT_GAP_DIP * s).round() as i32;
    let edge_margin = (6.0 * s).round() as i32;

    let anchor_cx = (anchor_left + anchor_right) / 2;
    let min_x = work_left + edge_margin - shadow_pad;
    let max_x = (work_right - edge_margin + shadow_pad - win_w).max(min_x);
    let win_x = (anchor_cx - win_w / 2).clamp(min_x, max_x);

    let desired_card_bottom = anchor_top - float_gap;
    let min_y = work_top + edge_margin - shadow_pad;
    let max_y = (work_bottom - edge_margin + shadow_pad - win_h).max(min_y);
    let win_y = (desired_card_bottom + shadow_pad - win_h).clamp(min_y, max_y);

    (win_x, win_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_and_hover_layouts_stay_in_bounds_without_overlap() {
        let levels = [0.2, 0.5, 0.9, 0.4, 0.7, 0.1, 0.6, 0.8, 0.3, 0.5];
        let rest = BarLayout::compute(48, 96, 0.0, &levels);
        assert_eq!(rest.width_px, BAR_WIDTH_DIP);
        assert!(rest.title_rect.width() > 110.0);
        assert!(rest.title_rect.right < rest.spectrum_rects[0].left);
        assert_eq!(rest.spectrum_alpha, 1.0);
        assert_eq!(rest.controls_alpha, 0.0);

        let hover = BarLayout::compute(48, 96, 1.0, &levels);
        assert!((rest.title_rect.width() - hover.title_rect.width()).abs() < 1e-3);
        assert!(hover.title_rect.right < hover.prev_btn_rect.left);
        assert!(hover.prev_btn_rect.right < hover.play_btn_rect.left);
        assert!(hover.play_btn_rect.right < hover.next_btn_rect.left);
        assert!(hover.next_btn_rect.right <= hover.width_px);
        assert_eq!(hover.spectrum_alpha, 0.0);
        assert_eq!(hover.controls_alpha, 1.0);

        // Spectrum zone and control button zone have identical width & exact bounds
        let spec_span = rest.spectrum_rects[NUM_BARS - 1].right - rest.spectrum_rects[0].left;
        let ctrl_span = hover.next_btn_rect.right - hover.prev_btn_rect.left;
        assert!((spec_span - RIGHT_ZONE_WIDTH_DIP).abs() < 1e-3);
        assert!((ctrl_span - RIGHT_ZONE_WIDTH_DIP).abs() < 1e-3);
        assert!((rest.spectrum_rects[0].left - hover.prev_btn_rect.left).abs() < 1e-3);
        assert!((rest.spectrum_rects[NUM_BARS - 1].right - hover.next_btn_rect.right).abs() < 1e-3);
    }

    #[test]
    fn hit_test_separates_media_info_and_control_buttons() {
        let levels = [0.0; NUM_BARS];
        let hover = BarLayout::compute(48, 96, 1.0, &levels);

        let cover_cx = (hover.cover_rect.left + hover.cover_rect.right) * 0.5;
        let cover_cy = (hover.cover_rect.top + hover.cover_rect.bottom) * 0.5;
        assert_eq!(hit_test(&hover, cover_cx, cover_cy, true), HitZone::MediaInfo);

        let text_cx = (hover.title_rect.left + hover.title_rect.right) * 0.5;
        let text_cy = (hover.title_rect.top + hover.title_rect.bottom) * 0.5;
        assert_eq!(hit_test(&hover, text_cx, text_cy, true), HitZone::MediaInfo);

        let prev_cx = (hover.prev_btn_rect.left + hover.prev_btn_rect.right) * 0.5;
        let btn_cy = (hover.prev_btn_rect.top + hover.prev_btn_rect.bottom) * 0.5;
        assert_eq!(hit_test(&hover, prev_cx, btn_cy, true), HitZone::PrevButton);

        let play_cx = (hover.play_btn_rect.left + hover.play_btn_rect.right) * 0.5;
        assert_eq!(hit_test(&hover, play_cx, btn_cy, true), HitZone::PlayPauseButton);

        let next_cx = (hover.next_btn_rect.left + hover.next_btn_rect.right) * 0.5;
        assert_eq!(hit_test(&hover, next_cx, btn_cy, true), HitZone::NextButton);

        // Before hover controls fade in, clicking anywhere on the bar opens the menu
        assert_eq!(hit_test(&hover, next_cx, btn_cy, false), HitZone::MediaInfo);

        assert_eq!(hit_test(&hover, -5.0, 24.0, true), HitZone::None);
    }

    #[test]
    fn volume_hud_layout_and_hit_test_mode_stay_clean() {
        let levels = [0.0; NUM_BARS];
        let layout = BarLayout::compute(48, 96, 1.0, &levels);

        assert!(layout.cover_rect.right < layout.vol_mute_btn_rect.left);
        assert!(layout.vol_mute_btn_rect.right < layout.vol_slider_hit_rect.left);
        assert!(layout.vol_slider_hit_rect.right < layout.vol_pct_rect.left);
        assert!(layout.vol_pct_rect.right <= layout.width_px);
        assert!(layout.vol_track_rect.width() > 100.0);

        let cov_cx = (layout.cover_rect.left + layout.cover_rect.right) * 0.5;
        let cy = layout.height_px * 0.5;
        assert_eq!(hit_test_mode(&layout, cov_cx, cy, true, true), HitZone::MediaInfo);

        let mute_cx = (layout.vol_mute_btn_rect.left + layout.vol_mute_btn_rect.right) * 0.5;
        assert_eq!(
            hit_test_mode(&layout, mute_cx, cy, true, true),
            HitZone::VolumeMuteButton
        );

        let slider_cx = (layout.vol_track_rect.left + layout.vol_track_rect.right) * 0.5;
        assert_eq!(
            hit_test_mode(&layout, slider_cx, cy, true, true),
            HitZone::VolumeSlider
        );
        // Tall vertical hit box above the thin track line should still hit VolumeSlider
        assert_eq!(
            hit_test_mode(&layout, slider_cx, layout.bg_rect.top + 2.0, true, true),
            HitZone::VolumeSlider
        );

        let pct_cx = (layout.vol_pct_rect.left + layout.vol_pct_rect.right) * 0.5;
        assert_eq!(hit_test_mode(&layout, pct_cx, cy, true, true), HitZone::None);

        // Slim taskbar progress hairline stays inside bg_rect bottom edge
        assert!(layout.progress_track_rect.left > layout.bg_rect.left);
        assert!(layout.progress_track_rect.right < layout.bg_rect.right);
        assert!(layout.progress_track_rect.bottom <= layout.bg_rect.bottom);
        assert!(layout.progress_track_rect.height() >= 1.5);
    }

    #[test]
    fn mini_player_layout_keeps_controls_separate_and_hit_tests_cleanly() {
        let levels = [0.2, 0.5, 0.9, 0.4, 0.7, 0.1, 0.6, 0.8, 0.3, 0.5];
        let with_tl = MiniPlayerLayout::compute(96, true, &levels);
        let no_tl = MiniPlayerLayout::compute(96, false, &levels);

        assert!(with_tl.win_h > no_tl.win_h);
        assert!(with_tl.cover_rect.right < with_tl.title_rect.left);
        assert!(with_tl.source_badge_rect.right < with_tl.mini_spectrum_rects[0].left);
        assert!(
            with_tl.mini_spectrum_rects[MINI_SPECTRUM_BARS - 1].right
                <= with_tl.title_rect.right + 1e-3
        );
        assert!(with_tl.cover_rect.bottom < with_tl.progress_track_rect.top);
        assert!(with_tl.elapsed_rect.bottom < with_tl.play_btn_rect.top);
        assert!(with_tl.more_btn_rect.right < with_tl.prev_btn_rect.left);
        assert!(with_tl.prev_btn_rect.right < with_tl.play_btn_rect.left);
        assert!(with_tl.play_btn_rect.right < with_tl.next_btn_rect.left);
        assert!(with_tl.next_btn_rect.right < with_tl.mute_btn_rect.left);
        assert!(with_tl.play_btn_rect.bottom < with_tl.card_rect.bottom);

        let more_cx = (with_tl.more_btn_rect.left + with_tl.more_btn_rect.right) * 0.5;
        let more_cy = (with_tl.more_btn_rect.top + with_tl.more_btn_rect.bottom) * 0.5;
        assert_eq!(
            with_tl.hit_test(more_cx, more_cy, true),
            MiniPlayerHitZone::MoreMenuButton
        );

        let prog_cx = (with_tl.progress_track_rect.left + with_tl.progress_track_rect.right) * 0.5;
        let prog_cy = (with_tl.progress_track_rect.top + with_tl.progress_track_rect.bottom) * 0.5;
        assert_eq!(
            with_tl.hit_test(prog_cx, prog_cy, true),
            MiniPlayerHitZone::ProgressSlider
        );
        assert_eq!(
            with_tl.hit_test(prog_cx, prog_cy, false),
            MiniPlayerHitZone::None
        );

        let play_cx = (with_tl.play_btn_rect.left + with_tl.play_btn_rect.right) * 0.5;
        let play_cy = (with_tl.play_btn_rect.top + with_tl.play_btn_rect.bottom) * 0.5;
        assert_eq!(
            with_tl.hit_test(play_cx, play_cy, true),
            MiniPlayerHitZone::PlayPauseButton
        );

        let cov_cx = (with_tl.cover_rect.left + with_tl.cover_rect.right) * 0.5;
        let cov_cy = (with_tl.cover_rect.top + with_tl.cover_rect.bottom) * 0.5;
        assert_eq!(
            with_tl.hit_test(cov_cx, cov_cy, true),
            MiniPlayerHitZone::CoverOrInfo
        );
    }

    #[test]
    fn mini_player_window_origin_centers_and_clamps_to_work_area() {
        let levels = [0.0; NUM_BARS];
        let layout = MiniPlayerLayout::compute(96, true, &levels);
        let shadow = crate::ui::SHADOW_PAD_DIP as i32;

        // Centered above middle taskbar anchor
        let (x, y) = compute_mini_player_window_origin(
            800,
            1032,
            1056,
            layout.win_w,
            layout.win_h,
            96,
            0,
            0,
            1920,
            1032,
        );
        let card_cx = x + layout.win_w / 2;
        assert!((card_cx - 928).abs() <= 1);
        assert!(y + layout.win_h - shadow < 1032);

        // Right-edge clamping keeps visible card inside work_right - 6 DIP
        let (rx, _) = compute_mini_player_window_origin(
            1750,
            1032,
            1910,
            layout.win_w,
            layout.win_h,
            96,
            0,
            0,
            1920,
            1032,
        );
        assert!(rx + layout.win_w - shadow <= 1920 - 6);
    }
}
