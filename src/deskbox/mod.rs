//! Desktop organizer module (`DeskBox`): persistent collection & live folder portal boxes.

pub mod config;
pub mod layout;
pub mod snap;

#[cfg(windows)]
mod box_wnd;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod dnd;
#[cfg(windows)]
mod icons;
#[cfg(windows)]
mod shell_menu;
#[cfg(windows)]
mod snap_overlay;
#[cfg(windows)]
mod watcher;

#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
pub use config::{BoxViewMode, DeskBoxTrayAction};

/// Z-order placement action chosen for a DeskBox window on the current pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum DeskBoxZOrderAction {
    /// Current Z-order is already in the desired band/position.
    Keep,
    /// Clear `WS_EX_TOPMOST` (if set) and park just above the desktop host and below normal app windows.
    NormalDesktop,
    ///Temporarily elevate into the lowest non-menu/non-taskbar topmost slot during Win+D (`RaisedDesktop`).
    LiftAboveRaisedDesktop,
}

/// Pure snapshot of top-level Z-order relationships around a single DeskBox window and desktop host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct DesktopZOrderSnapshot {
    pub box_is_topmost: bool,
    pub is_box_below_progman: bool,
    pub any_peer_below_progman: bool,
    pub any_peer_topmost: bool,
    pub any_real_app_below_progman: bool,
    pub any_real_app_above_progman: bool,
    pub third_party_app_foreground: bool,
    pub needs_normal_reanchor: bool,
    pub box_above_same_band_topmost: bool,
}

/// Decides whether `BoxWnd` should keep its Z-order, park at the normal desktop layer,
/// or temporarily lift above a Win11 24H2/26H2 `RaisedDesktop` (`Progman` during `Win+D`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn decide_deskbox_zorder(
    snap: DesktopZOrderSnapshot,
    force_apply: bool,
) -> DeskBoxZOrderAction {
    // During Win+D (`RaisedDesktop`), `Progman` is lifted above normal app windows while desktop/shell
    // owns foreground (`!third_party_app_foreground`). Keep all DeskBoxes lifted together when normal
    // apps sit below `Progman`, or when no real apps are above `Progman` and a peer slipped behind it.
    let raised_desktop_active = !snap.third_party_app_foreground
        && (snap.any_real_app_below_progman
            || (snap.any_peer_below_progman && !snap.any_real_app_above_progman));
    if raised_desktop_active {
        if force_apply
            || !snap.box_is_topmost
            || snap.is_box_below_progman
            || snap.box_above_same_band_topmost
        {
            return DeskBoxZOrderAction::LiftAboveRaisedDesktop;
        } else {
            return DeskBoxZOrderAction::Keep;
        }
    }

    let apps_active = snap.third_party_app_foreground || snap.any_real_app_above_progman;
    if apps_active {
        if force_apply
            || snap.box_is_topmost
            || snap.is_box_below_progman
            || snap.needs_normal_reanchor
        {
            DeskBoxZOrderAction::NormalDesktop
        } else {
            DeskBoxZOrderAction::Keep
        }
    } else if snap.box_is_topmost || snap.any_peer_topmost {
        if force_apply || !snap.box_is_topmost || snap.box_above_same_band_topmost {
            DeskBoxZOrderAction::LiftAboveRaisedDesktop
        } else {
            DeskBoxZOrderAction::Keep
        }
    } else if force_apply || snap.needs_normal_reanchor {
        DeskBoxZOrderAction::NormalDesktop
    } else {
        DeskBoxZOrderAction::Keep
    }
}

#[cfg(windows)]
pub use win::{
    pick_folder_for_new_portal, DeskBoxController, HOST_CMD_NEW_FOLDER_PORTAL,
    WM_APP_DESKBOX_HOST_CMD, WM_APP_DESKBOX_ZORDER_SYNC,
};

