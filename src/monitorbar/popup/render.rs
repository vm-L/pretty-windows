use super::{HoverTarget, MonitorPopupInner, SubHoverTarget, SubLayout, SubmenuKind};
use super::super::brightness::{format_brightness_pct, BrightnessMode};
use super::super::config::{LeftClickAction, MonitorConfig};
use crate::color::Rgb;
use crate::system::{active_taskbar_surface_style, resolve_unified_accent};
use crate::ui::glass::{build_unified_menu_geometry, create_dc_render_target, d2d_color_rgb, d2d_rect, danger_text_color, draw_divider, draw_geometry_rim, draw_glass_interior, draw_glass_slider, draw_pill_switch, draw_row_highlight, draw_text_in_rect, draw_text_in_rect_measured, draw_unified_shadows, identity_matrix, muted_text_color, primary_text_color, update_silk_bitmap};
use crate::ui::{animated_primary_rect, compute_halo_pair, ease_out_cubic, ease_retract, popup_glass_recipe, rgb_luma, row_stagger_dy, window_master_alpha, PopupAnimPhase, RectF};
use std::mem::ManuallyDrop;
use std::path::Path;
use std::time::Instant;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D_POINT_2F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{ID2D1Geometry, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT};

impl MonitorPopupInner {
    fn update_silk_and_accent(&mut self, pri_rect: RectF, s: f32, draw_silk: bool) {
        let now = Instant::now();
        let dt_color = now
            .duration_since(self.last_color_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_color_tick = now;

        let (ui_rgb, halo_base_dark) = resolve_unified_accent(self.backdrop_rgb, self.light_glass);
        let pair = compute_halo_pair(halo_base_dark, self.light_glass);
        let target = [pair[0].to_normalized(), pair[1].to_normalized()];
        let ui_target = ui_rgb.to_normalized();

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
                pri_rect.left,
                pri_rect.bottom,
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
        self.noise_cache.clear();
        self.last_silk_tick = None;
        let new_rt = create_dc_render_target(&self.d2d_factory)?;
        let new_layer = unsafe { new_rt.CreateLayer(None)? };
        let new_content_layer = unsafe { new_rt.CreateLayer(None)? };
        self.layer = new_layer;
        self.content_layer = new_content_layer;
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
        let s = self.scale();
        self.surface.ensure_size(self.win_w, self.win_h)?;
        self.ensure_formats(s)?;

        let target_primary = self.primary_card_rect();
        let recipe = popup_glass_recipe(
            active_taskbar_surface_style(),
            self.light_glass,
            rgb_luma(self.backdrop_rgb),
        );
        self.update_silk_and_accent(target_primary, s, recipe.silk_alpha > 0.002);

        let accent = Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        );
        let cfg = self.cfg_snapshot();

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
            PopupAnimPhase::Opening => (
                ease_out_cubic(((self.open_elapsed - 0.008) / 0.080).clamp(0.0, 1.0)),
                0.0,
            ),
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

        let bind_rect = RECT {
            left: 0,
            top: 0,
            right: self.surface.width(),
            bottom: self.surface.height(),
        };

        unsafe {
            self.rt.BindDC(self.surface.dc(), &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.SetTransform(&identity_matrix());
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

            // 2. Closed fused silhouette clipping layer
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

                // 2a. Primary card rows & divider
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
                        let _ = ManuallyDrop::into_inner(content_params.geometricMask);
                        let _ = ManuallyDrop::into_inner(content_params.opacityBrush);
                    }

                    let row_count = primary_rows.len();
                    for (idx, (item, mut r)) in primary_rows.into_iter().enumerate() {
                        match self.anim_phase {
                            PopupAnimPhase::Opening => {
                                let dy = row_stagger_dy(
                                    idx,
                                    row_count,
                                    self.open_elapsed,
                                    self.open_spring.pos,
                                    s,
                                );
                                r.top += dy;
                                r.bottom += dy;
                            }
                            PopupAnimPhase::Open => {}
                            PopupAnimPhase::Closing => {
                                r.top += close_dy;
                                r.bottom += close_dy;
                            }
                        }

                        let hovered = self.hover == Some(HoverTarget::Primary(item));
                        let expanded =
                            self.active_sub.is_some() && item.submenu() == self.active_sub;
                        self.draw_primary_row_bg(r, hovered, expanded, accent, s);

                        let mut fg = primary_text_color(self.light_glass);
                        if !hovered && !expanded {
                            fg.a *= 0.88;
                        }

                        if let Ok(icon_brush) = self.rt.CreateSolidColorBrush(&fg, None) {
                            let icx = r.left + 19.0 * s;
                            let icy = (r.top + r.bottom) * 0.5;
                            self.draw_primary_icon(item, icx, icy, s, &icon_brush);
                            if item.submenu().is_some() {
                                let mut chev_col = muted_text_color(self.light_glass);
                                if expanded || hovered {
                                    chev_col = fg;
                                }
                                if let Ok(cb) = self.rt.CreateSolidColorBrush(&chev_col, None) {
                                    self.draw_chevron(r.right - 12.0 * s, icy, s, &cb);
                                }
                            }
                        }

                        let label_r = RectF::new(
                            r.left + 34.0 * s,
                            r.top,
                            r.right - if item.submenu().is_some() { 22.0 * s } else { 10.0 * s },
                            r.bottom,
                        );
                        if let Some(ref fmt) = self.title_fmt {
                            draw_text_in_rect(
                                &self.dwrite_factory,
                                &self.rt,
                                item.label(),
                                label_r,
                                fmt,
                                fg,
                            );
                        }
                    }

                    if use_layer {
                        self.rt.PopLayer();
                    }
                }

                // 2b. Feathered seam hairline & sliding/budding submenu content
                if let (Some(kind), Some(anim_sub)) = (self.active_sub, anim_sub_opt) {
                    let target_sub = self.sub_card_rect(kind);
                    let close_factor = if self.anim_phase == PopupAnimPhase::Closing {
                        let r = ease_retract(self.close_progress);
                        (1.0 - 0.35 * r).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
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

                    let seam_fade = if self.sub_anim.is_some_and(|sa| sa.is_budding) {
                        fade_in.max(0.12)
                    } else {
                        1.0
                    };
                    self.draw_seam_hairline(anim_primary, anim_sub, seam_fade, s);

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
                            let _ = ManuallyDrop::into_inner(layer_params.geometricMask);
                            let _ = ManuallyDrop::into_inner(layer_params.opacityBrush);
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

                        let sub_layout = self.build_sub_layout(target_sub, kind);
                        self.draw_submenu_contents(kind, &sub_layout, &cfg, accent, s);

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
                let _ = ManuallyDrop::into_inner(outer_params.opacityBrush);

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

            let res = self.rt.EndDraw(None, None);
            if res.is_ok() {
                let alpha_u8 = (window_master_alpha(
                    self.anim_phase,
                    self.open_elapsed,
                    self.close_progress,
                ) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                self.surface
                    .commit_with_alpha(self.hwnd, self.win_x, self.win_y, alpha_u8);
            }
            res
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
            radiusX: 8.5 * s,
            radiusY: 8.5 * s,
        };
        let fill = if self.light_glass {
            D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: if hovered { 0.08 } else { 0.05 },
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: if hovered { 0.14 } else { 0.09 },
            }
        };
        if let Ok(b) = self.rt.CreateSolidColorBrush(&fill, None) {
            self.rt.FillRoundedRectangle(&rr, &b);
        }

        if expanded {
            let bar_w = 3.0 * s;
            let bar_h = (16.0 * s).min(rect.height() - 10.0 * s);
            let bar_left = rect.left + 3.0 * s;
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
            if let Ok(ab) = self.rt.CreateSolidColorBrush(&d2d_color_rgb(accent, 0.96), None) {
                self.rt.FillRoundedRectangle(&bar_rr, &ab);
            }
        }
    }

    unsafe fn draw_seam_hairline(&self, anim_pri: RectF, sub_rect: RectF, fade: f32, s: f32) {
        let seam_x = if self.open_left {
            anim_pri.left
        } else {
            anim_pri.right
        };
        let seam_top = anim_pri.top.max(sub_rect.top) + 8.0 * s;
        let seam_bottom = anim_pri.bottom.min(sub_rect.bottom) - 8.0 * s;
        if seam_bottom <= seam_top + 4.0 * s {
            return;
        }
        let (sr, sg, sb, base_a) = if self.light_glass {
            (0.0, 0.0, 0.0, 0.075)
        } else {
            (1.0, 1.0, 1.0, 0.10)
        };
        let peak_a = base_a * fade;
        let stops = [
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
            &stops,
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
            if let Ok(brush) = self.rt.CreateLinearGradientBrush(&props, None, &sc) {
                self.rt.DrawLine(
                    D2D_POINT_2F {
                        x: seam_x,
                        y: seam_top,
                    },
                    D2D_POINT_2F {
                        x: seam_x,
                        y: seam_bottom,
                    },
                    &brush,
                    1.0 * s,
                    None,
                );
            }
        }
    }

    unsafe fn draw_submenu_contents(
        &self,
        sub_kind: SubmenuKind,
        layout: &SubLayout,
        cfg: &MonitorConfig,
        accent: Rgb,
        s: f32,
    ) {
        let Some(ref fmt) = self.title_fmt else {
            return;
        };
        let Some(ref badge_fmt) = self.badge_fmt else {
            return;
        };

        let primary_fg = primary_text_color(self.light_glass);
        let muted_fg = muted_text_color(self.light_glass);

        for row in &layout.rows {
            let r = row.rect;
            let hovered = match row.target {
                Some(SubHoverTarget::BrightnessRow(idx)) => {
                    let is_dragging = self.dragging_brightness.as_deref().is_some_and(|id| {
                        self.brightness_snap
                            .displays
                            .get(idx)
                            .is_some_and(|d| d.id == id)
                    });
                    is_dragging
                        || matches!(
                            self.hover,
                            Some(HoverTarget::Sub(
                                SubHoverTarget::BrightnessRow(i)
                                    | SubHoverTarget::BrightnessSlider(i)
                            )) if i == idx
                        )
                }
                Some(target) => self.hover == Some(HoverTarget::Sub(target)),
                None => false,
            };
            let selected = match row.target {
                Some(SubHoverTarget::WifiNetwork(idx)) => self
                    .wireless_snap
                    .wifi_networks
                    .get(idx)
                    .is_some_and(|w| w.is_connected),
                Some(SubHoverTarget::BtDevice(idx)) => self
                    .wireless_snap
                    .bt_devices
                    .get(idx)
                    .is_some_and(|d| d.is_connected),
                Some(SubHoverTarget::LeftClickPreset(action)) => cfg.left_click == action,
                Some(SubHoverTarget::DisplayMetric(item)) => cfg.is_visible(item),
                Some(SubHoverTarget::SettingLock) => cfg.locked,
                _ => false,
            };

            if row.target.is_some() {
                draw_row_highlight(&self.rt, r, hovered, selected, accent, s, self.light_glass);
            }

            match row.target {
                Some(SubHoverTarget::WifiToggle) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "WLAN",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 46.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                    draw_pill_switch(
                        &self.rt,
                        r.right - 10.0 * s,
                        (r.top + r.bottom) * 0.5,
                        self.wireless_snap.wifi_enabled,
                        accent,
                        s,
                        self.light_glass,
                    );
                }
                Some(SubHoverTarget::BtToggle) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "蓝牙开关",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 46.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                    draw_pill_switch(
                        &self.rt,
                        r.right - 10.0 * s,
                        (r.top + r.bottom) * 0.5,
                        self.wireless_snap.bt_enabled,
                        accent,
                        s,
                        self.light_glass,
                    );
                }
                Some(SubHoverTarget::WifiNetwork(idx)) => {
                    if let Some(net) = self.wireless_snap.wifi_networks.get(idx) {
                        let icy = (r.top + r.bottom) * 0.5;
                        self.draw_wifi_signal_icon(
                            r.left + 18.0 * s,
                            icy,
                            s,
                            net.signal_quality,
                            primary_fg,
                            muted_fg,
                        );
                        let badge = if net.is_connected {
                            "已连接"
                        } else if !net.has_profile {
                            if net.is_secured { "加密" } else { "开放" }
                        } else if net.signal_quality == 0 {
                            "已保存"
                        } else {
                            ""
                        };
                        let right_pad = if badge.is_empty() { 10.0 * s } else { 54.0 * s };
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            &net.ssid,
                            RectF::new(r.left + 32.0 * s, r.top, r.right - right_pad, r.bottom),
                            fmt,
                            primary_fg,
                        );
                        if !badge.is_empty() {
                            draw_text_in_rect(
                                &self.dwrite_factory,
                                &self.rt,
                                badge,
                                RectF::new(r.right - 56.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                                badge_fmt,
                                if net.is_connected { primary_fg } else { muted_fg },
                            );
                        }
                    }
                }
                Some(SubHoverTarget::BtDevice(idx)) => {
                    if let Some(dev) = self.wireless_snap.bt_devices.get(idx) {
                        let icy = (r.top + r.bottom) * 0.5;
                        if let Ok(br) = self.rt.CreateSolidColorBrush(&primary_fg, None) {
                            self.draw_bluetooth_icon(r.left + 17.0 * s, icy, s, &br);
                        }
                        let badge = if dev.is_connected { "已连接" } else { "已配对" };
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            &dev.name,
                            RectF::new(r.left + 30.0 * s, r.top, r.right - 56.0 * s, r.bottom),
                            fmt,
                            primary_fg,
                        );
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            badge,
                            RectF::new(r.right - 56.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                            badge_fmt,
                            if dev.is_connected { primary_fg } else { muted_fg },
                        );
                    }
                }
                Some(SubHoverTarget::WifiMoreSettings) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "更多网络设置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 12.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::BtMoreSettings) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "更多蓝牙设置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 12.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::BrightnessRow(idx)) => {
                    if let Some(disp) = self.brightness_snap.displays.get(idx) {
                        let geom = self.brightness_row_geom(r, s);
                        let is_software = disp.mode == BrightnessMode::Software;
                        let name_rect = if is_software {
                            RectF::new(
                                geom.label_rect.left,
                                geom.label_rect.top,
                                (geom.label_rect.right - 10.0 * s)
                                    .max(geom.label_rect.left + 20.0 * s),
                                geom.label_rect.bottom,
                            )
                        } else {
                            geom.label_rect
                        };
                        let measured_w = draw_text_in_rect_measured(
                            &self.dwrite_factory,
                            &self.rt,
                            &disp.name,
                            name_rect,
                            fmt,
                            primary_fg,
                        );
                        if is_software {
                            let dot_x = (name_rect.left + measured_w + 4.5 * s)
                                .min(geom.pct_rect.left - 5.0 * s);
                            let dot_y = name_rect.top + 5.0 * s;
                            let dot_r = 2.6 * s;
                            let dot_col = if self.light_glass {
                                D2D1_COLOR_F {
                                    r: 0.86,
                                    g: 0.22,
                                    b: 0.24,
                                    a: 0.96,
                                }
                            } else {
                                D2D1_COLOR_F {
                                    r: 1.0,
                                    g: 0.36,
                                    b: 0.36,
                                    a: 0.98,
                                }
                            };
                            if let Ok(db) = self.rt.CreateSolidColorBrush(&dot_col, None) {
                                self.rt.FillEllipse(
                                    &D2D1_ELLIPSE {
                                        point: D2D_POINT_2F { x: dot_x, y: dot_y },
                                        radiusX: dot_r,
                                        radiusY: dot_r,
                                    },
                                    &db,
                                );
                            }
                        }

                        let pct_str = format_brightness_pct(disp.brightness);
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            &pct_str,
                            geom.pct_rect,
                            badge_fmt,
                            muted_fg,
                        );

                        let slider_active =
                            self.dragging_brightness.as_deref() == Some(disp.id.as_str())
                                || self.hover
                                    == Some(HoverTarget::Sub(SubHoverTarget::BrightnessSlider(idx)));
                        draw_glass_slider(
                            &self.rt,
                            geom.track_rect,
                            f32::from(disp.brightness) / 100.0,
                            false,
                            slider_active,
                            self.light_glass,
                            accent,
                            s,
                        );
                    }
                }
                Some(SubHoverTarget::BrightnessSlider(_)) => {}
                Some(SubHoverTarget::BrightnessMoreSettings) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "更多屏幕设置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 12.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::LeftClickPreset(action)) => {
                    let text = if action == LeftClickAction::CustomApp
                        && !cfg.custom_app_path.is_empty()
                    {
                        let name = Path::new(&cfg.custom_app_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("已选程序");
                        format!("自定义 ({name})")
                    } else {
                        action.label().to_string()
                    };
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        &text,
                        RectF::new(r.left + 14.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::DisplayMetric(item)) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        item.label(),
                        RectF::new(r.left + 14.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::SettingLock) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "锁定位置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::SettingReset) => {
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "重置位置",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        primary_fg,
                    );
                }
                Some(SubHoverTarget::SettingClose) => {
                    let mut close_col = danger_text_color(self.light_glass);
                    if hovered {
                        close_col.a = 1.0;
                    }
                    draw_text_in_rect(
                        &self.dwrite_factory,
                        &self.rt,
                        "关闭",
                        RectF::new(r.left + 12.0 * s, r.top, r.right - 10.0 * s, r.bottom),
                        fmt,
                        close_col,
                    );
                }
                None => {
                    let msg = match sub_kind {
                        SubmenuKind::WiFi => {
                            if !self.wireless_snap.wifi_available {
                                "未检测到 WLAN 适配器"
                            } else if !self.wireless_snap.wifi_enabled {
                                "WLAN 已关闭"
                            } else if self.wireless_snap.wifi_busy {
                                "正在扫描..."
                            } else {
                                "未发现可用网络"
                            }
                        }
                        SubmenuKind::Bluetooth => {
                            if !self.wireless_snap.bt_available {
                                "未检测到蓝牙适配器"
                            } else if !self.wireless_snap.bt_enabled {
                                "蓝牙已关闭"
                            } else if self.wireless_snap.bt_busy {
                                "正在刷新..."
                            } else {
                                "无已配对设备"
                            }
                        }
                        SubmenuKind::Brightness => {
                            if self.brightness_snap.busy {
                                "正在检测显示器..."
                            } else {
                                "未检测到显示器"
                            }
                        }
                        _ => "",
                    };
                    if !msg.is_empty() {
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            msg,
                            RectF::new(r.left + 12.0 * s, r.top, r.right - 12.0 * s, r.bottom),
                            fmt,
                            muted_fg,
                        );
                    }
                }
            }
        }

        for div in &layout.dividers {
            draw_divider(&self.rt, *div, s, self.light_glass);
        }

        if let Some(vp) = layout.list_viewport {
            let max_visible = layout.list_max_rows.max(1);
            if layout.list_total_items > max_visible {
                let top = vp.top + 4.0 * s;
                let bottom = vp.bottom - 4.0 * s;
                let track_h = (bottom - top).max(8.0);
                let ratio = (max_visible as f32) / (layout.list_total_items as f32);
                let thumb_h = (track_h * ratio).max(16.0 * s);
                let max_scroll = (layout.list_total_items - max_visible) as f32;
                let scroll_t = if max_scroll > 0.0 {
                    (layout.list_scroll as f32) / max_scroll
                } else {
                    0.0
                };
                let thumb_top = top + (track_h - thumb_h) * scroll_t;
                let bar_rr = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: vp.right - 3.0 * s,
                        top: thumb_top,
                        right: vp.right - 0.5 * s,
                        bottom: thumb_top + thumb_h,
                    },
                    radiusX: 1.5 * s,
                    radiusY: 1.5 * s,
                };
                let mut col = muted_fg;
                col.a *= 0.65;
                if let Ok(b) = self.rt.CreateSolidColorBrush(&col, None) {
                    self.rt.FillRoundedRectangle(&bar_rr, &b);
                }
            }
        }
    }

}
