//! Desktop Z-order maintenance, Win+D raised-desktop promotion, and Composition GPU backdrop blur sync.

use super::{
    BoxInner, EDGE_ANIM_DUR_S, TIMER_BOX_ANIM_ID, TIMER_HOVER_COLLAPSE_ID, TIMER_HOVER_MENU_ID,
};
use crate::deskbox::desktop::{
    find_desktop_defview_host, is_window_cloaked, probe_desktop_zorder, DesktopZOrderScan,
};
use crate::deskbox::{decide_deskbox_zorder, DeskBoxZOrderAction};
use crate::system::is_fullscreen_active_on_monitor;
use crate::ui::glass::CompositionEngine;
use crate::ui::{GlassSurfaceStyle, Spring1D, PANEL_RADIUS_DIP, SHADOW_PAD_DIP};
use std::mem::size_of;
use windows::core::Interface;
use windows::Foundation::Numerics::Vector2;
use windows::UI::Composition::{
    CompositionGeometry, CompositionRoundedRectangleGeometry, SpriteVisual,
};
use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CLOAK, DWMWA_COLOR_NONE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWM_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindow, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindowVisible, KillTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, GW_HWNDNEXT, GW_HWNDPREV,
    HWND_BOTTOM, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSENDCHANGING, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SW_HIDE, SW_SHOWNOACTIVATE, WS_EX_TOPMOST,
};

impl BoxInner {
    pub(super) fn hide_blur_layer(&mut self) {
        if self.blur_visible && !self.blur_hwnd.is_invalid() {
            unsafe {
                let _ = ShowWindow(self.blur_hwnd, SW_HIDE);
            }
            self.blur_visible = false;
            self.blur_hwnd_bounds = None;
        }
    }

    pub(super) fn sync_blur_layer(&mut self) {
        let recipe = backdrop_blur_params(self.surface_style, self.light_glass);
        if self.fullscreen_suppressed
            || !self.transparency_enabled
            || recipe.is_none()
            || self.comp.is_none()
            || self.comp_visual.is_none()
        {
            self.has_live_blur = false;
            self.hide_blur_layer();
            return;
        }
        let (sigma_dip, saturation) = recipe.unwrap();
        let s = (self.dpi.max(96) as f32) / 96.0;
        let pad = (SHADOW_PAD_DIP * s).round() as i32;
        let card_x = self.win_x + pad;
        let card_y = self.win_y + pad;
        let card_w = (self.win_w - pad * 2).max(1);
        let envelope_h = (self.win_h - pad * 2).max(1);
        let card_h = if self.collapse_animating {
            self.collapse_height_spring.pos.round().max(1.0) as i32
        } else {
            envelope_h
        };
        let hwnd_h = if self.collapse_animating {
            envelope_h.max(card_h)
        } else {
            card_h
        };
        let radius_px = (PANEL_RADIUS_DIP * s).round() as i32;

        let blur_key = (self.surface_style, self.light_glass, self.dpi);
        if self.last_blur_key != Some(blur_key) {
            let applied = self
                .comp
                .as_ref()
                .zip(self.comp_visual.as_ref())
                .and_then(|(comp, visual)| {
                    let brush = comp.build_backdrop_brush(sigma_dip * s, saturation)?;
                    visual.SetBrush(&brush).ok()?;
                    Some(())
                })
                .is_some();
            self.has_live_blur = applied;
            if applied {
                self.last_blur_key = Some(blur_key);
            } else {
                self.hide_blur_layer();
                return;
            }
        }

        if let Some(ref visual) = self.comp_visual {
            let _ = visual.SetSize(Vector2 {
                X: card_w as f32,
                Y: card_h as f32,
            });
            if self.last_clip_size != Some((card_w, card_h, radius_px)) {
                let clipped = update_rounded_visual_clip(
                    self.comp.as_deref(),
                    visual,
                    &mut self.comp_clip_geo,
                    self.blur_hwnd,
                    card_w as f32,
                    card_h as f32,
                    PANEL_RADIUS_DIP * s,
                );
                if clipped {
                    self.last_clip_size = Some((card_w, card_h, radius_px));
                }
            }
        }

        unsafe {
            let desired_bounds = (card_x, card_y, card_w, hwnd_h);
            let blur_is_vis = IsWindowVisible(self.blur_hwnd).as_bool();
            let blur_ex = GetWindowLongPtrW(self.blur_hwnd, GWL_EXSTYLE) as u32;
            let blur_is_topmost = (blur_ex & WS_EX_TOPMOST.0) != 0;
            let band_matches = blur_is_topmost == self.lifted_topmost;
            let already_behind_box =
                GetWindow(self.hwnd, GW_HWNDNEXT).is_ok_and(|h| h == self.blur_hwnd);
            // Skip Win32 `SetWindowPos` whenever `blur_hwnd` is visible, in the right topmost band,
            // directly behind `self.hwnd`, and already holds the desired envelope bounds.
            let skip_swp = self.blur_visible
                && blur_is_vis
                && band_matches
                && already_behind_box
                && self.blur_hwnd_bounds == Some(desired_bounds);
            if !skip_swp {
                if IsIconic(self.blur_hwnd).as_bool() {
                    let _ = ShowWindow(self.blur_hwnd, SW_SHOWNOACTIVATE);
                }
                if self.blur_visible && blur_is_vis && band_matches && already_behind_box {
                    // Bounds-only update (e.g., collapse envelope resize): keep existing Z-order
                    // directly without asking DWM to re-sort `blur_hwnd` relative to `self.hwnd`.
                    let _ = SetWindowPos(
                        self.blur_hwnd,
                        None,
                        card_x,
                        card_y,
                        card_w,
                        hwnd_h,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING,
                    );
                } else {
                    if !band_matches {
                        let band_anchor = if self.lifted_topmost {
                            HWND_TOPMOST
                        } else {
                            HWND_NOTOPMOST
                        };
                        let _ = SetWindowPos(
                            self.blur_hwnd,
                            band_anchor,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                        );
                    }
                    let mut flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING;
                    if !self.blur_visible || !blur_is_vis {
                        flags |= SWP_SHOWWINDOW;
                    }
                    let _ = SetWindowPos(
                        self.blur_hwnd,
                        self.hwnd,
                        card_x,
                        card_y,
                        card_w,
                        hwnd_h,
                        flags,
                    );
                }
                self.blur_hwnd_bounds = Some(desired_bounds);
                self.blur_visible = true;
            }
        }
    }

