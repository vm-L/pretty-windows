use crate::taskbar::TaskbarStyle;
use crate::ui::{
    ease_retract, PopupAnimPhase, RectF, DIVIDER_H_DIP, INNER_PAD_DIP, ROW_GAP_DIP, ROW_H_DIP,
    SHADOW_PAD_DIP,
};

/// Logical width of the primary tray menu card in DIPs.
#[cfg_attr(not(windows), allow(dead_code))]
pub const TRAY_MENU_WIDTH_DIP: f32 = 172.0;

/// Logical width of the cascading taskbar style submenu card in DIPs.
#[cfg_attr(not(windows), allow(dead_code))]
pub const TRAY_SUBMENU_WIDTH_DIP: f32 = 148.0;

/// Interactive rows exposed in the primary system tray Liquid Glass popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum TrayMenuItem {
    MediaBar,
    MonitorBar,
    TaskbarStyle,
    Autostart,
    Exit,
}

/// Interactive hover/click targets across the primary card and its taskbar style submenu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum TrayMenuHover {
    Primary(TrayMenuItem),
    SubTaskbarStyle(TaskbarStyle),
}

/// Complete physical-pixel layout snapshot for the cascading tray menu at a given DPI & direction.
#[derive(Clone, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct TrayMenuLayout {
    pub scale: f32,
    pub win_w: i32,
    pub win_h: i32,
    pub primary_rect: RectF,
    pub sub_rect: RectF,
    pub mediabar_rect: RectF,
    pub monitorbar_rect: RectF,
    pub taskbar_style_rect: RectF,
    pub autostart_rect: RectF,
    pub exit_rect: RectF,
    pub sub_style_rects: [RectF; 5],
}

#[cfg_attr(not(windows), allow(dead_code))]
impl TrayMenuLayout {
    pub fn compute(dpi: u32, sub_open_left: bool, open_down: bool) -> Self {
        let s = (dpi.max(96) as f32) / 96.0;
        let pad = SHADOW_PAD_DIP * s;
        let pri_w = TRAY_MENU_WIDTH_DIP * s;
        let sub_w = TRAY_SUBMENU_WIDTH_DIP * s;
        let inner = INNER_PAD_DIP * s;
        let rh = ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let div_h = DIVIDER_H_DIP * s;

        let pri_h = inner * 2.0 + rh * 5.0 + gap * 3.0 + div_h;
        let sub_h = inner * 2.0 + rh * 5.0 + gap * 4.0;
        let max_card_h = pri_h.max(sub_h);

        let win_w = (pad * 2.0 + pri_w + sub_w).round() as i32;
        let win_h = (pad * 2.0 + max_card_h).round() as i32;
        let content_right = (win_w as f32) - pad;
        let content_bottom = (win_h as f32) - pad;

        let (pri_top, pri_bottom, sub_top, sub_bottom) = if open_down {
            (pad, pad + pri_h, pad, pad + sub_h)
        } else {
            (content_bottom - pri_h, content_bottom, content_bottom - sub_h, content_bottom)
        };

        let (primary_rect, sub_rect) = if sub_open_left {
            let seam_x = pad + sub_w;
            (
                RectF::new(seam_x, pri_top, content_right, pri_bottom),
                RectF::new(pad, sub_top, seam_x, sub_bottom),
            )
        } else {
            let seam_x = pad + pri_w;
            (
                RectF::new(pad, pri_top, seam_x, pri_bottom),
                RectF::new(seam_x, sub_top, content_right, sub_bottom),
            )
        };

        let px0 = primary_rect.left + inner;
        let px1 = primary_rect.right - inner;
        let mut py = primary_rect.top + inner;

        let mediabar_rect = RectF::new(px0, py, px1, py + rh);
        py = mediabar_rect.bottom + gap;

        let monitorbar_rect = RectF::new(px0, py, px1, py + rh);
        py = monitorbar_rect.bottom + gap;

        let taskbar_style_rect = RectF::new(px0, py, px1, py + rh);
        py = taskbar_style_rect.bottom + div_h;

        let autostart_rect = RectF::new(px0, py, px1, py + rh);
        py = autostart_rect.bottom + gap;

        let exit_rect = RectF::new(px0, py, px1, py + rh);

        let sx0 = sub_rect.left + inner;
        let sx1 = sub_rect.right - inner;
        let mut sy = sub_rect.top + inner;
        let mut sub_style_rects = [RectF::new(0.0, 0.0, 0.0, 0.0); 5];
        for r in &mut sub_style_rects {
            *r = RectF::new(sx0, sy, sx1, sy + rh);
            sy += rh + gap;
        }

        Self {
            scale: s,
            win_w,
            win_h,
            primary_rect,
            sub_rect,
            mediabar_rect,
            monitorbar_rect,
            taskbar_style_rect,
            autostart_rect,
            exit_rect,
            sub_style_rects,
        }
    }

