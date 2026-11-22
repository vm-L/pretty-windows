use super::{HoverTarget, PopupInner, PrimaryCategory, SliderTarget, SubSpringState, VolumeRowGeom, DEVICES_SUB_WIDTH_DIP, DEVICE_ROW_H_DIP, PRIMARY_ROW_H_DIP, PRIMARY_WIDTH_DIP, SETTING_ROW_H_DIP, SETTINGS_SUB_WIDTH_DIP, SOURCES_SUB_WIDTH_DIP, SOURCE_ROW_H_DIP, SUBMENU_GAP_DIP, SUB_VERT_DAMPING, SUB_VERT_STIFFNESS, TIMER_POPUP_FLOW_ID, AUTO_ROW_H_DIP, SUB_WIDTH_DAMPING, SUB_WIDTH_STIFFNESS, VOLUME_ROW_H_DIP, VOLUME_SUB_WIDTH_DIP};
use super::super::layout::RectF;
use super::super::volume::{enumerate_app_volumes_with_watch, slider_ratio_from_x, MasterVolumeControl};
use crate::system::active_taskbar_surface_style;
use crate::ui::{animated_closing_sub_rect, animated_primary_rect, clamp_sub_spring_rect, compute_submenu_vertical_span, popup_glass_recipe, sub_bud_start_rect, PopupAnimPhase, Spring1D, CLOSE_ANIM_DUR_S, DIVIDER_H_DIP, INNER_PAD_DIP, OPEN_SPRING_DAMPING, OPEN_SPRING_STIFFNESS, POPUP_ANIM_INTERVAL_MS, POPUP_FLOW_INTERVAL_MS, ROW_GAP_DIP, SHADOW_PAD_DIP, STEP_MENISCUS_DIP};
use std::time::Instant;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

impl PopupInner {
    pub(super) fn scale(&self) -> f32 {
        (self.dpi.max(96) as f32) / 96.0
    }

    pub(super) fn reset_transient_state(&mut self) {
        unsafe {
            let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
        }
        self.timer_interval_ms = 0;
        self.backdrop_bmp = None;
        self.halo_colors_ready = false;
        self.last_silk_tick = None;
        self.anim_phase = PopupAnimPhase::Open;
        self.open_spring = Spring1D::still(1.0);
        self.open_elapsed = 1.0;
        self.close_progress = 0.0;
        self.close_started_at = None;
        self.sub_anim = None;
        self.active_category = None;
        self.hover_target = HoverTarget::None;
        self.dragging_slider = None;
        self.session_watch = None;
        self.pending_session_reload = false;
        self.app_volumes.clear();
        self.master_vol_ctrl = None;
    }

    fn needs_fast_anim(&self) -> bool {
        matches!(
            self.anim_phase,
            PopupAnimPhase::Opening | PopupAnimPhase::Closing
        ) || self.sub_anim.is_some()
    }

    pub(super) fn sync_timer_rate(&mut self) {
        let recipe = popup_glass_recipe(active_taskbar_surface_style(), self.light_glass, 0.5);
        let needs_silk = recipe.silk_alpha > 0.002;
        let desired = if self.needs_fast_anim() {
            POPUP_ANIM_INTERVAL_MS
        } else if needs_silk {
            POPUP_FLOW_INTERVAL_MS
        } else {
            0
        };
        if self.timer_interval_ms != desired {
            unsafe {
                if desired == 0 {
                    let _ = KillTimer(self.hwnd, TIMER_POPUP_FLOW_ID);
                } else {
                    if self.timer_interval_ms == 0 {
                        self.last_flow_tick = Instant::now();
                    }
                    let _ = SetTimer(self.hwnd, TIMER_POPUP_FLOW_ID, desired, None);
                }
            }
            self.timer_interval_ms = desired;
        }
    }

