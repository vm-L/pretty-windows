use super::{CachedTextFormats, HoverTarget, PopupInner, PrimaryCategory, compute_cover_halo_pair, resolve_dynamic_menu_accent};
use super::super::layout::RectF;
use super::super::render::draw_speaker_glyph;
use super::super::smtc::MediaCover;
use crate::color::{derive_background_accent, enforce_ui_accent_deadzone, Rgb};
use crate::system::active_taskbar_surface_style;
use crate::ui::glass::{build_unified_menu_geometry, create_dc_render_target, create_text_format, d2d_color_rgb, d2d_rect, draw_geometry_rim, draw_glass_interior, draw_music_note_icon, draw_settings_icon, draw_text_in_rect, draw_unified_shadows, identity_matrix, muted_text_color, primary_text_color, update_silk_bitmap};
use crate::ui::{animated_primary_rect, compute_halo_pair, ease_out_cubic, ease_retract, popup_glass_recipe, rgb_luma, row_stagger_dy, window_master_alpha, PopupAnimPhase};
use std::mem::ManuallyDrop;
use std::time::Instant;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_F, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{ID2D1Geometry, ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_BITMAP_PROPERTIES, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT, D2D1_SWEEP_DIRECTION_CLOCKWISE};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

impl PopupInner {
    fn ensure_cover_bitmaps(&mut self, cover: &MediaCover) {
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        unsafe {
            if !self.cover_bmps.contains_key(&cover.hash) {
                if let Ok(b) = self.rt.CreateBitmap(
                    D2D_SIZE_U {
                        width: cover.thumb_w,
                        height: cover.thumb_h,
                    },
                    Some(cover.thumb_pbgra.as_ptr().cast()),
                    cover.thumb_w * 4,
                    &props,
                ) {
                    if self.cover_bmps.len() > 24 {
                        self.cover_bmps.clear();
                    }
                    self.cover_bmps.insert(cover.hash, b);
                }
            }
        }
    }

    fn ensure_text_formats(&mut self) -> windows::core::Result<CachedTextFormats> {
        if let Some(ref cached) = self.cached_text_formats {
            if cached.dpi == self.dpi {
                return Ok(cached.clone());
            }
        }
        let s = self.scale();
        let title_fmt = create_text_format(&self.dwrite_factory, 13.0 * s, false, false, true)?;
        let sub_fmt = create_text_format(&self.dwrite_factory, 11.0 * s, false, false, true)?;
        let badge_fmt = create_text_format(&self.dwrite_factory, 10.5 * s, true, false, false)?;
        let pct_fmt = create_text_format(&self.dwrite_factory, 11.0 * s, false, true, false)?;
        let cached = CachedTextFormats {
            dpi: self.dpi,
            title_fmt,
            sub_fmt,
            badge_fmt,
            pct_fmt,
        };
        self.cached_text_formats = Some(cached.clone());
        Ok(cached)
    }