    pub(super) fn sync_desktop_zorder(&mut self, force_reposition: bool) {
        let desktop_host = find_desktop_defview_host().map(|(h, _)| h);
        self.sync_desktop_zorder_with_scan(force_reposition, desktop_host, None);
    }

    pub(super) fn sync_desktop_zorder_with_scan(
        &mut self,
        force_reposition: bool,
        desktop_host: Option<HWND>,
        shared_scan: Option<&DesktopZOrderScan>,
    ) {
        if self.cleaned_up || self.hwnd.is_invalid() {
            return;
        }
        // Suppress & hide ALL DeskBox windows (both normal desktop boxes and edge auto-hide
        // drawers) whenever an exclusive or borderless fullscreen app covers this monitor.
        // This guarantees DeskBoxes never float above protected/anti-cheat fullscreen games
        // (whose HWNDs reject UIPI SetWindowPos anchors) and stops 30 FPS glass compositing
        // while gaming or watching fullscreen video.
        let fs_active = is_fullscreen_active_on_monitor(self.hwnd);
        if fs_active {
            if !self.fullscreen_suppressed || unsafe { IsWindowVisible(self.hwnd).as_bool() } {
                self.fullscreen_suppressed = true;
                self.edge_drawer_open = false;
                self.edge_animating = false;
                self.edge_slide_spring = Spring1D::still(0.0);
                self.edge_anim_from_t = 0.0;
                self.edge_anim_to_t = 0.0;
                self.edge_anim_elapsed = EDGE_ANIM_DUR_S;
                if self.spec.collapsed && (self.hover_expanded || self.collapse_animating) {
                    self.hover_expanded = false;
                    self.collapse_animating = false;
                    let layout = self.current_layout();
                    self.win_w = layout.win_w;
                    self.win_h = layout.win_h;
                    self.collapse_height_spring = Spring1D::still(layout.card_rect.height());
                    self.collapse_anim_to_h = layout.card_rect.height();
                } else {
                    self.collapse_animating = false;
                }
                self.scroll_animating = false;
                self.header_opacity_animating = false;
                self.clear_anim_body_cache();
                self.high_res_timer = None;
                self.cancel_inline_rename();
                if self.box_menu.open_kind_for(self.hwnd).is_some() {
                    self.box_menu.hide();
                }
                self.cancel_pending_hover_menu();
                self.suppress_hover_menu_btn = None;
                self.header_revealed_at = None;
                self.hide_tooltip();
                self.hide_blur_layer();
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_HOVER_COLLAPSE_ID);
                    let _ = KillTimer(self.hwnd, TIMER_BOX_ANIM_ID);
                    let _ = KillTimer(self.hwnd, TIMER_HOVER_MENU_ID);
                    if IsWindowVisible(self.hwnd).as_bool() {
                        let _ = ShowWindow(self.hwnd, SW_HIDE);
                    }
                }
            }
            return;
        }
        let restoring_from_fullscreen = self.fullscreen_suppressed;
        if restoring_from_fullscreen {
            self.fullscreen_suppressed = false;
            if self.spec.edge_auto_hide {
                self.apply_edge_drawer_bounds(0.0);
                unsafe {
                    self.allow_zorder_change = true;
                    let _ = promote_hwnd_to_topmost(
                        self.hwnd,
                        self.win_x,
                        self.win_y,
                        self.win_w,
                        self.win_h,
                        true,
                        true,
                    );
                    self.allow_zorder_change = false;
                }
                self.lifted_topmost = true;
                self.sync_blur_layer();
                let _ = self.render();
                return;
            }
        }

        let (is_iconic, is_vis, is_cloaked, blur_cloaked, rect_drifted) = unsafe {
            let iconic = IsIconic(self.hwnd).as_bool();
            let vis = IsWindowVisible(self.hwnd).as_bool();
            let cloaked = is_window_cloaked(self.hwnd);
            let b_cloaked = !self.blur_hwnd.is_invalid() && is_window_cloaked(self.blur_hwnd);
            let drifted = if self.drag.is_none() && !self.collapse_animating && !self.edge_animating
            {
                let mut rc = RECT::default();
                GetWindowRect(self.hwnd, &mut rc).is_ok()
                    && (rc.left != self.win_x
                        || rc.top != self.win_y
                        || (rc.right - rc.left) != self.win_w
                        || (rc.bottom - rc.top) != self.win_h)
            } else {
                false
            };
            (iconic, vis, cloaked, b_cloaked, drifted)
        };
        if is_cloaked {
            unsafe {
                let uncloak: BOOL = BOOL(0);
                let _ = DwmSetWindowAttribute(
                    self.hwnd,
                    DWMWA_CLOAK,
                    (&uncloak as *const BOOL).cast(),
                    size_of::<BOOL>() as u32,
                );
                // If Windows 11 Show Desktop (`Win+D`) shell-cloaked (`DWM_CLOAKED_SHELL`) this window,
                // bounce visibility non-actively (`SW_HIDE` -> `SW_SHOWNOACTIVATE`) to clear the cloak.
                if is_window_cloaked(self.hwnd) {
                    self.allow_zorder_change = true;
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                    let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    self.allow_zorder_change = false;
                }
            }
        }
        if blur_cloaked && !self.blur_hwnd.is_invalid() {
            unsafe {
                let uncloak: BOOL = BOOL(0);
                let _ = DwmSetWindowAttribute(
                    self.blur_hwnd,
                    DWMWA_CLOAK,
                    (&uncloak as *const BOOL).cast(),
                    size_of::<BOOL>() as u32,
                );
                if is_window_cloaked(self.blur_hwnd) {
                    let _ = ShowWindow(self.blur_hwnd, SW_HIDE);
                    if self.blur_visible {
                        let _ = ShowWindow(self.blur_hwnd, SW_SHOWNOACTIVATE);
                    }
                }
            }
        }
        if is_iconic {
            unsafe {
                self.allow_zorder_change = true;
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.allow_zorder_change = false;
            }
        }
        let need_restore = is_iconic || !is_vis || is_cloaked || blur_cloaked || rect_drifted;
        let effective_force = force_reposition || need_restore;

        if self.spec.edge_auto_hide {
            let mut need_blur_sync = !self.blur_visible || effective_force;
            unsafe {
                let is_vis_now = IsWindowVisible(self.hwnd).as_bool();
                let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
                let is_topmost = (ex & WS_EX_TOPMOST.0) != 0;
                if !is_topmost || effective_force || !is_vis_now {
                    need_blur_sync = true;
                    self.allow_zorder_change = true;
                    let mut flags = SWP_NOACTIVATE;
                    if !effective_force {
                        flags |= SWP_NOMOVE | SWP_NOSIZE;
                    }
                    if !is_vis_now {
                        flags |= SWP_SHOWWINDOW;
                    }
                    let _ = SetWindowPos(
                        self.hwnd,
                        HWND_TOPMOST,
                        self.win_x,
                        self.win_y,
                        self.win_w,
                        self.win_h,
                        flags,
                    );
                    self.allow_zorder_change = false;
                }
            }
            self.lifted_topmost = true;
            if need_blur_sync {
                self.sync_blur_layer();
            }
            if need_restore {
                let _ = self.render();
            }
            return;
        }
        let Some(desktop_host) = desktop_host else {
            if effective_force {
                unsafe {
                    self.allow_zorder_change = true;
                    let mut flags = SWP_NOACTIVATE;
                    if !IsWindowVisible(self.hwnd).as_bool() {
                        flags |= SWP_SHOWWINDOW;
                    }
                    let _ = SetWindowPos(
                        self.hwnd,
                        HWND_BOTTOM,
                        self.win_x,
                        self.win_y,
                        self.win_w,
                        self.win_h,
                        flags,
                    );
                    self.allow_zorder_change = false;
                }
                self.sync_blur_layer();
                if need_restore {
                    let _ = self.render();
                }
            }
            return;
        };

        let probe = match shared_scan {
            Some(scan) if scan.desktop_host == desktop_host => scan.probe_for(self.hwnd),
            _ => probe_desktop_zorder(self.hwnd, desktop_host),
        };
        let action = decide_deskbox_zorder(probe.snapshot, effective_force);

        match action {
            DeskBoxZOrderAction::Keep => {
                if effective_force {
                    unsafe {
                        let is_vis_now = IsWindowVisible(self.hwnd).as_bool();
                        let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
                        if !is_vis_now {
                            flags |= SWP_SHOWWINDOW;
                        }
                        let _ = SetWindowPos(
                            self.hwnd,
                            None,
                            self.win_x,
                            self.win_y,
                            self.win_w,
                            self.win_h,
                            flags,
                        );
                    }
                    self.sync_blur_layer();
                } else if self.has_live_blur
                    && !self.blur_hwnd.is_invalid()
                    && unsafe { !IsWindowVisible(self.blur_hwnd).as_bool() }
                {
                    self.sync_blur_layer();
                }
            }
            DeskBoxZOrderAction::LiftAboveRaisedDesktop => {
                let mut bounced_visibility = false;
                unsafe {
                    self.allow_zorder_change = true;
                    let is_vis_now = IsWindowVisible(self.hwnd).as_bool();
                    if !probe.snapshot.box_is_topmost
                        || probe.snapshot.is_box_below_progman
                        || effective_force
                        || !is_vis_now
                    {
                        bounced_visibility |= promote_hwnd_to_topmost(
                            self.hwnd,
                            self.win_x,
                            self.win_y,
                            self.win_w,
                            self.win_h,
                            effective_force,
                            !is_vis_now,
                        );
                    }
                    if let Some(after_top) = probe.lowest_same_band_visible_topmost {
                        let still_topmost = IsWindowVisible(after_top).as_bool()
                            && ((GetWindowLongPtrW(after_top, GWL_EXSTYLE) as u32)
                                & WS_EX_TOPMOST.0)
                                != 0;
                        if still_topmost {
                            let _ = SetWindowPos(
                                self.hwnd,
                                after_top,
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE
                                    | SWP_NOSIZE
                                    | SWP_NOACTIVATE
                                    | SWP_NOOWNERZORDER
                                    | SWP_NOSENDCHANGING,
                            );
                            // Guard against race where `after_top` dropped out of topmost mid-call and
                            // inadvertently demoted `self.hwnd` back behind a raised `Progman`.
                            let ex_after = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
                            if (ex_after & WS_EX_TOPMOST.0) == 0 {
                                bounced_visibility |= promote_hwnd_to_topmost(
                                    self.hwnd,
                                    self.win_x,
                                    self.win_y,
                                    self.win_w,
                                    self.win_h,
                                    false,
                                    false,
                                );
                            }
                        }
                    }
                    if !IsWindowVisible(self.hwnd).as_bool() {
                        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                        bounced_visibility = true;
                    }
                    self.allow_zorder_change = false;
                }
                self.lifted_topmost = true;
                self.sync_blur_layer();
                if bounced_visibility {
                    let _ = self.render();
                }
            }
            DeskBoxZOrderAction::NormalDesktop => {
                unsafe {
                    self.allow_zorder_change = true;
                    let is_vis_now = IsWindowVisible(self.hwnd).as_bool();
                    if probe.snapshot.box_is_topmost {
                        let _ = SetWindowPos(
                            self.hwnd,
                            HWND_NOTOPMOST,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE
                                | SWP_NOSIZE
                                | SWP_NOACTIVATE
                                | SWP_NOOWNERZORDER
                                | SWP_NOSENDCHANGING,
                        );
                    }

                    let mut pos_flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING;
                    if !effective_force {
                        pos_flags |= SWP_NOMOVE | SWP_NOSIZE;
                    }
                    if !is_vis_now {
                        pos_flags |= SWP_SHOWWINDOW;
                    }

                    let mut placed = false;
                    for &anchor in &probe.apps_above_progman_bottom_up {
                        if SetWindowPos(
                            self.hwnd,
                            anchor,
                            self.win_x,
                            self.win_y,
                            self.win_w,
                            self.win_h,
                            pos_flags,
                        )
                        .is_ok()
                        {
                            placed = true;
                            break;
                        }
                    }
                    if !placed {
                        if probe.apps_above_progman_bottom_up.is_empty() {
                            let _ = SetWindowPos(
                                self.hwnd,
                                HWND_TOP,
                                self.win_x,
                                self.win_y,
                                self.win_w,
                                self.win_h,
                                pos_flags,
                            );
                        } else if probe.snapshot.is_box_below_progman
                            || probe.snapshot.needs_normal_reanchor
                        {
                            // If elevated app anchors rejected `SetWindowPos`, walk upward from
                            // `desktop_host` to find any non-topmost anchor that accepts placement.
                            // Only fall back to `HWND_TOP` if the box is stranded below Progman
                            // (never promote above an elevated foreground app when already above Progman).
                            let mut cur = desktop_host;
                            let mut rescued = false;
                            for _ in 0..64 {
                                let Ok(prev) = GetWindow(cur, GW_HWNDPREV) else {
                                    break;
                                };
                                if prev.is_invalid() {
                                    break;
                                }
                                cur = prev;
                                if prev == self.hwnd || prev == self.blur_hwnd {
                                    continue;
                                }
                                let prev_ex = GetWindowLongPtrW(prev, GWL_EXSTYLE) as u32;
                                if (prev_ex & WS_EX_TOPMOST.0) != 0 {
                                    break;
                                }
                                if SetWindowPos(
                                    self.hwnd,
                                    prev,
                                    self.win_x,
                                    self.win_y,
                                    self.win_w,
                                    self.win_h,
                                    pos_flags,
                                )
                                .is_ok()
                                {
                                    rescued = true;
                                    break;
                                }
                            }
                            if !rescued && probe.snapshot.is_box_below_progman {
                                let _ = SetWindowPos(
                                    self.hwnd,
                                    HWND_TOP,
                                    self.win_x,
                                    self.win_y,
                                    self.win_w,
                                    self.win_h,
                                    pos_flags,
                                );
                            }
                        } else if effective_force || !is_vis_now {
                            let _ = SetWindowPos(
                                self.hwnd,
                                None,
                                self.win_x,
                                self.win_y,
                                self.win_w,
                                self.win_h,
                                pos_flags | SWP_NOZORDER,
                            );
                        }
                    }
                    if !IsWindowVisible(self.hwnd).as_bool() {
                        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    }
                    self.allow_zorder_change = false;
                }
                self.lifted_topmost = false;
                self.sync_blur_layer();
            }
        }
        if need_restore {
            let _ = self.render();
        }
    }

    pub(super) fn reattach_desktop_layer_internal(&mut self) {
        if self.cleaned_up || self.hwnd.is_invalid() {
            return;
        }
        let prev_live_blur = self.has_live_blur;
        self.sync_desktop_zorder(true);
        if self.has_live_blur != prev_live_blur {
            let _ = self.render();
        }
    }
}