    pub(super) fn advance_animations(&mut self, dt: f32) -> bool {
        match self.anim_phase {
            PopupAnimPhase::Opening => {
                self.open_elapsed += dt;
                self.open_spring
                    .step(1.0, OPEN_SPRING_STIFFNESS, OPEN_SPRING_DAMPING, dt);
                if (self.open_spring.is_settled(1.0, 0.0015, 0.015) && self.open_elapsed >= 0.145)
                    || self.open_elapsed >= 0.38
                {
                    self.open_spring = Spring1D::still(1.0);
                    self.anim_phase = PopupAnimPhase::Open;
                }
            }
            PopupAnimPhase::Open => {}
            PopupAnimPhase::Closing => {
                self.close_progress = (self.close_progress + dt / CLOSE_ANIM_DUR_S).min(1.0);
                if self.close_progress >= 1.0 {
                    return true;
                }
            }
        }

        if let Some(cat) = self.active_category {
            if self.sub_anim.is_some() {
                let target = self.sub_card_rect(cat);
                let s = self.scale();
                if let Some(ref mut sa) = self.sub_anim {
                    sa.fade_elapsed += dt;
                    sa.top
                        .step(target.top, SUB_VERT_STIFFNESS, SUB_VERT_DAMPING, dt);
                    sa.bottom
                        .step(target.bottom, SUB_VERT_STIFFNESS, SUB_VERT_DAMPING, dt);
                    sa.width
                        .step(target.width(), SUB_WIDTH_STIFFNESS, SUB_WIDTH_DAMPING, dt);
                    let pos_eps = 0.35 * s;
                    let vel_eps = 4.0 * s;
                    let converged = sa.top.is_settled(target.top, pos_eps, vel_eps)
                        && sa.bottom.is_settled(target.bottom, pos_eps, vel_eps)
                        && sa.width.is_settled(target.width(), pos_eps, vel_eps);
                    if (converged && sa.fade_elapsed >= 0.095) || sa.fade_elapsed >= 0.38 {
                        self.sub_anim = None;
                    }
                }
            }
        }

        false
    }

