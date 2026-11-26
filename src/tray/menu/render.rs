use super::TrayMenuInner;
use super::super::layout::{animated_tray_card_rect, TrayMenuHover, TrayMenuItem, TraySubmenuKind};
use crate::color::Rgb;
use crate::deskbox::DeskBoxTrayAction;
use crate::ui::glass::{create_dc_render_target, d2d_rect, danger_text_color, draw_divider, draw_geometry_rim, draw_glass_interior, draw_row_highlight, draw_text_in_rect, identity_matrix, primary_text_color};
use crate::ui::{ease_out_cubic, ease_retract, popup_glass_recipe, rgb_luma, row_stagger_dy, window_master_alpha, GlassSurfaceStyle, PopupAnimPhase, RectF, PANEL_RADIUS_DIP};
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D_POINT_2F};
use windows::Win32::Graphics::Direct2D::{ID2D1Geometry, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_COMBINE_MODE_UNION, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT};

impl TrayMenuInner {
    fn build_fused_cards_geometry(
        &self,
        primary: RectF,
        sub_opt: Option<RectF>,
        eff_r: f32,
    ) -> windows::core::Result<ID2D1Geometry> {
        unsafe {
            let rr_pri = D2D1_ROUNDED_RECT {
                rect: d2d_rect(primary),
                radiusX: eff_r,
                radiusY: eff_r,
            };
            let g_pri: ID2D1Geometry = self
                .d2d_factory
                .CreateRoundedRectangleGeometry(&rr_pri)?
                .cast()?;

            let Some(sub) = sub_opt else {
                return Ok(g_pri);
            };

            let sub_r = eff_r.min((sub.width() * 0.45).max(3.0));
            let rr_sub = D2D1_ROUNDED_RECT {
                rect: d2d_rect(sub),
                radiusX: sub_r,
                radiusY: sub_r,
            };
            let g_sub: ID2D1Geometry = self
                .d2d_factory
                .CreateRoundedRectangleGeometry(&rr_sub)?
                .cast()?;

            let combined = self.d2d_factory.CreatePathGeometry()?;
            let sink = combined.Open()?;
            g_pri.CombineWithGeometry(&g_sub, D2D1_COMBINE_MODE_UNION, None, 0.25, &sink)?;
            sink.Close()?;
            let combined_geo: ID2D1Geometry = combined.cast()?;

            let seam_x = if self.sub_open_left {
                primary.left
            } else {
                primary.right
            };
            let overlap_top = primary.top.max(sub.top);
            let overlap_bot = primary.bottom.min(sub.bottom);
            if overlap_bot <= overlap_top + 1.0 {
                return Ok(combined_geo);
            }

            let left_lim = primary.left.min(sub.left);
            let right_lim = primary.right.max(sub.right);
            let bridge_half = eff_r.max(sub_r) + 1.0;
            let bridge_rect = RectF::new(
                (seam_x - bridge_half).max(left_lim),
                overlap_top,
                (seam_x + bridge_half).min(right_lim),
                overlap_bot,
            );
            let g_bridge: ID2D1Geometry = self
                .d2d_factory
                .CreateRectangleGeometry(&d2d_rect(bridge_rect))?
                .cast()?;
            let fused = self.d2d_factory.CreatePathGeometry()?;
            let sink2 = fused.Open()?;
            combined_geo.CombineWithGeometry(
                &g_bridge,
                D2D1_COMBINE_MODE_UNION,
                None,
                0.25,
                &sink2,
            )?;
            sink2.Close()?;
            fused.cast()
        }
    }

