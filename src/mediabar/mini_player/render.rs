use super::{CachedFormats, MiniPlayerInner};
use super::super::layout::{MiniPlayerHitZone, MiniPlayerLayout, RectF};
use super::super::marquee::MarqueeState;
use super::super::render::{draw_next_icon, draw_pause_icon, draw_play_icon, draw_prev_icon, draw_speaker_glyph};
use super::super::smtc::MediaCover;
use super::super::timeline::{current_filetime_100ns, format_media_time};
use super::super::volume::format_volume_pct;
use crate::color::Rgb;
use crate::system::active_taskbar_surface_style;
use crate::ui::glass::{create_dc_render_target, d2d_color_rgb, d2d_rect, draw_glass_interior, draw_glass_slider, draw_music_note_icon, draw_rounded_card_rim, draw_rounded_card_shadows, draw_text_in_rect, identity_matrix, muted_text_color, primary_text_color, update_silk_bitmap};
use crate::ui::{popup_glass_recipe, rgb_luma, window_master_alpha};
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::time::Instant;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D_POINT_2F};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1Brush, ID2D1Geometry, ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT};
use windows::Win32::Graphics::DirectWrite::IDWriteTextLayout;

impl MiniPlayerInner {
    fn recreate_render_target(&mut self) -> windows::core::Result<()> {
        self.backdrop_bmp = None;
        self.silk_bmp = None;
        self.cached_art = None;
        self.noise_cache.clear();
        self.last_silk_tick = None;
        let new_rt = create_dc_render_target(&self.d2d_factory)?;
        let new_card_layer = unsafe { new_rt.CreateLayer(None)? };
        let new_art_layer = unsafe { new_rt.CreateLayer(None)? };
        let new_marquee_layer = unsafe { new_rt.CreateLayer(None)? };
        self.card_layer = new_card_layer;
        self.art_layer = new_art_layer;
        self.marquee_layer = new_marquee_layer;
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
        let layout = self.current_layout();
        self.surface.ensure_size(layout.win_w, layout.win_h)?;
        let formats = self.ensure_formats()?;
        self.update_halo_colors();

        let s = layout.scale;
        let light = self.light_glass;
        let accent = self.current_ui_accent_rgb();
        let is_playing = self.is_playing();
        let (card_rect, content_alpha) = self.animated_card_rect(layout.card_rect, s);
        let master_alpha = window_master_alpha(self.anim_phase, self.open_elapsed, self.close_progress);

        let bg_luma = rgb_luma(self.backdrop_rgb);
        let recipe = popup_glass_recipe(active_taskbar_surface_style(), light, bg_luma);

        if recipe.silk_alpha > 0.002 {
            update_silk_bitmap(
                &self.rt,
                &mut self.silk_bmp,
                &mut self.silk_buf,
                &mut self.last_silk_tick,
                false,
                layout.win_w,
                layout.win_h,
                card_rect.left,
                card_rect.bottom,
                s,
                self.flow_phase,
                &self.halo_colors,
                light,
            );
        }

        let cover_arc: Option<Arc<MediaCover>> =
            self.source.as_ref().and_then(|src| src.cover.clone());
        let art_bmp = cover_arc.as_ref().and_then(|c| self.ensure_art_bitmap(c));

        let title_str = self
            .source
            .as_ref()
            .map(|src| src.title.as_str())
            .filter(|t| !t.is_empty())
            .unwrap_or("未在播放");
        let artist_str = self
            .source
            .as_ref()
            .map(|src| src.artist.as_str())
            .unwrap_or("");
        let badge_str = self
            .source
            .as_ref()
            .map(|src| src.app_name.as_str())
            .filter(|a| !a.is_empty())
            .unwrap_or("媒体控制");

        let title_layout = Self::measure_marquee_text(
            &self.dwrite_factory,
            &formats.title_fmt,
            &mut self.cached_title,
            title_str,
            self.dpi,
            layout.title_rect.height(),
        );
        if let Some((_, w)) = &title_layout {
            self.title_marquee.set_text(title_str, *w);
            self.title_marquee
                .set_widths(*w, layout.title_rect.width());
        } else {
            self.title_marquee.set_text("", 0.0);
        }

        let artist_layout = if !artist_str.is_empty() {
            Self::measure_marquee_text(
                &self.dwrite_factory,
                &formats.artist_fmt,
                &mut self.cached_artist,
                artist_str,
                self.dpi,
                layout.artist_rect.height(),
            )
        } else {
            None
        };
        if let Some((_, w)) = &artist_layout {
            self.artist_marquee.set_text(artist_str, *w);
            self.artist_marquee
                .set_widths(*w, layout.artist_rect.width());
        } else {
            self.artist_marquee.set_text("", 0.0);
        }

        let timeline = self.effective_timeline();
        let now_ft = current_filetime_100ns();
        let (slider_ratio, elapsed_secs, total_secs, can_seek) = if let Some(tl) = timeline {
            if let Some(drag_r) = self.dragging_seek_ratio {
                let (el, tot) = tl.preview_secs_for_ratio(drag_r).unwrap_or((0.0, 0.0));
                (drag_r, el, tot, tl.can_seek)
            } else {
                let r = tl.progress_ratio(now_ft, is_playing).unwrap_or(0.0);
                let (el, tot) = tl
                    .elapsed_and_total_secs(now_ft, is_playing)
                    .unwrap_or((0.0, 0.0));
                (r, el, tot, tl.can_seek)
            }
        } else {
            (0.0, 0.0, 0.0, false)
        };

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

            // 1. 3-pass ambient shadows
            draw_rounded_card_shadows(
                &self.rt,
                card_rect,
                layout.card_radius,
                layout.win_w as f32,
                layout.win_h as f32,
                s,
                light,
            );

            // 2. Clipped glass interior + subtle album artwork radial atmosphere glow
            let card_rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(card_rect),
                radiusX: layout.card_radius,
                radiusY: layout.card_radius,
            };
            if let Ok(geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&card_rr) {
                let geom: ID2D1Geometry = geo.into();
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(card_rect),
                    geometricMask: ManuallyDrop::new(Some(geom)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, &self.card_layer);

                draw_glass_interior(
                    &self.rt,
                    card_rect,
                    layout.win_w,
                    layout.win_h,
                    self.backdrop_bmp.as_ref(),
                    self.silk_bmp.as_ref(),
                    light,
                    &recipe,
                    &mut self.noise_cache,
                );

                // Soft album-tinted radial glow emanating from behind the artwork
                if self.source.is_some() {
                    let glow_cx = (layout.cover_rect.left + layout.cover_rect.right) * 0.5;
                    let glow_cy = (layout.cover_rect.top + layout.cover_rect.bottom) * 0.5
                        + (card_rect.top - layout.card_rect.top);
                    let glow_stops = [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: d2d_color_rgb(accent, if light { 0.14 } else { 0.22 }),
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.55,
                            color: d2d_color_rgb(accent, if light { 0.05 } else { 0.08 }),
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: d2d_color_rgb(accent, 0.0),
                        },
                    ];
                    if let Ok(sc) = self.rt.CreateGradientStopCollection(
                        &glow_stops,
                        D2D1_GAMMA_2_2,
                        D2D1_EXTEND_MODE_CLAMP,
                    ) {
                        let rad_props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                            center: D2D_POINT_2F {
                                x: glow_cx,
                                y: glow_cy,
                            },
                            gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                            radiusX: card_rect.width() * 0.68,
                            radiusY: card_rect.height() * 0.85,
                        };
                        if let Ok(glow_brush) =
                            self.rt.CreateRadialGradientBrush(&rad_props, None, &sc)
                        {
                            self.rt.FillRectangle(&d2d_rect(card_rect), &glow_brush);
                        }
                    }
                }

                // 3. Foreground content (fades smoothly with spring animation)
                if content_alpha > 0.01 {
                    let dy = card_rect.top - layout.card_rect.top;
                    self.draw_content(
                        &layout,
                        &formats,
                        dy,
                        content_alpha,
                        accent,
                        is_playing,
                        art_bmp.as_ref(),
                        badge_str,
                        title_layout.as_ref().map(|(tl, _)| tl),
                        artist_layout.as_ref().map(|(al, _)| al),
                        slider_ratio,
                        elapsed_secs,
                        total_secs,
                        can_seek,
                    );
                }

                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(params.geometricMask);
                let _ = ManuallyDrop::into_inner(params.opacityBrush);
            }

            // 4. Glass rim highlight
            draw_rounded_card_rim(&self.rt, card_rect, layout.card_radius, s, &recipe);

            self.rt.EndDraw(None, None)?;
        }

        let alpha_u8 = (master_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        self.surface
            .commit_with_alpha(self.hwnd, self.win_x, self.win_y, alpha_u8);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_content(
        &self,
        layout: &MiniPlayerLayout,
        formats: &CachedFormats,
        dy: f32,
        alpha: f32,
        accent: Rgb,
        is_playing: bool,
        art_bmp: Option<&ID2D1Bitmap>,
        badge_str: &str,
        title_layout: Option<&IDWriteTextLayout>,
        artist_layout: Option<&IDWriteTextLayout>,
        slider_ratio: f32,
        elapsed_secs: f64,
        total_secs: f64,
        can_seek: bool,
    ) {
        let s = layout.scale;
        let light = self.light_glass;
        let shift = |r: RectF| RectF::new(r.left, r.top + dy, r.right, r.bottom + dy);

        // Hover backdrop behind artwork + title when user hovers to jump to player
        if self.hover_zone == MiniPlayerHitZone::CoverOrInfo && self.source.is_some() {
            let hr = shift(layout.info_hit_rect);
            let pad = 4.0 * s;
            let pill = RectF::new(hr.left - pad, hr.top - pad, hr.right + pad, hr.bottom + pad);
            let bg = if light {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.045 * alpha,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.055 * alpha,
                }
            };
            if let Ok(b) = self.rt.CreateSolidColorBrush(&bg, None) {
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(pill),
                    radiusX: 11.0 * s,
                    radiusY: 11.0 * s,
                };
                self.rt.FillRoundedRectangle(&rr, &b);
            }
        }

        // 1. Large crisp album artwork (or default music note card)
        let cover_rect = shift(layout.cover_rect);
        let cover_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(cover_rect),
            radiusX: layout.cover_radius,
            radiusY: layout.cover_radius,
        };
        // Subtle elevation shadow under cover tile
        let art_shadow = RectF::new(
            cover_rect.left,
            cover_rect.top + 2.5 * s,
            cover_rect.right,
            cover_rect.bottom + 3.5 * s,
        );
        if let Ok(sb) = self.rt.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: (if light { 0.14 } else { 0.30 }) * alpha,
            },
            None,
        ) {
            let sh_rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(art_shadow),
                radiusX: layout.cover_radius,
                radiusY: layout.cover_radius,
            };
            self.rt.FillRoundedRectangle(&sh_rr, &sb);
        }

        if let Some(bmp) = art_bmp {
            if let Ok(geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&cover_rr) {
                let geom: ID2D1Geometry = geo.into();
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(cover_rect),
                    geometricMask: ManuallyDrop::new(Some(geom)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: alpha,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, &self.art_layer);
                self.rt.DrawBitmap(
                    bmp,
                    Some(&d2d_rect(cover_rect)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(params.geometricMask);
                let _ = ManuallyDrop::into_inner(params.opacityBrush);
            }
        } else {
            let tile_bg = if light {
                D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.07 * alpha,
                }
            } else {
                D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.10 * alpha,
                }
            };
            if let Ok(tb) = self.rt.CreateSolidColorBrush(&tile_bg, None) {
                self.rt.FillRoundedRectangle(&cover_rr, &tb);
            }
            let note_col = d2d_color_rgb(accent, 0.85 * alpha);
            if let Ok(nb) = self.rt.CreateSolidColorBrush(&note_col, None) {
                let cx = (cover_rect.left + cover_rect.right) * 0.5;
                let cy = (cover_rect.top + cover_rect.bottom) * 0.5;
                draw_music_note_icon(&self.rt, cx, cy, 2.2 * s, &nb);
            }
        }

        let cover_rim = if light {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.12 * alpha,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.20 * alpha,
            }
        };
        if let Ok(rb) = self.rt.CreateSolidColorBrush(&cover_rim, None) {
            self.rt.DrawRoundedRectangle(&cover_rr, &rb, 1.0 * s, None);
        }

        // 2. Source badge (status dot + app name)
        let badge_rect = shift(layout.source_badge_rect);
        let dot_r = 2.8 * s;
        let dot_cx = badge_rect.left + dot_r + 0.5 * s;
        let dot_cy = (badge_rect.top + badge_rect.bottom) * 0.5;
        let dot_col = if is_playing {
            d2d_color_rgb(accent, 0.95 * alpha)
        } else {
            let mut m = muted_text_color(light);
            m.a *= 0.72 * alpha;
            m
        };
        if let Ok(db) = self.rt.CreateSolidColorBrush(&dot_col, None) {
            self.rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: dot_cx,
                        y: dot_cy,
                    },
                    radiusX: dot_r,
                    radiusY: dot_r,
                },
                &db,
            );
        }
        let mut badge_text_col = muted_text_color(light);
        badge_text_col.a *= 0.92 * alpha;
        let badge_text_rect = RectF::new(
            dot_cx + dot_r + 5.0 * s,
            badge_rect.top,
            badge_rect.right,
            badge_rect.bottom,
        );
        draw_text_in_rect(
            &self.dwrite_factory,
            &self.rt,
            badge_str,
            badge_text_rect,
            &formats.badge_fmt,
            badge_text_col,
        );

        // 3. Compact live mini-spectrum in header
        let spec_alpha = (if self.source.is_some() { 0.90 } else { 0.30 }) * alpha;
        let spec_col = d2d_color_rgb(accent, spec_alpha);
        if let Ok(sb) = self.rt.CreateSolidColorBrush(&spec_col, None) {
            for r in &layout.mini_spectrum_rects {
                let sr = shift(*r);
                let rad = sr.width() * 0.5;
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(sr),
                    radiusX: rad,
                    radiusY: rad,
                };
                self.rt.FillRoundedRectangle(&rr, &sb);
            }
        }

        // 4. Title & Artist (marquee-aware)
        let mut title_col = primary_text_color(light);
        title_col.a *= if self.source.is_some() {
            alpha
        } else {
            0.60 * alpha
        };
        if let Some(tl) = title_layout {
            let t_rect = if artist_layout.is_none() {
                let mid = (layout.title_rect.top + layout.artist_rect.bottom) * 0.5;
                let h = layout.title_rect.height();
                shift(RectF::new(
                    layout.title_rect.left,
                    mid - h * 0.5,
                    layout.title_rect.right,
                    mid + h * 0.5,
                ))
            } else {
                shift(layout.title_rect)
            };
            self.draw_marquee_text(tl, t_rect, title_col, &self.title_marquee, s);
        }
        if let Some(al) = artist_layout {
            let mut artist_col = muted_text_color(light);
            artist_col.a *= alpha;
            self.draw_marquee_text(
                al,
                shift(layout.artist_rect),
                artist_col,
                &self.artist_marquee,
                s,
            );
        }

        // 6. Interactive timeline slider + elapsed / total timestamps
        if layout.has_timeline {
            let track = shift(layout.progress_track_rect);
            let active = self.dragging_seek_ratio.is_some()
                || (can_seek
                    && (self.hover_zone == MiniPlayerHitZone::ProgressSlider
                        || self.pressed_zone == MiniPlayerHitZone::ProgressSlider));
            draw_glass_slider(
                &self.rt,
                track,
                slider_ratio,
                !is_playing && self.dragging_seek_ratio.is_none(),
                active,
                light,
                accent,
                s,
            );

            let mut time_col = muted_text_color(light);
            time_col.a *= if self.dragging_seek_ratio.is_some() {
                0.96 * alpha
            } else {
                0.82 * alpha
            };
            let el_str = format_media_time(elapsed_secs);
            let tot_str = format_media_time(total_secs);
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                &el_str,
                shift(layout.elapsed_rect),
                &formats.time_l_fmt,
                time_col,
            );
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                &tot_str,
                shift(layout.duration_rect),
                &formats.time_r_fmt,
                time_col,
            );
        }

        // 6. Bottom control bar: Menu | Prev | Play/Pause (accent circle) | Next | Mute
        let more_rect = shift(layout.more_btn_rect);
        let more_hovered = self.hover_zone == MiniPlayerHitZone::MoreMenuButton;
        let more_pressed = self.pressed_zone == MiniPlayerHitZone::MoreMenuButton;
        self.draw_icon_button_bg(more_rect, 7.0 * s, more_hovered, more_pressed, alpha);
        let mut util_col = if more_hovered {
            primary_text_color(light)
        } else {
            muted_text_color(light)
        };
        util_col.a *= (if more_pressed { 0.72 } else { 1.0 }) * alpha;
        if let Ok(ub) = self.rt.CreateSolidColorBrush(&util_col, None) {
            self.draw_menu_icon(more_rect, s * if more_pressed { 0.92 } else { 1.0 }, &ub);
        }

        // Prev / Next buttons
        for (zone, base_rect) in [
            (MiniPlayerHitZone::PrevButton, layout.prev_btn_rect),
            (MiniPlayerHitZone::NextButton, layout.next_btn_rect),
        ] {
            let rect = shift(base_rect);
            let hovered = self.hover_zone == zone;
            let pressed = self.pressed_zone == zone;
            self.draw_icon_button_bg(rect, 8.0 * s, hovered, pressed, alpha);
            let mut fg = primary_text_color(light);
            fg.a *= (if pressed { 0.72 } else { 0.94 }) * alpha;
            if let Ok(b) = self.rt.CreateSolidColorBrush(&fg, None) {
                let cx = (rect.left + rect.right) * 0.5;
                let cy = (rect.top + rect.bottom) * 0.5;
                let u = (rect.width() / 24.0) * if pressed { 0.90 } else { 1.0 };
                match zone {
                    MiniPlayerHitZone::PrevButton => {
                        draw_prev_icon(&self.d2d_factory, &self.rt, cx, cy, u, &b)
                    }
                    MiniPlayerHitZone::NextButton => {
                        draw_next_icon(&self.d2d_factory, &self.rt, cx, cy, u, &b)
                    }
                    _ => {}
                }
            }
        }

        // Primary circular Play/Pause button tinted with theme-calibrated accent
        let play_rect = shift(layout.play_btn_rect);
        let play_hovered = self.hover_zone == MiniPlayerHitZone::PlayPauseButton;
        let play_pressed = self.pressed_zone == MiniPlayerHitZone::PlayPauseButton;
        let play_cx = (play_rect.left + play_rect.right) * 0.5;
        let play_cy = (play_rect.top + play_rect.bottom) * 0.5;
        let play_r = (play_rect.width() * 0.5) * if play_pressed { 0.94 } else { 1.0 };
        let accent_c = d2d_color_rgb(accent, (if play_hovered { 0.98 } else { 0.90 }) * alpha);
        let fill_col = if play_hovered {
            D2D1_COLOR_F {
                r: (accent_c.r * 1.08 + 0.04).min(1.0),
                g: (accent_c.g * 1.08 + 0.04).min(1.0),
                b: (accent_c.b * 1.08 + 0.04).min(1.0),
                a: accent_c.a,
            }
        } else {
            accent_c
        };
        if let Ok(pb) = self.rt.CreateSolidColorBrush(&fill_col, None) {
            self.rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: play_cx,
                        y: play_cy,
                    },
                    radiusX: play_r,
                    radiusY: play_r,
                },
                &pb,
            );
        }
        // Subtle specular top rim on the circular accent button
        let btn_rim = D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: (if light { 0.34 } else { 0.28 }) * alpha,
        };
        if let Ok(rb) = self.rt.CreateSolidColorBrush(&btn_rim, None) {
            self.rt.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: play_cx,
                        y: play_cy,
                    },
                    radiusX: play_r - 0.5 * s,
                    radiusY: play_r - 0.5 * s,
                },
                &rb,
                1.0 * s,
                None,
            );
        }
        // Choose crisp dark or white glyph depending on accent luminance for high contrast
        let accent_luma = rgb_luma(accent);
        let glyph_col = if accent_luma > 0.62 {
            D2D1_COLOR_F {
                r: 0.06,
                g: 0.07,
                b: 0.10,
                a: 0.96 * alpha,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.98 * alpha,
            }
        };
        if let Ok(gb) = self.rt.CreateSolidColorBrush(&glyph_col, None) {
            let u = (play_rect.width() / 28.0) * if play_pressed { 0.92 } else { 1.0 };
            if is_playing {
                draw_pause_icon(&self.rt, play_cx, play_cy, u, &gb);
            } else {
                draw_play_icon(&self.d2d_factory, &self.rt, play_cx + 0.8 * s, play_cy, u, &gb);
            }
        }

        // Mute button + ephemeral volume % feedback when scrolling wheel over the popup
        let mute_rect = shift(layout.mute_btn_rect);
        let mute_hovered = self.hover_zone == MiniPlayerHitZone::MuteButton;
        let mute_pressed = self.pressed_zone == MiniPlayerHitZone::MuteButton;
        self.draw_icon_button_bg(mute_rect, 7.0 * s, mute_hovered, mute_pressed, alpha);
        let mut spk_fg = if mute_hovered {
            primary_text_color(light)
        } else {
            muted_text_color(light)
        };
        spk_fg.a *= alpha;
        let spk_muted = D2D1_COLOR_F {
            a: spk_fg.a * 0.58,
            ..spk_fg
        };
        let spk_slash = if light {
            D2D1_COLOR_F {
                r: 0.80,
                g: 0.18,
                b: 0.20,
                a: 0.92 * alpha,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 0.46,
                b: 0.46,
                a: 0.95 * alpha,
            }
        };
        if let (Ok(fg_b), Ok(mu_b), Ok(sl_b)) = (
            self.rt.CreateSolidColorBrush(&spk_fg, None),
            self.rt.CreateSolidColorBrush(&spk_muted, None),
            self.rt.CreateSolidColorBrush(&spk_slash, None),
        ) {
            let cx = (mute_rect.left + mute_rect.right) * 0.5;
            let cy = (mute_rect.top + mute_rect.bottom) * 0.5;
            let u = (mute_rect.width() / 24.0) * if mute_pressed { 0.90 } else { 1.0 };
            draw_speaker_glyph(
                &self.d2d_factory,
                &self.rt,
                cx,
                cy,
                u,
                self.master_vol,
                self.master_muted,
                &fg_b,
                &mu_b,
                &sl_b,
                self.round_stroke.as_ref(),
            );
        }

        if self.vol_badge_until.is_some_and(|t| Instant::now() < t) {
            let pct_str = format_volume_pct(self.master_vol);
            let badge_w = 34.0 * s;
            let badge_h = 16.0 * s;
            let badge_cy = (mute_rect.top + mute_rect.bottom) * 0.5;
            let pct_rect = RectF::new(
                mute_rect.left - badge_w - 2.0 * s,
                badge_cy - badge_h * 0.5,
                mute_rect.left - 2.0 * s,
                badge_cy + badge_h * 0.5,
            );
            let mut pct_col = primary_text_color(light);
            pct_col.a *= 0.88 * alpha;
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                &pct_str,
                pct_rect,
                &formats.time_r_fmt,
                pct_col,
            );
        }
    }

    unsafe fn draw_icon_button_bg(
        &self,
        rect: RectF,
        radius: f32,
        hovered: bool,
        pressed: bool,
        alpha: f32,
    ) {
        if !hovered && !pressed {
            return;
        }
        let col = if self.light_glass {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: (if pressed { 0.13 } else { 0.07 }) * alpha,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: (if pressed { 0.17 } else { 0.10 }) * alpha,
            }
        };
        if let Ok(b) = self.rt.CreateSolidColorBrush(&col, None) {
            let rr = D2D1_ROUNDED_RECT {
                rect: d2d_rect(rect),
                radiusX: radius,
                radiusY: radius,
            };
            self.rt.FillRoundedRectangle(&rr, &b);
        }
    }

    unsafe fn draw_menu_icon(&self, rect: RectF, s: f32, brush: &ID2D1SolidColorBrush) {
        let cx = (rect.left + rect.right) * 0.5;
        let cy = (rect.top + rect.bottom) * 0.5;
        let half_w = 5.6 * s;
        let gap_y = 3.8 * s;
        let stroke = 1.45 * s;
        let rs = self.round_stroke.as_ref();

        for dy in [-gap_y, 0.0, gap_y] {
            self.rt.DrawLine(
                D2D_POINT_2F {
                    x: cx - half_w,
                    y: cy + dy,
                },
                D2D_POINT_2F {
                    x: cx + half_w,
                    y: cy + dy,
                },
                brush,
                stroke,
                rs,
            );
        }
    }

    unsafe fn draw_marquee_text(
        &self,
        text_layout: &IDWriteTextLayout,
        rect: RectF,
        color: D2D1_COLOR_F,
        marquee: &MarqueeState,
        scale: f32,
    ) {
        let Ok(text_brush) = self.rt.CreateSolidColorBrush(&color, None) else {
            return;
        };

        if !marquee.is_overflowing() {
            self.rt
                .PushAxisAlignedClip(&d2d_rect(rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: rect.left,
                    y: rect.top,
                },
                text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
            self.rt.PopAxisAlignedClip();
            return;
        }

        let (x1, x2_opt) = marquee.render_offsets();
        let w = rect.width().max(1.0);
        let fade_frac = (12.0 * scale / w).clamp(0.05, 0.22);
        let left_alpha = if x1 < -0.5 { 0.0 } else { 1.0 };

        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: left_alpha,
                },
            },
            D2D1_GRADIENT_STOP {
                position: fade_frac,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0 - fade_frac,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.0,
                },
            },
        ];

        let mut layer_params: Option<D2D1_LAYER_PARAMETERS> = None;
        if let Ok(sc) =
            self.rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
        {
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: rect.left,
                    y: rect.top,
                },
                endPoint: D2D_POINT_2F {
                    x: rect.right,
                    y: rect.top,
                },
            };
            if let Ok(grad) = self.rt.CreateLinearGradientBrush(&props, None, &sc) {
                let brush_iface: ID2D1Brush = grad.into();
                let lp = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(rect),
                    geometricMask: ManuallyDrop::new(None),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(Some(brush_iface)),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&lp, &self.marquee_layer);
                layer_params = Some(lp);
            }
        }

        if layer_params.is_none() {
            self.rt
                .PushAxisAlignedClip(&d2d_rect(rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        self.rt.DrawTextLayout(
            D2D_POINT_2F {
                x: rect.left + x1,
                y: rect.top,
            },
            text_layout,
            &text_brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );
        if let Some(x2) = x2_opt {
            self.rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: rect.left + x2,
                    y: rect.top,
                },
                text_layout,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        if let Some(lp) = layer_params {
            self.rt.PopLayer();
            let _ = ManuallyDrop::into_inner(lp.geometricMask);
            let _ = ManuallyDrop::into_inner(lp.opacityBrush);
        } else {
            self.rt.PopAxisAlignedClip();
        }
    }
}