    pub fn primary_rows(&self) -> [(TrayMenuItem, RectF); 5] {
        [
            (TrayMenuItem::MediaBar, self.mediabar_rect),
            (TrayMenuItem::MonitorBar, self.monitorbar_rect),
            (TrayMenuItem::TaskbarStyle, self.taskbar_style_rect),
            (TrayMenuItem::Autostart, self.autostart_rect),
            (TrayMenuItem::Exit, self.exit_rect),
        ]
    }

    pub fn sub_rows(&self) -> [(TaskbarStyle, RectF); 5] {
        [
            (TaskbarStyle::ALL[0], self.sub_style_rects[0]),
            (TaskbarStyle::ALL[1], self.sub_style_rects[1]),
            (TaskbarStyle::ALL[2], self.sub_style_rects[2]),
            (TaskbarStyle::ALL[3], self.sub_style_rects[3]),
            (TaskbarStyle::ALL[4], self.sub_style_rects[4]),
        ]
    }

    pub fn visible_contains(&self, x: f32, y: f32, submenu_open: bool) -> bool {
        self.primary_rect.contains(x, y) || (submenu_open && self.sub_rect.contains(x, y))
    }

    pub fn hit_test(&self, x: f32, y: f32, submenu_open: bool) -> Option<TrayMenuHover> {
        if self.primary_rect.contains(x, y) {
            for (item, rect) in self.primary_rows() {
                if rect.contains(x, y) {
                    return Some(TrayMenuHover::Primary(item));
                }
            }
            return None;
        }
        if submenu_open && self.sub_rect.contains(x, y) {
            for (style, rect) in self.sub_rows() {
                if rect.contains(x, y) {
                    return Some(TrayMenuHover::SubTaskbarStyle(style));
                }
            }
        }
        None
    }
}

