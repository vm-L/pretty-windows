//! DeskBox item grid/list body rendering, marquee selection overlay, and empty-state illustration.

use super::super::{BoxInner, DragAction, HoverTarget};
use super::primitives::{
    draw_item_cell_highlight, draw_item_remove_button, draw_missing_badge, draw_placeholder_icon,
    draw_scrollbar, draw_wrapped_text,
};
use crate::color::Rgb;
use crate::deskbox::config::{BoxKind, BoxViewMode};
use crate::deskbox::layout::DeskBoxLayout;
use crate::ui::glass::{d2d_color_rgb, d2d_rect, muted_text_color, primary_text_color};
use crate::ui::{GlassSurfaceStyle, RectF};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_POINT_2F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ROUNDED_RECT, ID2D1Bitmap, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{IDWriteTextFormat, IDWriteTextLayout};

impl BoxInner {
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn draw_content_body(
        &mut self,
        layout: &DeskBoxLayout,
        resolved_icons: &[Option<ID2D1Bitmap>],
        grid_fmt: &IDWriteTextFormat,
        list_fmt: &IDWriteTextFormat,
        hint_fmt: &IDWriteTextFormat,
        accent: Rgb,
        s: f32,
    ) {
        let rt_base: ID2D1RenderTarget = self.rt.clone().into();
        self.draw_content_body_on(
            &rt_base,
            layout,
            resolved_icons,
            grid_fmt,
            list_fmt,
            hint_fmt,
            accent,
            s,
            false,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn draw_content_body_on(
        &mut self,
        rt: &ID2D1RenderTarget,
        layout: &DeskBoxLayout,
        resolved_icons: &[Option<ID2D1Bitmap>],
        grid_fmt: &IDWriteTextFormat,
        list_fmt: &IDWriteTextFormat,
        hint_fmt: &IDWriteTextFormat,
        accent: Rgb,
        s: f32,
        suppress_hover_highlight: bool,
    ) {
        let mut clip_content = layout.content_rect;
        if self.collapse_animating {
            clip_content.bottom = clip_content
                .bottom
                .min((layout.card_rect.bottom - 3.0 * s).max(clip_content.top));
        }
        rt.PushAxisAlignedClip(
            &d2d_rect(clip_content),
            D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        );

        if self.items.is_empty() {
            let hint = if self.search_active && !self.search_query.trim().is_empty() {
                "未找到匹配项目"
            } else if self.dir_scan_loading {
                "正在加载..."
            } else {
                match self.spec.kind {
                    BoxKind::Collection => "拖入文件或应用到此收纳格",
                    BoxKind::FolderPortal => {
                        if self.portal_dir_missing {
                            "关联的文件夹不存在或无法访问"
                        } else {
                            "此文件夹为空"
                        }
                    }
                }
            };
            let cx = (layout.content_rect.left + layout.content_rect.right) * 0.5;
            let cy = (layout.content_rect.top + layout.content_rect.bottom) * 0.5;
            self.draw_empty_state_icon(rt, cx, cy - 16.0 * s, s);

            let mut col = muted_text_color(self.light_glass);
            col.a *= 0.86;
            let hint_top = cy + 8.0 * s;
            let hint_bot = (layout.content_rect.bottom - 8.0 * s).max(hint_top + 18.0 * s);
            let hint_r = RectF::new(
                layout.content_rect.left + 14.0 * s,
                hint_top,
                layout.content_rect.right - 14.0 * s,
                hint_bot,
            );
            draw_wrapped_text(&self.dwrite_factory, rt, hint, hint_r, hint_fmt, col);
        } else {
            let primary_col = primary_text_color(self.light_glass);
            let is_list = self.spec.view_mode == BoxViewMode::CompactList;
            let mode_id: u8 = if is_list { 1 } else { 0 };
            let active_fmt = if is_list { list_fmt } else { grid_fmt };
            let use_text_shadow = !self.light_glass
                && matches!(
                    self.surface_style,
                    GlassSurfaceStyle::Transparent
                        | GlassSurfaceStyle::LiquidGlass
                        | GlassSurfaceStyle::Blur
                );
            let shadow_alpha_mult = if is_list { 0.36 } else { 0.38 };
            let primary_brush = rt.CreateSolidColorBrush(&primary_col, None).ok();
            let shadow_brush = if use_text_shadow {
                let sh_col = D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: shadow_alpha_mult * primary_col.a,
                };
                rt.CreateSolidColorBrush(&sh_col, None).ok()
            } else {
                None
            };
            let reorder_state = match self.drag {
                Some(DragAction::ReorderItem {
                    from_idx,
                    target_idx,
                }) => Some((from_idx, target_idx)),
                _ => None,
            };

            for (vis_idx, cell) in layout.visible_cells.iter().enumerate() {
                let Some(item) = self.items.get(cell.index) else {
                    continue;
                };
                let item_exists = item.exists;
                let is_drag_src = reorder_state.is_some_and(|(from, _)| from == cell.index);
                let remove_hovered = !suppress_hover_highlight
                    && reorder_state.is_none()
                    && self.hover == Some(HoverTarget::ItemRemove(cell.index));
                let hovered = !suppress_hover_highlight
                    && reorder_state.is_none()
                    && (self.hover == Some(HoverTarget::Item(cell.index)) || remove_hovered);
                let selected = self.selected.contains(&cell.index);
                draw_item_cell_highlight(
                    rt,
                    cell.rect,
                    hovered,
                    selected,
                    accent,
                    s,
                    self.light_glass,
                );

                // Icon
                let mut icon_alpha = if item_exists { 1.0 } else { 0.44 };
                if is_drag_src {
                    icon_alpha *= 0.45;
                }
                if let Some(Some(bmp)) = resolved_icons.get(vis_idx) {
                    rt.DrawBitmap(
                        bmp,
                        Some(&d2d_rect(cell.icon_rect)),
                        icon_alpha,
                        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                        None,
                    );
                } else {
                    draw_placeholder_icon(rt, cell.icon_rect, s, self.light_glass);
                }

                if !item_exists {
                    draw_missing_badge(rt, cell.icon_rect, s);
                }

                // Cached DirectWrite text layout + hoisted brushes (zero shaping on scroll frames)
                let label_w = cell.label_rect.width().max(4.0);
                let label_h = cell.label_rect.height().max(12.0);
                let cache_key = (
                    item.name.clone(),
                    label_w.round() as i32,
                    label_h.round() as i32,
                    mode_id,
                    self.dpi,
                );
                let text_layout: Option<IDWriteTextLayout> =
                    if let Some(cached) = self.text_layout_cache.get(&cache_key) {
                        Some(cached.clone())
                    } else {
                        if self.text_layout_cache.len() >= 512 {
                            self.text_layout_cache.clear();
                        }
                        let wide: Vec<u16> = cache_key.0.encode_utf16().collect();
                        if let Ok(created) = self
                            .dwrite_factory
                            .CreateTextLayout(&wide, active_fmt, label_w, label_h)
                        {
                            self.text_layout_cache.insert(cache_key, created.clone());
                            Some(created)
                        } else {
                            None
                        }
                    };

                if let Some(ref tl) = text_layout {
                    let is_faded = !item_exists || is_drag_src;
                    if use_text_shadow {
                        let sh_origin = D2D_POINT_2F {
                            x: cell.label_rect.left,
                            y: cell.label_rect.top + 0.85 * s,
                        };
                        if !is_faded {
                            if let Some(ref sb) = shadow_brush {
                                rt.DrawTextLayout(
                                    sh_origin,
                                    tl,
                                    sb,
                                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                                );
                            }
                        } else {
                            let mut a = shadow_alpha_mult * primary_col.a;
                            if !item_exists {
                                a *= 0.56;
                            }
                            if is_drag_src {
                                a *= 0.45;
                            }
                            let faded_sh = D2D1_COLOR_F {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a,
                            };
                            if let Ok(sb) = rt.CreateSolidColorBrush(&faded_sh, None) {
                                rt.DrawTextLayout(
                                    sh_origin,
                                    tl,
                                    &sb,
                                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                                );
                            }
                        }
                    }

                    let origin = D2D_POINT_2F {
                        x: cell.label_rect.left,
                        y: cell.label_rect.top,
                    };
                    if !is_faded {
                        if let Some(ref pb) = primary_brush {
                            rt.DrawTextLayout(
                                origin,
                                tl,
                                pb,
                                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                            );
                        }
                    } else {
                        let mut text_col = primary_col;
                        if !item_exists {
                            text_col.a *= 0.56;
                        }
                        if is_drag_src {
                            text_col.a *= 0.45;
                        }
                        if let Ok(pb) = rt.CreateSolidColorBrush(&text_col, None) {
                            rt.DrawTextLayout(
                                origin,
                                tl,
                                &pb,
                                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                            );
                        }
                    }
                }

                if (hovered || remove_hovered) && cell.remove_btn_rect.width() > 1.0 {
                    draw_item_remove_button(
                        rt,
                        cell.remove_btn_rect,
                        remove_hovered,
                        s,
                        self.light_glass,
                        self.round_stroke.as_ref(),
                    );
                }
            }

            // Reorder drop insertion indicator bar
            if let Some((from_idx, target_idx)) = reorder_state {
                if target_idx != from_idx {
                    if let Some(target_cell) = layout
                        .visible_cells
                        .iter()
                        .find(|c| c.index == target_idx)
                    {
                        let bar_col = d2d_color_rgb(accent, 0.95);
                        if let Ok(bb) = rt.CreateSolidColorBrush(&bar_col, None) {
                            let bar_rr = if self.spec.view_mode == BoxViewMode::CompactList {
                                let y = if target_idx < from_idx {
                                    target_cell.rect.top
                                } else {
                                    target_cell.rect.bottom
                                };
                                D2D1_ROUNDED_RECT {
                                    rect: D2D_RECT_F {
                                        left: target_cell.rect.left + 4.0 * s,
                                        top: y - 1.25 * s,
                                        right: target_cell.rect.right - 4.0 * s,
                                        bottom: y + 1.25 * s,
                                    },
                                    radiusX: 1.25 * s,
                                    radiusY: 1.25 * s,
                                }
                            } else {
                                let x = if target_idx < from_idx {
                                    target_cell.rect.left
                                } else {
                                    target_cell.rect.right
                                };
                                D2D1_ROUNDED_RECT {
                                    rect: D2D_RECT_F {
                                        left: x - 1.25 * s,
                                        top: target_cell.rect.top + 6.0 * s,
                                        right: x + 1.25 * s,
                                        bottom: target_cell.rect.bottom - 6.0 * s,
                                    },
                                    radiusX: 1.25 * s,
                                    radiusY: 1.25 * s,
                                }
                            };
                            rt.FillRoundedRectangle(&bar_rr, &bb);
                        }
                    }
                }
            }

            if let Some(marquee) = self.marquee_rect_in_content(layout.content_rect) {
                let fill = d2d_color_rgb(accent, 0.18);
                if let Ok(fb) = rt.CreateSolidColorBrush(&fill, None) {
                    rt.FillRectangle(&d2d_rect(marquee), &fb);
                }
                let stroke = d2d_color_rgb(accent, 0.78);
                if let Ok(sb) = rt.CreateSolidColorBrush(&stroke, None) {
                    rt.DrawRectangle(&d2d_rect(marquee), &sb, 1.0 * s, None);
                }
            }
        }

        rt.PopAxisAlignedClip();

        if layout.max_scroll > 0.5 {
            let sb_active = self.scrollbar_hovered
                || self.scroll_animating
                || matches!(self.drag, Some(DragAction::ScrollbarThumb { .. }));
            draw_scrollbar(
                rt,
                layout.scrollbar_thumb_rect,
                self.light_glass,
                sb_active,
            );
        }
    }

    unsafe fn draw_empty_state_icon(&self, rt: &ID2D1RenderTarget, cx: f32, cy: f32, s: f32) {
        let mut icon_col = primary_text_color(self.light_glass);
        icon_col.a *= 0.42;
        let Ok(brush) = rt.CreateSolidColorBrush(&icon_col, None) else {
            return;
        };
        let stroke = self.round_stroke.as_ref();
        match self.spec.kind {
            BoxKind::Collection => {
                let half = 12.0 * s;
                let rr = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - half,
                        top: cy - half,
                        right: cx + half,
                        bottom: cy + half,
                    },
                    radiusX: 5.5 * s,
                    radiusY: 5.5 * s,
                };
                rt.DrawRoundedRectangle(&rr, &brush, 1.5 * s, stroke);
                let arm = 4.2 * s;
                rt.DrawLine(
                    D2D_POINT_2F { x: cx - arm, y: cy },
                    D2D_POINT_2F { x: cx + arm, y: cy },
                    &brush,
                    1.5 * s,
                    stroke,
                );
                rt.DrawLine(
                    D2D_POINT_2F { x: cx, y: cy - arm },
                    D2D_POINT_2F { x: cx, y: cy + arm },
                    &brush,
                    1.5 * s,
                    stroke,
                );
            }
            BoxKind::FolderPortal => {
                let left = cx - 12.0 * s;
                let top = cy - 10.0 * s;
                let right = cx + 12.0 * s;
                let bot = cy + 10.0 * s;
                let pts = [
                    D2D_POINT_2F { x: left, y: top + 2.0 * s },
                    D2D_POINT_2F { x: left + 2.0 * s, y: top },
                    D2D_POINT_2F { x: left + 8.5 * s, y: top },
                    D2D_POINT_2F { x: left + 11.0 * s, y: top + 3.0 * s },
                    D2D_POINT_2F { x: right - 2.0 * s, y: top + 3.0 * s },
                    D2D_POINT_2F { x: right, y: top + 5.0 * s },
                    D2D_POINT_2F { x: right, y: bot - 2.0 * s },
                    D2D_POINT_2F { x: right - 2.0 * s, y: bot },
                    D2D_POINT_2F { x: left + 2.0 * s, y: bot },
                    D2D_POINT_2F { x: left, y: bot - 2.0 * s },
                    D2D_POINT_2F { x: left, y: top + 2.0 * s },
                ];
                for seg in pts.windows(2) {
                    rt.DrawLine(seg[0], seg[1], &brush, 1.5 * s, stroke);
                }
                rt.DrawLine(
                    D2D_POINT_2F { x: left + 2.0 * s, y: top + 6.5 * s },
                    D2D_POINT_2F { x: right - 2.0 * s, y: top + 6.5 * s },
                    &brush,
                    1.2 * s,
                    stroke,
                );
            }
        }
    }

}
