//! DeskBox header bar rendering: title, search pill badge, navigation, and action icon buttons.

use super::super::{BoxInner, BoxMenuKind, HoverTarget, InlineEditMode};
use super::primitives::{draw_icon_btn_hover, draw_padlock_icon};
use crate::color::Rgb;
use crate::deskbox::layout::DeskBoxLayout;
use crate::ui::glass::{
    d2d_color_rgb, d2d_rect, danger_text_color, draw_text_in_rect, primary_text_color,
};
use crate::ui::{GlassSurfaceStyle, RectF};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_POINT_2F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

impl BoxInner {
    pub(super) unsafe fn draw_header(
        &mut self,
        layout: &DeskBoxLayout,
        title_fmt: &IDWriteTextFormat,
        edit_fmt: &IDWriteTextFormat,
        accent: Rgb,
        s: f32,
    ) {
        let fg = primary_text_color(self.light_glass);
        let stroke = self.round_stroke.as_ref();

        if layout.back_btn_rect.width() > 1.0 {
            if self.hover == Some(HoverTarget::BackButton) {
                draw_icon_btn_hover(&self.rt, layout.back_btn_rect, s, self.light_glass);
            }
            if let Ok(b) = self.rt.CreateSolidColorBrush(&fg, None) {
                let cx = (layout.back_btn_rect.left + layout.back_btn_rect.right) * 0.5;
                let cy = (layout.back_btn_rect.top + layout.back_btn_rect.bottom) * 0.5;
                self.rt.DrawLine(
                    D2D_POINT_2F { x: cx - 4.5 * s, y: cy },
                    D2D_POINT_2F { x: cx + 4.5 * s, y: cy },
                    &b,
                    1.4 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F { x: cx - 4.5 * s, y: cy },
                    D2D_POINT_2F {
                        x: cx - 0.8 * s,
                        y: cy - 3.6 * s,
                    },
                    &b,
                    1.4 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F { x: cx - 4.5 * s, y: cy },
                    D2D_POINT_2F {
                        x: cx - 0.8 * s,
                        y: cy + 3.6 * s,
                    },
                    &b,
                    1.4 * s,
                    stroke,
                );
            }
        }

        if self.inline_edit_mode == Some(InlineEditMode::Rename) && !self.inline_edit_hwnd.is_invalid()
        {
            self.draw_inline_rename_pill(layout, edit_fmt, accent, s);
        } else if self.search_active || self.inline_edit_mode == Some(InlineEditMode::Search) {
            self.draw_inline_search_pill(layout, edit_fmt, accent, s);
        } else if layout.title_rect.width() > 4.0 {
            let title_r = layout.title_rect;
            let title = self.header_display_title();
            if !self.light_glass
                && matches!(
                    self.surface_style,
                    GlassSurfaceStyle::Transparent
                        | GlassSurfaceStyle::LiquidGlass
                        | GlassSurfaceStyle::Blur
                )
            {
                let sh_r = RectF::new(
                    title_r.left,
                    title_r.top + 0.8 * s,
                    title_r.right,
                    title_r.bottom + 0.8 * s,
                );
                let sh_col = D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.34,
                };
                draw_text_in_rect(
                    &self.dwrite_factory,
                    &self.rt,
                    &title,
                    sh_r,
                    title_fmt,
                    sh_col,
                );
            }
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                &title,
                title_r,
                title_fmt,
                fg,
            );
        }

        if !layout.show_header_actions {
            return;
        }

        let open_kind = self.open_menu_kind_for_self();
        let stroke = self.round_stroke.as_ref();

        // 1. "+" New box dropdown trigger
        if layout.new_btn_rect.width() > 1.0 {
            let new_active =
                self.hover == Some(HoverTarget::NewButton) || open_kind == Some(BoxMenuKind::NewBox);
            if new_active {
                draw_icon_btn_hover(&self.rt, layout.new_btn_rect, s, self.light_glass);
            }
            let mut new_col = fg;
            new_col.a *= if new_active { 0.96 } else { 0.76 };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&new_col, None) {
                let cx = (layout.new_btn_rect.left + layout.new_btn_rect.right) * 0.5;
                let cy = (layout.new_btn_rect.top + layout.new_btn_rect.bottom) * 0.5;
                let arm = 4.2 * s;
                self.rt.DrawLine(
                    D2D_POINT_2F { x: cx - arm, y: cy },
                    D2D_POINT_2F { x: cx + arm, y: cy },
                    &b,
                    1.35 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F { x: cx, y: cy - arm },
                    D2D_POINT_2F { x: cx, y: cy + arm },
                    &b,
                    1.35 * s,
                    stroke,
                );
            }
        }

        // 2. "⇅" Dedicated sort dropdown trigger
        if layout.sort_btn_rect.width() > 1.0 {
            let sort_active =
                self.hover == Some(HoverTarget::SortButton) || open_kind == Some(BoxMenuKind::Sort);
            if sort_active {
                draw_icon_btn_hover(&self.rt, layout.sort_btn_rect, s, self.light_glass);
            }
            let mut sort_col = fg;
            sort_col.a *= if sort_active { 0.96 } else { 0.76 };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&sort_col, None) {
                let cx = (layout.sort_btn_rect.left + layout.sort_btn_rect.right) * 0.5;
                let cy = (layout.sort_btn_rect.top + layout.sort_btn_rect.bottom) * 0.5;
                // Left up-arrow
                let lx = cx - 2.3 * s;
                self.rt.DrawLine(
                    D2D_POINT_2F { x: lx, y: cy + 3.8 * s },
                    D2D_POINT_2F { x: lx, y: cy - 3.8 * s },
                    &b,
                    1.25 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F { x: lx, y: cy - 3.8 * s },
                    D2D_POINT_2F {
                        x: lx - 2.0 * s,
                        y: cy - 1.6 * s,
                    },
                    &b,
                    1.25 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F { x: lx, y: cy - 3.8 * s },
                    D2D_POINT_2F {
                        x: lx + 2.0 * s,
                        y: cy - 1.6 * s,
                    },
                    &b,
                    1.25 * s,
                    stroke,
                );
                // Right down-arrow
                let rx = cx + 2.3 * s;
                self.rt.DrawLine(
                    D2D_POINT_2F { x: rx, y: cy - 3.8 * s },
                    D2D_POINT_2F { x: rx, y: cy + 3.8 * s },
                    &b,
                    1.25 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F { x: rx, y: cy + 3.8 * s },
                    D2D_POINT_2F {
                        x: rx - 2.0 * s,
                        y: cy + 1.6 * s,
                    },
                    &b,
                    1.25 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F { x: rx, y: cy + 3.8 * s },
                    D2D_POINT_2F {
                        x: rx + 2.0 * s,
                        y: cy + 1.6 * s,
                    },
                    &b,
                    1.25 * s,
                    stroke,
                );
            }
        }

        // 3. View & tools 2x2 grid dropdown trigger
        if layout.view_btn_rect.width() > 1.0 {
            let view_active = self.hover == Some(HoverTarget::ViewButton)
                || open_kind == Some(BoxMenuKind::ViewAndTools);
            if view_active {
                draw_icon_btn_hover(&self.rt, layout.view_btn_rect, s, self.light_glass);
            }
            let mut view_col = fg;
            view_col.a *= if view_active { 0.96 } else { 0.76 };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&view_col, None) {
                let cx = (layout.view_btn_rect.left + layout.view_btn_rect.right) * 0.5;
                let cy = (layout.view_btn_rect.top + layout.view_btn_rect.bottom) * 0.5;
                for (dx, dy) in [(-2.3, -2.3), (2.3, -2.3), (-2.3, 2.3), (2.3, 2.3)] {
                    let tile = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: cx + (dx - 1.55) * s,
                            top: cy + (dy - 1.55) * s,
                            right: cx + (dx + 1.55) * s,
                            bottom: cy + (dy + 1.55) * s,
                        },
                        radiusX: 0.75 * s,
                        radiusY: 0.75 * s,
                    };
                    self.rt.DrawRoundedRectangle(&tile, &b, 1.15 * s, stroke);
                }
            }
        }

        // 4. Dedicated Lock position/size toggle button (accent highlighted when locked)
        if layout.lock_btn_rect.width() > 1.0 {
            let lock_hovered = self.hover == Some(HoverTarget::LockButton);
            if self.spec.locked {
                let fill_a = if self.light_glass {
                    if lock_hovered { 0.28 } else { 0.20 }
                } else if lock_hovered {
                    0.34
                } else {
                    0.26
                };
                let fill = d2d_color_rgb(accent, fill_a);
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(layout.lock_btn_rect),
                    radiusX: 6.5 * s,
                    radiusY: 6.5 * s,
                };
                if let Ok(fb) = self.rt.CreateSolidColorBrush(&fill, None) {
                    self.rt.FillRoundedRectangle(&rr, &fb);
                }
                let rim = d2d_color_rgb(accent, if self.light_glass { 0.52 } else { 0.62 });
                if let Ok(rb) = self.rt.CreateSolidColorBrush(&rim, None) {
                    self.rt.DrawRoundedRectangle(&rr, &rb, 1.0 * s, None);
                }
            } else if lock_hovered {
                draw_icon_btn_hover(&self.rt, layout.lock_btn_rect, s, self.light_glass);
            }
            let lock_col = if self.spec.locked {
                d2d_color_rgb(accent, 0.98)
            } else {
                let mut c = fg;
                c.a *= if lock_hovered { 0.96 } else { 0.74 };
                c
            };
            let lcx = (layout.lock_btn_rect.left + layout.lock_btn_rect.right) * 0.5;
            let lcy = (layout.lock_btn_rect.top + layout.lock_btn_rect.bottom) * 0.5;
            draw_padlock_icon(&self.rt, lcx, lcy, self.spec.locked, lock_col, s, stroke);
        }

        // 5. Collapse / Expand chevron
        if layout.collapse_btn_rect.width() > 1.0 {
            let col_active = self.hover == Some(HoverTarget::CollapseButton);
            if col_active {
                draw_icon_btn_hover(&self.rt, layout.collapse_btn_rect, s, self.light_glass);
            }
            let mut col_col = fg;
            col_col.a *= if col_active { 0.96 } else { 0.76 };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&col_col, None) {
                let cx = (layout.collapse_btn_rect.left + layout.collapse_btn_rect.right) * 0.5;
                let cy = (layout.collapse_btn_rect.top + layout.collapse_btn_rect.bottom) * 0.5;
                let (edge_dy, tip_dy) = if self.spec.collapsed {
                    (-1.7 * s, 2.1 * s)
                } else {
                    (1.7 * s, -2.1 * s)
                };
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 3.9 * s,
                        y: cy + edge_dy,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy + tip_dy,
                    },
                    &b,
                    1.35 * s,
                    stroke,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx,
                        y: cy + tip_dy,
                    },
                    D2D_POINT_2F {
                        x: cx + 3.9 * s,
                        y: cy + edge_dy,
                    },
                    &b,
                    1.35 * s,
                    stroke,
                );
            }
        }

        // 6. Delete trash button (semantic danger tint on hover)
        if layout.delete_btn_rect.width() <= 1.0 {
            return;
        }
        let del_active = self.hover == Some(HoverTarget::DeleteButton);
        if del_active {
            let mut danger_bg = danger_text_color(self.light_glass);
            danger_bg.a = if self.light_glass { 0.14 } else { 0.20 };
            let rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(layout.delete_btn_rect),
                radiusX: 6.5 * s,
                radiusY: 6.5 * s,
            };
            if let Ok(bg) = self.rt.CreateSolidColorBrush(&danger_bg, None) {
                self.rt.FillRoundedRectangle(&rr, &bg);
            }
        }
        let del_col = if del_active {
            danger_text_color(self.light_glass)
        } else {
            let mut c = fg;
            c.a *= 0.72;
            c
        };
        if let Ok(b) = self.rt.CreateSolidColorBrush(&del_col, None) {
            let cx = (layout.delete_btn_rect.left + layout.delete_btn_rect.right) * 0.5;
            let cy = (layout.delete_btn_rect.top + layout.delete_btn_rect.bottom) * 0.5;
            // Handle + lid
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: cx - 1.5 * s,
                    y: cy - 4.2 * s,
                },
                D2D_POINT_2F {
                    x: cx + 1.5 * s,
                    y: cy - 4.2 * s,
                },
                &b,
                1.2 * s,
                stroke,
            );
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: cx - 1.5 * s,
                    y: cy - 4.2 * s,
                },
                D2D_POINT_2F {
                    x: cx - 1.5 * s,
                    y: cy - 2.6 * s,
                },
                &b,
                1.2 * s,
                stroke,
            );
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: cx + 1.5 * s,
                    y: cy - 4.2 * s,
                },
                D2D_POINT_2F {
                    x: cx + 1.5 * s,
                    y: cy - 2.6 * s,
                },
                &b,
                1.2 * s,
                stroke,
            );
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: cx - 4.3 * s,
                    y: cy - 2.5 * s,
                },
                D2D_POINT_2F {
                    x: cx + 4.3 * s,
                    y: cy - 2.5 * s,
                },
                &b,
                1.25 * s,
                stroke,
            );
            // Bucket outline + inner ribs
            let bin_pts = [
                D2D_POINT_2F {
                    x: cx - 3.2 * s,
                    y: cy - 2.4 * s,
                },
                D2D_POINT_2F {
                    x: cx - 2.6 * s,
                    y: cy + 4.2 * s,
                },
                D2D_POINT_2F {
                    x: cx + 2.6 * s,
                    y: cy + 4.2 * s,
                },
                D2D_POINT_2F {
                    x: cx + 3.2 * s,
                    y: cy - 2.4 * s,
                },
            ];
            for seg in bin_pts.windows(2) {
                self.rt.DrawLine(seg[0], seg[1], &b, 1.2 * s, stroke);
            }
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: cx - 1.0 * s,
                    y: cy - 0.5 * s,
                },
                D2D_POINT_2F {
                    x: cx - 0.9 * s,
                    y: cy + 2.3 * s,
                },
                &b,
                1.05 * s,
                stroke,
            );
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: cx + 1.0 * s,
                    y: cy - 0.5 * s,
                },
                D2D_POINT_2F {
                    x: cx + 0.9 * s,
                    y: cy + 2.3 * s,
                },
                &b,
                1.05 * s,
                stroke,
            );
        }
    }

}