#[cfg(windows)]
mod win {
    use super::box_wnd::{
        BatchAutoFitInfo, BoxContextMenu, BoxTooltip, BoxWnd, PeerRegistry, HOST_CMD_DELETE_BOX,
        HOST_CMD_NEW_COLLECTION, HOST_CMD_PERSIST_CONFIG, WM_APP_DESKBOX_MENU_ACTION,
    };
    pub use super::box_wnd::{
        HOST_CMD_NEW_FOLDER_PORTAL, WM_APP_DESKBOX_HOST_CMD, WM_APP_DESKBOX_ZORDER_SYNC,
    };
    use super::config::{
        load_or_create, portable_config_path, save_config, BoxViewMode, DeskBoxConfig, DeskBoxSpec,
        DeskBoxTrayAction,
    };
    use super::desktop::{
        clear_zorder_notify_pending, find_desktop_defview_host, pick_folder_dialog,
        resolve_monitor_info, DeskBoxZOrderEventHook, DesktopIconGuard, DesktopZOrderScan,
    };
    use super::icons::IconService;
    use super::layout::clamp_box_bounds_to_work_area;
    use super::shell_menu::spawn_shell_menu_prewarm_once;
    use super::snap::{
        reflow_batch_auto_fit_chain, resolve_non_overlapping_move_rect, BoxRectPx, ChainBoxNode,
    };
    use super::snap_overlay::SnapGuideOverlay;
    use crate::ui::glass::CompositionEngine;
    use crate::ui::GlassSurfaceStyle;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Direct2D::ID2D1Factory1;
    use windows::Win32::Graphics::DirectWrite::IDWriteFactory;

    const ZORDER_STEADY_INTERVAL_MS: u64 = 600;
    const ZORDER_FOLLOWUP_INTERVAL_MS: u64 = 150;
    const ZORDER_FOLLOWUP_WINDOW_MS: u64 = 1200;

    /// Opens the modal folder picker dialog cleanly without holding any `RefCell` borrows.
    pub fn pick_folder_for_new_portal(owner: HWND) -> Option<PathBuf> {
        pick_folder_dialog(owner)
    }

    /// Top-level coordinator owned by the tray host managing all active `BoxWnd` instances and `DeskBox.toml`.
    pub struct DeskBoxController {
        config_path: PathBuf,
        config: DeskBoxConfig,
        host_hwnd: HWND,
        d2d_factory: ID2D1Factory1,
        dwrite_factory: IDWriteFactory,
        comp: Option<Rc<CompositionEngine>>,
        icon_service: Rc<RefCell<IconService>>,
        box_menu: Rc<BoxContextMenu>,
        box_tooltip: Rc<RefCell<BoxTooltip>>,
        snap_overlay: Rc<RefCell<SnapGuideOverlay>>,
        peer_rects: PeerRegistry,
        desktop_icons: DesktopIconGuard,
        zorder_hook: DeskBoxZOrderEventHook,
        boxes: Vec<BoxWnd>,
        surface_style: GlassSurfaceStyle,
        light_glass: bool,
        transparency_enabled: bool,
        last_zorder_check: Instant,
        zorder_followup_until: Option<Instant>,
    }