    fn update_silk_bitmap(&mut self, primary_rect: RectF, s: f32, draw_silk: bool) {
        let now = Instant::now();
        let dt_color = now
            .duration_since(self.last_color_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_color_tick = now;

        let bg_dark = derive_background_accent(self.backdrop_rgb, false);
        let bg_ui = derive_background_accent(self.backdrop_rgb, self.light_glass);
        let pair = if self.auto_color {
            let cover = self
                .active_id
                .as_ref()
                .and_then(|aid| self.sources.iter().find(|src| &src.id == aid))
                .and_then(|src| src.cover.as_deref())
                .or_else(|| self.sources.iter().find_map(|src| src.cover.as_deref()));
            compute_cover_halo_pair(cover, bg_dark, self.light_glass)
        } else {
            compute_halo_pair(bg_dark, self.light_glass)
        };
        let target = [pair[0].to_normalized(), pair[1].to_normalized()];
        let ui_target = if self.auto_color {
            resolve_dynamic_menu_accent(
                &self.sources,
                self.active_id.as_deref(),
                bg_ui,
                self.light_glass,
            )
        } else {
            enforce_ui_accent_deadzone(bg_ui, self.light_glass)
        }
        .to_normalized();

        let was_ready = self.halo_colors_ready;
        if !was_ready {
            self.halo_colors = target;
            self.ui_accent = ui_target;
            self.halo_colors_ready = true;
        } else {
            let k = (dt_color * 4.5).clamp(0.02, 1.0);
            for (cur, tgt) in self.halo_colors.iter_mut().zip(target.iter()) {
                cur.0 += (tgt.0 - cur.0) * k;
                cur.1 += (tgt.1 - cur.1) * k;
                cur.2 += (tgt.2 - cur.2) * k;
            }
            let k_ui = (dt_color * 6.0).clamp(0.02, 1.0);
            self.ui_accent.0 += (ui_target.0 - self.ui_accent.0) * k_ui;
            self.ui_accent.1 += (ui_target.1 - self.ui_accent.1) * k_ui;
            self.ui_accent.2 += (ui_target.2 - self.ui_accent.2) * k_ui;
        }

        if draw_silk {
            update_silk_bitmap(
                &self.rt,
                &mut self.silk_bmp,
                &mut self.silk_buf,
                &mut self.last_silk_tick,
                !was_ready,
                self.win_w,
                self.win_h,
                primary_rect.left,
                primary_rect.bottom,
                s,
                self.flow_phase,
                &self.halo_colors,
                self.light_glass,
            );
        }
    }

    fn recreate_render_target(&mut self) -> windows::core::Result<()> {
        self.backdrop_bmp = None;
        self.silk_bmp = None;
        self.cover_bmps.clear();
        self.noise_cache.clear();
        self.last_silk_tick = None;
        let new_rt = create_dc_render_target(&self.d2d_factory)?;
        let new_layer = unsafe { new_rt.CreateLayer(None)? };
        let new_content_layer = unsafe { new_rt.CreateLayer(None)? };
        let new_thumb_layer = unsafe { new_rt.CreateLayer(None)? };
        self.layer = new_layer;
        self.content_layer = new_content_layer;
        self.thumb_layer = new_thumb_layer;
        self.rt = new_rt;
        Ok(())
    }

    pub(super) fn render(&mut self) -> windows::core::Result<()> {
        match self.render_once() {
            Err(e) if e.code() == D2DERR_RECREATE_TARGET => {
                self.recreate_render_target()?;
                self.render_once()
            }
            res => res,
        }
    }

    fn render_once(&mut self) -> windows::core::Result<()> {
        self.surface.ensure_size(self.win_w, self.win_h)?;

        let covers_to_load: Vec<_> = self
            .sources
            .iter()
            .filter_map(|s| s.cover.clone())
            .collect();
        for cov in covers_to_load {
            self.ensure_cover_bitmaps(&cov);
        }

        let s = self.scale();
        let fmts = self.ensure_text_formats()?;

        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };

        let target_primary = self.primary_card_rect();
        let recipe = popup_glass_recipe(
            active_taskbar_surface_style(),
            self.light_glass,
            rgb_luma(self.backdrop_rgb),
        );
        self.update_silk_bitmap(target_primary, s, recipe.silk_alpha > 0.002);
        let auto_accent = Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        );

        let anim_primary = animated_primary_rect(
            target_primary,
            self.anim_phase,
            self.open_spring.pos,
            self.close_progress,
            s,
        );
        let primary_rows = self.primary_row_rects();
        let anim_sub_opt = self.current_animated_sub_rect();

        let (primary_alpha, close_dy) = match self.anim_phase {
            PopupAnimPhase::Opening => {
                let a = ease_out_cubic(((self.open_elapsed - 0.008) / 0.080).clamp(0.0, 1.0));
                (a, 0.0)
            }
            PopupAnimPhase::Open => (1.0, 0.0),
            PopupAnimPhase::Closing => {
                let r = ease_retract(self.close_progress);
                ((1.0 - 0.30 * r).clamp(0.0, 1.0), 5.0 * s * r)
            }
        };

