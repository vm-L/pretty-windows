//! Scroll metrics, auto-fit sizing, visible cell generation, and multi-monitor work-area clamping.

use super::{
    cols_for_width, measure_content_height, BoxItemCell, ViewMetrics, BOX_HEADER_H_DIP,
    BOX_MIN_HEIGHT_DIP, BOX_MIN_WIDTH_DIP, HEADER_ACTION_BTN_COUNT, HEADER_BTN_GAP_DIP,
    HEADER_BTN_SIZE_DIP,
};
use crate::deskbox::config::BoxViewMode;
use crate::ui::{RectF, SHADOW_PAD_DIP};

/// Computes `max_scroll` (physical px) without allocating `visible_cells`, for hot scroll ticks.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn compute_max_scroll_px(
    width_dip: f32,
    height_dip: f32,
    dpi: u32,
    item_count: usize,
    view_mode: BoxViewMode,
    auto_hide_header: bool,
    collapsed: bool,
) -> f32 {
    if collapsed || item_count == 0 {
        return 0.0;
    }
    let s = (dpi.max(96) as f32) / 96.0;
    let header_h = BOX_HEADER_H_DIP * s;
    let card_w = (width_dip.max(BOX_MIN_WIDTH_DIP) * s).round();
    let card_h = (height_dip.max(BOX_MIN_HEIGHT_DIP) * s).round();
    let content_top = if auto_hide_header {
        6.0 * s
    } else {
        header_h + 4.0 * s
    };
    let content_w = (card_w - 10.0 * s).max(1.0);
    let full_view_h = (card_h - 5.0 * s - content_top).max(1.0);
    let metrics = ViewMetrics::for_mode(view_mode, s);
    let raw_h = measure_content_height(item_count, content_w, &metrics);
    if raw_h <= full_view_h + 0.5 {
        return 0.0;
    }
    let area_w = (content_w - 9.0 * s).max(metrics.cell_min_w);
    let content_h = measure_content_height(item_count, area_w, &metrics);
    (content_h - full_view_h).max(0.0)
}

/// Estimates the logical width (DIP) of a box header title (`12.5 DIP` SemiBold) for cross-platform
/// fallback and unit tests when DirectWrite text measurement is unavailable.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn estimate_title_width_dip(title: &str) -> f32 {
    let mut w = 0.0f32;
    for ch in title.chars() {
        if ch.is_ascii() {
            w += match ch {
                ' ' | '.' | ',' | ':' | ';' | '\'' | '!' | '|' | 'i' | 'l' | 'I' | 't' | 'f'
                | 'j' => 4.2,
                'm' | 'w' | 'M' | 'W' => 10.5,
                'A'..='Z' => 8.2,
                _ => 7.0,
            };
        } else {
            // CJK / full-width glyphs at 12.5 DIP SemiBold.
            w += 13.0;
        }
    }
    w
}

/// Computes the minimum card width (DIP) needed to show the full title plus all right-side header
/// action buttons (and the left Back button when `can_go_back` is true) without truncation.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn header_min_width_for_title_dip(title_w_dip: f32, can_go_back: bool) -> f32 {
    let left_pad = if can_go_back {
        6.0 + HEADER_BTN_SIZE_DIP + 6.0
    } else {
        10.0
    };
    let buttons_w = (HEADER_ACTION_BTN_COUNT as f32) * HEADER_BTN_SIZE_DIP
        + ((HEADER_ACTION_BTN_COUNT.saturating_sub(1)) as f32) * HEADER_BTN_GAP_DIP;
    let gap_title_to_btns = 8.0;
    let right_pad = 6.0;
    (left_pad + title_w_dip.max(0.0) + gap_title_to_btns + buttons_w + right_pad).ceil()
}

/// Computes the minimum card width (DIP) required for a `CompactList` row to display `max_label_w_dip`
/// without truncating filenames, accounting for outer padding, icon, label gap, and optional remove `×` button.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn compact_list_min_width_for_label_dip(max_label_w_dip: f32, show_item_remove: bool) -> f32 {
    // Left: outer pad (5) + icon_left (4) + icon_sz (20) + label_gap (6) = 35 DIP.
    let left_chrome = 35.0;
    // Right: Collection reserves 4 (gap) + 16 (× btn) + 4 (inset) + 5 (outer pad) = 29 DIP;
    // FolderPortal reserves 6 (label right pad) + 5 (outer pad) = 11 DIP.
    let right_chrome = if show_item_remove { 29.0 } else { 11.0 };
    let cushion = 4.0;
    (left_chrome + max_label_w_dip.max(0.0) + right_chrome + cushion).ceil()
}