/// Computes `(win_x, win_y, sub_open_left, open_down)` anchored to `(cursor_x, cursor_y)`
/// within full monitor bounds `[mon_left..mon_right, mon_top..mon_bottom]`.
/// Primary prefers right-up; submenu opens to the side with space (preferring right of primary).
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(windows), allow(dead_code))]
pub fn compute_tray_cascading_placement(
    cursor_x: i32,
    cursor_y: i32,
    mon_left: i32,
    mon_top: i32,
    mon_right: i32,
    mon_bottom: i32,
    dpi: u32,
) -> (i32, i32, bool, bool) {
    let base = TrayMenuLayout::compute(dpi, false, false);
    let s = base.scale;
    let pad = (SHADOW_PAD_DIP * s).round() as i32;
    let pri_w = base.primary_rect.width().round() as i32;
    let sub_w = base.sub_rect.width().round() as i32;
    let total_w = (base.win_w - pad * 2).max(1);
    let max_card_h = (base.win_h - pad * 2).max(1);
    let screen_margin = (4.0 * s).round() as i32;
    let cursor_offset = (2.0 * s).round() as i32;

    let min_pri_left = mon_left + screen_margin;
    let max_pri_left = (mon_right - screen_margin - pri_w).max(min_pri_left);
    let raw_pri_left = if cursor_x + cursor_offset + pri_w <= mon_right - screen_margin {
        cursor_x + cursor_offset
    } else {
        cursor_x - cursor_offset - pri_w
    };
    let pri_left = raw_pri_left.clamp(min_pri_left, max_pri_left);

    let right_room = (mon_right - screen_margin) - (pri_left + pri_w);
    let left_room = pri_left - (mon_left + screen_margin);
    let sub_open_left = if right_room >= sub_w {
        false
    } else if left_room >= sub_w {
        true
    } else {
        left_room > right_room
    };

    let min_card_top = mon_top + screen_margin;
    let max_card_top = (mon_bottom - screen_margin - max_card_h).max(min_card_top);
    let (raw_card_top, open_down) = if cursor_y - cursor_offset - max_card_h >= min_card_top {
        (cursor_y - cursor_offset - max_card_h, false)
    } else {
        (cursor_y + cursor_offset, true)
    };
    let card_top = raw_card_top.clamp(min_card_top, max_card_top);
    let win_y = card_top - pad;

    let placed_layout = TrayMenuLayout::compute(dpi, sub_open_left, open_down);
    let pri_offset_in_win = placed_layout.primary_rect.left.round() as i32;
    let raw_win_x = pri_left - pri_offset_in_win;
    let min_win_x = mon_left + screen_margin - pad;
    let max_win_x = (mon_right - screen_margin - total_w - pad).max(min_win_x);
    let win_x = raw_win_x.clamp(min_win_x, max_win_x);

    (win_x, win_y, sub_open_left, open_down)
}

