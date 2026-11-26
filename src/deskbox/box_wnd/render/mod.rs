//! Direct2D / DirectWrite Liquid Glass rendering for a DeskBox desktop organizer window.

mod chrome;
mod content;
mod header;
mod inline_edit;
mod primitives;

use super::{BoxInner, DragAction, DESKBOX_GLASS_REF_H_DIP, WM_APP_DESKBOX_ICON_READY};
use primitives::fill_grad;
use crate::color::{Rgb, NEUTRAL_BACKDROP_FALLBACK};
use crate::deskbox::layout::DeskBoxLayout;
use crate::deskbox::snap::DockEdge;
use crate::ui::glass::{
    d2d_color_rgb, d2d_rect, draw_rounded_card_rim, identity_matrix, update_silk_bitmap,
};
use crate::ui::{popup_glass_recipe, rgb_luma, GlassSurfaceStyle, RectF, SHADOW_PAD_DIP};
use std::mem::ManuallyDrop;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_ROUNDED_RECT, ID2D1Bitmap,
    ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

impl BoxInner {
    pub(in super::super) fn render_once(&mut self) -> windows::core::Result<()> {
        if self.is_edge_tucked() {
            return self.render_tucked_edge_tab();
        }
        let layout = self.current_layout();
        let s = layout.scale;
        // Keep the DIB surface at the full animation envelope height during collapse/expand so
        // intermediate frames never re-allocate `CreateDIBSection` or force DWM texture resizes.
        let surf_w = layout.win_w;
        let surf_h = if self.collapse_animating {
            self.win_h.max(layout.win_h)
        } else {
            layout.win_h
        };
        self.surface.ensure_size(surf_w, surf_h)?;
        let (title_fmt, edit_fmt, grid_fmt, list_fmt, hint_fmt) = self.ensure_formats(s)?;

        let recipe = popup_glass_recipe(
            self.surface_style,
            self.light_glass,
            rgb_luma(NEUTRAL_BACKDROP_FALLBACK),
        );
        let accent = Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        );

        let full_card_h = self
            .full_expanded_card_height_px(s)
            .max(layout.card_rect.height());
        let glass_ref_h = (DESKBOX_GLASS_REF_H_DIP * s).round().max(1.0);
        let grad_rect = RectF::new(
            layout.card_rect.left,
            layout.card_rect.top,
            layout.card_rect.right,
            layout.card_rect.top + glass_ref_h,
        );

        let allow_silk_regen = !self.edge_animating
            && !self.scroll_animating
            && !matches!(self.drag, Some(DragAction::ScrollbarThumb { .. }));
        if self.surface_style == GlassSurfaceStyle::LiquidGlass
            && recipe.silk_alpha > 0.002
            && (allow_silk_regen || self.silk_bmp.is_none())
        {
            self.flow_phase = crate::ui::shared_liquid_flow_phase();
            let prev_bmp_ptr = self
                .silk_bmp
                .as_ref()
                .map(|b| windows::core::Interface::as_raw(b) as usize);
            let force_silk = self.silk_bmp.is_none();
            let silk_w_px = layout.card_rect.width().round().max(1.0) as i32;
            let silk_h_px = full_card_h.round().max(1.0) as i32;
            update_silk_bitmap(
                &self.rt,
                &mut self.silk_bmp,
                &mut self.silk_buf,
                &mut self.last_silk_tick,
                force_silk,
                silk_w_px,
                silk_h_px,
                0.0,
                glass_ref_h,
                s,
                self.flow_phase,
                &self.halo_colors,
                self.light_glass,
            );
            let next_bmp_ptr = self
                .silk_bmp
                .as_ref()
                .map(|b| windows::core::Interface::as_raw(b) as usize);
            if next_bmp_ptr != prev_bmp_ptr {
                self.silk_brush = None;
            }
        }