    unsafe fn draw_fused_shadows(
        &self,
        anim_primary: RectF,
        anim_sub_opt: Option<RectF>,
        eff_r: f32,
        s: f32,
    ) {
        let max_w = self.win_w as f32;
        let max_h = self.win_h as f32;
        let shadow_layers = if self.light_glass {
            [(7.5 * s, 0.022), (4.5 * s, 0.036), (2.0 * s, 0.058)]
        } else {
            [(7.5 * s, 0.035), (4.5 * s, 0.055), (2.0 * s, 0.09)]
        };

        for (expand, alpha) in shadow_layers {
            let side_exp = expand * 0.55;
            let top_exp = expand * 0.45;
            let bot_exp = expand * 0.65;

            let sh_pri = RectF::new(
                if self.sub_open_left && anim_sub_opt.is_some() {
                    anim_primary.left
                } else {
                    (anim_primary.left - side_exp).max(0.0)
                },
                (anim_primary.top - top_exp).max(0.0),
                if !self.sub_open_left && anim_sub_opt.is_some() {
                    anim_primary.right
                } else {
                    (anim_primary.right + side_exp).min(max_w)
                },
                (anim_primary.bottom + bot_exp).min(max_h),
            );

            let sh_sub = anim_sub_opt.map(|sr| {
                RectF::new(
                    if !self.sub_open_left {
                        sr.left
                    } else {
                        (sr.left - side_exp).max(0.0)
                    },
                    (sr.top - top_exp).max(0.0),
                    if self.sub_open_left {
                        sr.right
                    } else {
                        (sr.right + side_exp).min(max_w)
                    },
                    (sr.bottom + bot_exp).min(max_h),
                )
            });

            if let (Ok(geo), Ok(sh_brush)) = (
                self.build_fused_cards_geometry(sh_pri, sh_sub, eff_r + side_exp * 0.45),
                self.rt.CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: alpha,
                    },
                    None,
                ),
            ) {
                self.rt.FillGeometry(&geo, &sh_brush, None);
            }
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
        let layout = self.current_layout();
        let s = layout.scale;
        self.win_w = layout.win_w;
        self.win_h = layout.win_h;
        self.surface.ensure_size(self.win_w, self.win_h)?;
        let title_fmt = self.ensure_title_format(s)?;

        let recipe = popup_glass_recipe(
            GlassSurfaceStyle::from(self.taskbar_style),
            self.light_glass,
            rgb_luma(self.backdrop_rgb),
        );
        self.update_silk(layout.primary_rect, s, recipe.silk_alpha > 0.002);

