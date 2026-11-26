//! Cross-platform geometry, grid/list cell layout, scroll metrics, and hit-testing for DeskBox.

mod sizing;
#[cfg(test)]
mod tests;

pub use sizing::*;

use super::config::BoxViewMode;
use crate::ui::{RectF, PANEL_RADIUS_DIP, SHADOW_PAD_DIP};

/// Safety minimum box dimensions (DIP) — prevents degenerate 0px or collapsed-height windows
/// while allowing arbitrarily compact single-icon boxes.
#[cfg_attr(not(windows), allow(dead_code))]
pub const BOX_MIN_WIDTH_DIP: f32 = 60.0;
#[cfg_attr(not(windows), allow(dead_code))]
pub const BOX_MIN_HEIGHT_DIP: f32 = 48.0;
/// Default size when creating a new box (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const BOX_DEFAULT_WIDTH_DIP: f32 = 340.0;
#[cfg_attr(not(windows), allow(dead_code))]
pub const BOX_DEFAULT_HEIGHT_DIP: f32 = 300.0;
/// Height of the box header bar (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const BOX_HEADER_H_DIP: f32 = 32.0;
/// Width & height of each header action button (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const HEADER_BTN_SIZE_DIP: f32 = 20.0;
/// Gap between adjacent header action buttons (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const HEADER_BTN_GAP_DIP: f32 = 2.0;
/// Number of right-side header action buttons (`+`, Sort, View, Lock, Collapse, Delete).
#[cfg_attr(not(windows), allow(dead_code))]
pub const HEADER_ACTION_BTN_COUNT: usize = 6;
/// Outer resize grip hit zone around card edges (DIP).
#[cfg_attr(not(windows), allow(dead_code))]
pub const BOX_RESIZE_BORDER_DIP: f32 = 6.0;

/// Resize handles along box edges and corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum ResizeEdge {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Interactive hit-test zones inside one DeskBox window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum BoxHitZone {
    None,
    Resize(ResizeEdge),
    BackButton,
    SearchClearButton,
    NewButton,
    SortButton,
    ViewButton,
    LockButton,
    CollapseButton,
    DeleteButton,
    HeaderDrag,
    Item(usize),
    ItemRemove(usize),
    ScrollbarThumb,
    ScrollbarTrack,
    ContentBlank,
}

/// Geometry of a single visible item inside the box content viewport.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct BoxItemCell {
    pub index: usize,
    pub rect: RectF,
    pub icon_rect: RectF,
    pub label_rect: RectF,
    pub remove_btn_rect: RectF,
}

/// Input parameters for computing a `DeskBoxLayout`.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct DeskBoxLayoutParams {
    pub width_dip: f32,
    pub height_dip: f32,
    pub dpi: u32,
    pub item_count: usize,
    pub view_mode: BoxViewMode,
    pub scroll_y: f32,
    pub can_go_back: bool,
    pub show_item_remove: bool,
    pub collapsed: bool,
    pub show_header_actions: bool,
    pub locked: bool,
    pub auto_hide_header: bool,
    pub header_visible: bool,
    pub search_active: bool,
    pub card_height_override_px: Option<f32>,
}

/// Complete physical-pixel layout snapshot for a DeskBox window.
#[derive(Clone, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct DeskBoxLayout {
    pub scale: f32,
    pub win_w: i32,
    pub win_h: i32,
    pub collapsed: bool,
    pub show_header_actions: bool,
    pub locked: bool,
    pub auto_hide_header: bool,
    pub header_visible: bool,
    pub card_rect: RectF,
    pub card_radius: f32,
    pub header_rect: RectF,
    pub back_btn_rect: RectF,
    pub search_clear_btn_rect: RectF,
    pub new_btn_rect: RectF,
    pub sort_btn_rect: RectF,
    pub view_btn_rect: RectF,
    pub lock_btn_rect: RectF,
    pub collapse_btn_rect: RectF,
    pub delete_btn_rect: RectF,
    pub title_rect: RectF,
    pub content_rect: RectF,
    pub icon_size_px: f32,
    pub scroll_y: f32,
    pub max_scroll: f32,
    pub scrollbar_track_rect: RectF,
    pub scrollbar_thumb_rect: RectF,
    pub visible_cells: Vec<BoxItemCell>,
}

