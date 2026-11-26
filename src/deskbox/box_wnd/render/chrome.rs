//! Cached glass card chrome, liquid silk refraction, and concentric hollow shadow rings.

use super::super::{AnimBodyCacheKey, BoxInner, ChromeCacheKey, DragAction};
use super::primitives::{fill_grad, fill_grad_rounded};
use crate::color::Rgb;
use crate::deskbox::config::BoxKind;
use crate::deskbox::layout::{DeskBoxLayout, DeskBoxLayoutParams};
use crate::ui::glass::{d2d_rect, identity_matrix};
use crate::ui::{lift_backdrop_alpha, GlassSurfaceStyle, PopupGlassRecipe, RectF, SILK_H, SILK_W};
use std::mem::ManuallyDrop;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D_POINT_2F, D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_BRUSH_PROPERTIES,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_COMBINE_MODE_EXCLUDE, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_ROUNDED_RECT,
    ID2D1Bitmap, ID2D1BitmapBrush, ID2D1Geometry, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

impl BoxInner {
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn ensure_chrome_bitmap(
        &mut self,
        card_rect: RectF,
        card_rr: &D2D1_ROUNDED_RECT,
        card_radius: f32,
        grad_rect: RectF,
        surf_w: i32,
        surf_h: i32,
        recipe: &PopupGlassRecipe,
        s: f32,
    ) -> Option<ID2D1Bitmap> {
        let card_h_px = card_rect.height().round().max(1.0) as i32;
        let grad_h_px = grad_rect.height().round().max(1.0) as i32;
        let key = ChromeCacheKey {
            surf_w,
            surf_h,
            card_h_px,
            grad_h_px,
            dpi: self.dpi,
            style: self.surface_style,
            light_glass: self.light_glass,
            has_live_blur: self.has_live_blur,
            transparency_enabled: self.transparency_enabled,
        };
        for (cached_key, bmp) in self.chrome_cache.iter().flatten() {
            if *cached_key == key {
                return Some(bmp.clone());
            }
        }

        // Pre-populate cached shadow rings and rounded clip geometry on self before drawing to compat RT.
        self.ensure_shadow_geometries(card_rect, card_rr, card_radius, surf_w, surf_h, s);
        let clip_geom = self.cached_clip_geo.clone()?;

        let desired_size = D2D_SIZE_F {
            width: surf_w as f32,
            height: surf_h as f32,
        };
        let compat_rt = self
            .rt
            .CreateCompatibleRenderTarget(
                Some(&desired_size),
                None,
                None,
                D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE,
            )
            .ok()?;
        let compat_layer = compat_rt.CreateLayer(None).ok()?;
        let rt_base: ID2D1RenderTarget = compat_rt.clone().into();

        compat_rt.BeginDraw();
        compat_rt.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));
        self.draw_cached_shadow_rings_on(&rt_base);

        let params = D2D1_LAYER_PARAMETERS {
            contentBounds: d2d_rect(card_rect),
            geometricMask: ManuallyDrop::new(Some(clip_geom)),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: identity_matrix(),
            opacity: 1.0,
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        compat_rt.PushLayer(&params, &compat_layer);
        self.draw_card_glass_body_on(&rt_base, &card_rect, &grad_rect, recipe, s);
        compat_rt.PopLayer();
        let _ = ManuallyDrop::into_inner(params.geometricMask);
        let _ = ManuallyDrop::into_inner(params.opacityBrush);

        if compat_rt.EndDraw(None, None).is_err() {
            return None;
        }
        let bmp = compat_rt.GetBitmap().ok()?;
        // Dual-slot chrome cache retains both collapsed and expanded backdrops simultaneously
        // so hover-expand/retract cycles never evict one another and never re-run geometry combine.
        let slot = self
            .chrome_cache
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.chrome_cache_next_slot % self.chrome_cache.len());
        self.chrome_cache[slot] = Some((key, bmp.clone()));
        self.chrome_cache_next_slot = (slot + 1) % self.chrome_cache.len();
        Some(bmp)
    }

    /// Bakes the full-expanded card interior (glass tint + frost veil + noise + sheen + item grid/list
    /// icons & DirectWrite labels) once into an offscreen `ID2D1BitmapBrush`. During hover-expand and
    /// collapse springs, every frame renders the entire interior clipped to the animated rounded card
    /// via a single `FillRoundedRectangle` call instead of re-rasterizing icons & text on CPU D2D.
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn ensure_anim_body_brush(
        &mut self,
        card_w_px: i32,
        full_card_h_px: i32,
        grad_h_px_f: f32,
        recipe: &PopupGlassRecipe,
        grid_fmt: &IDWriteTextFormat,
        list_fmt: &IDWriteTextFormat,
        hint_fmt: &IDWriteTextFormat,
        accent: Rgb,
        s: f32,
    ) -> Option<ID2D1BitmapBrush> {
        let grad_h_px = grad_h_px_f.round().max(1.0) as i32;
        let key = AnimBodyCacheKey {
            card_w_px: card_w_px.max(1),
            full_card_h_px: full_card_h_px.max(1),
            grad_h_px,
            dpi: self.dpi,
            style: self.surface_style,
            light_glass: self.light_glass,
            has_live_blur: self.has_live_blur,
            transparency_enabled: self.transparency_enabled,
            view_mode: self.spec.view_mode,
            auto_hide_header: self.spec.auto_hide_header,
            scroll_y_px: self.scroll_y.round() as i32,
            selected_count: self.selected.len(),
            item_count: self.items.len(),
            content_rev: self.content_rev,
        };
        if self.anim_body_key == Some(key) {
            if let Some(ref brush) = self.anim_body_brush {
                return Some(brush.clone());
            }
        }

        let full_layout = DeskBoxLayout::compute(DeskBoxLayoutParams {
            width_dip: self.spec.w_dip,
            height_dip: self.spec.h_dip,
            dpi: self.dpi,
            item_count: self.items.len(),
            view_mode: self.spec.view_mode,
            scroll_y: self.scroll_y,
            can_go_back: self.can_go_back(),
            show_item_remove: self.spec.kind == BoxKind::Collection,
            collapsed: false,
            show_header_actions: self.should_show_header_actions(),
            locked: self.spec.locked,
            auto_hide_header: self.spec.auto_hide_header,
            header_visible: self.is_header_visible(),
            search_active: self.search_active,
            card_height_override_px: Some(key.full_card_h_px as f32),
        });
        let full_icons = self.resolve_icons_for_layout(&full_layout);

        let desired_size = D2D_SIZE_F {
            width: key.card_w_px as f32,
            height: key.full_card_h_px as f32,
        };
        let compat_rt = self
            .rt
            .CreateCompatibleRenderTarget(
                Some(&desired_size),
                None,
                None,
                D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE,
            )
            .ok()?;
        let rt_base: ID2D1RenderTarget = compat_rt.clone().into();

        compat_rt.BeginDraw();
        compat_rt.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));
        compat_rt.SetTransform(&Matrix3x2 {
            M11: 1.0,
            M12: 0.0,
            M21: 0.0,
            M22: 1.0,
            M31: -full_layout.card_rect.left,
            M32: -full_layout.card_rect.top,
        });
        let grad_rect = RectF::new(
            full_layout.card_rect.left,
            full_layout.card_rect.top,
            full_layout.card_rect.right,
            full_layout.card_rect.top + grad_h_px_f.max(1.0),
        );
        self.draw_card_glass_body_on(&rt_base, &full_layout.card_rect, &grad_rect, recipe, s);
        self.draw_content_body_on(
            &rt_base,
            &full_layout,
            &full_icons,
            grid_fmt,
            list_fmt,
            hint_fmt,
            accent,
            s,
            true,
        );
        compat_rt.SetTransform(&identity_matrix());
        if compat_rt.EndDraw(None, None).is_err() {
            self.clear_anim_body_cache();
            return None;
        }
        let bmp = compat_rt.GetBitmap().ok()?;
        let brush_props = D2D1_BITMAP_BRUSH_PROPERTIES {
            extendModeX: D2D1_EXTEND_MODE_CLAMP,
            extendModeY: D2D1_EXTEND_MODE_CLAMP,
            interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
        };
        let brush = self.rt.CreateBitmapBrush(&bmp, Some(&brush_props), None).ok()?;
        self.anim_body_bmp = Some(bmp);
        self.anim_body_brush = Some(brush.clone());
        self.anim_body_key = Some(key);
        Some(brush)
    }

    pub(super) unsafe fn draw_card_glass_body_on(
        &mut self,
        rt: &ID2D1RenderTarget,
        card_rect: &RectF,
        grad_rect: &RectF,
        recipe: &PopupGlassRecipe,
        s: f32,
    ) {
        let card_d2d = d2d_rect(*card_rect);
        let (tr, tg, tb) = recipe.tint_rgb;
        let base_a = match (self.surface_style, self.light_glass, self.has_live_blur) {
            (GlassSurfaceStyle::Transparent, false, _) => lift_backdrop_alpha(0.18),
            (GlassSurfaceStyle::Transparent, true, _) => lift_backdrop_alpha(0.24),
            (GlassSurfaceStyle::LiquidGlass, false, true) => lift_backdrop_alpha(0.24),
            (GlassSurfaceStyle::LiquidGlass, false, false) => lift_backdrop_alpha(0.38),
            (GlassSurfaceStyle::LiquidGlass, true, true) => lift_backdrop_alpha(0.30),
            (GlassSurfaceStyle::LiquidGlass, true, false) => lift_backdrop_alpha(0.44),
            (GlassSurfaceStyle::Blur, false, true) => lift_backdrop_alpha(0.34),
            (GlassSurfaceStyle::Blur, false, false) => lift_backdrop_alpha(0.48),
            (GlassSurfaceStyle::Blur, true, true) => lift_backdrop_alpha(0.40),
            (GlassSurfaceStyle::Blur, true, false) => lift_backdrop_alpha(0.54),
            (GlassSurfaceStyle::Acrylic, false, true) => lift_backdrop_alpha(0.42),
            (GlassSurfaceStyle::Acrylic, false, false) => lift_backdrop_alpha(0.56),
            (GlassSurfaceStyle::Acrylic, true, true) => lift_backdrop_alpha(0.50),
            (GlassSurfaceStyle::Acrylic, true, false) => lift_backdrop_alpha(0.62),
            (GlassSurfaceStyle::Default, false, _) => lift_backdrop_alpha(0.58),
            (GlassSurfaceStyle::Default, true, _) => lift_backdrop_alpha(0.66),
        };
        let base_col = D2D1_COLOR_F {
            r: tr,
            g: tg,
            b: tb,
            a: base_a,
        };
        if let Ok(b) = rt.CreateSolidColorBrush(&base_col, None) {
            rt.FillRectangle(&card_d2d, &b);
        }

        if recipe.frost_veil_top_alpha > 0.002 {
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: recipe.frost_veil_top_alpha,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.55,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: recipe.frost_veil_mid_alpha,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: recipe.frost_veil_bot_alpha,
                    },
                },
            ];
            fill_grad(
                rt,
                card_d2d,
                D2D_POINT_2F {
                    x: grad_rect.left,
                    y: grad_rect.top,
                },
                D2D_POINT_2F {
                    x: grad_rect.left,
                    y: grad_rect.bottom,
                },
                &stops,
            );
        }

        if recipe.grain_max_alpha > 0 {
            if let Some(nb) = self.noise_cache.ensure_brush(&self.rt, recipe.grain_max_alpha) {
                rt.FillRectangle(&card_d2d, nb);
            }
        }

        if recipe.sheen_scale > 0.01 {
            let ss = recipe.sheen_scale;
            let sheen_stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (if self.light_glass { 0.20 } else { 0.12 }) * ss,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.45,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (if self.light_glass { 0.025 } else { 0.015 }) * ss,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (if self.light_glass { 0.08 } else { 0.05 }) * ss,
                    },
                },
            ];
            fill_grad(
                rt,
                card_d2d,
                D2D_POINT_2F {
                    x: grad_rect.left,
                    y: grad_rect.top,
                },
                D2D_POINT_2F {
                    x: grad_rect.right,
                    y: grad_rect.bottom,
                },
                &sheen_stops,
            );
        }

        let _ = s;
    }

    /// Renders the glass backdrop directly via native `FillRoundedRectangle` without requiring
    /// an enclosing `PushLayer(geometricMask)`, eliminating per-frame CPU alpha-mask compositing
    /// during collapse/expand springs and interactive resize drags.
    pub(super) unsafe fn draw_card_glass_body_rounded_on(
        &mut self,
        rt: &ID2D1RenderTarget,
        card_rect: &RectF,
        card_rr: &D2D1_ROUNDED_RECT,
        grad_rect: &RectF,
        recipe: &PopupGlassRecipe,
    ) {
        let _ = card_rect;
        let (tr, tg, tb) = recipe.tint_rgb;
        let base_a = match (self.surface_style, self.light_glass, self.has_live_blur) {
            (GlassSurfaceStyle::Transparent, false, _) => lift_backdrop_alpha(0.18),
            (GlassSurfaceStyle::Transparent, true, _) => lift_backdrop_alpha(0.24),
            (GlassSurfaceStyle::LiquidGlass, false, true) => lift_backdrop_alpha(0.24),
            (GlassSurfaceStyle::LiquidGlass, false, false) => lift_backdrop_alpha(0.38),
            (GlassSurfaceStyle::LiquidGlass, true, true) => lift_backdrop_alpha(0.30),
            (GlassSurfaceStyle::LiquidGlass, true, false) => lift_backdrop_alpha(0.44),
            (GlassSurfaceStyle::Blur, false, true) => lift_backdrop_alpha(0.34),
            (GlassSurfaceStyle::Blur, false, false) => lift_backdrop_alpha(0.48),
            (GlassSurfaceStyle::Blur, true, true) => lift_backdrop_alpha(0.40),
            (GlassSurfaceStyle::Blur, true, false) => lift_backdrop_alpha(0.54),
            (GlassSurfaceStyle::Acrylic, false, true) => lift_backdrop_alpha(0.42),
            (GlassSurfaceStyle::Acrylic, false, false) => lift_backdrop_alpha(0.56),
            (GlassSurfaceStyle::Acrylic, true, true) => lift_backdrop_alpha(0.50),
            (GlassSurfaceStyle::Acrylic, true, false) => lift_backdrop_alpha(0.62),
            (GlassSurfaceStyle::Default, false, _) => lift_backdrop_alpha(0.58),
            (GlassSurfaceStyle::Default, true, _) => lift_backdrop_alpha(0.66),
        };
        let base_col = D2D1_COLOR_F {
            r: tr,
            g: tg,
            b: tb,
            a: base_a,
        };
        if let Ok(b) = rt.CreateSolidColorBrush(&base_col, None) {
            rt.FillRoundedRectangle(card_rr, &b);
        }

        if recipe.frost_veil_top_alpha > 0.002 {
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: recipe.frost_veil_top_alpha,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.55,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: recipe.frost_veil_mid_alpha,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: recipe.frost_veil_bot_alpha,
                    },
                },
            ];
            fill_grad_rounded(
                rt,
                card_rr,
                D2D_POINT_2F {
                    x: grad_rect.left,
                    y: grad_rect.top,
                },
                D2D_POINT_2F {
                    x: grad_rect.left,
                    y: grad_rect.bottom,
                },
                &stops,
            );
        }

        if recipe.grain_max_alpha > 0 {
            if let Some(nb) = self.noise_cache.ensure_brush(&self.rt, recipe.grain_max_alpha) {
                rt.FillRoundedRectangle(card_rr, nb);
            }
        }

        if recipe.sheen_scale > 0.01 {
            let ss = recipe.sheen_scale;
            let sheen_stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (if self.light_glass { 0.20 } else { 0.12 }) * ss,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 0.45,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (if self.light_glass { 0.025 } else { 0.015 }) * ss,
                    },
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (if self.light_glass { 0.08 } else { 0.05 }) * ss,
                    },
                },
            ];
            fill_grad_rounded(
                rt,
                card_rr,
                D2D_POINT_2F {
                    x: grad_rect.left,
                    y: grad_rect.top,
                },
                D2D_POINT_2F {
                    x: grad_rect.right,
                    y: grad_rect.bottom,
                },
                &sheen_stops,
            );
        }
    }

    /// Draws the animated liquid-silk bitmap directly on `self.rt` using a cached `ID2D1BitmapBrush`
    /// clipped to `card_rr`, keeping `cached_chrome_bmp` completely static across silk & scroll frames.
    pub(super) unsafe fn draw_silk_overlay(
        &mut self,
        card_rect: RectF,
        card_rr: &D2D1_ROUNDED_RECT,
        full_card_h: f32,
        recipe: &PopupGlassRecipe,
    ) {
        if self.surface_style != GlassSurfaceStyle::LiquidGlass || recipe.silk_alpha <= 0.002 {
            return;
        }
        let Some(ref silk) = self.silk_bmp else {
            return;
        };
        if self.silk_brush.is_none() {
            let props = D2D1_BITMAP_BRUSH_PROPERTIES {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            };
            self.silk_brush = self.rt.CreateBitmapBrush(silk, Some(&props), None).ok();
        }
        if let Some(ref brush) = self.silk_brush {
            let mat = Matrix3x2 {
                M11: card_rect.width().max(1.0) / (SILK_W as f32),
                M12: 0.0,
                M21: 0.0,
                M22: full_card_h.max(1.0) / (SILK_H as f32),
                M31: card_rect.left,
                M32: card_rect.top,
            };
            brush.SetTransform(&mat);
            brush.SetOpacity((recipe.silk_alpha * 0.68).clamp(0.0, 1.0));
            self.rt.FillRoundedRectangle(card_rr, brush);
        }
    }

    unsafe fn ensure_shadow_geometries(
        &mut self,
        card_rect: RectF,
        card_rr: &D2D1_ROUNDED_RECT,
        card_radius: f32,
        win_w: i32,
        win_h: i32,
        s: f32,
    ) {
        let shadow_layers: [(f32, f32); 3] = if self.light_glass {
            [(7.5, 0.022), (4.5, 0.036), (2.0, 0.058)]
        } else {
            [(7.5, 0.035), (4.5, 0.055), (2.0, 0.090)]
        };
        let card_h_px = card_rect.height().round().max(1.0) as i32;
        let geo_key = (win_w, win_h, card_h_px, self.dpi);
        if self.shadow_geo_key == Some(geo_key) && self.cached_clip_geo.is_some() {
            return;
        }
        self.cached_clip_geo = None;
        self.cached_shadow_rings = [None, None, None];
        if let Ok(clip_raw) = self.d2d_factory.CreateRoundedRectangleGeometry(card_rr) {
            let clip_geo: ID2D1Geometry = clip_raw.into();
            let wf = win_w as f32;
            let hf = win_h as f32;
            for (idx, (expand_dip, _)) in shadow_layers.iter().enumerate() {
                let expand = expand_dip * s;
                let side_exp = expand * 0.55;
                let top_exp = expand * 0.45;
                let bot_exp = expand * 0.65;
                let sh_rect = RectF::new(
                    (card_rect.left - side_exp).max(0.0),
                    (card_rect.top - top_exp).max(0.0),
                    (card_rect.right + side_exp).min(wf),
                    (card_rect.bottom + bot_exp).min(hf),
                );
                let sh_rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(sh_rect),
                    radiusX: card_radius + side_exp * 0.45,
                    radiusY: card_radius + side_exp * 0.45,
                };
                if let Ok(outer_raw) = self.d2d_factory.CreateRoundedRectangleGeometry(&sh_rr) {
                    let outer_geo: ID2D1Geometry = outer_raw.into();
                    if let Ok(ring) = self.d2d_factory.CreatePathGeometry() {
                        if let Ok(sink) = ring.Open() {
                            if outer_geo
                                .CombineWithGeometry(
                                    &clip_geo,
                                    D2D1_COMBINE_MODE_EXCLUDE,
                                    None,
                                    0.25,
                                    &sink,
                                )
                                .is_ok()
                                && sink.Close().is_ok()
                            {
                                self.cached_shadow_rings[idx] = Some(ring.into());
                            }
                        }
                    }
                }
            }
            self.cached_clip_geo = Some(clip_geo);
            self.shadow_geo_key = Some(geo_key);
        } else {
            self.shadow_geo_key = None;
        }
    }

    unsafe fn draw_cached_shadow_rings_on(&self, rt: &ID2D1RenderTarget) {
        let shadow_layers: [(f32, f32); 3] = if self.light_glass {
            [(7.5, 0.022), (4.5, 0.036), (2.0, 0.058)]
        } else {
            [(7.5, 0.035), (4.5, 0.055), (2.0, 0.090)]
        };
        let mult = if self.surface_style == GlassSurfaceStyle::Transparent {
            0.65
        } else {
            1.0
        };
        for (idx, &(_, base_alpha)) in shadow_layers.iter().enumerate() {
            if let Some(ref ring_geo) = self.cached_shadow_rings[idx] {
                let sh_col = D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: base_alpha * mult,
                };
                if let Ok(sh_brush) = rt.CreateSolidColorBrush(&sh_col, None) {
                    rt.FillGeometry(ring_geo, &sh_brush, None);
                }
            }
        }
    }

    pub(super) unsafe fn ensure_and_draw_card_shadows(
        &mut self,
        card_rect: RectF,
        card_rr: &D2D1_ROUNDED_RECT,
        card_radius: f32,
        win_w: i32,
        win_h: i32,
        s: f32,
    ) {
        let shadow_layers: [(f32, f32); 3] = if self.light_glass {
            [(7.5, 0.022), (4.5, 0.036), (2.0, 0.058)]
        } else {
            [(7.5, 0.035), (4.5, 0.055), (2.0, 0.090)]
        };
        let mult = if self.surface_style == GlassSurfaceStyle::Transparent {
            0.65
        } else {
            1.0
        };
        // During interactive height-spring / live-resize frames, draw 3 concentric stroked outer
        // shadow bands strictly outside `card_rr` (inner boundary == `card_rr`, zero pixels inside
        // the glass interior) so card brightness matches the settled `chrome_bmp` bit-for-bit with
        // zero `CombineWithGeometry` overhead.
        let interactive_sizing = self.collapse_animating
            || matches!(self.drag, Some(DragAction::Resize { .. }));
        if interactive_sizing {
            let wf = win_w as f32;
            let hf = win_h as f32;
            for &(expand_dip, base_alpha) in &shadow_layers {
                let stroke_w = (expand_dip * 0.55 * s).max(1.0);
                let half_w = stroke_w * 0.5;
                let sh_rect = RectF::new(
                    (card_rect.left - half_w).max(half_w),
                    (card_rect.top - half_w).max(half_w),
                    (card_rect.right + half_w).min(wf - half_w),
                    (card_rect.bottom + half_w).min(hf - half_w),
                );
                let sh_rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(sh_rect),
                    radiusX: card_radius + half_w,
                    radiusY: card_radius + half_w,
                };
                let sh_col = D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: base_alpha * mult,
                };
                if let Ok(sh_brush) = self.rt.CreateSolidColorBrush(&sh_col, None) {
                    self.rt.DrawRoundedRectangle(&sh_rr, &sh_brush, stroke_w, None);
                }
            }
            return;
        }

        self.ensure_shadow_geometries(card_rect, card_rr, card_radius, win_w, win_h, s);
        let rt_base: ID2D1RenderTarget = self.rt.clone().into();
        self.draw_cached_shadow_rings_on(&rt_base);
    }

}