        let left_bound = anim_sub_opt.map_or(anim_primary.left, |sr| anim_primary.left.min(sr.left));
        let right_bound =
            anim_sub_opt.map_or(anim_primary.right, |sr| anim_primary.right.max(sr.right));
        let top_bound = anim_sub_opt.map_or(anim_primary.top, |sr| anim_primary.top.min(sr.top));
        let bottom_bound =
            anim_sub_opt.map_or(anim_primary.bottom, |sr| anim_primary.bottom.max(sr.bottom));
        let union_bounds = RectF::new(
            left_bound.max(0.0),
            top_bound.max(0.0),
            right_bound.min(self.win_w as f32),
            bottom_bound.min(self.win_h as f32),
        );

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            // 1. Standalone ambient shadows
            draw_unified_shadows(
                &self.d2d_factory,
                &self.rt,
                anim_primary,
                anim_sub_opt,
                self.open_left,
                self.win_w,
                self.win_h,
                self.light_glass,
                s,
            );

            // 2. Closed fused silhouette layer clipping glass interior, primary rows & submenu content
            if let Ok(unified_geo) = build_unified_menu_geometry(
                &self.d2d_factory,
                anim_primary,
                anim_sub_opt,
                self.open_left,
                s,
            ) {
                let geom: ID2D1Geometry = unified_geo.into();
                let geom_for_rim = geom.clone();
                let outer_params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(union_bounds),
                    geometricMask: ManuallyDrop::new(Some(geom)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&outer_params, &self.layer);

                // 2a. Style-harmonized frosted / acrylic / liquid glass interior
                draw_glass_interior(
                    &self.rt,
                    union_bounds,
                    self.win_w,
                    self.win_h,
                    self.backdrop_bmp.as_ref(),
                    self.silk_bmp.as_ref(),
                    self.light_glass,
                    &recipe,
                    &mut self.noise_cache,
                );

                // 2b. Primary row backgrounds & staggered content (skip content_layer when steady Open)
                if primary_alpha > 0.005 {
                    let use_layer = self.anim_phase != PopupAnimPhase::Open;
                    if use_layer {
                        let content_params = D2D1_LAYER_PARAMETERS {
                            contentBounds: d2d_rect(anim_primary),
                            geometricMask: ManuallyDrop::new(None),
                            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                            maskTransform: identity_matrix(),
                            opacity: primary_alpha,
                            opacityBrush: ManuallyDrop::new(None),
                            layerOptions: D2D1_LAYER_OPTIONS_NONE,
                        };
                        self.rt.PushLayer(&content_params, &self.content_layer);
                    }
                    for (idx, (cat, mut rect)) in primary_rows.into_iter().enumerate() {
                        match self.anim_phase {
                            PopupAnimPhase::Opening => {
                                let dy = row_stagger_dy(
                                    idx,
                                    5,
                                    self.open_elapsed,
                                    self.open_spring.pos,
                                    s,
                                );
                                rect.top += dy;
                                rect.bottom += dy;
                            }
                            PopupAnimPhase::Open => {}
                            PopupAnimPhase::Closing => {
                                rect.top += close_dy;
                                rect.bottom += close_dy;
                            }
                        }
                        let expanded = self.active_category == Some(cat);
                        let hovered = self.hover_target == HoverTarget::Primary(cat);
                        self.draw_primary_row_bg(rect, hovered, expanded, auto_accent, s);
                        self.draw_primary_row_content(cat, rect, expanded, &fmts.title_fmt, s);
                    }
                    if use_layer {
                        self.rt.PopLayer();
                    }
                }

                // 2c. Clean feathered vertical seam divider & animated submenu content
                if let (Some(cat), Some(anim_sub)) = (self.active_category, anim_sub_opt) {
                    let target_sub = self.sub_card_rect(cat);
                    let (fade_in, dx_cur, dy_cur) = match self.sub_anim {
                        Some(sa) if sa.is_budding => {
                            let a =
                                ease_out_cubic(((sa.fade_elapsed - 0.008) / 0.078).clamp(0.0, 1.0));
                            let slide_t = ease_out_cubic((sa.fade_elapsed / 0.105).clamp(0.0, 1.0));
                            let dir = if self.open_left { 1.0 } else { -1.0 };
                            let dx = dir * 10.0 * s * (1.0 - slide_t);
                            let dy = (anim_sub.top - target_sub.top) * 0.25;
                            (a, dx, dy)
                        }
                        Some(sa) => {
                            let a =
                                ease_out_cubic(((sa.fade_elapsed - 0.010) / 0.070).clamp(0.0, 1.0));
                            let dy = (anim_sub.top - target_sub.top) * 0.22;
                            (a, 0.0, dy)
                        }
                        None => (1.0, 0.0, (anim_sub.top - target_sub.top) * 0.45),
                    };

                    let seam_x = if self.open_left {
                        anim_primary.left
                    } else {
                        anim_primary.right
                    };
                    let seam_top = anim_primary.top.max(anim_sub.top) + 8.0 * s;
                    let seam_bottom = anim_primary.bottom.min(anim_sub.bottom) - 8.0 * s;
                    let seam_fade = if self.sub_anim.is_some_and(|sa| sa.is_budding) {
                        fade_in.max(0.12)
                    } else {
                        1.0
                    };
                    if seam_bottom > seam_top + 4.0 * s {
                        let (sr, sg, sb, base_a) = if self.light_glass {
                            (0.0, 0.0, 0.0, 0.075)
                        } else {
                            (1.0, 1.0, 1.0, 0.10)
                        };
                        let peak_a = base_a * seam_fade;
                        let seam_stops = [
                            D2D1_GRADIENT_STOP {
                                position: 0.0,
                                color: D2D1_COLOR_F {
                                    r: sr,
                                    g: sg,
                                    b: sb,
                                    a: 0.0,
                                },
                            },
                            D2D1_GRADIENT_STOP {
                                position: 0.14,
                                color: D2D1_COLOR_F {
                                    r: sr,
                                    g: sg,
                                    b: sb,
                                    a: peak_a,
                                },
                            },
                            D2D1_GRADIENT_STOP {
                                position: 0.86,
                                color: D2D1_COLOR_F {
                                    r: sr,
                                    g: sg,
                                    b: sb,
                                    a: peak_a,
                                },
                            },
                            D2D1_GRADIENT_STOP {
                                position: 1.0,
                                color: D2D1_COLOR_F {
                                    r: sr,
                                    g: sg,
                                    b: sb,
                                    a: 0.0,
                                },
                            },
                        ];
                        if let Ok(sc) = self.rt.CreateGradientStopCollection(
                            &seam_stops,
                            D2D1_GAMMA_2_2,
                            D2D1_EXTEND_MODE_CLAMP,
                        ) {
                            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                                startPoint: D2D_POINT_2F {
                                    x: seam_x,
                                    y: seam_top,
                                },
                                endPoint: D2D_POINT_2F {
                                    x: seam_x,
                                    y: seam_bottom,
                                },
                            };
                            if let Ok(seam_brush) =
                                self.rt.CreateLinearGradientBrush(&props, None, &sc)
                            {
                                self.rt.DrawLine(
                                    D2D_POINT_2F {
                                        x: seam_x,
                                        y: seam_top,
                                    },
                                    D2D_POINT_2F {
                                        x: seam_x,
                                        y: seam_bottom,
                                    },
                                    &seam_brush,
                                    1.0 * s,
                                    None,
                                );
                            }
                        }
                    }

                    let close_factor = if self.anim_phase == PopupAnimPhase::Closing {
                        (1.0 - 0.30 * ease_retract(self.close_progress)).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };

                    if let Some(sa) = self.sub_anim {
                        if let Some(prev_cat) = sa.prev_category {
                            if sa.fade_elapsed < 0.048 {
                                let fade_out =
                                    (1.0 - ease_out_cubic((sa.fade_elapsed / 0.048).clamp(0.0, 1.0)))
                                        * close_factor;
                                if fade_out > 0.005 {
                                    let prev_target = self.sub_card_rect(prev_cat);
                                    let dy_prev = (anim_sub.top - prev_target.top) * 0.30;
                                    let layer_params = D2D1_LAYER_PARAMETERS {
                                        contentBounds: d2d_rect(anim_sub),
                                        geometricMask: ManuallyDrop::new(None),
                                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                        maskTransform: identity_matrix(),
                                        opacity: fade_out,
                                        opacityBrush: ManuallyDrop::new(None),
                                        layerOptions: D2D1_LAYER_OPTIONS_NONE,
                                    };
                                    self.rt.PushLayer(&layer_params, &self.content_layer);
                                    self.rt.PushAxisAlignedClip(
                                        &d2d_rect(anim_sub),
                                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                    );
                                    if dy_prev.abs() > 0.05 {
                                        self.rt.SetTransform(&Matrix3x2 {
                                            M11: 1.0,
                                            M12: 0.0,
                                            M21: 0.0,
                                            M22: 1.0,
                                            M31: 0.0,
                                            M32: dy_prev,
                                        });
                                    }
                                    self.render_submenu_content(
                                        prev_cat,
                                        prev_target,
                                        auto_accent,
                                        &fmts.title_fmt,
                                        &fmts.sub_fmt,
                                        &fmts.badge_fmt,
                                        &fmts.pct_fmt,
                                        s,
                                    );
                                    if dy_prev.abs() > 0.05 {
                                        self.rt.SetTransform(&identity_matrix());
                                    }
                                    self.rt.PopAxisAlignedClip();
                                    self.rt.PopLayer();
                                }
                            }
                        }
                    }

                    let cur_alpha = fade_in * close_factor;
                    if cur_alpha > 0.005 {
                        let need_layer =
                            self.sub_anim.is_some() || self.anim_phase != PopupAnimPhase::Open;
                        if need_layer {
                            let layer_params = D2D1_LAYER_PARAMETERS {
                                contentBounds: d2d_rect(anim_sub),
                                geometricMask: ManuallyDrop::new(None),
                                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                maskTransform: identity_matrix(),
                                opacity: cur_alpha,
                                opacityBrush: ManuallyDrop::new(None),
                                layerOptions: D2D1_LAYER_OPTIONS_NONE,
                            };
                            self.rt.PushLayer(&layer_params, &self.content_layer);
                        }
                        self.rt.PushAxisAlignedClip(
                            &d2d_rect(anim_sub),
                            D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        );
                        if dx_cur.abs() > 0.05 || dy_cur.abs() > 0.05 {
                            self.rt.SetTransform(&Matrix3x2 {
                                M11: 1.0,
                                M12: 0.0,
                                M21: 0.0,
                                M22: 1.0,
                                M31: dx_cur,
                                M32: dy_cur,
                            });
                        }
                        self.render_submenu_content(
                            cat,
                            target_sub,
                            auto_accent,
                            &fmts.title_fmt,
                            &fmts.sub_fmt,
                            &fmts.badge_fmt,
                            &fmts.pct_fmt,
                            s,
                        );
                        if dx_cur.abs() > 0.05 || dy_cur.abs() > 0.05 {
                            self.rt.SetTransform(&identity_matrix());
                        }
                        self.rt.PopAxisAlignedClip();
                        if need_layer {
                            self.rt.PopLayer();
                        }
                    }
                }

                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(outer_params.geometricMask);