    impl DeskBoxController {
        pub fn new(
            host_hwnd: HWND,
            d2d_factory: ID2D1Factory1,
            dwrite_factory: IDWriteFactory,
            surface_style: GlassSurfaceStyle,
            light_glass: bool,
        ) -> windows::core::Result<Self> {
            let config_path = portable_config_path();
            let config = load_or_create(&config_path);
            let comp = CompositionEngine::try_new().map(Rc::new);
            let icon_service = Rc::new(RefCell::new(IconService::new()));
            let box_menu = Rc::new(BoxContextMenu::new(
                d2d_factory.clone(),
                dwrite_factory.clone(),
                WM_APP_DESKBOX_MENU_ACTION,
            )?);
            let box_tooltip = Rc::new(RefCell::new(BoxTooltip::new(
                d2d_factory.clone(),
                dwrite_factory.clone(),
            )?));
            let snap_overlay = Rc::new(RefCell::new(SnapGuideOverlay::new(d2d_factory.clone())?));
            let peer_rects = Rc::new(RefCell::new(HashMap::new()));
            let transparency_enabled = crate::system::is_transparency_enabled();

            let mut ctrl = Self {
                config_path,
                config,
                host_hwnd,
                d2d_factory,
                dwrite_factory,
                comp,
                icon_service,
                box_menu,
                box_tooltip,
                snap_overlay,
                peer_rects,
                desktop_icons: DesktopIconGuard::default(),
                zorder_hook: DeskBoxZOrderEventHook::default(),
                boxes: Vec::new(),
                surface_style,
                light_glass,
                transparency_enabled,
                last_zorder_check: Instant::now()
                    .checked_sub(Duration::from_millis(ZORDER_STEADY_INTERVAL_MS))
                    .unwrap_or_else(Instant::now),
                zorder_followup_until: None,
            };

            if ctrl.config.enabled {
                if ctrl.config.boxes.is_empty() {
                    ctrl.create_collection_box("桌面收纳".to_string());
                } else {
                    ctrl.spawn_all_configured_boxes();
                    ctrl.desktop_icons.set_hidden(ctrl.config.hide_desktop_icons);
                }
            }
            ctrl.sync_zorder_hook();
            Ok(ctrl)
        }

        pub fn is_enabled(&self) -> bool {
            self.config.enabled
        }

        pub fn hide_desktop_icons(&self) -> bool {
            self.config.hide_desktop_icons
        }

        pub fn default_view_mode(&self) -> BoxViewMode {
            self.config.default_view_mode
        }

        pub fn needs_timer(&self) -> bool {
            self.config.enabled && !self.boxes.is_empty()
        }

        pub fn is_animating(&self) -> bool {
            self.boxes.iter().any(|b| b.is_animating())
        }

        pub fn needs_fast_tick(&self) -> bool {
            let followup = self
                .zorder_followup_until
                .is_some_and(|until| Instant::now() < until);
            self.needs_timer()
                && (self.surface_style == GlassSurfaceStyle::LiquidGlass || followup)
        }

        pub fn handle_tray_action(&mut self, action: DeskBoxTrayAction) {
            match action {
                DeskBoxTrayAction::ToggleEnabled => {
                    let next = !self.config.enabled;
                    self.set_enabled(next);
                }
                DeskBoxTrayAction::ToggleHideDesktopIcons => {
                    self.config.hide_desktop_icons = !self.config.hide_desktop_icons;
                    if self.config.enabled {
                        self.desktop_icons.set_hidden(self.config.hide_desktop_icons);
                    }
                    self.save();
                }
                DeskBoxTrayAction::AutoFitAll => {
                    if self.config.enabled && !self.boxes.is_empty() {
                        self.auto_fit_all_boxes_with_chain();
                        self.sync_specs_from_windows();
                        self.save();
                    }
                }
                DeskBoxTrayAction::LockAll => {
                    for spec in &mut self.config.boxes {
                        spec.locked = true;
                    }
                    for b in &mut self.boxes {
                        b.set_locked(true);
                    }
                    self.sync_specs_from_windows();
                    self.save();
                }
                DeskBoxTrayAction::UnlockAll => {
                    for spec in &mut self.config.boxes {
                        spec.locked = false;
                    }
                    for b in &mut self.boxes {
                        b.set_locked(false);
                    }
                    self.sync_specs_from_windows();
                    self.save();
                }
                DeskBoxTrayAction::SetDefaultView(mode) => {
                    self.config.default_view_mode = mode;
                    for spec in &mut self.config.boxes {
                        spec.view_mode = mode;
                    }
                    for b in &mut self.boxes {
                        b.set_view_mode(mode);
                    }
                    self.sync_specs_from_windows();
                    self.save();
                }
            }
        }

