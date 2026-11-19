//! Geometric layout calculation for the MonitorBar taskbar widget.

use super::config::{MetricItem, MonitorConfig};
pub use crate::ui::RectF;

/// Horizontal outer padding on left and right edges of MonitorBar (DIP at 96 DPI).
pub const PAD_X_DIP: f32 = 7.0;
/// Horizontal gap between adjacent metric columns (DIP at 96 DPI).
pub const COL_GAP_DIP: f32 = 8.0;
/// Logical width of the network upload/download column (DIP at 96 DPI).
pub const NET_COL_W_DIP: f32 = 76.0;
/// Logical width of CPU/Memory and GPU/Disk percentage columns (DIP at 96 DPI).
pub const PCT_COL_W_DIP: f32 = 58.0;

/// Individual display rows rendered inside MonitorBar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricSlot {
    NetUp,
    NetDown,
    Cpu,
    Memory,
    Gpu,
    Disk,
}

impl MetricSlot {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn label(self) -> &'static str {
        match self {
            MetricSlot::NetUp => "↑:",
            MetricSlot::NetDown => "↓:",
            MetricSlot::Cpu => "CPU:",
            MetricSlot::Memory => "内存:",
            MetricSlot::Gpu => "显卡:",
            MetricSlot::Disk => "硬盘:",
        }
    }

    pub fn is_network(self) -> bool {
        matches!(self, MetricSlot::NetUp | MetricSlot::NetDown)
    }
}

/// Resolved draw rectangles for a single metric slot in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotCell {
    pub slot: MetricSlot,
    pub label_rect: RectF,
    pub value_rect: RectF,
}

/// Complete physical-pixel layout snapshot for MonitorBar at a given height, DPI, and config.
#[derive(Clone, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct MonitorLayout {
    pub scale: f32,
    pub width_px: f32,
    pub height_px: f32,
    pub bg_rect: RectF,
    pub bg_radius: f32,
    pub cells: Vec<SlotCell>,
}

impl MonitorLayout {
    /// Computes the logical DIP width required for the active metric columns.
    pub fn desired_width_dip(cfg: &MonitorConfig) -> f32 {
        let mut normalized = cfg.clone();
        normalized.ensure_at_least_one_metric();

        let mut col_widths = Vec::with_capacity(3);
        if normalized.is_visible(MetricItem::Network) {
            col_widths.push(NET_COL_W_DIP);
        }
        if normalized.is_visible(MetricItem::Memory) || normalized.is_visible(MetricItem::Disk) {
            col_widths.push(PCT_COL_W_DIP);
        }
        if normalized.is_visible(MetricItem::Cpu) || normalized.is_visible(MetricItem::Gpu) {
            col_widths.push(PCT_COL_W_DIP);
        }

        let content_w: f32 = col_widths.iter().sum();
        let gaps_w = (col_widths.len().saturating_sub(1) as f32) * COL_GAP_DIP;
        PAD_X_DIP * 2.0 + content_w + gaps_w
    }