/// Computes the tightest `(w_dip, h_dip)` within `[work_w_dip, work_h_dip]` so every item is
/// visible without scrollbars (or as many as fit on screen when items exceed full monitor area),
/// while guaranteeing at least `min_header_w_dip` so the header title and action buttons stay intact.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(windows), allow(dead_code))]
pub fn compute_auto_fit_size_dip(
    item_count: usize,
    view_mode: BoxViewMode,
    auto_hide_header: bool,
    current_w_dip: f32,
    min_header_w_dip: f32,
    preferred_list_w_dip: f32,
    work_w_dip: f32,
    work_h_dip: f32,
) -> (f32, f32) {
    let max_w = work_w_dip.max(BOX_MIN_WIDTH_DIP);
    let max_h = work_h_dip.max(BOX_MIN_HEIGHT_DIP);
    let min_w = min_header_w_dip.clamp(BOX_MIN_WIDTH_DIP, max_w);
    let outer_x = 10.0f32;
    let outer_top = if auto_hide_header {
        6.0
    } else {
        BOX_HEADER_H_DIP + 4.0
    };
    let outer_y = outer_top + 5.0;
    let m = ViewMetrics::for_mode(view_mode, 1.0);

    if item_count == 0 {
        let w = min_w.clamp(BOX_MIN_WIDTH_DIP, max_w);
        let h = (outer_y + m.cell_h).clamp(BOX_MIN_HEIGHT_DIP, max_h);
        return (w, h);
    }

    if m.is_list {
        let content_h =
            (item_count as f32) * m.cell_h + ((item_count.saturating_sub(1)) as f32) * m.row_gap;
        let raw_h = outer_y + content_h + 1.0;
        let needs_sb = raw_h > max_h;
        let sb_extra = if needs_sb { 9.0 } else { 0.0 };
        let list_target_w = (preferred_list_w_dip + sb_extra).max(min_w).max(160.0);
        let w = list_target_w.ceil().clamp(BOX_MIN_WIDTH_DIP, max_w);
        // +1 DIP rounding cushion guarantees zero scrollbar across fractional DPI scales (125%/150%).
        let h = raw_h.ceil().clamp(BOX_MIN_HEIGHT_DIP, max_h);
        return (w, h);
    }

    let max_area_w = (max_w - outer_x).max(m.cell_min_w);
    let max_area_h = (max_h - outer_y).max(m.cell_h);
    let max_cols_screen =
        (((max_area_w + m.col_gap) / (m.cell_min_w + m.col_gap)).floor() as usize).max(1);
    let max_rows_screen =
        (((max_area_h + m.row_gap) / (m.cell_h + m.row_gap)).floor() as usize).max(1);

    let cur_area_w = (current_w_dip - outer_x).max(m.cell_min_w);
    let cur_cols = cols_for_width(cur_area_w, &m).min(max_cols_screen);
    let header_area_w = (min_w - outer_x).max(m.cell_min_w);
    let header_cols = cols_for_width(header_area_w, &m).min(max_cols_screen);

    let mut target_cols = cur_cols.max(header_cols).min(item_count).max(1);
    if item_count.div_ceil(target_cols) > max_rows_screen {
        target_cols = item_count
            .div_ceil(max_rows_screen)
            .clamp(1, max_cols_screen);
    }

    // +1 DIP cushion prevents fractional DPI rounding from shrinking `area_w` by 0.5px and dropping a column.
    let grid_w = outer_x
        + (target_cols as f32) * m.cell_min_w
        + ((target_cols.saturating_sub(1)) as f32) * m.col_gap
        + 1.0;
    let fit_w = grid_w.max(min_w).ceil().clamp(BOX_MIN_WIDTH_DIP, max_w);
    let actual_cols = cols_for_width((fit_w - outer_x).max(m.cell_min_w), &m).max(1);
    let target_rows = item_count.div_ceil(actual_cols).max(1);
    let fit_h = (outer_y
        + (target_rows as f32) * m.cell_h
        + ((target_rows.saturating_sub(1)) as f32) * m.row_gap
        + 1.0)
        .ceil()
        .clamp(BOX_MIN_HEIGHT_DIP, max_h);
    (fit_w, fit_h)
}