/// Promotes `hwnd` into the topmost band with three-tier Win11 resilience:
/// 1. Standard `SetWindowPos(HWND_TOPMOST)` with `SWP_NOOWNERZORDER | SWP_NOSENDCHANGING`.
/// 2. Explicit `WS_EX_TOPMOST` style write + `SWP_FRAMECHANGED` if Tier 1 was ignored by Win11.
/// 3. Non-activating `SW_HIDE` -> `SetWindowPos(HWND_TOPMOST | SWP_SHOWWINDOW)` bounce if Win11
///    throttled background-window topmost promotion while `Progman` owned foreground.
///
/// Returns `true` if a visibility bounce occurred and callers should re-commit the layered surface.
unsafe fn promote_hwnd_to_topmost(
    hwnd: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    force_bounds: bool,
    force_show: bool,
) -> bool {
    let mut flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING;
    if !force_bounds {
        flags |= SWP_NOMOVE | SWP_NOSIZE;
    }
    if force_show {
        flags |= SWP_SHOWWINDOW;
    }
    let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, flags);
    if ((GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOPMOST.0) != 0 {
        return false;
    }

    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | WS_EX_TOPMOST.0) as isize);
    let _ = SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        x,
        y,
        w,
        h,
        flags | SWP_FRAMECHANGED,
    );
    if ((GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOPMOST.0) != 0 {
        return false;
    }

    let _ = ShowWindow(hwnd, SW_HIDE);
    let _ = SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        x,
        y,
        w,
        h,
        SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSENDCHANGING,
    );
    true
}