impl DeskBoxLayout {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn compute(p: DeskBoxLayoutParams) -> Self {
        let s = (p.dpi.max(96) as f32) / 96.0;
        let pad = (SHADOW_PAD_DIP * s).round();
        let header_h = BOX_HEADER_H_DIP * s;
        let card_w = (p.width_dip.max(BOX_MIN_WIDTH_DIP) * s).round();
        let card_h = p
            .card_height_override_px
            .map(|h| h.max(header_h).round())
            .unwrap_or_else(|| {
                if p.collapsed {
                    header_h.round()
                } else {
                    (p.height_dip.max(BOX_MIN_HEIGHT_DIP) * s).round()
                }
            });
        let visually_collapsed = p.collapsed || card_h <= header_h + 1.0;

        let win_w = (card_w + pad * 2.0).round() as i32;
        let win_h = (card_h + pad * 2.0).round() as i32;

        let card_rect = RectF::new(pad, pad, pad + card_w, pad + card_h);
        let card_radius = PANEL_RADIUS_DIP * s;

        let header_rect = RectF::new(
            card_rect.left,
            card_rect.top,
            card_rect.right,
            card_rect.top + header_h,
        );
        let header_cy = (header_rect.top + header_rect.bottom) * 0.5;
        let btn_size = 20.0 * s;
        let btn_gap = 2.0 * s;
        let zero_rect = RectF::new(0.0, 0.0, 0.0, 0.0);

        let show_actions = p.header_visible && p.show_header_actions && !p.search_active;
        let btn_min_left = card_rect.left + 6.0 * s;
        let (
            new_btn_rect,
            sort_btn_rect,
            view_btn_rect,
            lock_btn_rect,
            collapse_btn_rect,
            delete_btn_rect,
            leftmost_action_x,
        ) = if show_actions {
            let mut cursor_right = card_rect.right - 6.0 * s;
            let mut leftmost = card_rect.right - 6.0 * s;
            let mut next_btn = || {
                let left = cursor_right - btn_size;
                if left < btn_min_left - 0.5 {
                    return zero_rect;
                }
                let r = RectF::new(
                    left,
                    header_cy - btn_size * 0.5,
                    cursor_right,
                    header_cy + btn_size * 0.5,
                );
                leftmost = left;
                cursor_right = left - btn_gap;
                r
            };
            // Allocate right-to-left; narrow boxes gracefully drop lower-priority buttons
            // instead of overflowing past `card_rect.left`.
            let del = next_btn();
            let col = next_btn();
            let lock = next_btn();
            let view = next_btn();
            let sort = next_btn();
            let newb = next_btn();
            (newb, sort, view, lock, col, del, leftmost)
        } else {
            (
                zero_rect,
                zero_rect,
                zero_rect,
                zero_rect,
                zero_rect,
                zero_rect,
                card_rect.right - 6.0 * s,
            )
        };

        let search_clear_btn_rect = if p.search_active && p.header_visible {
            let sz = 18.0 * s;
            let right = card_rect.right - 10.0 * s;
            let left = right - sz;
            if left >= btn_min_left {
                RectF::new(
                    left,
                    header_cy - sz * 0.5,
                    right,
                    header_cy + sz * 0.5,
                )
            } else {
                zero_rect
            }
        } else {
            zero_rect
        };

        let back_btn_rect = if p.can_go_back && p.header_visible && !p.search_active {
            let back_left = card_rect.left + 6.0 * s;
            if back_left + btn_size <= card_rect.right - 6.0 * s {
                RectF::new(
                    back_left,
                    header_cy - btn_size * 0.5,
                    back_left + btn_size,
                    header_cy + btn_size * 0.5,
                )
            } else {
                zero_rect
            }
        } else {
            zero_rect
        };