        /// Handles commands posted from an individual `BoxWnd` or its context menu.
        /// Returns `true` if the tray host needs to open the folder picker dialog (`HOST_CMD_NEW_FOLDER_PORTAL`).
        pub fn handle_host_cmd(&mut self, cmd: usize, box_id: u64) -> bool {
            match cmd {
                HOST_CMD_NEW_COLLECTION => {
                    self.config.enabled = true;
                    self.create_collection_box("新建收纳格".to_string());
                    self.sync_zorder_hook();
                    false
                }
                HOST_CMD_NEW_FOLDER_PORTAL => true,
                HOST_CMD_DELETE_BOX => {
                    self.delete_box_by_id(box_id);
                    false
                }
                HOST_CMD_PERSIST_CONFIG => {
                    self.sync_specs_from_windows();
                    self.save();
                    false
                }
                _ => false,
            }
        }

        pub fn create_folder_portal_box(&mut self, folder: PathBuf) {
            if !folder.is_dir() {
                return;
            }
            if !self.config.enabled {
                self.config.enabled = true;
            }
            if self.boxes.is_empty() && !self.config.boxes.is_empty() {
                self.spawn_all_configured_boxes();
            }

            let mon = resolve_monitor_info("");
            let (ox, oy) = DeskBoxConfig::default_cascade_origin(
                self.config.boxes.len(),
                mon.work.left,
                mon.work.top,
                mon.work.right,
                mon.work.bottom,
                mon.dpi,
            );
            let id = self.config.alloc_box_id();
            let mut spec = DeskBoxSpec::new_folder_portal(
                id,
                folder.to_string_lossy().into_owned(),
                ox,
                oy,
            );
            spec.view_mode = self.config.default_view_mode;
            spec.monitor_id = mon.monitor_id.clone();
            self.place_spec_without_overlap(&mut spec, mon.work, mon.dpi);
            if self.spawn_one_box(spec.clone()) {
                self.config.boxes.push(spec);
                self.desktop_icons.set_hidden(self.config.hide_desktop_icons);
                self.sync_zorder_hook();
                self.save();
            }
        }

        pub fn refresh_theme(&mut self, light_glass: bool) {
            self.light_glass = light_glass;
            self.transparency_enabled = crate::system::is_transparency_enabled();
            for b in &mut self.boxes {
                b.set_appearance(self.surface_style, light_glass, self.transparency_enabled);
            }
        }

        pub fn refresh_surface_style(&mut self, style: GlassSurfaceStyle) {
            self.surface_style = style;
            self.transparency_enabled = crate::system::is_transparency_enabled();
            for b in &mut self.boxes {
                b.set_appearance(style, self.light_glass, self.transparency_enabled);
            }
        }

        pub fn on_display_change(&mut self) {
            for b in &mut self.boxes {
                b.on_display_change();
            }
            self.sync_specs_from_windows();
            self.save();
        }

        pub fn on_taskbar_created(&mut self) {
            if self.config.enabled {
                for b in &mut self.boxes {
                    b.reattach_desktop_layer();
                }
                self.desktop_icons.reapply_after_shell_restart();
            }
        }

        pub fn sync_zorder_now(&mut self) {
            clear_zorder_notify_pending();
            if !self.needs_timer() {
                return;
            }
            let now = Instant::now();
            self.last_zorder_check = now;
            self.zorder_followup_until =
                Some(now + Duration::from_millis(ZORDER_FOLLOWUP_WINDOW_MS));
            self.maintain_all_boxes_zorder();
        }

        pub fn tick(&mut self) {
            if !self.needs_timer() {
                return;
            }
            let now = Instant::now();
            let popup_anim_busy = crate::ui::is_interactive_anim_active(160);
            let any_box_animating = self.boxes.iter().any(|b| b.is_animating());
            let in_followup = match self.zorder_followup_until {
                Some(until) if now < until => true,
                Some(_) => {
                    self.zorder_followup_until = None;
                    false
                }
                None => false,
            };
            let zorder_interval = if in_followup {
                Duration::from_millis(ZORDER_FOLLOWUP_INTERVAL_MS)
            } else {
                Duration::from_millis(ZORDER_STEADY_INTERVAL_MS)
            };
            if ((!popup_anim_busy && !any_box_animating) || in_followup)
                && now.duration_since(self.last_zorder_check) >= zorder_interval
            {
                self.last_zorder_check = now;
                self.maintain_all_boxes_zorder();
            }

            if self.surface_style == GlassSurfaceStyle::LiquidGlass {
                for b in &mut self.boxes {
                    b.tick_flow();
                }
            }
        }

