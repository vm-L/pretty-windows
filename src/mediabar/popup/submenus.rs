use super::{HoverTarget, PopupInner, SliderTarget, VolumeRowGeom};
use super::super::arbitration::{PlaybackState, SelectionMode};
use super::super::audio_devices::AudioEndpointItem;
use super::super::layout::RectF;
use super::super::render::{draw_speaker_glyph, draw_volume_slider};
use super::super::smtc::SourceItem;
use super::super::volume::format_volume_pct;
use crate::color::{enforce_ui_accent_deadzone, Rgb};
use crate::ui::glass::{d2d_color_rgb, d2d_rect, danger_text_color, draw_divider, draw_row_highlight, draw_text_in_rect, identity_matrix, muted_text_color, primary_text_color};
use crate::ui::{DIVIDER_H_DIP, INNER_PAD_DIP};
use std::mem::ManuallyDrop;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_POINT_2F};
use windows::Win32::Graphics::Direct2D::{ID2D1Geometry, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_ELLIPSE, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_ROUNDED_RECT};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

impl PopupInner {
    pub(super) unsafe fn render_display_submenu(
        &self,
        card: RectF,
        auto_accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        badge_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let (auto_rect, source_rects) = self.display_row_rects(card);
        let is_auto = matches!(self.mode, SelectionMode::Auto);
        draw_row_highlight(
            &self.rt,
            auto_rect,
            self.hover_target == HoverTarget::SubAuto,
            is_auto,
            auto_accent,
            s,
            self.light_glass,
        );
        let mut auto_col = primary_text_color(self.light_glass);
        if !is_auto {
            auto_col.a *= 0.85;
        }
        let auto_label_r = RectF::new(
            auto_rect.left + 12.0 * s,
            auto_rect.top,
            auto_rect.right - 12.0 * s,
            auto_rect.bottom,
        );
        draw_text_in_rect(&self.dwrite_factory, &self.rt, "自动", auto_label_r, title_fmt, auto_col);

        draw_divider(&self.rt, auto_rect, s, self.light_glass);

        if self.sources.is_empty() {
            let empty_y = auto_rect.bottom + DIVIDER_H_DIP * s;
            let empty_rect = RectF::new(
                auto_rect.left + 4.0 * s,
                empty_y,
                auto_rect.right - 4.0 * s,
                empty_y + 34.0 * s,
            );
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                "暂无其他活跃媒体会话",
                empty_rect,
                sub_fmt,
                muted_text_color(self.light_glass),
            );
            return;
        }