        let card_rect = layout.card_rect;
        let card_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(card_rect),
            radiusX: layout.card_radius,
            radiusY: layout.card_radius,
        };
        let interactive_sizing = self.collapse_animating
            || matches!(self.drag, Some(DragAction::Resize { .. }));
        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            // During collapse/expand springs, bake the static expanded glass + content once into an
            // offscreen bitmap brush so every 120 FPS frame blits the full interior via a single
            // `FillRoundedRectangle` without re-rasterizing icons or DirectWrite text on CPU D2D.
            let anim_body_brush = if self.collapse_animating {
                let pad = (SHADOW_PAD_DIP * s).round() as i32;
                let card_w_px = card_rect.width().round().max(1.0) as i32;
                let full_card_h_px = (surf_h - pad * 2).max(1);
                self.ensure_anim_body_brush(
                    card_w_px,
                    full_card_h_px,
                    glass_ref_h,
                    &recipe,
                    &grid_fmt,
                    &list_fmt,
                    &hint_fmt,
                    accent,
                    s,
                )
            } else {
                None
            };
            let resolved_icons = if anim_body_brush.is_some() {
                Vec::new()
            } else {
                self.resolve_icons_for_layout(&layout)
            };
            // During normal / scroll frames, cache the static shadows + rounded-clipped glass
            // backdrop into an offscreen bitmap so each scroll frame skips 3-pass shadow geometry,
            // noise/silk/gradient fills, and CPU `PushLayer(geometricMask)` alpha compositing.
            let chrome_bmp = if !interactive_sizing {
                self.ensure_chrome_bitmap(
                    card_rect,
                    &card_rr,
                    layout.card_radius,
                    grad_rect,
                    surf_w,
                    surf_h,
                    &recipe,
                    s,
                )
            } else {
                None
            };

            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            if let Some(ref anim_brush) = anim_body_brush {
                self.ensure_and_draw_card_shadows(
                    card_rect,
                    &card_rr,
                    layout.card_radius,
                    surf_w,
                    surf_h,
                    s,
                );
                anim_brush.SetTransform(&Matrix3x2 {
                    M11: 1.0,
                    M12: 0.0,
                    M21: 0.0,
                    M22: 1.0,
                    M31: card_rect.left,
                    M32: card_rect.top,
                });
                self.rt.FillRoundedRectangle(&card_rr, anim_brush);
                self.draw_silk_overlay(card_rect, &card_rr, full_card_h, &recipe);
                self.draw_header_only(&layout, &title_fmt, &edit_fmt, accent, s);
            } else if let Some(ref bmp) = chrome_bmp {
                let full_d2d = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: surf_w as f32,
                    bottom: surf_h as f32,
                };
                self.rt.DrawBitmap(
                    bmp,
                    Some(&full_d2d),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                    None,
                );
                self.draw_silk_overlay(card_rect, &card_rr, full_card_h, &recipe);
                self.draw_foreground_contents(
                    &layout,
                    &resolved_icons,
                    &title_fmt,
                    &edit_fmt,
                    &grid_fmt,
                    &list_fmt,
                    &hint_fmt,
                    accent,
                    s,
                );
            } else {
                self.ensure_and_draw_card_shadows(
                    card_rect,
                    &card_rr,
                    layout.card_radius,
                    surf_w,
                    surf_h,
                    s,
                );

                // Fast zero-PushLayer path during live resize drags:
                // native `FillRoundedRectangle` renders rounded glass + silk directly on the DIB
                // without allocating/compositing a full-card CPU `geometricMask` layer every frame.
                let rt_base: ID2D1RenderTarget = self.rt.clone().into();
                self.draw_card_glass_body_rounded_on(
                    &rt_base,
                    &card_rect,
                    &card_rr,
                    &grad_rect,
                    &recipe,
                );
                self.draw_silk_overlay(card_rect, &card_rr, full_card_h, &recipe);
                self.draw_foreground_contents(
                    &layout,
                    &resolved_icons,
                    &title_fmt,
                    &edit_fmt,
                    &grid_fmt,
                    &list_fmt,
                    &hint_fmt,
                    accent,
                    s,
                );
            }

            if self.is_drop_hovered {
                let drop_col = d2d_color_rgb(accent, 0.85);
                if let Ok(db) = self.rt.CreateSolidColorBrush(&drop_col, None) {
                    self.rt.DrawRoundedRectangle(&card_rr, &db, 2.0 * s, None);
                }
            } else {
                draw_rounded_card_rim(&self.rt, card_rect, layout.card_radius, s, &recipe);
            }

            self.rt.EndDraw(None, None)?;
        }

        self.surface.commit(self.hwnd, self.win_x, self.win_y);
        Ok(())
    }

    pub(super) fn resolve_icons_for_layout(
        &mut self,
        layout: &DeskBoxLayout,
    ) -> Vec<Option<ID2D1Bitmap>> {
        // Resolve already-uploaded D2D icon bitmaps in O(1) without touching IconService or
        // normalizing paths; only query IconService for cache misses.
        let want_jumbo = layout.icon_size_px > 48.0;
        let mut resolved_icons: Vec<Option<ID2D1Bitmap>> =
            Vec::with_capacity(layout.visible_cells.len());
        let mut any_missing_icon = false;
        for cell in &layout.visible_cells {
            if let Some(item) = self.items.get(cell.index) {
                let key = if want_jumbo {
                    item.icon_key_j.as_str()
                } else {
                    item.icon_key_l.as_str()
                };
                let hit = self.icon_bitmaps.get_cached(key);
                if hit.is_none() {
                    any_missing_icon = true;
                }
                resolved_icons.push(hit);
            } else {
                resolved_icons.push(None);
            }
        }
        if any_missing_icon {
            if let Ok(mut svc) = self.icon_service.try_borrow_mut() {
                svc.drain_completed();
                for (vis_idx, cell) in layout.visible_cells.iter().enumerate() {
                    if resolved_icons[vis_idx].is_some() {
                        continue;
                    }
                    if let Some(item) = self.items.get(cell.index) {
                        let key = if want_jumbo {
                            item.icon_key_j.as_str()
                        } else {
                            item.icon_key_l.as_str()
                        };
                        if let Some(decoded) = svc.request_or_get_by_key(
                            key,
                            want_jumbo,
                            &item.path,
                            self.hwnd,
                            WM_APP_DESKBOX_ICON_READY,
                        ) {
                            resolved_icons[vis_idx] =
                                self.icon_bitmaps.get_or_upload(&self.rt, key, &decoded);
                        }
                    }
                }
            }
        }
        resolved_icons
    }

    unsafe fn draw_header_only(
        &mut self,
        layout: &DeskBoxLayout,
        title_fmt: &IDWriteTextFormat,
        edit_fmt: &IDWriteTextFormat,
        accent: Rgb,
        s: f32,
    ) {
        if layout.auto_hide_header {
            let hdr_alpha = if layout.collapsed {
                1.0
            } else {
                self.header_opacity_spring.pos.clamp(0.0, 1.0)
            };
            if hdr_alpha > 0.01 {
                if hdr_alpha < 0.99 {
                    let hdr_params = D2D1_LAYER_PARAMETERS {
                        contentBounds: d2d_rect(layout.header_rect),
                        geometricMask: ManuallyDrop::new(None),
                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        maskTransform: identity_matrix(),
                        opacity: hdr_alpha,
                        opacityBrush: ManuallyDrop::new(None),
                        layerOptions: D2D1_LAYER_OPTIONS_NONE,
                    };
                    self.rt.PushLayer(&hdr_params, &self.content_layer);
                    if !layout.collapsed {
                        self.draw_floating_header_backdrop(layout.header_rect, s);
                        self.draw_header_hairline(layout.header_rect, s);
                    }
                    self.draw_header(layout, title_fmt, edit_fmt, accent, s);
                    self.rt.PopLayer();
                    let _ = ManuallyDrop::into_inner(hdr_params.geometricMask);
                    let _ = ManuallyDrop::into_inner(hdr_params.opacityBrush);
                } else {
                    if !layout.collapsed {
                        self.draw_floating_header_backdrop(layout.header_rect, s);
                        self.draw_header_hairline(layout.header_rect, s);
                    }
                    self.draw_header(layout, title_fmt, edit_fmt, accent, s);
                }
            }
        } else {
            if !layout.collapsed {
                self.draw_header_hairline(layout.header_rect, s);
            }
            self.draw_header(layout, title_fmt, edit_fmt, accent, s);
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_foreground_contents(
        &mut self,
        layout: &DeskBoxLayout,
        resolved_icons: &[Option<ID2D1Bitmap>],
        title_fmt: &IDWriteTextFormat,
        edit_fmt: &IDWriteTextFormat,
        grid_fmt: &IDWriteTextFormat,
        list_fmt: &IDWriteTextFormat,
        hint_fmt: &IDWriteTextFormat,
        accent: Rgb,
        s: f32,
    ) {
        if layout.auto_hide_header {
            if !layout.collapsed {
                self.draw_content_body(
                    layout,
                    resolved_icons,
                    grid_fmt,
                    list_fmt,
                    hint_fmt,
                    accent,
                    s,
                );
            }
            let hdr_alpha = if layout.collapsed {
                1.0
            } else {
                self.header_opacity_spring.pos.clamp(0.0, 1.0)
            };
            if hdr_alpha > 0.01 {
                if hdr_alpha < 0.99 {
                    let hdr_params = D2D1_LAYER_PARAMETERS {
                        contentBounds: d2d_rect(layout.header_rect),
                        geometricMask: ManuallyDrop::new(None),
                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        maskTransform: identity_matrix(),
                        opacity: hdr_alpha,
                        opacityBrush: ManuallyDrop::new(None),
                        layerOptions: D2D1_LAYER_OPTIONS_NONE,
                    };
                    self.rt.PushLayer(&hdr_params, &self.content_layer);
                    if !layout.collapsed {
                        self.draw_floating_header_backdrop(layout.header_rect, s);
                        self.draw_header_hairline(layout.header_rect, s);
                    }
                    self.draw_header(layout, title_fmt, edit_fmt, accent, s);
                    self.rt.PopLayer();
                    let _ = ManuallyDrop::into_inner(hdr_params.geometricMask);
                    let _ = ManuallyDrop::into_inner(hdr_params.opacityBrush);
                } else {
                    if !layout.collapsed {
                        self.draw_floating_header_backdrop(layout.header_rect, s);
                        self.draw_header_hairline(layout.header_rect, s);
                    }
                    self.draw_header(layout, title_fmt, edit_fmt, accent, s);
                }
            }
        } else {
            if !layout.collapsed {
                self.draw_header_hairline(layout.header_rect, s);
            }
            self.draw_header(layout, title_fmt, edit_fmt, accent, s);
            if !layout.collapsed {
                self.draw_content_body(
                    layout,
                    resolved_icons,
                    grid_fmt,
                    list_fmt,
                    hint_fmt,
                    accent,
                    s,
                );
            }
        }
    }

    fn render_tucked_edge_tab(&mut self) -> windows::core::Result<()> {
        let s = (self.dpi.max(96) as f32) / 96.0;
        let w = self.win_w.max(4);
        let h = self.win_h.max(4);
        self.surface.ensure_size(w, h)?;

        let pad = SHADOW_PAD_DIP * s;
        let card_rect = RectF::new(
            pad,
            pad,
            (w as f32 - pad).max(pad + 4.0),
            (h as f32 - pad).max(pad + 4.0),
        );
        let radius = (card_rect.width().min(card_rect.height()) * 0.5).clamp(2.0, 8.0 * s);
        let recipe = popup_glass_recipe(
            self.surface_style,
            self.light_glass,
            rgb_luma(NEUTRAL_BACKDROP_FALLBACK),
        );
        let accent = Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        );
        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            let tab_rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(card_rect),
                radiusX: radius,
                radiusY: radius,
            };
            // Subtle outer glow shadow for edge sensor pill
            let sh_col = D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: if self.light_glass { 0.14 } else { 0.28 },
            };
            let sh_rr = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: card_rect.left - 1.5 * s,
                    top: card_rect.top - 1.5 * s,
                    right: card_rect.right + 1.5 * s,
                    bottom: card_rect.bottom + 1.5 * s,
                },
                radiusX: radius + 1.5 * s,
                radiusY: radius + 1.5 * s,
            };
            if let Ok(sb) = self.rt.CreateSolidColorBrush(&sh_col, None) {
                self.rt.FillRoundedRectangle(&sh_rr, &sb);
            }

            let (tr, tg, tb) = recipe.tint_rgb;
            let tab_alpha = match (self.surface_style, self.light_glass) {
                (GlassSurfaceStyle::Transparent, false) => 0.44,
                (GlassSurfaceStyle::Transparent, true) => 0.50,
                (GlassSurfaceStyle::LiquidGlass, false) => 0.56,
                (GlassSurfaceStyle::LiquidGlass, true) => 0.62,
                (GlassSurfaceStyle::Blur, false) => 0.62,
                (GlassSurfaceStyle::Blur, true) => 0.68,
                (GlassSurfaceStyle::Acrylic, false) => 0.68,
                (GlassSurfaceStyle::Acrylic, true) => 0.74,
                (GlassSurfaceStyle::Default, false) => 0.78,
                (GlassSurfaceStyle::Default, true) => 0.82,
            };
            let fill_col = D2D1_COLOR_F {
                r: tr,
                g: tg,
                b: tb,
                a: tab_alpha,
            };
            if let Ok(fb) = self.rt.CreateSolidColorBrush(&fill_col, None) {
                self.rt.FillRoundedRectangle(&tab_rr, &fb);
            }

            // Centered Liquid Glass accent handle strip
            let cx = (card_rect.left + card_rect.right) * 0.5;
            let cy = (card_rect.top + card_rect.bottom) * 0.5;
            let grip_col = d2d_color_rgb(accent, if self.light_glass { 0.80 } else { 0.88 });
            if let Ok(gb) = self.rt.CreateSolidColorBrush(&grip_col, None) {
                let grip_rr = match self.edge_dock {
                    DockEdge::Left | DockEdge::Right => {
                        let half_l = (card_rect.height() * 0.28).max(8.0 * s);
                        D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: cx - 1.2 * s,
                                top: cy - half_l,
                                right: cx + 1.2 * s,
                                bottom: cy + half_l,
                            },
                            radiusX: 1.2 * s,
                            radiusY: 1.2 * s,
                        }
                    }
                    DockEdge::Top => {
                        let half_l = (card_rect.width() * 0.28).max(8.0 * s);
                        D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: cx - half_l,
                                top: cy - 1.2 * s,
                                right: cx + half_l,
                                bottom: cy + 1.2 * s,
                            },
                            radiusX: 1.2 * s,
                            radiusY: 1.2 * s,
                        }
                    }
                };
                self.rt.FillRoundedRectangle(&grip_rr, &gb);
            }

            draw_rounded_card_rim(&self.rt, card_rect, radius, s, &recipe);
            self.rt.EndDraw(None, None)?;
        }

        self.surface.commit(self.hwnd, self.win_x, self.win_y);
        Ok(())
    }

    unsafe fn draw_header_hairline(&self, header_rect: RectF, s: f32) {
        let sep_col = if self.light_glass {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.08,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.10,
            }
        };
        if let Ok(sb) = self.rt.CreateSolidColorBrush(&sep_col, None) {
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: header_rect.left + 8.0 * s,
                    y: header_rect.bottom,
                },
                D2D_POINT_2F {
                    x: header_rect.right - 8.0 * s,
                    y: header_rect.bottom,
                },
                &sb,
                1.0 * s,
                None,
            );
        }
    }

    unsafe fn draw_floating_header_backdrop(&self, header_rect: RectF, s: f32) {
        let stops = if self.light_glass {
            [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 0.96,
                        g: 0.97,
                        b: 0.99,
                        a: 0.78,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 0.96,
                        g: 0.97,
                        b: 0.99,
                        a: 0.56,
                    },
                },
            ]
        } else {
            [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 0.09,
                        g: 0.11,
                        b: 0.15,
                        a: 0.72,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 0.09,
                        g: 0.11,
                        b: 0.15,
                        a: 0.48,
                    },
                },
            ]
        };
        let rt_base: ID2D1RenderTarget = self.rt.clone().into();
        fill_grad(
            &rt_base,
            d2d_rect(header_rect),
            D2D_POINT_2F {
                x: header_rect.left,
                y: header_rect.top,
            },
            D2D_POINT_2F {
                x: header_rect.left,
                y: header_rect.bottom + 2.0 * s,
            },
            &stops,
        );
    }
}