    pub(super) fn current_animated_sub_rect(&self) -> Option<RectF> {
        let cat = self.active_category?;
        let s = self.scale();
        let target_primary = self.primary_card_rect();
        let anim_primary = animated_primary_rect(
            target_primary,
            self.anim_phase,
            self.open_spring.pos,
            self.close_progress,
            s,
        );
        let target_sub = self.sub_card_rect(cat);
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

    pub(super) fn reload_volumes(&mut self) {
        self.master_vol_ctrl = MasterVolumeControl::open_default();
        if let Some((vol, muted)) = self.master_vol_ctrl.as_mut().and_then(|c| c.get_state()) {
            self.master_vol = vol;
            self.master_muted = muted;
        } else {
            self.master_vol = 1.0;
            self.master_muted = false;
        }
        self.reload_app_sessions_only();
    }

    pub(super) fn reload_app_sessions_only(&mut self) {
        // Drop old COM session notifications before re-enumerating to avoid overlapping watches.
        self.session_watch = None;
        self.pending_session_reload = false;
        let (apps, guard) =
            enumerate_app_volumes_with_watch(self.main_hwnd, self.volume_shared.as_ref());
        self.app_volumes = apps;
        self.session_watch = guard;
    }

    fn set_master_volume_ratio(&mut self, ratio: f32) {
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        if let Some((actual_vol, actual_muted)) = self
            .master_vol_ctrl
            .as_mut()
            .and_then(|ctrl| ctrl.set_volume_and_readback(ratio))
        {
            self.master_vol = actual_vol;
            self.master_muted = actual_muted;
        } else {
            let clamped = ratio.clamp(0.0, 1.0);
            self.master_vol = clamped;
            if self.master_muted && clamped > 0.0 {
                self.master_muted = false;
            }
        }
    }

    pub(super) fn step_master_volume_wheel(&mut self, wheel_delta: i16) {
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        if let Some((actual_vol, actual_muted)) = self
            .master_vol_ctrl
            .as_mut()
            .and_then(|ctrl| ctrl.step_wheel_and_readback(wheel_delta))
        {
            self.master_vol = actual_vol;
            self.master_muted = actual_muted;
        }
    }

    pub(super) fn toggle_master_mute(&mut self) {
        if self.master_vol_ctrl.is_none() {
            self.master_vol_ctrl = MasterVolumeControl::open_default();
        }
        let next_muted = !self.master_muted;
        if let Some((actual_vol, actual_muted)) = self
            .master_vol_ctrl
            .as_mut()
            .and_then(|ctrl| ctrl.set_mute_and_readback(next_muted))
        {
            self.master_vol = actual_vol;
            self.master_muted = actual_muted;
        } else {
            self.master_muted = next_muted;
        }
    }

    pub(super) fn primary_height_px(&self) -> f32 {
        let s = self.scale();
        (INNER_PAD_DIP * 2.0 + PRIMARY_ROW_H_DIP * 5.0 + ROW_GAP_DIP * 4.0) * s
    }

    pub(super) fn sub_natural_size_px(&self, cat: PrimaryCategory) -> (f32, f32) {
        let s = self.scale();
        let w_dip = match cat {
            PrimaryCategory::DisplayContent => SOURCES_SUB_WIDTH_DIP,
            PrimaryCategory::VolumeControl => VOLUME_SUB_WIDTH_DIP,
            PrimaryCategory::PlaybackDevices | PrimaryCategory::RecordingDevices => {
                DEVICES_SUB_WIDTH_DIP
            }
            PrimaryCategory::Settings => SETTINGS_SUB_WIDTH_DIP,
        };
        let h_dip = match cat {
            PrimaryCategory::DisplayContent => {
                let rows_h = if self.sources.is_empty() {
                    34.0
                } else {
                    let n = self.sources.len() as f32;
                    n * SOURCE_ROW_H_DIP + (n - 1.0).max(0.0) * ROW_GAP_DIP
                };
                INNER_PAD_DIP * 2.0 + AUTO_ROW_H_DIP + DIVIDER_H_DIP + rows_h
            }
            PrimaryCategory::VolumeControl => {
                let apps_h = if self.app_volumes.is_empty() {
                    34.0
                } else {
                    let n = self.app_volumes.len() as f32;
                    n * VOLUME_ROW_H_DIP + (n - 1.0).max(0.0) * ROW_GAP_DIP
                };
                INNER_PAD_DIP * 2.0 + VOLUME_ROW_H_DIP + DIVIDER_H_DIP + apps_h
            }
            PrimaryCategory::PlaybackDevices => {
                let rows_h = if self.playback_devices.is_empty() {
                    34.0
                } else {
                    let n = self.playback_devices.len() as f32;
                    n * DEVICE_ROW_H_DIP + (n - 1.0).max(0.0) * ROW_GAP_DIP
                };
                INNER_PAD_DIP * 2.0 + rows_h
            }
            PrimaryCategory::RecordingDevices => {
                let rows_h = if self.recording_devices.is_empty() {
                    34.0
                } else {
                    let n = self.recording_devices.len() as f32;
                    n * DEVICE_ROW_H_DIP + (n - 1.0).max(0.0) * ROW_GAP_DIP
                };
                INNER_PAD_DIP * 2.0 + rows_h
            }
            PrimaryCategory::Settings => {
                INNER_PAD_DIP * 2.0 + SETTING_ROW_H_DIP * 4.0 + ROW_GAP_DIP * 2.0 + DIVIDER_H_DIP
            }
        };
        (w_dip * s, h_dip * s)
    }

    pub(super) fn max_popup_reach_px(&self) -> i32 {
        let mut max_h = self.primary_height_px();
        for &cat in &PrimaryCategory::ALL {
            let (_, sh) = self.sub_natural_size_px(cat);
            if sh > max_h {
                max_h = sh;
            }
        }
        max_h.ceil() as i32
    }

    pub(super) fn primary_card_rect(&self) -> RectF {
        let s = self.scale();
        let pad = SHADOW_PAD_DIP * s;
        let pw = PRIMARY_WIDTH_DIP * s;
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

    pub(super) fn category_row_rect(&self, cat: PrimaryCategory) -> RectF {
        for (c, r) in self.primary_row_rects() {
            if c == cat {
                return r;
            }
        }
        self.primary_card_rect()
    }

    pub(super) fn sub_card_rect(&self, cat: PrimaryCategory) -> RectF {
        let s = self.scale();
        let pad = SHADOW_PAD_DIP * s;
        let gap = SUBMENU_GAP_DIP * s;
        let primary = self.primary_card_rect();
        let row = self.category_row_rect(cat);
        let (sw, natural_sh) = self.sub_natural_size_px(cat);

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
            RectF::new(primary.left - gap - sw, top, primary.left - gap, bottom)
        } else {
            RectF::new(primary.right + gap, top, primary.right + gap + sw, bottom)
        }
    }

    pub(super) fn primary_row_rects(&self) -> [(PrimaryCategory, RectF); 5] {
        let s = self.scale();
        let card = self.primary_card_rect();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let rh = PRIMARY_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let mut y = card.top + pad;

        let r0 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r1 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r2 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r3 = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r4 = RectF::new(x0, y, x1, y + rh);

        [
            (PrimaryCategory::DisplayContent, r0),
            (PrimaryCategory::VolumeControl, r1),
            (PrimaryCategory::PlaybackDevices, r2),
            (PrimaryCategory::RecordingDevices, r3),
            (PrimaryCategory::Settings, r4),
        ]
    }

    pub(super) fn display_row_rects(&self, card: RectF) -> (RectF, Vec<RectF>) {
        let s = self.scale();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let mut y = card.top + pad;

        let auto_rect = RectF::new(x0, y, x1, y + AUTO_ROW_H_DIP * s);
        y = auto_rect.bottom + DIVIDER_H_DIP * s;

        let mut source_rects = Vec::with_capacity(self.sources.len());
        let rh = SOURCE_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        for _ in &self.sources {
            if y + rh > card.bottom - pad + 2.0 {
                break;
            }
            source_rects.push(RectF::new(x0, y, x1, y + rh));
            y += rh + gap;
        }
        (auto_rect, source_rects)
    }

    fn volume_row_geom(&self, row_rect: RectF) -> VolumeRowGeom {
        let s = self.scale();
        let mute_size = 28.0 * s;
        let mute_left = row_rect.left + 8.0 * s;
        let mute_top = (row_rect.top + row_rect.bottom - mute_size) * 0.5;
        let mute_rect = RectF::new(
            mute_left,
            mute_top,
            mute_left + mute_size,
            mute_top + mute_size,
        );

        let col_left = mute_rect.right + 8.0 * s;
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
        let track_cy = row_rect.top + 31.5 * s;
        let track_left = col_left + 4.0 * s;
        let track_right = (col_right - 4.0 * s).max(track_left + 24.0 * s);
        let track_rect = RectF::new(
            track_left,
            track_cy - track_h * 0.5,
            track_right,
            track_cy + track_h * 0.5,
        );
        let slider_hit_rect = RectF::new(
            track_left - 4.0 * s,
            row_rect.top + 18.0 * s,
            track_right + 4.0 * s,
            row_rect.bottom,
        );

        VolumeRowGeom {
            row_rect,
            mute_rect,
            track_rect,
            slider_hit_rect,
            label_rect,
            pct_rect,
        }
    }

    pub(super) fn volume_row_rects(&self, card: RectF) -> (VolumeRowGeom, Vec<VolumeRowGeom>) {
        let s = self.scale();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let mut y = card.top + pad;
        let rh = VOLUME_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;

        let master_row = RectF::new(x0, y, x1, y + rh);
        let master_geom = self.volume_row_geom(master_row);
        y = master_row.bottom + DIVIDER_H_DIP * s;

        let mut app_geoms = Vec::with_capacity(self.app_volumes.len());
        for _ in &self.app_volumes {
            if y + rh > card.bottom - pad + 2.0 {
                break;
            }
            app_geoms.push(self.volume_row_geom(RectF::new(x0, y, x1, y + rh)));
            y += rh + gap;
        }
        (master_geom, app_geoms)
    }

    pub(super) fn device_row_rects(&self, card: RectF, count: usize) -> Vec<RectF> {
        let s = self.scale();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let mut y = card.top + pad;
        let rh = DEVICE_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let mut rects = Vec::with_capacity(count);
        for _ in 0..count {
            if y + rh > card.bottom - pad + 2.0 {
                break;
            }
            rects.push(RectF::new(x0, y, x1, y + rh));
            y += rh + gap;
        }
        rects
    }

    pub(super) fn settings_row_rects(&self, card: RectF) -> [RectF; 4] {
        let s = self.scale();
        let pad = INNER_PAD_DIP * s;
        let x0 = card.left + pad;
        let x1 = card.right - pad;
        let rh = SETTING_ROW_H_DIP * s;
        let gap = ROW_GAP_DIP * s;
        let mut y = card.top + pad;

        let r_auto_color = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r_lock = RectF::new(x0, y, x1, y + rh);
        y += rh + gap;
        let r_reset = RectF::new(x0, y, x1, y + rh);
        y = r_reset.bottom + DIVIDER_H_DIP * s;
        let r_close = RectF::new(x0, y, x1, y + rh);
        [r_auto_color, r_lock, r_reset, r_close]
    }

    pub(super) fn point_inside_cards(&self, x: f32, y: f32) -> bool {
        let s = self.scale();
        let step = STEP_MENISCUS_DIP * s;
        let primary = self.primary_card_rect();
        if primary.contains(x, y) {
            return true;
        }
        if let Some(cat) = self.active_category {
            let sub = self.sub_card_rect(cat);
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
            if y >= top_band_min && y <= top_band_max && (x - seam_x).abs() <= step.max(14.0 * s) {
                return true;
            }
            let bot_band_min = primary.bottom.min(sub.bottom) - step;
            let bot_band_max = primary.bottom.max(sub.bottom);
            if y >= bot_band_min && y <= bot_band_max && (x - seam_x).abs() <= step.max(14.0 * s) {
                return true;
            }
        }
        false
    }

    pub(super) fn update_hover_at(&mut self, x: f32, y: f32) -> bool {
        let prev_cat = self.active_category;
        let prev_hover = self.hover_target;

        let primary_rect = self.primary_card_rect();
        if self.dragging_slider.is_none() && primary_rect.contains(x, y) {
            let mut next_hover = HoverTarget::None;
            for (cat, r) in self.primary_row_rects() {
                if r.contains(x, y) {
                    if self.active_category != Some(cat) {
                        let s = self.scale();
                        let prev_anim_sub = self.current_animated_sub_rect();
                        self.active_category = Some(cat);
                        if cat == PrimaryCategory::VolumeControl {
                            if self.session_watch.is_none() {
                                self.reload_volumes();
                            } else {
                                if self.master_vol_ctrl.is_none() {
                                    self.master_vol_ctrl = MasterVolumeControl::open_default();
                                }
                                if let Some((vol, muted)) =
                                    self.master_vol_ctrl.as_mut().and_then(|c| c.get_state())
                                {
                                    self.master_vol = vol;
                                    self.master_muted = muted;
                                }
                            }
                        }
                        let target_sub = self.sub_card_rect(cat);
                        let seam_x = if self.open_left {
                            primary_rect.left
                        } else {
                            primary_rect.right
                        };
                        self.sub_anim = match (prev_cat, self.sub_anim) {
                            (Some(pcat), Some(mut existing)) => {
                                if let Some(prev_rect) = prev_anim_sub {
                                    existing.top.pos = prev_rect.top;
                                    existing.bottom.pos = prev_rect.bottom;
                                    existing.width.pos = prev_rect.width();
                                }
                                existing.fade_elapsed = 0.0;
                                existing.prev_category = Some(pcat);
                                existing.is_budding = false;
                                Some(existing)
                            }
                            (Some(pcat), None) => {
                                let old_rect = prev_anim_sub.unwrap_or_else(|| self.sub_card_rect(pcat));
                                Some(SubSpringState {
                                    top: Spring1D::still(old_rect.top),
                                    bottom: Spring1D::still(old_rect.bottom),
                                    width: Spring1D::still(old_rect.width()),
                                    fade_elapsed: 0.0,
                                    prev_category: Some(pcat),
                                    is_budding: false,
                                })
                            }
                            (None, _) => {
                                let bud =
                                    sub_bud_start_rect(target_sub, r, seam_x, self.open_left, s);
                                Some(SubSpringState {
                                    top: Spring1D::new(
                                        bud.top,
                                        (target_sub.top - bud.top) * 2.2,
                                    ),
                                    bottom: Spring1D::new(
                                        bud.bottom,
                                        (target_sub.bottom - bud.bottom) * 2.2,
                                    ),
                                    width: Spring1D::new(bud.width(), target_sub.width() * 2.6),
                                    fade_elapsed: 0.0,
                                    prev_category: None,
                                    is_budding: true,
                                })
                            }
                        };
                        self.sync_timer_rate();
                    }
                    next_hover = HoverTarget::Primary(cat);
                    break;
                }
            }
            self.hover_target = next_hover;
        } else if let Some(cat) = self.active_category {
            let sub_rect = self.sub_card_rect(cat);
            if sub_rect.contains(x, y) {
                self.hover_target = match cat {
                    PrimaryCategory::DisplayContent => {
                        let (auto_rect, source_rects) = self.display_row_rects(sub_rect);
                        if auto_rect.contains(x, y) {
                            HoverTarget::SubAuto
                        } else {
                            source_rects
                                .iter()
                                .position(|r| r.contains(x, y))
                                .map(HoverTarget::SubSource)
                                .unwrap_or(HoverTarget::None)
                        }
                    }
                    PrimaryCategory::VolumeControl => {
                        let (master_geom, app_geoms) = self.volume_row_rects(sub_rect);
                        if master_geom.row_rect.contains(x, y) {
                            if master_geom.mute_rect.contains(x, y) {
                                HoverTarget::SubVolMasterMute
                            } else if master_geom.slider_hit_rect.contains(x, y) {
                                HoverTarget::SubVolMasterSlider
                            } else {
                                HoverTarget::SubVolMasterRow
                            }
                        } else if let Some((idx, g)) = app_geoms
                            .iter()
                            .enumerate()
                            .find(|(_, g)| g.row_rect.contains(x, y))
                        {
                            if g.mute_rect.contains(x, y) {
                                HoverTarget::SubVolAppMute(idx)
                            } else if g.slider_hit_rect.contains(x, y) {
                                HoverTarget::SubVolAppSlider(idx)
                            } else {
                                HoverTarget::SubVolAppRow(idx)
                            }
                        } else {
                            HoverTarget::None
                        }
                    }
                    PrimaryCategory::PlaybackDevices => {
                        let rects = self.device_row_rects(sub_rect, self.playback_devices.len());
                        rects
                            .iter()
                            .position(|r| r.contains(x, y))
                            .map(HoverTarget::SubPlayback)
                            .unwrap_or(HoverTarget::None)
                    }
                    PrimaryCategory::RecordingDevices => {
                        let rects = self.device_row_rects(sub_rect, self.recording_devices.len());
                        rects
                            .iter()
                            .position(|r| r.contains(x, y))
                            .map(HoverTarget::SubRecording)
                            .unwrap_or(HoverTarget::None)
                    }
                    PrimaryCategory::Settings => {
                        let [r_auto_color, r_lock, r_reset, r_close] =
                            self.settings_row_rects(sub_rect);
                        if r_auto_color.contains(x, y) {
                            HoverTarget::SubSettingAutoColor
                        } else if r_lock.contains(x, y) {
                            HoverTarget::SubSettingLock
                        } else if r_reset.contains(x, y) {
                            HoverTarget::SubSettingResetPos
                        } else if r_close.contains(x, y) {
                            HoverTarget::SubSettingClose
                        } else {
                            HoverTarget::None
                        }
                    }
                };
            } else {
                self.hover_target = HoverTarget::None;
            }
        } else {
            self.hover_target = HoverTarget::None;
        }

        prev_cat != self.active_category || prev_hover != self.hover_target
    }

    pub(super) fn apply_slider_drag(&mut self, target: SliderTarget, cursor_x: f32) -> bool {
        if self.active_category != Some(PrimaryCategory::VolumeControl) {
            return false;
        }
        let card = self.sub_card_rect(PrimaryCategory::VolumeControl);
        let (master_geom, app_geoms) = self.volume_row_rects(card);
        match target {
            SliderTarget::Master => {
                let ratio = slider_ratio_from_x(
                    master_geom.track_rect.left,
                    master_geom.track_rect.right,
                    cursor_x,
                );
                self.set_master_volume_ratio(ratio);
                true
            }
            SliderTarget::App(idx) => {
                if let (Some(app), Some(geom)) =
                    (self.app_volumes.get_mut(idx), app_geoms.get(idx))
                {
                    let ratio =
                        slider_ratio_from_x(geom.track_rect.left, geom.track_rect.right, cursor_x);
                    app.set_group_volume(ratio);
                    true
                } else {
                    false
                }
            }
        }
    }

}