        fn maintain_all_boxes_zorder(&mut self) {
            if self.boxes.is_empty() {
                return;
            }
            let desktop_host = find_desktop_defview_host().map(|(h, _)| h);
            let box_hwnds: Vec<HWND> = self
                .boxes
                .iter()
                .filter(|b| !b.is_edge_auto_hide())
                .map(|b| b.hwnd())
                .collect();
            let ref_hwnd = box_hwnds.first().copied().unwrap_or_default();
            let scan = desktop_host.map(|dh| DesktopZOrderScan::capture(dh, ref_hwnd, &box_hwnds));
            for b in &mut self.boxes {
                b.maintain_desktop_zorder_with_scan(desktop_host, scan.as_ref());
            }
        }

        pub fn shutdown_and_restore(&mut self) {
            self.zorder_hook
                .set_active(false, self.host_hwnd, WM_APP_DESKBOX_ZORDER_SYNC);
            self.sync_specs_from_windows();
            if self.config.enabled {
                let _ = save_config(&self.config_path, &self.config);
            }
            self.box_menu.hide();
            if let Ok(mut tip) = self.box_tooltip.try_borrow_mut() {
                tip.hide();
            }
            if let Ok(mut snap) = self.snap_overlay.try_borrow_mut() {
                snap.hide();
            }
            if let Ok(mut peers) = self.peer_rects.try_borrow_mut() {
                peers.clear();
            }
            self.boxes.clear();
            self.desktop_icons.restore_now();
        }

        fn set_enabled(&mut self, enable: bool) {
            if enable == self.config.enabled {
                return;
            }
            self.config.enabled = enable;
            if enable {
                if self.config.boxes.is_empty() {
                    self.create_collection_box("桌面收纳".to_string());
                } else {
                    self.spawn_all_configured_boxes();
                }
                self.desktop_icons.set_hidden(self.config.hide_desktop_icons);
            } else {
                self.sync_specs_from_windows();
                self.box_menu.hide();
                if let Ok(mut tip) = self.box_tooltip.try_borrow_mut() {
                    tip.hide();
                }
                self.boxes.clear();
                self.desktop_icons.restore_now();
            }
            self.sync_zorder_hook();
            self.save();
        }

        fn create_collection_box(&mut self, title: String) {
            let mon = resolve_monitor_info("");
            let (ox, oy) = DeskBoxConfig::default_cascade_origin(
                self.config.boxes.len(),
                mon.work.left,
                mon.work.top,
                mon.work.right,
                mon.work.bottom,
                mon.dpi,
            );
            let id = self.config.alloc_box_id();
            let mut spec = DeskBoxSpec::new_collection(id, title, ox, oy);
            spec.view_mode = self.config.default_view_mode;
            spec.monitor_id = mon.monitor_id.clone();
            self.place_spec_without_overlap(&mut spec, mon.work, mon.dpi);
            if self.spawn_one_box(spec.clone()) {
                self.config.boxes.push(spec);
                self.desktop_icons.set_hidden(self.config.hide_desktop_icons);
                self.sync_zorder_hook();
                self.save();
            }
        }

        fn delete_box_by_id(&mut self, box_id: u64) {
            self.box_menu.hide();
            if let Ok(mut tip) = self.box_tooltip.try_borrow_mut() {
                tip.hide();
            }
            if let Ok(mut snap) = self.snap_overlay.try_borrow_mut() {
                snap.hide();
            }
            if let Ok(mut peers) = self.peer_rects.try_borrow_mut() {
                peers.remove(&box_id);
            }
            self.boxes.retain(|b| b.box_id() != box_id);
            self.config.boxes.retain(|s| s.id != box_id);
            if self.config.boxes.is_empty() && self.config.enabled {
                self.config.enabled = false;
                self.desktop_icons.restore_now();
            }
            self.sync_zorder_hook();
            self.save();
        }