pub(super) fn compute_visible_cells(
    item_count: usize,
    content_rect: RectF,
    area_w: f32,
    scroll_y: f32,
    m: &ViewMetrics,
    show_item_remove: bool,
) -> Vec<BoxItemCell> {
    if item_count == 0 {
        return Vec::new();
    }
    let cols = cols_for_width(area_w, m);
    let stride_y = m.cell_h + m.row_gap;
    let first_row = ((scroll_y / stride_y).floor().max(0.0) as usize) * cols;
    let max_vis_y = scroll_y + content_rect.height();

    let (cell_w, col_step, x_offset) = if m.is_list {
        (area_w, area_w, 0.0)
    } else {
        // Keep grid cell width and inter-column gap strictly fixed so drag-resizing the box
        // never stretches cells or jitters icon positions between column breakpoints.
        let cw = m.cell_min_w.min(area_w);
        (cw, cw + m.col_gap, 0.0)
    };

    let mut cells = Vec::new();
    for idx in first_row..item_count {
        let row = idx / cols;
        let col = idx % cols;
        let content_top = (row as f32) * stride_y;
        if content_top > max_vis_y {
            break;
        }
        let cell_top = content_rect.top + content_top - scroll_y;
        let cell_bot = cell_top + m.cell_h;
        if cell_bot < content_rect.top {
            continue;
        }
        let cell_left = content_rect.left + x_offset + (col as f32) * col_step;
        let cell_rect = RectF::new(cell_left, cell_top, cell_left + cell_w, cell_bot);

        let btn_sz = 16.0 * m.scale;
        let (icon_rect, label_rect, remove_btn_rect) = if m.is_list {
            let cy = (cell_rect.top + cell_rect.bottom) * 0.5;
            let icon_left = cell_rect.left + 4.0 * m.scale;
            let icon_r = RectF::new(
                icon_left,
                cy - m.icon_sz * 0.5,
                icon_left + m.icon_sz,
                cy + m.icon_sz * 0.5,
            );
            let rm_r = if show_item_remove {
                let rm_right = cell_rect.right - 4.0 * m.scale;
                RectF::new(
                    (rm_right - btn_sz).max(icon_r.right + 8.0 * m.scale),
                    cy - btn_sz * 0.5,
                    rm_right,
                    cy + btn_sz * 0.5,
                )
            } else {
                RectF::new(0.0, 0.0, 0.0, 0.0)
            };
            let label_right = if show_item_remove {
                (rm_r.left - 4.0 * m.scale).max(icon_r.right + 8.0 * m.scale)
            } else {
                (cell_rect.right - 6.0 * m.scale).max(icon_r.right + 8.0 * m.scale)
            };
            let label_r = RectF::new(
                icon_r.right + 6.0 * m.scale,
                cell_rect.top,
                label_right,
                cell_rect.bottom,
            );
            (icon_r, label_r, rm_r)
        } else {
            let cx = (cell_rect.left + cell_rect.right) * 0.5;
            let icon_top = cell_rect.top + m.pad_top;
            let icon_r = RectF::new(
                cx - m.icon_sz * 0.5,
                icon_top,
                cx + m.icon_sz * 0.5,
                icon_top + m.icon_sz,
            );
            let label_r = RectF::new(
                cell_rect.left + 3.0 * m.scale,
                icon_r.bottom + 0.5 * m.scale,
                cell_rect.right - 3.0 * m.scale,
                (cell_rect.bottom - 2.0 * m.scale).max(icon_r.bottom + 14.0 * m.scale),
            );
            let rm_r = if show_item_remove {
                let inset = 2.0 * m.scale;
                RectF::new(
                    (cell_rect.right - inset - btn_sz).max(cell_rect.left),
                    cell_rect.top + inset,
                    cell_rect.right - inset,
                    (cell_rect.top + inset + btn_sz).min(cell_rect.bottom),
                )
            } else {
                RectF::new(0.0, 0.0, 0.0, 0.0)
            };
            (icon_r, label_r, rm_r)
        };

        cells.push(BoxItemCell {
            index: idx,
            rect: cell_rect,
            icon_rect,
            label_rect,
            remove_btn_rect,
        });
    }
    cells
}