        let accent = Rgb::new(
            (self.ui_accent.0 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.1 * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.ui_accent.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        );

        let anim_primary = animated_tray_card_rect(
            layout.primary_rect,
            self.win_h as f32,
            self.anim_phase,
            self.open_spring.pos,
            self.close_progress,
            self.open_down,
            s,
        );
        let anim_sub_opt = self.current_animated_sub_rect(&layout, anim_primary, s);
        let eff_r = (PANEL_RADIUS_DIP * s).min((anim_primary.height() * 0.45).max(4.0 * s));

        let union_bounds = match anim_sub_opt {
            Some(sr) => RectF::new(
                anim_primary.left.min(sr.left),
                anim_primary.top.min(sr.top),
                anim_primary.right.max(sr.right),
                anim_primary.bottom.max(sr.bottom),
            ),
            None => anim_primary,
        };

        let (content_alpha, close_dy) = match self.anim_phase {
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

            // 1. 3-pass ambient shadows behind fused card silhouette
            self.draw_fused_shadows(anim_primary, anim_sub_opt, eff_r, s);

            // 2. Clipped glass card interior & menu rows
            if let Ok(clip_geo) = self.build_fused_cards_geometry(anim_primary, anim_sub_opt, eff_r) {
                let clip_geo_for_rim = clip_geo.clone();
                let outer_params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(union_bounds),
                    geometricMask: ManuallyDrop::new(Some(clip_geo)),
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
                if content_alpha > 0.005 {
                    let use_layer = self.anim_phase != PopupAnimPhase::Open;
                    if use_layer {
                        let content_params = D2D1_LAYER_PARAMETERS {
                            contentBounds: d2d_rect(anim_primary),
                            geometricMask: ManuallyDrop::new(None),
                            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                            maskTransform: identity_matrix(),
                            opacity: content_alpha,
                            opacityBrush: ManuallyDrop::new(None),
                            layerOptions: D2D1_LAYER_OPTIONS_NONE,
                        };
                        self.rt.PushLayer(&content_params, &self.content_layer);
                    }

                    let mut div_anchor_dy = 0.0f32;
                    for (idx, (item, mut rect)) in layout.primary_rows().into_iter().enumerate() {
                        let dy = match self.anim_phase {
                            PopupAnimPhase::Opening => {
                                if self.open_down {
                                    -row_stagger_dy(
                                        5usize.saturating_sub(idx),
                                        6,
                                        self.open_elapsed,
                                        self.open_spring.pos,
                                        s,
                                    )
                                } else {
                                    row_stagger_dy(
                                        idx,
                                        6,
                                        self.open_elapsed,
                                        self.open_spring.pos,
                                        s,
                                    )
                                }
                            }
                            PopupAnimPhase::Open => 0.0,
                            PopupAnimPhase::Closing => {
                                if self.open_down {
                                    -close_dy
                                } else {
                                    close_dy
                                }
                            }
                        };
                        if item == TrayMenuItem::TaskbarStyle {
                            div_anchor_dy = dy;
                        }
                        rect.top += dy;
                        rect.bottom += dy;

                        let hovered = self.hover == Some(TrayMenuHover::Primary(item));
                        let selected = match item {
                            TrayMenuItem::MediaBar => self.mediabar_visible,
                            TrayMenuItem::MonitorBar => self.monitorbar_visible,
                            TrayMenuItem::DeskBox => self.deskbox_state.enabled,
                            TrayMenuItem::TaskbarStyle => {
                                self.active_submenu == Some(TraySubmenuKind::TaskbarStyle)
                            }
                            TrayMenuItem::Autostart => self.autostart,
                            TrayMenuItem::Exit => false,
                        };
                        let has_sub = matches!(item, TrayMenuItem::DeskBox | TrayMenuItem::TaskbarStyle);
                        let sub_open_here = match item {
                            TrayMenuItem::DeskBox => {
                                self.active_submenu == Some(TraySubmenuKind::DeskBox)
                            }
                            TrayMenuItem::TaskbarStyle => {
                                self.active_submenu == Some(TraySubmenuKind::TaskbarStyle)
                            }
                            _ => false,
                        };
                        draw_row_highlight(
                            &self.rt,
                            rect,
                            hovered || (sub_open_here && item == TrayMenuItem::DeskBox && !selected),
                            selected && item != TrayMenuItem::TaskbarStyle,
                            accent,
                            s,
                            self.light_glass,
                        );
                        if item == TrayMenuItem::TaskbarStyle && sub_open_here && !hovered {
                            draw_row_highlight(
                                &self.rt,
                                rect,
                                true,
                                false,
                                accent,
                                s,
                                self.light_glass,
                            );
                        }

                        let mut fg = match item {
                            TrayMenuItem::Exit => danger_text_color(self.light_glass),
                            TrayMenuItem::MediaBar
                            | TrayMenuItem::MonitorBar
                            | TrayMenuItem::DeskBox
                            | TrayMenuItem::TaskbarStyle
                            | TrayMenuItem::Autostart => {
                                let mut c = primary_text_color(self.light_glass);
                                if item != TrayMenuItem::TaskbarStyle && !selected {
                                    c.a *= 0.90;
                                }
                                c
                            }
                        };
                        if hovered && item == TrayMenuItem::Exit {
                            fg.a = 1.0;
                        }

                        let icon_cx = rect.left + 19.0 * s;
                        let icon_cy = (rect.top + rect.bottom) * 0.5;
                        if let Ok(icon_brush) = self.rt.CreateSolidColorBrush(&fg, None) {
                            self.draw_primary_icon(item, icon_cx, icon_cy, s, &icon_brush);
                        }

                        let label = match item {
                            TrayMenuItem::MediaBar => "媒体控制",
                            TrayMenuItem::MonitorBar => "系统监控",
                            TrayMenuItem::DeskBox => "桌面收纳",
                            TrayMenuItem::TaskbarStyle => "美化风格",
                            TrayMenuItem::Autostart => "开机自启",
                            TrayMenuItem::Exit => "退出",
                        };
                        let right_pad = if has_sub { 22.0 * s } else { 10.0 * s };
                        let label_rect = RectF::new(
                            rect.left + 34.0 * s,
                            rect.top,
                            rect.right - right_pad,
                            rect.bottom,
                        );
                        draw_text_in_rect(
                            &self.dwrite_factory,
                            &self.rt,
                            label,
                            label_rect,
                            &title_fmt,
                            fg,
                        );

                        if has_sub {
                            let mut chev_col = fg;
                            chev_col.a *= if sub_open_here || hovered { 0.95 } else { 0.65 };
                            if let Ok(cb) = self.rt.CreateSolidColorBrush(&chev_col, None) {
                                let mid_y = (rect.top + rect.bottom) * 0.5;
                                let tip_x = rect.right - 9.5 * s;
                                let back_x = tip_x - 3.8 * s;
                                let half_h = 3.6 * s;
                                let stroke = self.round_stroke.as_ref();
                                self.rt.DrawLine(
                                    D2D_POINT_2F {
                                        x: back_x,
                                        y: mid_y - half_h,
                                    },
                                    D2D_POINT_2F {
                                        x: tip_x,
                                        y: mid_y,
                                    },
                                    &cb,
                                    1.3 * s,
                                    stroke,
                                );
                                self.rt.DrawLine(
                                    D2D_POINT_2F {
                                        x: tip_x,
                                        y: mid_y,
                                    },
                                    D2D_POINT_2F {
                                        x: back_x,
                                        y: mid_y + half_h,
                                    },
                                    &cb,
                                    1.3 * s,
                                    stroke,
                                );
                            }
                        }
                    }

                    let mut div_upper = layout.taskbar_style_rect;
                    div_upper.top += div_anchor_dy;
                    div_upper.bottom += div_anchor_dy;
                    draw_divider(&self.rt, div_upper, s, self.light_glass);

                    if use_layer {
                        self.rt.PopLayer();
                    }
                }

                // 2b. Vertical feathered seam divider + submenu rows
                if let Some(anim_sub) = anim_sub_opt {
                    let seam_x = if self.sub_open_left {
                        anim_primary.left
                    } else {
                        anim_primary.right
                    };
                    let seam_top = anim_primary.top.max(anim_sub.top) + 8.0 * s;
                    let seam_bottom = anim_primary.bottom.min(anim_sub.bottom) - 8.0 * s;
                    let sub_alpha_in =
                        ease_out_cubic(((self.sub_fade_elapsed - 0.006) / 0.078).clamp(0.0, 1.0));
                    let sub_alpha = if self.anim_phase == PopupAnimPhase::Closing {
                        sub_alpha_in * content_alpha
                    } else {
                        sub_alpha_in
                    };
                    if seam_bottom > seam_top + 4.0 * s {
                        let (sr, sg, sb, base_a) = if self.light_glass {
                            (0.0, 0.0, 0.0, 0.075)
                        } else {
                            (1.0, 1.0, 1.0, 0.10)
                        };
                        let peak_a = base_a * sub_alpha.max(0.15);
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
                            if let Ok(sb_brush) =
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
                                    &sb_brush,
                                    1.0 * s,
                                    None,
                                );
                            }
                        }
                    }

                    if sub_alpha > 0.005 {
                        let slide_t = ease_out_cubic((self.sub_fade_elapsed / 0.105).clamp(0.0, 1.0));
                        let slide_dx = if self.sub_open_left {
                            10.0 * s * (1.0 - slide_t)
                        } else {
                            -10.0 * s * (1.0 - slide_t)
                        };
                        let sub_layer_params = D2D1_LAYER_PARAMETERS {
                            contentBounds: d2d_rect(anim_sub),
                            geometricMask: ManuallyDrop::new(None),
                            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                            maskTransform: identity_matrix(),
                            opacity: sub_alpha,
                            opacityBrush: ManuallyDrop::new(None),
                            layerOptions: D2D1_LAYER_OPTIONS_NONE,
                        };
                        self.rt.PushLayer(&sub_layer_params, &self.content_layer);

                        let sub_dy = if self.anim_phase == PopupAnimPhase::Closing {
                            if self.open_down {
                                -close_dy
                            } else {
                                close_dy
                            }
                        } else {
                            0.0
                        };

                        match self.active_submenu {
                            Some(TraySubmenuKind::DeskBox) => {
                                let db_rows = layout.sub_deskbox_rows();
                                for (idx, (action, mut rect)) in db_rows.into_iter().enumerate() {
                                    rect.left += slide_dx;
                                    rect.right += slide_dx;
                                    rect.top += sub_dy;
                                    rect.bottom += sub_dy;

                                    let hovered =
                                        self.hover == Some(TrayMenuHover::SubDeskBox(action));
                                    let selected = match action {
                                        DeskBoxTrayAction::ToggleEnabled => {
                                            self.deskbox_state.enabled
                                        }
                                        DeskBoxTrayAction::ToggleHideDesktopIcons => {
                                            self.deskbox_state.hide_desktop_icons
                                        }
                                        DeskBoxTrayAction::AutoFitAll
                                        | DeskBoxTrayAction::LockAll
                                        | DeskBoxTrayAction::UnlockAll => false,
                                        DeskBoxTrayAction::SetDefaultView(mode) => {
                                            self.deskbox_state.default_view_mode == mode
                                        }
                                    };
                                    draw_row_highlight(
                                        &self.rt,
                                        rect,
                                        hovered,
                                        selected,
                                        accent,
                                        s,
                                        self.light_glass,
                                    );

                                    let mut fg = primary_text_color(self.light_glass);
                                    if !selected && !hovered {
                                        fg.a *= 0.90;
                                    }
                                    let icon_cx = rect.left + 19.0 * s;
                                    let icon_cy = (rect.top + rect.bottom) * 0.5;
                                    if let Ok(icon_brush) = self.rt.CreateSolidColorBrush(&fg, None)
                                    {
                                        self.draw_deskbox_action_icon(
                                            action,
                                            icon_cx,
                                            icon_cy,
                                            s,
                                            &icon_brush,
                                        );
                                    }

                                    let label = match action {
                                        DeskBoxTrayAction::ToggleEnabled => "启用桌面收纳",
                                        DeskBoxTrayAction::ToggleHideDesktopIcons => {
                                            "隐藏系统桌面图标"
                                        }
                                        DeskBoxTrayAction::AutoFitAll => "自适应尺寸",
                                        DeskBoxTrayAction::LockAll => "锁定全部",
                                        DeskBoxTrayAction::UnlockAll => "解锁全部",
                                        DeskBoxTrayAction::SetDefaultView(mode) => mode.label(),
                                    };
                                    let label_rect = RectF::new(
                                        rect.left + 34.0 * s,
                                        rect.top,
                                        rect.right - 10.0 * s,
                                        rect.bottom,
                                    );
                                    draw_text_in_rect(
                                        &self.dwrite_factory,
                                        &self.rt,
                                        label,
                                        label_rect,
                                        &title_fmt,
                                        fg,
                                    );
                                    if idx == 1 || idx == 4 {
                                        draw_divider(&self.rt, rect, s, self.light_glass);
                                    }
                                }
                            }
                            Some(TraySubmenuKind::TaskbarStyle) => {
                                for (style, mut rect) in layout.sub_rows() {
                                    rect.left += slide_dx;
                                    rect.right += slide_dx;
                                    rect.top += sub_dy;
                                    rect.bottom += sub_dy;

                                    let hovered =
                                        self.hover == Some(TrayMenuHover::SubTaskbarStyle(style));
                                    let selected = self.taskbar_style == style;
                                    draw_row_highlight(
                                        &self.rt,
                                        rect,
                                        hovered,
                                        selected,
                                        accent,
                                        s,
                                        self.light_glass,
                                    );

                                    let mut fg = primary_text_color(self.light_glass);
                                    if !selected && !hovered {
                                        fg.a *= 0.90;
                                    }
                                    let icon_cx = rect.left + 19.0 * s;
                                    let icon_cy = (rect.top + rect.bottom) * 0.5;
                                    if let Ok(icon_brush) = self.rt.CreateSolidColorBrush(&fg, None)
                                    {
                                        self.draw_style_icon(
                                            style,
                                            icon_cx,
                                            icon_cy,
                                            s,
                                            &icon_brush,
                                        );
                                    }

                                    let label_rect = RectF::new(
                                        rect.left + 34.0 * s,
                                        rect.top,
                                        rect.right - 10.0 * s,
                                        rect.bottom,
                                    );
                                    draw_text_in_rect(
                                        &self.dwrite_factory,
                                        &self.rt,
                                        style.label(),
                                        label_rect,
                                        &title_fmt,
                                        fg,
                                    );
                                }
                            }
                            None => {}
                        }

                        self.rt.PopLayer();
                    }
                }

                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(outer_params.geometricMask);

                // 3. Restrained specular glass rim highlight around unified contour
                draw_geometry_rim(
                    &self.rt,
                    &clip_geo_for_rim,
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

}