        fn sync_zorder_hook(&mut self) {
            let active = self.needs_timer();
            self.zorder_hook
                .set_active(active, self.host_hwnd, WM_APP_DESKBOX_ZORDER_SYNC);
        }

        fn spawn_all_configured_boxes(&mut self) {
            if !self.boxes.is_empty() {
                return;
            }
            let specs = self.config.boxes.clone();
            for spec in specs {
                self.spawn_one_box(spec);
            }
            self.sync_specs_from_windows();
        }

        fn spawn_one_box(&mut self, spec: DeskBoxSpec) -> bool {
            if let Ok(bw) = BoxWnd::create(
                self.host_hwnd,
                self.d2d_factory.clone(),
                self.dwrite_factory.clone(),
                self.comp.clone(),
                Rc::clone(&self.icon_service),
                Rc::clone(&self.box_menu),
                Rc::clone(&self.box_tooltip),
                Rc::clone(&self.snap_overlay),
                Rc::clone(&self.peer_rects),
                spec,
                self.surface_style,
                self.light_glass,
                self.transparency_enabled,
            ) {
                self.boxes.push(bw);
                spawn_shell_menu_prewarm_once();
                true
            } else {
                false
            }
        }

        fn place_spec_without_overlap(
            &self,
            spec: &mut DeskBoxSpec,
            work: windows::Win32::Foundation::RECT,
            dpi: u32,
        ) {
            let s = (dpi.max(96) as f32) / 96.0;
            let pad = (crate::ui::SHADOW_PAD_DIP * s).round() as i32;
            let margin = (6.0 * s).round() as i32;
            let (_, _, _, _, wx, wy, ww, wh) = clamp_box_bounds_to_work_area(
                spec.x_dip,
                spec.y_dip,
                spec.w_dip,
                spec.h_dip,
                spec.collapsed,
                work.left,
                work.top,
                work.right,
                work.bottom,
                dpi,
            );
            let desired = BoxRectPx::new(
                spec.id,
                wx + pad,
                wy + pad,
                wx + ww - pad,
                wy + wh - pad,
            );
            let work_px = BoxRectPx::new(0, work.left, work.top, work.right, work.bottom);
            let peers: Vec<BoxRectPx> = self
                .peer_rects
                .try_borrow()
                .map(|m| m.values().map(|e| e.rect).collect())
                .unwrap_or_default();
            let resolved =
                resolve_non_overlapping_move_rect(desired, desired, &peers, work_px, dpi);
            spec.x_dip = ((resolved.left - work.left - margin) as f32 / s).max(0.0);
            spec.y_dip = ((resolved.top - work.top - margin) as f32 / s).max(0.0);
        }

        fn auto_fit_all_boxes_with_chain(&mut self) {
            struct MonitorGroup {
                work: windows::Win32::Foundation::RECT,
                dpi: u32,
                entries: Vec<(usize, BatchAutoFitInfo)>,
            }
            let mut groups: HashMap<String, MonitorGroup> = HashMap::new();
            for (idx, b) in self.boxes.iter_mut().enumerate() {
                if b.is_edge_auto_hide() {
                    b.auto_fit_size();
                    continue;
                }
                if let Some(info) = b.prepare_batch_auto_fit() {
                    groups
                        .entry(info.monitor_id.clone())
                        .or_insert_with(|| MonitorGroup {
                            work: info.work,
                            dpi: info.dpi,
                            entries: Vec::new(),
                        })
                        .entries
                        .push((idx, info));
                }
            }

            for group in groups.into_values() {
                let nodes: Vec<ChainBoxNode> =
                    group.entries.iter().map(|(_, info)| info.node).collect();
                let targets: Vec<(u64, i32, i32)> = group
                    .entries
                    .iter()
                    .map(|(_, info)| (info.node.rect.id, info.target_rest_w, info.target_rest_h))
                    .collect();
                let work_px = BoxRectPx::new(
                    0,
                    group.work.left,
                    group.work.top,
                    group.work.right,
                    group.work.bottom,
                );
                let resolved = reflow_batch_auto_fit_chain(&nodes, &targets, work_px, group.dpi);
                for ((box_idx, info), rect) in group.entries.into_iter().zip(resolved) {
                    self.boxes[box_idx].apply_batch_auto_fit(
                        rect,
                        info.fit_w_dip,
                        info.fit_h_dip,
                        group.work,
                        group.dpi,
                    );
                }
            }
        }