pub(super) fn backdrop_blur_params(style: GlassSurfaceStyle, light_glass: bool) -> Option<(f32, f32)> {
    match style {
        GlassSurfaceStyle::Default | GlassSurfaceStyle::Transparent => None,
        GlassSurfaceStyle::Blur => Some((18.0, 1.08)),
        GlassSurfaceStyle::Acrylic => Some((30.0, 1.16)),
        GlassSurfaceStyle::LiquidGlass => Some((24.0, if light_glass { 1.22 } else { 1.34 })),
    }
}

pub(super) fn update_rounded_visual_clip(
    comp: Option<&CompositionEngine>,
    visual: &SpriteVisual,
    cached_geo: &mut Option<CompositionRoundedRectangleGeometry>,
    blur_hwnd: HWND,
    width: f32,
    height: f32,
    radius: f32,
) -> bool {
    let w = width.max(1.0);
    let h = height.max(1.0);
    let r = radius.clamp(2.0, (w.min(h) * 0.45).max(2.0));
    if let Some(geo) = cached_geo.as_ref() {
        if geo.SetSize(Vector2 { X: w, Y: h }).is_ok()
            && geo.SetCornerRadius(Vector2 { X: r, Y: r }).is_ok()
        {
            return true;
        }
        *cached_geo = None;
    }
    if let Some(c) = comp {
        let clip_ok = (|| -> windows::core::Result<CompositionRoundedRectangleGeometry> {
            let geo = c.compositor.CreateRoundedRectangleGeometry()?;
            geo.SetSize(Vector2 { X: w, Y: h })?;
            geo.SetCornerRadius(Vector2 { X: r, Y: r })?;
            let base: CompositionGeometry = geo.cast()?;
            let clip = c.compositor.CreateGeometricClipWithGeometry(&base)?;
            visual.SetClip(&clip)?;
            Ok(geo)
        })();
        if let Ok(geo) = clip_ok {
            *cached_geo = Some(geo);
            return true;
        }
    }

    // Fallback to system rounded corner preference if Composition geometric clip isn't supported.
    unsafe {
        let round = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            blur_hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&round as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let no_border = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            blur_hwnd,
            DWMWA_BORDER_COLOR,
            (&no_border as *const u32).cast(),
            size_of::<u32>() as u32,
        );
    }
    true
}