/// Computes the animated card bounds while opening, steady, or retracting.
/// Anchors to `target.bottom` when expanding upward (`!open_down`) and `target.top` when expanding downward (`open_down`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn animated_tray_card_rect(
    target: RectF,
    win_h: f32,
    phase: PopupAnimPhase,
    open_pos: f32,
    close_progress: f32,
    open_down: bool,
    s: f32,
) -> RectF {
    let target_h = target.height().max(1.0);
    let min_h = (38.0 * s).min(target_h * 0.65);
    let h = match phase {
        PopupAnimPhase::Opening => (min_h + (target_h - min_h) * open_pos.max(0.0)).max(min_h),
        PopupAnimPhase::Open => return target,
        PopupAnimPhase::Closing => {
            let r = ease_retract(close_progress);
            (target_h * (1.0 - 0.42 * r)).max(min_h)
        }
    };
    if open_down {
        let max_bottom = (win_h - 4.0 * s).max(target.bottom);
        RectF::new(
            target.left,
            target.top,
            target.right,
            (target.top + h).min(max_bottom),
        )
    } else {
        RectF::new(
            target.left,
            (target.bottom - h).max(4.0 * s),
            target.right,
            target.bottom,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cascading_layout_orders_rows_cleanly_in_all_orientations() {
        for dpi in [96u32, 144, 192] {
            for (sub_open_left, open_down) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                let layout = TrayMenuLayout::compute(dpi, sub_open_left, open_down);
                let s = layout.scale;

                assert!(layout.primary_rect.left >= 0.0);
                assert!(layout.primary_rect.right <= layout.win_w as f32);
                assert!(layout.sub_rect.left >= 0.0);
                assert!(layout.sub_rect.right <= layout.win_w as f32);

                if sub_open_left {
                    assert!((layout.sub_rect.right - layout.primary_rect.left).abs() < 1e-3);
                } else {
                    assert!((layout.primary_rect.right - layout.sub_rect.left).abs() < 1e-3);
                }
                if open_down {
                    assert!((layout.primary_rect.top - layout.sub_rect.top).abs() < 1e-3);
                } else {
                    assert!((layout.primary_rect.bottom - layout.sub_rect.bottom).abs() < 1e-3);
                }

                assert!(layout.mediabar_rect.bottom < layout.monitorbar_rect.top);
                assert!(
                    (layout.monitorbar_rect.top - layout.mediabar_rect.bottom - ROW_GAP_DIP * s)
                        .abs()
                        < 0.05
                );
                assert!(layout.monitorbar_rect.bottom < layout.taskbar_style_rect.top);
                assert!(
                    (layout.taskbar_style_rect.top - layout.monitorbar_rect.bottom
                        - ROW_GAP_DIP * s)
                        .abs()
                        < 0.05
                );
                assert!(
                    (layout.autostart_rect.top - layout.taskbar_style_rect.bottom
                        - DIVIDER_H_DIP * s)
                        .abs()
                        < 0.05
                );
                assert!(
                    (layout.exit_rect.top - layout.autostart_rect.bottom - ROW_GAP_DIP * s).abs()
                        < 0.05
                );
                assert!(layout.exit_rect.bottom <= layout.primary_rect.bottom);

                for pair in layout.sub_style_rects.windows(2) {
                    assert!((pair[1].top - pair[0].bottom - ROW_GAP_DIP * s).abs() < 0.05);
                }
                assert!(layout.sub_style_rects[4].bottom <= layout.sub_rect.bottom);
            }
        }
    }

    #[test]
    fn hit_test_and_visible_bounds_respect_submenu_state() {
        let layout = TrayMenuLayout::compute(96, false, false);
        let px = (layout.primary_rect.left + layout.primary_rect.right) * 0.5;
        let sx = (layout.sub_rect.left + layout.sub_rect.right) * 0.5;

        assert_eq!(
            layout.hit_test(
                px,
                (layout.mediabar_rect.top + layout.mediabar_rect.bottom) * 0.5,
                false
            ),
            Some(TrayMenuHover::Primary(TrayMenuItem::MediaBar))
        );
        assert_eq!(
            layout.hit_test(
                px,
                (layout.monitorbar_rect.top + layout.monitorbar_rect.bottom) * 0.5,
                false
            ),
            Some(TrayMenuHover::Primary(TrayMenuItem::MonitorBar))
        );
        assert_eq!(
            layout.hit_test(
                px,
                (layout.taskbar_style_rect.top + layout.taskbar_style_rect.bottom) * 0.5,
                false
            ),
            Some(TrayMenuHover::Primary(TrayMenuItem::TaskbarStyle))
        );

        let div_y = (layout.taskbar_style_rect.bottom + layout.autostart_rect.top) * 0.5;
        assert!(layout.visible_contains(px, div_y, false));
        assert_eq!(layout.hit_test(px, div_y, false), None);

        // Submenu region must remain click-through and un-hittable while closed
        let sub2_y = (layout.sub_style_rects[2].top + layout.sub_style_rects[2].bottom) * 0.5;
        assert!(!layout.visible_contains(sx, sub2_y, false));
        assert_eq!(layout.hit_test(sx, sub2_y, false), None);

        // Once submenu opens, submenu rows hit accurately
        assert!(layout.visible_contains(sx, sub2_y, true));
        assert_eq!(
            layout.hit_test(sx, sub2_y, true),
            Some(TrayMenuHover::SubTaskbarStyle(TaskbarStyle::ALL[2]))
        );
    }

    #[test]
    fn cascading_placement_handles_all_four_quadrants_and_submenu_flip() {
        let layout_r = TrayMenuLayout::compute(96, false, false);
        let pad = SHADOW_PAD_DIP as i32;
        let pri_w = layout_r.primary_rect.width().round() as i32;
        let sub_w = layout_r.sub_rect.width().round() as i32;
        let pri_h = layout_r.primary_rect.height().round() as i32;
        let max_h = pri_h.max(layout_r.sub_rect.height().round() as i32);

        // 1. Mid-screen overflow click: right-up + submenu to right of primary
        let (x1, y1, sub_left1, down1) =
            compute_tray_cascading_placement(1200, 820, 0, 0, 1920, 1080, 96);
        assert!(!sub_left1);
        assert!(!down1);
        assert_eq!(x1 + pad, 1200 + 2);
        assert_eq!(y1 + pad + max_h, 820 - 2);

        // 2. Bottom-right taskbar tray click: flips primary left-up AND submenu left of primary
        let (x2, y2, sub_left2, down2) =
            compute_tray_cascading_placement(1880, 1060, 0, 0, 1920, 1080, 96);
        assert!(sub_left2);
        assert!(!down2);
        assert_eq!(x2 + pad + sub_w + pri_w, 1880 - 2);
        assert_eq!(y2 + pad + max_h, 1060 - 2);

        // 3. Cursor allows primary right, but not submenu right -> primary stays right, submenu flips left
        let (x3, _, sub_left3, down3) =
            compute_tray_cascading_placement(1700, 700, 0, 0, 1920, 1080, 96);
        assert!(sub_left3);
        assert!(!down3);
        assert_eq!(x3 + pad + sub_w, 1700 + 2);

        // 4. Top-left edge: flips downward + submenu right
        let (x4, y4, sub_left4, down4) =
            compute_tray_cascading_placement(30, 40, 0, 0, 1920, 1080, 96);
        assert!(!sub_left4);
        assert!(down4);
        assert_eq!(x4 + pad, 30 + 2);
        assert_eq!(y4 + pad, 40 + 2);
        assert!(y4 + pad + pri_h <= 1080 - 4);

        // 5. Extreme corners & secondary monitor negative bounds clamp cleanly
        for (cx, cy, ml, mt, mr, mb) in [
            (0, 0, 0, 0, 1920, 1080),
            (1919, 1079, 0, 0, 1920, 1080),
            (-5, 1078, -1920, 0, 0, 1080),
            (-1918, 2, -1920, 0, 0, 1080),
        ] {
            let (wx, wy, sl, _) = compute_tray_cascading_placement(cx, cy, ml, mt, mr, mb, 144);
            let l_hi = TrayMenuLayout::compute(144, sl, false);
            let pad_hi = (SHADOW_PAD_DIP * l_hi.scale).round() as i32;
            let margin_hi = (4.0 * l_hi.scale).round() as i32;
            let vis_w = l_hi.win_w - pad_hi * 2;
            let vis_h = l_hi.win_h - pad_hi * 2;
            assert!(wx + pad_hi >= ml + margin_hi);
            assert!(wx + pad_hi + vis_w <= mr - margin_hi);
            assert!(wy + pad_hi >= mt + margin_hi);
            assert!(wy + pad_hi + vis_h <= mb - margin_hi);
        }
    }

    #[test]
    fn animated_tray_card_anchors_cleanly_for_both_vertical_directions() {
        let layout = TrayMenuLayout::compute(96, false, false);
        let target = layout.primary_rect;
        let win_h = layout.win_h as f32;
        for step in 0..=20 {
            let p = (step as f32) / 20.0;
            let op_up = animated_tray_card_rect(
                target,
                win_h,
                PopupAnimPhase::Opening,
                0.10 + 0.96 * p,
                0.0,
                false,
                1.0,
            );
            assert!((op_up.bottom - target.bottom).abs() < 1e-4);
            assert!(op_up.top >= 4.0);
            assert!(op_up.height() >= 38.0);

            let op_dn = animated_tray_card_rect(
                target,
                win_h,
                PopupAnimPhase::Opening,
                0.10 + 0.96 * p,
                0.0,
                true,
                1.0,
            );
            assert!((op_dn.top - target.top).abs() < 1e-4);
            assert!(op_dn.bottom <= win_h - 4.0);
            assert!(op_dn.height() >= 38.0);
        }
    }
}