        fn sync_specs_from_windows(&mut self) {
            if self.boxes.is_empty() {
                return;
            }
            for bw in &self.boxes {
                let updated = bw.spec();
                if let Some(target) = self.config.boxes.iter_mut().find(|s| s.id == updated.id) {
                    *target = updated;
                }
            }
        }

        fn save(&self) {
            let _ = save_config(&self.config_path, &self.config);
        }
    }

    impl Drop for DeskBoxController {
        fn drop(&mut self) {
            self.shutdown_and_restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{decide_deskbox_zorder, DeskBoxZOrderAction, DesktopZOrderSnapshot};

    #[test]
    fn keeps_normal_desktop_order_while_apps_cover_desktop() {
        let snap = DesktopZOrderSnapshot {
            box_is_topmost: false,
            is_box_below_progman: false,
            any_peer_below_progman: false,
            any_peer_topmost: false,
            any_real_app_below_progman: false,
            any_real_app_above_progman: true,
            third_party_app_foreground: true,
            needs_normal_reanchor: false,
            box_above_same_band_topmost: false,
        };
        assert_eq!(decide_deskbox_zorder(snap, false), DeskBoxZOrderAction::Keep);
        assert_eq!(decide_deskbox_zorder(snap, true), DeskBoxZOrderAction::NormalDesktop);
    }

    #[test]
    fn reanchors_normal_desktop_when_box_dropped_below_progman_with_active_apps() {
        let below = DesktopZOrderSnapshot {
            any_real_app_above_progman: true,
            is_box_below_progman: true,
            any_peer_below_progman: true,
            ..Default::default()
        };
        assert_eq!(decide_deskbox_zorder(below, false), DeskBoxZOrderAction::NormalDesktop);

        let misplaced = DesktopZOrderSnapshot {
            any_real_app_above_progman: true,
            needs_normal_reanchor: true,
            ..Default::default()
        };
        assert_eq!(decide_deskbox_zorder(misplaced, false), DeskBoxZOrderAction::NormalDesktop);
    }

    #[test]
    fn lifts_above_raised_desktop_on_win_d_or_startup_and_stays_steady() {
        let win_d_enter = DesktopZOrderSnapshot {
            box_is_topmost: false,
            is_box_below_progman: true,
            any_peer_below_progman: true,
            any_peer_topmost: false,
            any_real_app_below_progman: true,
            any_real_app_above_progman: false,
            third_party_app_foreground: false,
            needs_normal_reanchor: false,
            box_above_same_band_topmost: false,
        };
        assert_eq!(
            decide_deskbox_zorder(win_d_enter, false),
            DeskBoxZOrderAction::LiftAboveRaisedDesktop
        );
        assert_eq!(
            decide_deskbox_zorder(win_d_enter, true),
            DeskBoxZOrderAction::LiftAboveRaisedDesktop
        );

        let win_d_steady = DesktopZOrderSnapshot {
            box_is_topmost: true,
            is_box_below_progman: false,
            any_peer_below_progman: false,
            any_peer_topmost: true,
            any_real_app_below_progman: true,
            any_real_app_above_progman: false,
            third_party_app_foreground: false,
            needs_normal_reanchor: false,
            box_above_same_band_topmost: false,
        };
        assert_eq!(decide_deskbox_zorder(win_d_steady, false), DeskBoxZOrderAction::Keep);

        // Peer consistency: if all apps are minimized and one DeskBox is already lifted topmost,
        // non-topmost sibling boxes lift together so Win+D cannot sweep one box behind wallpaper.
        let sibling_lifted = DesktopZOrderSnapshot {
            box_is_topmost: false,
            any_peer_topmost: true,
            ..Default::default()
        };
        assert_eq!(
            decide_deskbox_zorder(sibling_lifted, false),
            DeskBoxZOrderAction::LiftAboveRaisedDesktop
        );

        // Even if all apps were manually minimized before Win+D (no apps below Progman),
        // staying topmost avoids ping-pong once lifted.
        let all_min_lifted = DesktopZOrderSnapshot {
            box_is_topmost: true,
            ..Default::default()
        };
        assert_eq!(decide_deskbox_zorder(all_min_lifted, false), DeskBoxZOrderAction::Keep);

        // If Win+D raises Progman above an already-topmost box, we MUST re-lift it immediately.
        let topmost_dropped_below_progman = DesktopZOrderSnapshot {
            box_is_topmost: true,
            is_box_below_progman: true,
            any_real_app_below_progman: true,
            ..Default::default()
        };
        assert_eq!(
            decide_deskbox_zorder(topmost_dropped_below_progman, false),
            DeskBoxZOrderAction::LiftAboveRaisedDesktop
        );
    }

    #[test]
    fn tucks_topmost_box_under_same_band_tray_or_menus_when_raised_above_them() {
        let above_tray = DesktopZOrderSnapshot {
            box_is_topmost: true,
            any_real_app_below_progman: true,
            box_above_same_band_topmost: true,
            ..Default::default()
        };
        assert_eq!(
            decide_deskbox_zorder(above_tray, false),
            DeskBoxZOrderAction::LiftAboveRaisedDesktop
        );
    }

    #[test]
    fn clears_topmost_immediately_when_app_activates_or_restores() {
        let app_fg = DesktopZOrderSnapshot {
            box_is_topmost: true,
            third_party_app_foreground: true,
            ..Default::default()
        };
        assert_eq!(decide_deskbox_zorder(app_fg, false), DeskBoxZOrderAction::NormalDesktop);

        let app_restored = DesktopZOrderSnapshot {
            box_is_topmost: true,
            any_real_app_above_progman: true,
            ..Default::default()
        };
        assert_eq!(decide_deskbox_zorder(app_restored, false), DeskBoxZOrderAction::NormalDesktop);

        // During Win+D (`RaisedDesktop`), if a stubborn background/secondary-monitor window stays
        // above Progman while normal apps are below Progman and desktop owns foreground, keep lifted.
        let win_d_with_stray_bg_window = DesktopZOrderSnapshot {
            box_is_topmost: false,
            is_box_below_progman: true,
            any_peer_below_progman: true,
            any_peer_topmost: false,
            any_real_app_below_progman: true,
            any_real_app_above_progman: true,
            third_party_app_foreground: false,
            needs_normal_reanchor: false,
            box_above_same_band_topmost: false,
        };
        assert_eq!(
            decide_deskbox_zorder(win_d_with_stray_bg_window, false),
            DeskBoxZOrderAction::LiftAboveRaisedDesktop
        );

        // Normal desktop where one DeskBox peer transiently sits below Progman while a real app
        // window is still above Progman (not Win+D): must stay in NormalDesktop and NOT lift
        // boxes into TOPMOST over the user's open application.
        let stray_peer_with_real_app = DesktopZOrderSnapshot {
            any_real_app_above_progman: true,
            any_real_app_below_progman: false,
            is_box_below_progman: true,
            any_peer_below_progman: true,
            third_party_app_foreground: false,
            ..Default::default()
        };
        assert_eq!(
            decide_deskbox_zorder(stray_peer_with_real_app, false),
            DeskBoxZOrderAction::NormalDesktop
        );
    }
}
