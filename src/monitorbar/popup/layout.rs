use super::{BrightnessRowGeom, HoverTarget, MonitorPopupInner, PrimaryItem, SubHoverTarget, SubLayout, SubRowRect, SubSpringState, SubmenuKind, BRIGHTNESS_ROW_H_DIP, MAX_BRIGHTNESS_ROWS, MAX_LIST_ROWS, PRIMARY_W_DIP, SUB_VERT_DAMPING, SUB_VERT_STIFFNESS, TIMER_POPUP_FLOW_ID};
use super::super::config::{LeftClickAction, MetricItem, MonitorConfig};
use crate::system::active_taskbar_surface_style;
use crate::ui::glass::create_text_format;
use crate::ui::{animated_closing_sub_rect, animated_primary_rect, clamp_sub_spring_rect, compute_submenu_vertical_span, popup_glass_recipe, sub_bud_start_rect, PopupAnimPhase, RectF, Spring1D, CLOSE_ANIM_DUR_S, DIVIDER_H_DIP, INNER_PAD_DIP, OPEN_SPRING_DAMPING, OPEN_SPRING_STIFFNESS, POPUP_ANIM_INTERVAL_MS, POPUP_FLOW_INTERVAL_MS, ROW_GAP_DIP, ROW_H_DIP, SHADOW_PAD_DIP, STEP_MENISCUS_DIP, SUB_WIDTH_DAMPING, SUB_WIDTH_STIFFNESS};
use std::time::Instant;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

fn submenu_height_dip(list_rows: usize) -> f32 {
    let rows = list_rows.max(1);
    let total_rows = 2 + rows;
    let gaps = rows.saturating_sub(1);
    INNER_PAD_DIP * 2.0
        + ROW_H_DIP * (total_rows as f32)
        + ROW_GAP_DIP * (gaps as f32)
        + DIVIDER_H_DIP * 2.0
}

fn brightness_submenu_height_dip(display_rows: usize) -> f32 {
    if display_rows == 0 {
        INNER_PAD_DIP * 2.0 + ROW_H_DIP * 2.0 + DIVIDER_H_DIP
    } else {
        let rows = display_rows.min(MAX_BRIGHTNESS_ROWS);
        let gaps = rows.saturating_sub(1);
        INNER_PAD_DIP * 2.0
            + BRIGHTNESS_ROW_H_DIP * (rows as f32)
            + ROW_GAP_DIP * (gaps as f32)
            + DIVIDER_H_DIP
            + ROW_H_DIP
    }
}

fn primary_rows_height_dip() -> f32 {
    INNER_PAD_DIP * 2.0 + ROW_H_DIP * 6.0 + ROW_GAP_DIP * 5.0
}

impl MonitorPopupInner {
    pub(super) fn scale(&self) -> f32 {
        (self.dpi.max(96) as f32) / 96.0
    }