        let title_left = if back_btn_rect.width() > 1.0 {
            back_btn_rect.right + 6.0 * s
        } else {
            card_rect.left + 10.0 * s
        };
        let title_right = if show_actions {
            (leftmost_action_x - 6.0 * s).max(title_left)
        } else if search_clear_btn_rect.width() > 1.0 {
            (search_clear_btn_rect.left - 4.0 * s).max(title_left)
        } else {
            (card_rect.right - 10.0 * s).max(title_left)
        };
        let title_h = 24.0 * s;
        let title_rect = RectF::new(
            title_left,
            header_cy - title_h * 0.5,
            title_right,
            header_cy + title_h * 0.5,
        );

        let metrics = ViewMetrics::for_mode(p.view_mode, s);
        if visually_collapsed {
            return Self {
                scale: s,
                win_w,
                win_h,
                collapsed: true,
                show_header_actions: show_actions,
                locked: p.locked,
                auto_hide_header: p.auto_hide_header,
                header_visible: p.header_visible,
                card_rect,
                card_radius,
                header_rect,
                back_btn_rect,
                search_clear_btn_rect,
                new_btn_rect,
                sort_btn_rect,
                view_btn_rect,
                lock_btn_rect,
                collapse_btn_rect,
                delete_btn_rect,
                title_rect,
                content_rect: zero_rect,
                icon_size_px: metrics.icon_sz,
                scroll_y: 0.0,
                max_scroll: 0.0,
                scrollbar_track_rect: zero_rect,
                scrollbar_thumb_rect: zero_rect,
                visible_cells: Vec::new(),
            };
        }

        let content_top = if p.auto_hide_header {
            card_rect.top + 6.0 * s
        } else {
            header_rect.bottom + 4.0 * s
        };
        let content_bottom = (card_rect.bottom - 5.0 * s).max(content_top);
        let content_rect = RectF::new(
            card_rect.left + 5.0 * s,
            content_top,
            card_rect.right - 5.0 * s,
            content_bottom,
        );

        let view_h = content_rect.height().max(1.0);
        let full_card_h = (p.height_dip.max(BOX_MIN_HEIGHT_DIP) * s).round();
        let full_content_bottom = (card_rect.top + full_card_h - 5.0 * s).max(content_top);
        let full_view_h = (full_content_bottom - content_top).max(1.0);

        // Always evaluate scrollbar reservation and max_scroll against the full expanded card height
        // so hover-expand/collapse height springs never narrow/widen `area_w` and shift icons.
        let raw_content_h = measure_content_height(p.item_count, content_rect.width(), &metrics);
        let needs_scrollbar = raw_content_h > full_view_h + 0.5;
        let area_right = if needs_scrollbar {
            content_rect.right - 9.0 * s
        } else {
            content_rect.right
        };
        let area_w = (area_right - content_rect.left).max(metrics.cell_min_w);
        let content_height = if needs_scrollbar {
            measure_content_height(p.item_count, area_w, &metrics)
        } else {
            raw_content_h
        };
        let max_scroll = (content_height - full_view_h).max(0.0);
        let clamped_scroll = p.scroll_y.clamp(0.0, max_scroll);

        let (scrollbar_track_rect, scrollbar_thumb_rect) = if needs_scrollbar
            && max_scroll > 0.5
            && p.card_height_override_px.is_none()
            && content_rect.height() >= 16.0 * s
        {
            let track = RectF::new(
                content_rect.right - 5.5 * s,
                content_rect.top + 2.0 * s,
                content_rect.right - 1.5 * s,
                content_rect.bottom - 2.0 * s,
            );
            let track_h = track.height().max(12.0 * s);
            let min_thumb = (22.0 * s).min(track_h);
            let thumb_h =
                ((view_h / content_height.max(1.0)) * track_h).clamp(min_thumb, track_h);
            let t = (clamped_scroll / max_scroll).clamp(0.0, 1.0);
            let thumb_top = track.top + t * (track_h - thumb_h);
            let thumb = RectF::new(track.left, thumb_top, track.right, thumb_top + thumb_h);
            (track, thumb)
        } else {
            (zero_rect, zero_rect)
        };

        let visible_cells = compute_visible_cells(
            p.item_count,
            content_rect,
            area_w,
            clamped_scroll,
            &metrics,
            p.show_item_remove,
        );