    /// Computes physical pixel rects for active metric slots and the hover glass capsule.
    pub fn compute(height_px: i32, dpi: u32, cfg: &MonitorConfig) -> Self {
        let mut normalized = cfg.clone();
        normalized.ensure_at_least_one_metric();

        let scale = (dpi.max(96) as f32) / 96.0;
        let width_px = (Self::desired_width_dip(&normalized) * scale).round();
        let h_px = (height_px.max(28)) as f32;

        // Match MediaBar hover pill inset and radius so adjacent taskbar widgets look cohesive.
        let bg_margin_x = 2.0 * scale;
        let bg_margin_y = (3.5 * scale).min((h_px - 26.0 * scale).max(2.0) * 0.5);
        let bg_rect = RectF::new(
            bg_margin_x,
            bg_margin_y,
            width_px - bg_margin_x,
            h_px - bg_margin_y,
        );
        let bg_radius = 7.5 * scale;

        let mut columns: Vec<(f32, Vec<MetricSlot>)> = Vec::with_capacity(3);
        if normalized.is_visible(MetricItem::Network) {
            columns.push((
                NET_COL_W_DIP * scale,
                vec![MetricSlot::NetUp, MetricSlot::NetDown],
            ));
        }

        let mut mid_slots = Vec::with_capacity(2);
        if normalized.is_visible(MetricItem::Memory) {
            mid_slots.push(MetricSlot::Memory);
        }
        if normalized.is_visible(MetricItem::Disk) {
            mid_slots.push(MetricSlot::Disk);
        }
        if !mid_slots.is_empty() {
            columns.push((PCT_COL_W_DIP * scale, mid_slots));
        }

        let mut right_slots = Vec::with_capacity(2);
        if normalized.is_visible(MetricItem::Cpu) {
            right_slots.push(MetricSlot::Cpu);
        }
        if normalized.is_visible(MetricItem::Gpu) {
            right_slots.push(MetricSlot::Gpu);
        }
        if !right_slots.is_empty() {
            columns.push((PCT_COL_W_DIP * scale, right_slots));
        }

        let row_gap = 1.0 * scale;
        let row_h = (16.5 * scale).min((h_px - 8.0 * scale - row_gap) * 0.5).max(12.0);
        let col_gap = COL_GAP_DIP * scale;
        let mut cursor_x = PAD_X_DIP * scale;
        let mut cells = Vec::with_capacity(6);

        for (col_idx, (col_w, slots)) in columns.into_iter().enumerate() {
            if col_idx > 0 {
                cursor_x += col_gap;
            }
            let total_rows_h = if slots.len() == 2 {
                row_h * 2.0 + row_gap
            } else {
                row_h
            };
            let start_y = (h_px - total_rows_h) * 0.5;

            for (row_idx, slot) in slots.into_iter().enumerate() {
                let top = start_y + (row_idx as f32) * (row_h + row_gap);
                let bottom = top + row_h;
                let (label_reserve, label_pad) = if slot.is_network() {
                    (13.5 * scale, 17.5 * scale)
                } else {
                    (28.5 * scale, 34.0 * scale)
                };
                let label_rect = RectF::new(
                    cursor_x,
                    top,
                    (cursor_x + label_pad).min(cursor_x + col_w),
                    bottom,
                );
                let value_rect = RectF::new(
                    (cursor_x + label_reserve).min(cursor_x + col_w),
                    top,
                    cursor_x + col_w,
                    bottom,
                );
                cells.push(SlotCell {
                    slot,
                    label_rect,
                    value_rect,
                });
            }

            cursor_x += col_w;
        }

        Self {
            scale,
            width_px,
            height_px: h_px,
            bg_rect,
            bg_radius,
            cells,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_has_three_columns_and_six_slots() {
        let cfg = MonitorConfig::default();
        assert!((MonitorLayout::desired_width_dip(&cfg) - 222.0).abs() < 0.01);

        let layout = MonitorLayout::compute(48, 96, &cfg);
        assert_eq!(layout.cells.len(), 6);
        assert_eq!(layout.width_px as i32, 222);
        let slots: Vec<MetricSlot> = layout.cells.iter().map(|c| c.slot).collect();
        assert_eq!(
            slots,
            vec![
                MetricSlot::NetUp,
                MetricSlot::NetDown,
                MetricSlot::Memory,
                MetricSlot::Disk,
                MetricSlot::Cpu,
                MetricSlot::Gpu,
            ]
        );
        // Row-major visual reading alignment: top row (NetUp / Memory / Cpu), bottom row (NetDown / Disk / Gpu).
        assert!((layout.cells[0].label_rect.top - layout.cells[2].label_rect.top).abs() < 0.01);
        assert!((layout.cells[2].label_rect.top - layout.cells[4].label_rect.top).abs() < 0.01);
        assert!((layout.cells[1].label_rect.top - layout.cells[3].label_rect.top).abs() < 0.01);
        assert!((layout.cells[3].label_rect.top - layout.cells[5].label_rect.top).abs() < 0.01);
    }

    #[test]
    fn toggling_items_contracts_width_and_centers_single_row() {
        let cfg = MonitorConfig {
            show_network: false,
            show_memory: true,
            show_disk: false,
            show_cpu: true,
            show_gpu: true,
            ..MonitorConfig::default()
        };
        let layout = MonitorLayout::compute(48, 96, &cfg);
        assert_eq!(layout.cells.len(), 3);
        let expected_w = PAD_X_DIP * 2.0 + PCT_COL_W_DIP * 2.0 + COL_GAP_DIP;
        assert!((layout.width_px - expected_w).abs() < 0.5);

        // Single-row column centers vertically inside the 48px bar.
        let mem_cell = layout.cells[0];
        let cpu_cell = layout.cells[1];
        let gpu_cell = layout.cells[2];
        assert_eq!(mem_cell.slot, MetricSlot::Memory);
        assert_eq!(cpu_cell.slot, MetricSlot::Cpu);
        assert_eq!(gpu_cell.slot, MetricSlot::Gpu);
        assert!(mem_cell.label_rect.top > cpu_cell.label_rect.top);
        assert!(mem_cell.label_rect.bottom < gpu_cell.label_rect.bottom);
    }

    #[test]
    fn layout_scales_proportionally_at_150_percent_dpi() {
        let cfg = MonitorConfig::default();
        let l100 = MonitorLayout::compute(48, 96, &cfg);
        let l150 = MonitorLayout::compute(72, 144, &cfg);
        assert!((l150.scale - 1.5).abs() < 0.001);
        assert!((l150.width_px - l100.width_px * 1.5).abs() <= 1.0);
    }
}