    pub(super) fn cfg_snapshot(&self) -> MonitorConfig {
        self.cfg.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub(super) fn primary_height_px(&self) -> f32 {
        primary_rows_height_dip() * self.scale()
    }

    pub(super) fn sub_natural_size_px(&self, kind: SubmenuKind) -> (f32, f32) {
        let s = self.scale();
        let w = kind.width_dip() * s;
        let h_dip = match kind {
            SubmenuKind::WiFi => {
                let n = if self.wireless_snap.wifi_available && self.wireless_snap.wifi_enabled {
                    self.wireless_snap.wifi_networks.len().clamp(1, MAX_LIST_ROWS)
                } else {
                    1
                };
                submenu_height_dip(n)
            }
            SubmenuKind::Bluetooth => {
                let n = if self.wireless_snap.bt_available && self.wireless_snap.bt_enabled {
                    self.wireless_snap.bt_devices.len().clamp(1, MAX_LIST_ROWS)
                } else {
                    1
                };
                submenu_height_dip(n)
            }
            SubmenuKind::Brightness => {
                brightness_submenu_height_dip(self.brightness_snap.displays.len())
            }
            SubmenuKind::LeftClick => INNER_PAD_DIP * 2.0 + ROW_H_DIP * 5.0 + ROW_GAP_DIP * 4.0,
            SubmenuKind::Display => INNER_PAD_DIP * 2.0 + ROW_H_DIP * 5.0 + ROW_GAP_DIP * 4.0,
            SubmenuKind::Settings => {
                INNER_PAD_DIP * 2.0 + ROW_H_DIP * 3.0 + ROW_GAP_DIP + DIVIDER_H_DIP
            }
        };
        (w, h_dip * s)
    }

    pub(super) fn max_popup_reach_px(&self) -> i32 {
        let s = self.scale();
        let max_sub_h = submenu_height_dip(MAX_LIST_ROWS)
            .max(brightness_submenu_height_dip(MAX_BRIGHTNESS_ROWS))
            * s;
        let pri_h = self.primary_height_px();
        pri_h.max(max_sub_h).ceil() as i32
    }

    pub(super) fn primary_card_rect(&self) -> RectF {
        let s = self.scale();
        let pad = SHADOW_PAD_DIP * s;
        let pw = PRIMARY_W_DIP * s;
        let ph = self.primary_height_px();
        let bottom = ((self.win_h as f32) - pad).max(pad + 44.0 * s);
        let top = (bottom - ph).max(pad);
        let left = if self.open_left {
            (self.win_w as f32) - pad - pw
        } else {
            pad
        };
        RectF::new(left, top, left + pw, bottom)
    }

    pub(super) fn primary_row_rects(&self) -> [(PrimaryItem, RectF); 6] {
        let s = self.scale();
        let card = self.primary_card_rect();
        let inner = INNER_PAD_DIP * s;
        let x0 = card.left + inner;
        let x1 = card.right - inner;
        let rh = ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let mut y = card.top + inner;

        let r0 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r1 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r2 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r3 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r4 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r5 = RectF::new(x0, y, x1, y + rh);

        [
            (PrimaryItem::WiFi, r0),
            (PrimaryItem::Bluetooth, r1),
            (PrimaryItem::Brightness, r2),
            (PrimaryItem::LeftClick, r3),
            (PrimaryItem::Display, r4),
            (PrimaryItem::Settings, r5),
        ]
    }

    pub(super) fn category_row_rect(&self, kind: SubmenuKind) -> RectF {
        for (item, r) in self.primary_row_rects() {
            if item.submenu() == Some(kind) {
                return r;
            }
        }
        self.primary_card_rect()
    }

    pub(super) fn sub_card_rect(&self, kind: SubmenuKind) -> RectF {
        let s = self.scale();
        let pad = SHADOW_PAD_DIP * s;
        let primary = self.primary_card_rect();
        let row = self.category_row_rect(kind);
        let (sw, natural_sh) = self.sub_natural_size_px(kind);

        let bottom_limit = primary.bottom.max(primary.top + 58.0 * s);
        let top_limit = if self.win_h > 10 {
            pad
        } else {
            primary.top - natural_sh
        };
        let (top, bottom) = compute_submenu_vertical_span(
            row.top,
            INNER_PAD_DIP * s,
            natural_sh,
            top_limit,
            bottom_limit,
        );
        if self.open_left {
            RectF::new(primary.left - sw, top, primary.left, bottom)
        } else {
            RectF::new(primary.right, top, primary.right + sw, bottom)
        }
    }

    pub(super) fn current_animated_sub_rect(&self) -> Option<RectF> {
        let kind = self.active_sub?;
        let s = self.scale();
        let target_primary = self.primary_card_rect();
        let anim_primary = animated_primary_rect(
            target_primary,
            self.anim_phase,
            self.open_spring.pos,
            self.close_progress,
            s,
        );
        let target_sub = self.sub_card_rect(kind);
        let seam_x = if self.open_left {
            anim_primary.left
        } else {
            anim_primary.right
        };
        let bottom_limit = target_primary.bottom.max(target_primary.top + 58.0 * s);
        let base_sub = match self.sub_anim {
            Some(sa) => clamp_sub_spring_rect(
                sa.top.pos,
                sa.bottom.pos,
                sa.width.pos,
                target_sub,
                seam_x,
                bottom_limit,
                self.win_w as f32,
                self.open_left,
                s,
            ),
            None => target_sub,
        };

        if self.anim_phase == PopupAnimPhase::Closing {
            Some(animated_closing_sub_rect(
                base_sub,
                anim_primary,
                seam_x,
                bottom_limit,
                self.open_left,
                self.close_progress,
                s,
            ))
        } else {
            Some(base_sub)
        }
    }

    pub(super) fn brightness_row_geom(&self, row_rect: RectF, s: f32) -> BrightnessRowGeom {
        let col_left = row_rect.left + 12.0 * s;
        let col_right = row_rect.right - 10.0 * s;
        let pct_w = 40.0 * s;

        let upper_top = row_rect.top + 4.0 * s;
        let upper_bottom = row_rect.top + 22.0 * s;
        let label_rect = RectF::new(
            col_left,
            upper_top,
            (col_right - pct_w).max(col_left + 30.0 * s),
            upper_bottom,
        );
        let pct_rect = RectF::new(col_right - pct_w, upper_top, col_right, upper_bottom);

        let track_h = 4.0 * s;
        let track_cy = row_rect.top + 32.5 * s;
        let track_left = col_left + 4.0 * s;
        let track_right = (col_right - 4.0 * s).max(track_left + 24.0 * s);
        let track_rect = RectF::new(
            track_left,
            track_cy - track_h * 0.5,
            track_right,
            track_cy + track_h * 0.5,
        );
        let slider_hit_rect = RectF::new(
            track_left - 8.0 * s,
            row_rect.top + 18.0 * s,
            track_right + 8.0 * s,
            row_rect.bottom,
        );

        BrightnessRowGeom {
            label_rect,
            pct_rect,
            track_rect,
            slider_hit_rect,
        }
    }

    pub(super) fn build_sub_layout(&self, sr: RectF, kind: SubmenuKind) -> SubLayout {
        let s = self.scale();
        let inner = INNER_PAD_DIP * s;
        let rh = ROW_H_DIP * s;
        let brh = BRIGHTNESS_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let div_h = DIVIDER_H_DIP * s;

        let sx0 = sr.left + inner;
        let sx1 = sr.right - inner;
        let mut sy = sr.top + inner;

        let mut rows = Vec::new();
        let mut dividers = Vec::new();
        let mut list_viewport = None;
        let mut list_total_items = 0usize;
        let mut list_scroll = 0usize;
        let mut list_max_rows = MAX_LIST_ROWS;

        match kind {
            SubmenuKind::WiFi => {
                let toggle_r = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::WifiToggle),
                    rect: toggle_r,
                });
                dividers.push(toggle_r);
                sy = toggle_r.bottom + div_h;

                let list_top = sy;
                if self.wireless_snap.wifi_available && self.wireless_snap.wifi_enabled {
                    list_total_items = self.wireless_snap.wifi_networks.len();
                    list_scroll = self.wifi_scroll;
                    if list_total_items == 0 {
                        let r = RectF::new(sx0, sy, sx1, sy + rh);
                        rows.push(SubRowRect { target: None, rect: r });
                        sy = r.bottom;
                    } else {
                        let visible = (list_total_items - list_scroll).min(MAX_LIST_ROWS);
                        for i in 0..visible {
                            let r = RectF::new(sx0, sy, sx1, sy + rh);
                            rows.push(SubRowRect {
                                target: Some(SubHoverTarget::WifiNetwork(list_scroll + i)),
                                rect: r,
                            });
                            sy = r.bottom + if i + 1 < visible { gap } else { 0.0 };
                        }
                        if list_total_items > MAX_LIST_ROWS {
                            list_viewport = Some(RectF::new(sx0, list_top, sx1, sy));
                        }
                    }
                } else {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect { target: None, rect: r });
                    sy = r.bottom;
                }