/// Clamps logical box origin & dimensions to stay inside a monitor work area rectangle.
/// When `collapsed` is true, `h_dip` (the saved expanded height) is preserved while
/// `win_h` and vertical placement clamping use `BOX_HEADER_H_DIP`.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(windows), allow(dead_code))]
pub fn clamp_box_bounds_to_work_area(
    x_dip: f32,
    y_dip: f32,
    w_dip: f32,
    h_dip: f32,
    collapsed: bool,
    work_left_px: i32,
    work_top_px: i32,
    work_right_px: i32,
    work_bottom_px: i32,
    dpi: u32,
) -> (f32, f32, f32, f32, i32, i32, i32, i32) {
    let s = (dpi.max(96) as f32) / 96.0;
    let margin = (6.0 * s).round();
    let pad = (SHADOW_PAD_DIP * s).round() as i32;

    let work_w_dip = ((work_right_px - work_left_px).max(260) as f32 - margin * 2.0) / s;
    let work_h_dip = ((work_bottom_px - work_top_px).max(220) as f32 - margin * 2.0) / s;
    let clamped_w_dip = w_dip.clamp(BOX_MIN_WIDTH_DIP, work_w_dip.max(BOX_MIN_WIDTH_DIP));
    let clamped_h_dip = h_dip.clamp(BOX_MIN_HEIGHT_DIP, work_h_dip.max(BOX_MIN_HEIGHT_DIP));
    let eff_h_dip = if collapsed {
        BOX_HEADER_H_DIP
    } else {
        clamped_h_dip
    };

    let max_x_dip = (work_w_dip - clamped_w_dip).max(0.0);
    let max_y_dip = (work_h_dip - eff_h_dip).max(0.0);
    let clamped_x_dip = x_dip.clamp(0.0, max_x_dip);
    let clamped_y_dip = y_dip.clamp(0.0, max_y_dip);

    let card_left_px = work_left_px + margin as i32 + (clamped_x_dip * s).round() as i32;
    let card_top_px = work_top_px + margin as i32 + (clamped_y_dip * s).round() as i32;
    let card_w_px = (clamped_w_dip * s).round() as i32;
    let card_h_px = (eff_h_dip * s).round() as i32;

    let win_x = card_left_px - pad;
    let win_y = card_top_px - pad;
    let win_w = card_w_px + pad * 2;
    let win_h = card_h_px + pad * 2;

    (
        clamped_x_dip,
        clamped_y_dip,
        clamped_w_dip,
        clamped_h_dip,
        win_x,
        win_y,
        win_w,
        win_h,
    )
}

/// Cross-platform monitor work-area snapshot used for deterministic multi-monitor placement & fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct MonitorWorkRect {
    pub monitor_id: String,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub dpi: u32,
    pub is_primary: bool,
}

/// Resolved logical & physical DeskBox bounds paired with the matched monitor index.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct ResolvedMonitorBounds {
    pub x_dip: f32,
    pub y_dip: f32,
    pub w_dip: f32,
    pub h_dip: f32,
    pub win_x: i32,
    pub win_y: i32,
    pub win_w: i32,
    pub win_h: i32,
    pub monitor_idx: usize,
}

/// Resolves a box's target monitor (matching `monitor_id` case-insensitively, falling back to
/// primary or first monitor after hot-unplug) and returns clamped logical/physical bounds.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn resolve_monitor_and_clamp_bounds(
    monitors: &[MonitorWorkRect],
    monitor_id: &str,
    x_dip: f32,
    y_dip: f32,
    w_dip: f32,
    h_dip: f32,
    collapsed: bool,
) -> Option<ResolvedMonitorBounds> {
    if monitors.is_empty() {
        return None;
    }
    let trimmed = monitor_id.trim();
    let idx = if !trimmed.is_empty() {
        monitors
            .iter()
            .position(|m| m.monitor_id.eq_ignore_ascii_case(trimmed))
            .or_else(|| monitors.iter().position(|m| m.is_primary))
            .unwrap_or(0)
    } else {
        monitors.iter().position(|m| m.is_primary).unwrap_or(0)
    };
    let m = &monitors[idx];
    let (cx, cy, cw, ch, wx, wy, ww, wh) = clamp_box_bounds_to_work_area(
        x_dip,
        y_dip,
        w_dip,
        h_dip,
        collapsed,
        m.left,
        m.top,
        m.right,
        m.bottom,
        m.dpi,
    );
    Some(ResolvedMonitorBounds {
        x_dip: cx,
        y_dip: cy,
        w_dip: cw,
        h_dip: ch,
        win_x: wx,
        win_y: wy,
        win_w: ww,
        win_h: wh,
        monitor_idx: idx,
    })
}

