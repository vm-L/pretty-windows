use crate::dsp::NUM_BARS;

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[inline]
    pub fn width(self) -> f32 {
        (self.right - self.left).max(0.0)
    }

    #[inline]
    pub fn height(self) -> f32 {
        (self.bottom - self.top).max(0.0)
    }

    #[inline]
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }
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
    }
}