                dividers.push(RectF::new(sx0, sy - rh, sx1, sy));
                sy += div_h;
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::WifiMoreSettings),
                    rect: RectF::new(sx0, sy, sx1, sy + rh),
                });
            }
            SubmenuKind::Bluetooth => {
                let toggle_r = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::BtToggle),
                    rect: toggle_r,
                });
                dividers.push(toggle_r);
                sy = toggle_r.bottom + div_h;

                let list_top = sy;
                if self.wireless_snap.bt_available && self.wireless_snap.bt_enabled {
                    list_total_items = self.wireless_snap.bt_devices.len();
                    list_scroll = self.bt_scroll;
                    if list_total_items == 0 {
                        let r = RectF::new(sx0, sy, sx1, sy + rh);
                        rows.push(SubRowRect { target: None, rect: r });
                        sy = r.bottom;
                    } else {
                        let visible = (list_total_items - list_scroll).min(MAX_LIST_ROWS);
                        for i in 0..visible {
                            let r = RectF::new(sx0, sy, sx1, sy + rh);
                            rows.push(SubRowRect {
                                target: Some(SubHoverTarget::BtDevice(list_scroll + i)),
                                rect: r,
                            });
                            sy = r.bottom + if i + 1 < visible { gap } else { 0.0 };
                        }
                        if list_total_items > MAX_LIST_ROWS {
                            list_viewport = Some(RectF::new(sx0, list_top, sx1, sy));
                        }
                    }
                } else {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect { target: None, rect: r });
                    sy = r.bottom;
                }

                dividers.push(RectF::new(sx0, sy - rh, sx1, sy));
                sy += div_h;
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::BtMoreSettings),
                    rect: RectF::new(sx0, sy, sx1, sy + rh),
                });
            }
            SubmenuKind::Brightness => {
                list_max_rows = MAX_BRIGHTNESS_ROWS;
                list_total_items = self.brightness_snap.displays.len();
                list_scroll = self.brightness_scroll;
                let list_top = sy;
                if list_total_items == 0 {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect { target: None, rect: r });
                    sy = r.bottom;
                    dividers.push(r);
                } else {
                    let visible = (list_total_items - list_scroll).min(MAX_BRIGHTNESS_ROWS);
                    for i in 0..visible {
                        let r = RectF::new(sx0, sy, sx1, sy + brh);
                        rows.push(SubRowRect {
                            target: Some(SubHoverTarget::BrightnessRow(list_scroll + i)),
                            rect: r,
                        });
                        sy = r.bottom + if i + 1 < visible { gap } else { 0.0 };
                    }
                    if list_total_items > MAX_BRIGHTNESS_ROWS {
                        list_viewport = Some(RectF::new(sx0, list_top, sx1, sy));
                    }
                    dividers.push(RectF::new(sx0, sy - brh, sx1, sy));
                }
                sy += div_h;
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::BrightnessMoreSettings),
                    rect: RectF::new(sx0, sy, sx1, sy + rh),
                });
            }
            SubmenuKind::LeftClick => {
                for action in LeftClickAction::ALL {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect {
                        target: Some(SubHoverTarget::LeftClickPreset(action)),
                        rect: r,
                    });
                    sy = r.bottom + gap;
                }
            }
            SubmenuKind::Display => {
                for item in MetricItem::ALL {
                    let r = RectF::new(sx0, sy, sx1, sy + rh);
                    rows.push(SubRowRect {
                        target: Some(SubHoverTarget::DisplayMetric(item)),
                        rect: r,
                    });
                    sy = r.bottom + gap;
                }
            }
            SubmenuKind::Settings => {
                let r_lock = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::SettingLock),
                    rect: r_lock,
                });
                sy = r_lock.bottom + gap;

                let r_reset = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::SettingReset),
                    rect: r_reset,
                });
                dividers.push(r_reset);
                sy = r_reset.bottom + div_h;

                let r_close = RectF::new(sx0, sy, sx1, sy + rh);
                rows.push(SubRowRect {
                    target: Some(SubHoverTarget::SettingClose),
                    rect: r_close,
                });
            }
        }

        SubLayout {
            rows,
            dividers,
            list_viewport,
            list_total_items,
            list_scroll,
            list_max_rows,
        }
    }

    pub(super) fn clamp_scrolls(&mut self) {
        let max_wifi = self.wireless_snap.wifi_networks.len().saturating_sub(MAX_LIST_ROWS);
        if self.wifi_scroll > max_wifi {
            self.wifi_scroll = max_wifi;
        }
        let max_bt = self.wireless_snap.bt_devices.len().saturating_sub(MAX_LIST_ROWS);
        if self.bt_scroll > max_bt {
            self.bt_scroll = max_bt;
        }
        let max_bright = self
            .brightness_snap
            .displays
            .len()
            .saturating_sub(MAX_BRIGHTNESS_ROWS);
        if self.brightness_scroll > max_bright {
            self.brightness_scroll = max_bright;
        }
    }

    pub(super) fn ensure_formats(&mut self, s: f32) -> windows::core::Result<()> {
        let key = self.dpi.max(96);
        if self.fmt_key == key && self.title_fmt.is_some() && self.badge_fmt.is_some() {
            return Ok(());
        }
        self.title_fmt = Some(create_text_format(
            &self.dwrite_factory,
            13.0 * s,
            false,
            false,
            true,
        )?);
        self.badge_fmt = Some(create_text_format(
            &self.dwrite_factory,
            11.0 * s,
            false,
            true,
            false,
        )?);
        self.fmt_key = key;
        Ok(())
    }

    pub(super) fn point_inside_cards(&self, x: f32, y: f32) -> bool {
        let s = self.scale();
        let step = STEP_MENISCUS_DIP * s;
        let primary = self.primary_card_rect();
        if primary.contains(x, y) {
            return true;
        }
        if let Some(kind) = self.active_sub {
            let sub = self.sub_card_rect(kind);
            if sub.contains(x, y) {
                return true;
            }
            let seam_x = if self.open_left {
                primary.left
            } else {
                primary.right
            };
            let top_band_min = primary.top.min(sub.top);
            let top_band_max = primary.top.max(sub.top) + step;
            if y >= top_band_min && y <= top_band_max && (x - seam_x).abs() <= step.max(14.0 * s)
            {
                return true;
            }
            let bot_band_min = primary.bottom.min(sub.bottom) - step;
            let bot_band_max = primary.bottom.max(sub.bottom);
            if y >= bot_band_min && y <= bot_band_max && (x - seam_x).abs() <= step.max(14.0 * s)
            {
                return true;
            }
        }
        false
    }

    pub(super) fn hit_test(&self, x: f32, y: f32) -> Option<HoverTarget> {
        let primary = self.primary_card_rect();
        if primary.contains(x, y) {
            for (item, r) in self.primary_row_rects() {
                if r.contains(x, y) {
                    return Some(HoverTarget::Primary(item));
                }
            }
            return None;
        }
        if let Some(kind) = self.active_sub {
            let sr = self.sub_card_rect(kind);
            if sr.contains(x, y) {
                let sub_layout = self.build_sub_layout(sr, kind);
                let s = self.scale();
                for row in sub_layout.rows {
                    if row.rect.contains(x, y) {
                        if let Some(SubHoverTarget::BrightnessRow(idx)) = row.target {
                            let geom = self.brightness_row_geom(row.rect, s);
                            if geom.slider_hit_rect.contains(x, y) {
                                return Some(HoverTarget::Sub(SubHoverTarget::BrightnessSlider(idx)));
                            }
                            return Some(HoverTarget::Sub(SubHoverTarget::BrightnessRow(idx)));
                        }
                        return row.target.map(HoverTarget::Sub);
                    }
                }
            }
        }
        None
    }

    pub(super) fn brightness_index_by_id(&self, id: &str) -> Option<usize> {
        self.brightness_snap
            .displays
            .iter()
            .position(|d| d.id == id)
    }

    fn brightness_track_rect_for_idx(&self, idx: usize) -> Option<RectF> {
        let sr = self.sub_card_rect(SubmenuKind::Brightness);
        let sub_layout = self.build_sub_layout(sr, SubmenuKind::Brightness);
        let s = self.scale();
        for row in sub_layout.rows {
            if row.target == Some(SubHoverTarget::BrightnessRow(idx)) {
                return Some(self.brightness_row_geom(row.rect, s).track_rect);
            }
        }
        None
    }

    pub(super) fn apply_brightness_drag(&mut self, display_id: &str, cursor_x: f32) -> bool {
        let Some(idx) = self.brightness_index_by_id(display_id) else {
            return false;
        };
        let Some(track) = self.brightness_track_rect_for_idx(idx) else {
            return false;
        };
        let t = ((cursor_x - track.left) / track.width().max(1.0)).clamp(0.0, 1.0);
        let pct = (t * 100.0).round() as u8;
        let Some(disp) = self.brightness_snap.displays.get_mut(idx) else {
            return false;
        };
        if disp.brightness == pct {
            return false;
        }
        disp.brightness = pct;
        self.brightness.set_brightness(display_id, pct);
        true
    }

    pub(super) fn step_brightness_wheel(&mut self, idx: usize, wheel_delta: i16) -> bool {
        let step = if wheel_delta > 0 { 5i16 } else { -5i16 };
        let Some(disp) = self.brightness_snap.displays.get_mut(idx) else {
            return false;
        };
        let next = (i16::from(disp.brightness) + step).clamp(0, 100) as u8;
        if next == disp.brightness {
            return false;
        }
        disp.brightness = next;
        let id = disp.id.clone();
        self.brightness.set_brightness(&id, next);
        true
    }

    pub(super) fn set_active_submenu(&mut self, next: Option<SubmenuKind>) -> bool {
        if self.active_sub == next {
            return false;
        }
        self.wheel_accum = 0;
        let s = self.scale();
        let primary_rect = self.primary_card_rect();
        let prev_anim_sub = self.current_animated_sub_rect();
        let prev_sub = self.active_sub;
        self.active_sub = next;

        if let Some(kind) = next {
            let target_sub = self.sub_card_rect(kind);
            let seam_x = if self.open_left {
                primary_rect.left
            } else {
                primary_rect.right
            };
            self.sub_anim = match (prev_sub, self.sub_anim) {
                (Some(_), Some(mut existing)) => {
                    if let Some(prev_rect) = prev_anim_sub {
                        existing.top.pos = prev_rect.top;
                        existing.bottom.pos = prev_rect.bottom;
                        existing.width.pos = prev_rect.width();
                    }
                    existing.fade_elapsed = 0.0;
                    existing.is_budding = false;
                    Some(existing)
                }
                (Some(pkind), None) => {
                    let old_rect = prev_anim_sub.unwrap_or_else(|| self.sub_card_rect(pkind));
                    Some(SubSpringState {
                        top: Spring1D::still(old_rect.top),
                        bottom: Spring1D::still(old_rect.bottom),
                        width: Spring1D::still(old_rect.width()),
                        fade_elapsed: 0.0,
                        is_budding: false,
                    })
                }
                (None, _) => {
                    let row = self.category_row_rect(kind);
                    let bud = sub_bud_start_rect(target_sub, row, seam_x, self.open_left, s);
                    Some(SubSpringState {
                        top: Spring1D::new(bud.top, (target_sub.top - bud.top) * 2.2),
                        bottom: Spring1D::new(bud.bottom, (target_sub.bottom - bud.bottom) * 2.2),
                        width: Spring1D::new(bud.width(), target_sub.width() * 2.6),
                        fade_elapsed: 0.0,
                        is_budding: true,
                    })
                }
            };

            match kind {
                SubmenuKind::WiFi => self.wireless.request_refresh(true),
                SubmenuKind::Bluetooth => self.wireless.request_refresh(false),
                SubmenuKind::Brightness => self.brightness.request_refresh(),
                _ => {}
            }
        } else {
            self.sub_anim = None;
        }

        self.sync_timer_rate();
        true
    }

    pub(super) fn sync_timer_rate(&mut self) {
        let need_fast = self.anim_phase != PopupAnimPhase::Open || self.sub_anim.is_some();
        let desired = if need_fast {
            POPUP_ANIM_INTERVAL_MS
        } else {
            let recipe =
                popup_glass_recipe(active_taskbar_surface_style(), self.light_glass, 0.5);
            if recipe.silk_alpha > 0.002 {
                POPUP_FLOW_INTERVAL_MS
            } else {
                0
            }
        };
        if desired != self.timer_interval_ms {
            let prev = self.timer_interval_ms;
            self.timer_interval_ms = desired;
            unsafe {
                if desired == 0 {
                    let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
                } else {
                    if prev == 0 {
                        self.last_flow_tick = Instant::now();
                    }
                    let _ = SetTimer(self.hwnd, TIMER_POPUP_FLOW_ID, desired, None);
                }
            }
        }
    }

    pub(super) fn advance_animations(&mut self, dt: f32) -> bool {
        match self.anim_phase {
            PopupAnimPhase::Opening => {
                self.open_elapsed += dt;
                self.open_spring
                    .step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, dt);
                if self.open_spring.is_settled(1.0, 0.002, 0.03) && self.open_elapsed >= 0.14 {
                    self.open_spring = Spring1D::still(1.0);
                    self.anim_phase = PopupAnimPhase::Open;
                }
            }
            PopupAnimPhase::Open => {}
            PopupAnimPhase::Closing => {
                self.close_progress = (self.close_progress + dt / CLOSE_ANIM_DUR_S).clamp(0.0, 1.0);
                if self.close_progress >= 1.0 {
                    return true;
                }
            }
        }

        if let (Some(kind), Some(mut sa)) = (self.active_sub, self.sub_anim) {
            let target_sub = self.sub_card_rect(kind);
            sa.top
                .step(target_sub.top, SUB_VERT_STIFFNESS, SUB_VERT_DAMPING, dt);
            sa.bottom
                .step(target_sub.bottom, SUB_VERT_STIFFNESS, SUB_VERT_DAMPING, dt);
            sa.width.step(
                target_sub.width(),
                SUB_WIDTH_STIFFNESS,
                SUB_WIDTH_DAMPING,
                dt,
            );
            sa.fade_elapsed = (sa.fade_elapsed + dt).min(1.0);

            let settled = sa.top.is_settled(target_sub.top, 0.35, 3.0)
                && sa.bottom.is_settled(target_sub.bottom, 0.35, 3.0)
                && sa.width.is_settled(target_sub.width(), 0.35, 3.0)
                && sa.fade_elapsed >= 0.11;
            self.sub_anim = if settled { None } else { Some(sa) };
        } else {
            self.sub_anim = None;
        }

        false
    }

    pub(super) fn reset_transient_state(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
        }
        self.dragging_brightness = None;
        self.wheel_accum = 0;
        self.timer_interval_ms = 0;
        self.backdrop_bmp = None;
        self.halo_colors_ready = false;
        self.last_silk_tick = None;
        self.anim_phase = PopupAnimPhase::Open;
        self.open_spring = Spring1D::still(1.0);
        self.open_elapsed = 1.0;
        self.close_progress = 0.0;
        self.close_started_at = None;
        self.active_sub = None;
        self.sub_anim = None;
        self.hover = None;
        self.tracking_leave = false;
    }

}