                // 3. Restrained closed glass rim stroke harmonized with taskbar style
                draw_geometry_rim(
                    &self.rt,
                    &geom_for_rim,
                    union_bounds.left,
                    union_bounds.top,
                    union_bounds.bottom,
                    s,
                    &recipe,
                );
            }

            self.rt.EndDraw(None, None)?;
        }

        let win_alpha =
            window_master_alpha(self.anim_phase, self.open_elapsed, self.close_progress);
        let alpha_u8 = (win_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        self.surface
            .commit_with_alpha(self.hwnd, self.win_x, self.win_y, alpha_u8);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn render_submenu_content(
        &self,
        cat: PrimaryCategory,
        sub_rect: RectF,
        auto_accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        badge_fmt: &IDWriteTextFormat,
        pct_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        match cat {
            PrimaryCategory::DisplayContent => {
                self.render_display_submenu(
                    sub_rect,
                    auto_accent,
                    title_fmt,
                    sub_fmt,
                    badge_fmt,
                    s,
                );
            }
            PrimaryCategory::VolumeControl => {
                self.render_volume_submenu(
                    sub_rect,
                    auto_accent,
                    title_fmt,
                    sub_fmt,
                    pct_fmt,
                    s,
                );
            }
            PrimaryCategory::PlaybackDevices => {
                self.render_devices_submenu(
                    sub_rect,
                    &self.playback_devices,
                    true,
                    auto_accent,
                    title_fmt,
                    sub_fmt,
                    s,
                );
            }
            PrimaryCategory::RecordingDevices => {
                self.render_devices_submenu(
                    sub_rect,
                    &self.recording_devices,
                    false,
                    auto_accent,
                    title_fmt,
                    sub_fmt,
                    s,
                );
            }
            PrimaryCategory::Settings => {
                self.render_settings_submenu(sub_rect, auto_accent, title_fmt, s);
            }
        }
    }

    unsafe fn draw_primary_row_bg(
        &self,
        rect: RectF,
        hovered: bool,
        expanded: bool,
        accent: Rgb,
        s: f32,
    ) {
        if !hovered && !expanded {
            return;
        }
        let rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(rect),
            radiusX: 8.0 * s,
            radiusY: 8.0 * s,
        };
        let fill = if expanded {
            if self.light_glass {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: if hovered { 0.10 } else { 0.07 },
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: if hovered { 0.17 } else { 0.12 },
                }
            }
        } else if self.light_glass {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.06,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.10,
            }
        };
        if let Ok(b) = self.rt.CreateSolidColorBrush(&fill, None) {
            self.rt.FillRoundedRectangle(&rr, &b);
        }
        if expanded {
            let bar_w = 3.0 * s;
            let bar_h = 14.0 * s;
            let bar_left = if self.open_left {
                rect.right - 3.0 * s - bar_w
            } else {
                rect.left + 3.0 * s
            };
            let bar_top = (rect.top + rect.bottom - bar_h) * 0.5;
            let bar_rr = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: bar_left,
                    top: bar_top,
                    right: bar_left + bar_w,
                    bottom: bar_top + bar_h,
                },
                radiusX: bar_w * 0.5,
                radiusY: bar_w * 0.5,
            };
            if let Ok(ab) = self
                .rt
                .CreateSolidColorBrush(&d2d_color_rgb(accent, 0.95), None)
            {
                self.rt.FillRoundedRectangle(&bar_rr, &ab);
            }
        }
    }

    unsafe fn draw_primary_row_content(
        &self,
        cat: PrimaryCategory,
        rect: RectF,
        expanded: bool,
        title_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let mut fg = primary_text_color(self.light_glass);
        if !expanded {
            fg.a *= 0.90;
        }
        let icon_cx = rect.left + 19.0 * s;
        let icon_cy = (rect.top + rect.bottom) * 0.5;
        let arrow_cx = rect.right - 13.0 * s;

        if let Ok(brush) = self.rt.CreateSolidColorBrush(&fg, None) {
            self.draw_category_icon(cat, icon_cx, icon_cy, s, &brush);
            let mut arrow_col = muted_text_color(self.light_glass);
            if expanded {
                arrow_col.a = 0.92;
            }
            if let Ok(ab) = self.rt.CreateSolidColorBrush(&arrow_col, None) {
                self.draw_chevron(arrow_cx, icon_cy, self.open_left, s, &ab);
            }
        }

        let label_rect = RectF::new(
            rect.left + 34.0 * s,
            rect.top,
            rect.right - 22.0 * s,
            rect.bottom,
        );
        draw_text_in_rect(&self.dwrite_factory, &self.rt, cat.label(), label_rect, title_fmt, fg);
    }

    unsafe fn draw_category_icon(
        &self,
        cat: PrimaryCategory,
        cx: f32,
        cy: f32,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        match cat {
            PrimaryCategory::DisplayContent => {
                draw_music_note_icon(&self.rt, cx, cy, s, brush);
            }
            PrimaryCategory::VolumeControl => {
                // Speaker cone with double sound wave arcs
                draw_speaker_glyph(
                    &self.d2d_factory,
                    &self.rt,
                    cx,
                    cy,
                    0.92 * s,
                    0.75,
                    false,
                    brush,
                    brush,
                    brush,
                    self.round_stroke.as_ref(),
                );
            }
            PrimaryCategory::PlaybackDevices => {
                // Over-ear headphones (top headband arc + left/right earcups)
                if let Ok(band) = self.d2d_factory.CreatePathGeometry() {
                    if let Ok(sink) = band.Open() {
                        sink.BeginFigure(
                            D2D_POINT_2F {
                                x: cx - 4.3 * s,
                                y: cy + 0.4 * s,
                            },
                            D2D1_FIGURE_BEGIN_HOLLOW,
                        );
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: D2D_POINT_2F {
                                x: cx + 4.3 * s,
                                y: cy + 0.4 * s,
                            },
                            size: D2D_SIZE_F {
                                width: 4.3 * s,
                                height: 4.9 * s,
                            },
                            rotationAngle: 0.0,
                            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                            arcSize: D2D1_ARC_SIZE_SMALL,
                        });
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        let _ = sink.Close();
                        self.rt
                            .DrawGeometry(&band, brush, 1.45 * s, self.round_stroke.as_ref());
                    }
                }
                let left_cup = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 5.5 * s,
                        top: cy - 0.3 * s,
                        right: cx - 2.9 * s,
                        bottom: cy + 4.6 * s,
                    },
                    radiusX: 1.3 * s,
                    radiusY: 1.3 * s,
                };
                let right_cup = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx + 2.9 * s,
                        top: cy - 0.3 * s,
                        right: cx + 5.5 * s,
                        bottom: cy + 4.6 * s,
                    },
                    radiusX: 1.3 * s,
                    radiusY: 1.3 * s,
                };
                self.rt.FillRoundedRectangle(&left_cup, brush);
                self.rt.FillRoundedRectangle(&right_cup, brush);
            }
            PrimaryCategory::RecordingDevices => {
                // Microphone capsule + stand
                let mic_rr = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx - 2.1 * s,
                        top: cy - 5.5 * s,
                        right: cx + 2.1 * s,
                        bottom: cy + 1.2 * s,
                    },
                    radiusX: 2.1 * s,
                    radiusY: 2.1 * s,
                };
                self.rt.FillRoundedRectangle(&mic_rr, brush);
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 3.8 * s,
                        y: cy - 0.5 * s,
                    },
                    D2D_POINT_2F {
                        x: cx - 3.8 * s,
                        y: cy + 2.4 * s,
                    },
                    brush,
                    1.2 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx + 3.8 * s,
                        y: cy - 0.5 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 3.8 * s,
                        y: cy + 2.4 * s,
                    },
                    brush,
                    1.2 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 3.8 * s,
                        y: cy + 2.4 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 3.8 * s,
                        y: cy + 2.4 * s,
                    },
                    brush,
                    1.2 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx,
                        y: cy + 2.4 * s,
                    },
                    D2D_POINT_2F {
                        x: cx,
                        y: cy + 5.2 * s,
                    },
                    brush,
                    1.3 * s,
                    None,
                );
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: cx - 2.6 * s,
                        y: cy + 5.2 * s,
                    },
                    D2D_POINT_2F {
                        x: cx + 2.6 * s,
                        y: cy + 5.2 * s,
                    },
                    brush,
                    1.3 * s,
                    None,
                );
            }
            PrimaryCategory::Settings => {
                draw_settings_icon(&self.rt, cx, cy, s, brush);
            }
        }
    }

    unsafe fn draw_chevron(
        &self,
        cx: f32,
        cy: f32,
        point_left: bool,
        s: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let dir = if point_left { -1.0 } else { 1.0 };
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx - 1.8 * dir * s,
                y: cy - 3.4 * s,
            },
            D2D_POINT_2F {
                x: cx + 1.8 * dir * s,
                y: cy,
            },
            brush,
            1.35 * s,
            None,
        );
        self.rt.DrawLine(
            D2D_POINT_2F {
                x: cx + 1.8 * dir * s,
                y: cy,
            },
            D2D_POINT_2F {
                x: cx - 1.8 * dir * s,
                y: cy + 3.4 * s,
            },
            brush,
            1.35 * s,
            None,
        );
    }

}