        for (idx, (src, row_rect)) in self.sources.iter().zip(source_rects.iter()).enumerate() {
            let hovered = self.hover_target == HoverTarget::SubSource(idx);
            let selected = matches!(self.mode, SelectionMode::Manual(ref id) if id == &src.id);
            let row_accent = if self.auto_color {
                src.cover
                    .as_ref()
                    .map(|c| {
                        let raw = if self.light_glass {
                            c.accent_light
                        } else {
                            c.accent_dark
                        };
                        enforce_ui_accent_deadzone(raw, self.light_glass)
                    })
                    .unwrap_or(auto_accent)
            } else {
                auto_accent
            };
            draw_row_highlight(
                &self.rt,
                *row_rect,
                hovered,
                selected,
                row_accent,
                s,
                self.light_glass,
            );
            self.draw_source_row(
                *row_rect,
                src,
                selected,
                auto_accent,
                title_fmt,
                sub_fmt,
                badge_fmt,
                s,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn render_volume_submenu(
        &self,
        card: RectF,
        accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        pct_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let (master_geom, app_geoms) = self.volume_row_rects(card);

        let master_hovered = matches!(
            self.hover_target,
            HoverTarget::SubVolMasterMute
                | HoverTarget::SubVolMasterSlider
                | HoverTarget::SubVolMasterRow
        );
        draw_row_highlight(
            &self.rt,
            master_geom.row_rect,
            master_hovered,
            false,
            accent,
            s,
            self.light_glass,
        );
        self.draw_volume_row_content(
            master_geom,
            "总音量",
            self.master_vol,
            self.master_muted,
            true,
            self.hover_target == HoverTarget::SubVolMasterMute,
            self.dragging_slider == Some(SliderTarget::Master)
                || self.hover_target == HoverTarget::SubVolMasterSlider,
            accent,
            title_fmt,
            pct_fmt,
            s,
        );

        draw_divider(&self.rt, master_geom.row_rect, s, self.light_glass);

        if self.app_volumes.is_empty() {
            let empty_y = master_geom.row_rect.bottom + DIVIDER_H_DIP * s;
            let empty_rect = RectF::new(
                master_geom.row_rect.left + 6.0 * s,
                empty_y,
                master_geom.row_rect.right - 6.0 * s,
                empty_y + 34.0 * s,
            );
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                "暂无正在使用音频的应用",
                empty_rect,
                sub_fmt,
                muted_text_color(self.light_glass),
            );
            return;
        }

        for (idx, (app, geom)) in self.app_volumes.iter().zip(app_geoms.iter()).enumerate() {
            let hovered = matches!(
                self.hover_target,
                HoverTarget::SubVolAppMute(i)
                    | HoverTarget::SubVolAppSlider(i)
                    | HoverTarget::SubVolAppRow(i)
                    if i == idx
            );
            draw_row_highlight(
                &self.rt,
                geom.row_rect,
                hovered,
                false,
                accent,
                s,
                self.light_glass,
            );
            self.draw_volume_row_content(
                *geom,
                &app.name,
                app.volume,
                app.muted,
                app.is_active,
                self.hover_target == HoverTarget::SubVolAppMute(idx),
                self.dragging_slider == Some(SliderTarget::App(idx))
                    || self.hover_target == HoverTarget::SubVolAppSlider(idx),
                accent,
                title_fmt,
                pct_fmt,
                s,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_volume_row_content(
        &self,
        geom: VolumeRowGeom,
        label: &str,
        volume: f32,
        muted: bool,
        is_active: bool,
        mute_hovered: bool,
        slider_active: bool,
        accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        pct_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        if mute_hovered {
            let bg = if self.light_glass {
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
                    a: 0.13,
                }
            };
            if let Ok(bb) = self.rt.CreateSolidColorBrush(&bg, None) {
                let rr = D2D1_ROUNDED_RECT {
                    rect: d2d_rect(geom.mute_rect),
                    radiusX: 6.0 * s,
                    radiusY: 6.0 * s,
                };
                self.rt.FillRoundedRectangle(&rr, &bb);
            }
        }

        let mut label_col = primary_text_color(self.light_glass);
        if !is_active {
            label_col.a *= 0.78;
        }
        let muted_icon_col = D2D1_COLOR_F {
            r: label_col.r,
            g: label_col.g,
            b: label_col.b,
            a: label_col.a * 0.52,
        };
        let slash_col = if self.light_glass {
            D2D1_COLOR_F {
                r: 0.80,
                g: 0.18,
                b: 0.20,
                a: 0.92,
            }
        } else {
            D2D1_COLOR_F {
                r: 1.0,
                g: 0.46,
                b: 0.46,
                a: 0.95,
            }
        };

        if let (Ok(fg_b), Ok(mu_b), Ok(sl_b)) = (
            self.rt.CreateSolidColorBrush(&label_col, None),
            self.rt.CreateSolidColorBrush(&muted_icon_col, None),
            self.rt.CreateSolidColorBrush(&slash_col, None),
        ) {
            let cx = (geom.mute_rect.left + geom.mute_rect.right) * 0.5;
            let cy = (geom.mute_rect.top + geom.mute_rect.bottom) * 0.5;
            let u = geom.mute_rect.width() / 22.0;
            draw_speaker_glyph(
                &self.d2d_factory,
                &self.rt,
                cx,
                cy,
                u,
                volume,
                muted,
                &fg_b,
                &mu_b,
                &sl_b,
                self.round_stroke.as_ref(),
            );
        }

        draw_text_in_rect(
            &self.dwrite_factory,
            &self.rt,
            label,
            geom.label_rect,
            title_fmt,
            label_col,
        );

        let mut pct_col = muted_text_color(self.light_glass);
        if muted {
            pct_col.a *= 0.68;
        }
        let pct_str = format_volume_pct(volume);
        draw_text_in_rect(
            &self.dwrite_factory,
            &self.rt,
            &pct_str,
            geom.pct_rect,
            pct_fmt,
            pct_col,
        );

        draw_volume_slider(
            &self.rt,
            geom.track_rect,
            volume,
            muted,
            slider_active,
            self.light_glass,
            accent,
            s,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn render_devices_submenu(
        &self,
        card: RectF,
        devices: &[AudioEndpointItem],
        is_playback: bool,
        accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let pad = INNER_PAD_DIP * s;
        if devices.is_empty() {
            let empty_r = RectF::new(
                card.left + pad + 4.0 * s,
                card.top + pad,
                card.right - pad - 4.0 * s,
                card.top + pad + 34.0 * s,
            );
            let msg = if is_playback {
                "未检测到可用播放设备"
            } else {
                "未检测到可用录音设备"
            };
            draw_text_in_rect(
                &self.dwrite_factory,
                &self.rt,
                msg,
                empty_r,
                sub_fmt,
                muted_text_color(self.light_glass),
            );
            return;
        }

        let rects = self.device_row_rects(card, devices.len());
        for (idx, (dev, row_rect)) in devices.iter().zip(rects.iter()).enumerate() {
            let hovered = if is_playback {
                self.hover_target == HoverTarget::SubPlayback(idx)
            } else {
                self.hover_target == HoverTarget::SubRecording(idx)
            };
            draw_row_highlight(
                &self.rt,
                *row_rect,
                hovered,
                dev.is_default,
                accent,
                s,
                self.light_glass,
            );

            let text_left = row_rect.left + 12.0 * s;
            let text_right = row_rect.right - 10.0 * s;
            let mid_y = (row_rect.top + row_rect.bottom) * 0.5;
            if dev.sub_name.is_empty() {
                let single_r = RectF::new(text_left, row_rect.top, text_right, row_rect.bottom);
                draw_text_in_rect(
                    &self.dwrite_factory,
                    &self.rt,
                    &dev.main_name,
                    single_r,
                    title_fmt,
                    primary_text_color(self.light_glass),
                );
            } else {
                let top_r = RectF::new(text_left, mid_y - 16.0 * s, text_right, mid_y + 1.0 * s);
                let bot_r = RectF::new(text_left, mid_y + 0.5 * s, text_right, mid_y + 15.5 * s);
                draw_text_in_rect(
                    &self.dwrite_factory,
                    &self.rt,
                    &dev.main_name,
                    top_r,
                    title_fmt,
                    primary_text_color(self.light_glass),
                );
                draw_text_in_rect(
                    &self.dwrite_factory,
                    &self.rt,
                    &dev.sub_name,
                    bot_r,
                    sub_fmt,
                    muted_text_color(self.light_glass),
                );
            }
        }
    }

    pub(super) unsafe fn render_settings_submenu(
        &self,
        card: RectF,
        accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let [r_auto_color, r_lock, r_reset, r_close] = self.settings_row_rects(card);

        draw_row_highlight(
            &self.rt,
            r_auto_color,
            self.hover_target == HoverTarget::SubSettingAutoColor,
            self.auto_color,
            accent,
            s,
            self.light_glass,
        );
        self.draw_setting_label(
            r_auto_color,
            "自动取色",
            primary_text_color(self.light_glass),
            title_fmt,
            s,
        );

        draw_row_highlight(
            &self.rt,
            r_lock,
            self.hover_target == HoverTarget::SubSettingLock,
            self.locked,
            accent,
            s,
            self.light_glass,
        );
        self.draw_setting_label(
            r_lock,
            "锁定位置",
            primary_text_color(self.light_glass),
            title_fmt,
            s,
        );

        draw_row_highlight(
            &self.rt,
            r_reset,
            self.hover_target == HoverTarget::SubSettingResetPos,
            false,
            accent,
            s,
            self.light_glass,
        );
        self.draw_setting_label(
            r_reset,
            "重置位置",
            primary_text_color(self.light_glass),
            title_fmt,
            s,
        );

        draw_divider(&self.rt, r_reset, s, self.light_glass);

        draw_row_highlight(
            &self.rt,
            r_close,
            self.hover_target == HoverTarget::SubSettingClose,
            false,
            accent,
            s,
            self.light_glass,
        );
        self.draw_setting_label(
            r_close,
            "关闭",
            danger_text_color(self.light_glass),
            title_fmt,
            s,
        );
    }

    unsafe fn draw_setting_label(
        &self,
        rect: RectF,
        label: &str,
        color: D2D1_COLOR_F,
        fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let label_r = RectF::new(
            rect.left + 12.0 * s,
            rect.top,
            rect.right - 10.0 * s,
            rect.bottom,
        );
        draw_text_in_rect(&self.dwrite_factory, &self.rt, label, label_r, fmt, color);
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_source_row(
        &self,
        rect: RectF,
        src: &SourceItem,
        selected: bool,
        fallback_accent: Rgb,
        title_fmt: &IDWriteTextFormat,
        sub_fmt: &IDWriteTextFormat,
        badge_fmt: &IDWriteTextFormat,
        s: f32,
    ) {
        let thumb_size = 30.0 * s;
        let thumb_left = rect.left + 10.0 * s;
        let thumb_top = (rect.top + rect.bottom - thumb_size) * 0.5;
        let thumb_rect = RectF::new(
            thumb_left,
            thumb_top,
            thumb_left + thumb_size,
            thumb_top + thumb_size,
        );
        let thumb_rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(thumb_rect),
            radiusX: 6.0 * s,
            radiusY: 6.0 * s,
        };

        let bmp = src
            .cover
            .as_ref()
            .and_then(|c| self.cover_bmps.get(&c.hash));
        if let Some(cover_bmp) = bmp {
            if let Ok(geo) = self.d2d_factory.CreateRoundedRectangleGeometry(&thumb_rr) {
                let geom: ID2D1Geometry = geo.into();
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: d2d_rect(thumb_rect),
                    geometricMask: ManuallyDrop::new(Some(geom)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: identity_matrix(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.rt.PushLayer(&params, &self.thumb_layer);
                self.rt.DrawBitmap(
                    cover_bmp,
                    Some(&d2d_rect(thumb_rect)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
                self.rt.PopLayer();
                let _ = ManuallyDrop::into_inner(params.geometricMask);
            }
        } else {
            let tile_col = if self.light_glass {
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
                    a: 0.12,
                }
            };
            if let Ok(tb) = self.rt.CreateSolidColorBrush(&tile_col, None) {
                self.rt.FillRoundedRectangle(&thumb_rr, &tb);
            }
            if let Ok(nb) = self
                .rt
                .CreateSolidColorBrush(&primary_text_color(self.light_glass), None)
            {
                self.rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: D2D_POINT_2F {
                            x: (thumb_rect.left + thumb_rect.right) * 0.5,
                            y: (thumb_rect.top + thumb_rect.bottom) * 0.5,
                        },
                        radiusX: 3.8 * s,
                        radiusY: 3.8 * s,
                    },
                    &nb,
                );
            }
        }

        let text_left = thumb_rect.right + 10.0 * s;
        let status_w = 48.0 * s;
        let text_right = (rect.right - status_w).max(text_left + 40.0 * s);
        let mid_y = (rect.top + rect.bottom) * 0.5;
        let title_r = RectF::new(text_left, mid_y - 16.0 * s, text_right, mid_y + 1.0 * s);
        let sub_r = RectF::new(text_left, mid_y + 0.5 * s, text_right, mid_y + 15.5 * s);

        draw_text_in_rect(
            &self.dwrite_factory,
            &self.rt,
            &src.title,
            title_r,
            title_fmt,
            primary_text_color(self.light_glass),
        );
        let subtitle = if src.artist.is_empty() {
            src.app_name.clone()
        } else {
            format!("{} · {}", src.artist, src.app_name)
        };
        draw_text_in_rect(
            &self.dwrite_factory,
            &self.rt,
            &subtitle,
            sub_r,
            sub_fmt,
            muted_text_color(self.light_glass),
        );

        let is_playing = src.status == PlaybackState::Playing;
        let accent = if self.auto_color {
            src.cover
                .as_ref()
                .map(|c| {
                    let raw = if self.light_glass {
                        c.accent_light
                    } else {
                        c.accent_dark
                    };
                    enforce_ui_accent_deadzone(raw, self.light_glass)
                })
                .unwrap_or(fallback_accent)
        } else {
            fallback_accent
        };
        let badge_col = if is_playing {
            d2d_color_rgb(accent, 0.98)
        } else if selected {
            primary_text_color(self.light_glass)
        } else {
            muted_text_color(self.light_glass)
        };
        let badge_text = if is_playing { "播放中" } else { "已暂停" };
        let badge_r = RectF::new(rect.right - status_w, rect.top, rect.right - 8.0 * s, rect.bottom);
        draw_text_in_rect(
            &self.dwrite_factory,
            &self.rt,
            badge_text,
            badge_r,
            badge_fmt,
            badge_col,
        );
    }
}