        Self {
            scale: s,
            win_w,
            win_h,
            collapsed: false,
            show_header_actions: show_actions,
            locked: p.locked,
            auto_hide_header: p.auto_hide_header,
            header_visible: p.header_visible,
            card_rect,
            card_radius,
            header_rect,
            back_btn_rect,
            search_clear_btn_rect,
            new_btn_rect,
            sort_btn_rect,
            view_btn_rect,
            lock_btn_rect,
            collapse_btn_rect,
            delete_btn_rect,
            title_rect,
            content_rect,
            icon_size_px: metrics.icon_sz,
            scroll_y: clamped_scroll,
            max_scroll,
            scrollbar_track_rect,
            scrollbar_thumb_rect,
            visible_cells,
        }
    }

    /// Returns which interactive zone contains window-local coordinates `(x, y)`.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn hit_test(&self, x: f32, y: f32) -> BoxHitZone {
        if !self.card_rect.contains(x, y) {
            return BoxHitZone::None;
        }

        let b = BOX_RESIZE_BORDER_DIP * self.scale;
        let on_left = !self.locked && x < self.card_rect.left + b;
        let on_right = !self.locked && x > self.card_rect.right - b;
        let on_top = !self.locked && !self.collapsed && y < self.card_rect.top + b;
        let on_bottom = !self.locked && !self.collapsed && y > self.card_rect.bottom - b;

        match (on_left, on_right, on_top, on_bottom) {
            (true, false, true, false) => return BoxHitZone::Resize(ResizeEdge::TopLeft),
            (false, true, true, false) => return BoxHitZone::Resize(ResizeEdge::TopRight),
            (true, false, false, true) => return BoxHitZone::Resize(ResizeEdge::BottomLeft),
            (false, true, false, true) => return BoxHitZone::Resize(ResizeEdge::BottomRight),
            (true, false, false, false) => return BoxHitZone::Resize(ResizeEdge::Left),
            (false, true, false, false) => return BoxHitZone::Resize(ResizeEdge::Right),
            _ => {}
        }

        if self.header_visible {
            if self.search_clear_btn_rect.width() > 1.0 && self.search_clear_btn_rect.contains(x, y) {
                return BoxHitZone::SearchClearButton;
            }
            if self.back_btn_rect.width() > 1.0 && self.back_btn_rect.contains(x, y) {
                return BoxHitZone::BackButton;
            }
            if self.show_header_actions {
                if self.new_btn_rect.width() > 1.0 && self.new_btn_rect.contains(x, y) {
                    return BoxHitZone::NewButton;
                }
                if self.sort_btn_rect.width() > 1.0 && self.sort_btn_rect.contains(x, y) {
                    return BoxHitZone::SortButton;
                }
                if self.view_btn_rect.width() > 1.0 && self.view_btn_rect.contains(x, y) {
                    return BoxHitZone::ViewButton;
                }
                if self.lock_btn_rect.width() > 1.0 && self.lock_btn_rect.contains(x, y) {
                    return BoxHitZone::LockButton;
                }
                if self.collapse_btn_rect.width() > 1.0 && self.collapse_btn_rect.contains(x, y) {
                    return BoxHitZone::CollapseButton;
                }
                if self.delete_btn_rect.width() > 1.0 && self.delete_btn_rect.contains(x, y) {
                    return BoxHitZone::DeleteButton;
                }
            }
        }

        if on_top {
            return BoxHitZone::Resize(ResizeEdge::Top);
        }
        if on_bottom {
            return BoxHitZone::Resize(ResizeEdge::Bottom);
        }

        if self.header_visible && self.header_rect.contains(x, y) {
            return BoxHitZone::HeaderDrag;
        }

        if self.collapsed {
            return BoxHitZone::None;
        }

        if self.max_scroll > 0.5 {
            let sb_hit = RectF::new(
                self.scrollbar_track_rect.left - 3.0 * self.scale,
                self.scrollbar_track_rect.top,
                self.scrollbar_track_rect.right + 3.0 * self.scale,
                self.scrollbar_track_rect.bottom,
            );
            if sb_hit.contains(x, y) {
                if y >= self.scrollbar_thumb_rect.top && y <= self.scrollbar_thumb_rect.bottom {
                    return BoxHitZone::ScrollbarThumb;
                }
                return BoxHitZone::ScrollbarTrack;
            }
        }

        if self.content_rect.contains(x, y) {
            for cell in &self.visible_cells {
                if cell.rect.contains(x, y) {
                    if cell.remove_btn_rect.width() > 1.0 && cell.remove_btn_rect.contains(x, y) {
                        return BoxHitZone::ItemRemove(cell.index);
                    }
                    return BoxHitZone::Item(cell.index);
                }
            }
            return BoxHitZone::ContentBlank;
        }

        BoxHitZone::None
    }

    /// Returns the target item slot index when dragging an item to reorder within `content_rect`.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn drop_target_index_at(&self, x: f32, y: f32, total_items: usize) -> Option<usize> {
        if total_items == 0 || !self.content_rect.contains(x, y) || self.visible_cells.is_empty() {
            return None;
        }
        for cell in &self.visible_cells {
            if cell.rect.contains(x, y) {
                return Some(cell.index.min(total_items - 1));
            }
        }
        let mut best: Option<(f32, usize)> = None;
        for cell in &self.visible_cells {
            let cx = (cell.rect.left + cell.rect.right) * 0.5;
            let cy = (cell.rect.top + cell.rect.bottom) * 0.5;
            let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            if best.is_none_or(|b| d2 < b.0) {
                best = Some((d2, cell.index));
            }
        }
        best.map(|(_, idx)| idx.min(total_items - 1))
    }
}

pub(super) struct ViewMetrics {
    pub(super) is_list: bool,
    pub(super) icon_sz: f32,
    pub(super) cell_min_w: f32,
    pub(super) cell_h: f32,
    pub(super) pad_top: f32,
    pub(super) row_gap: f32,
    pub(super) col_gap: f32,
    pub(super) scale: f32,
}

impl ViewMetrics {
    pub(super) fn for_mode(mode: BoxViewMode, s: f32) -> Self {
        match mode {
            BoxViewMode::LargeIcons => Self {
                is_list: false,
                icon_sz: 48.0 * s,
                cell_min_w: 84.0 * s,
                cell_h: 70.0 * s,
                pad_top: 3.0 * s,
                row_gap: 2.0 * s,
                col_gap: 2.0 * s,
                scale: s,
            },
            BoxViewMode::MediumIcons => Self {
                is_list: false,
                icon_sz: 36.0 * s,
                cell_min_w: 74.0 * s,
                cell_h: 58.0 * s,
                pad_top: 3.0 * s,
                row_gap: 2.0 * s,
                col_gap: 2.0 * s,
                scale: s,
            },
            BoxViewMode::SmallIcons => Self {
                is_list: false,
                icon_sz: 24.0 * s,
                cell_min_w: 64.0 * s,
                cell_h: 46.0 * s,
                pad_top: 2.5 * s,
                row_gap: 2.0 * s,
                col_gap: 2.0 * s,
                scale: s,
            },
            BoxViewMode::CompactList => Self {
                is_list: true,
                icon_sz: 20.0 * s,
                cell_min_w: 120.0 * s,
                cell_h: 27.0 * s,
                pad_top: 0.0,
                row_gap: 1.5 * s,
                col_gap: 0.0,
                scale: s,
            },
        }
    }
}

pub(super) fn cols_for_width(area_w: f32, m: &ViewMetrics) -> usize {
    if m.is_list {
        return 1;
    }
    (((area_w + m.col_gap) / (m.cell_min_w + m.col_gap)).floor() as usize).max(1)
}

pub(super) fn measure_content_height(item_count: usize, area_w: f32, m: &ViewMetrics) -> f32 {
    if item_count == 0 {
        return 0.0;
    }
    let cols = cols_for_width(area_w, m);
    let rows = item_count.div_ceil(cols);
    (rows as f32) * m.cell_h + ((rows.saturating_sub(1)) as f32) * m.row_gap
}

