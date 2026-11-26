//! General/shape/shell-card target tracking, handle detach/purge, and BrushPropKind read/write/clear ops.

use super::super::super::tree_match::TargetKind;
use super::super::raw_com::*;
use super::*;
use std::ffi::c_void;
use windows::core::HRESULT;


pub(in super::super) unsafe fn track_general_modified_target(
    key: u64,
    kind: TargetKind,
    insp_ptr: *mut c_void,
) {
    if key == 0 || insp_ptr.is_null() {
        return;
    }
    let (existing_weak, need_prune) = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .map(|s| {
                let existing = s
                    .general_modified_targets
                    .get(&key)
                    .map(|&(k, ref w)| (k, w.clone_ref()));
                let prune = existing.is_none() && s.general_modified_targets.len() >= 128;
                (existing, prune)
            })
            .unwrap_or((None, false))
    });
    if let Some((existing_kind, ew)) = existing_weak {
        if weak_matches_live_object(&ew, insp_ptr) {
            if existing_kind != kind {
                UI_BRUSH_STORE.with(|cell| {
                    if let Ok(mut s) = cell.try_borrow_mut() {
                        if let Some((k, _)) = s.general_modified_targets.get_mut(&key) {
                            *k = kind;
                        }
                    }
                });
            }
            return;
        }
        purge_saved_brushes_for_current_thread(&[key]);
    }
    if need_prune {
        let candidates: Vec<(u64, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .map(|s| {
                    s.general_modified_targets
                        .iter()
                        .map(|(&k, (_, w))| (k, w.clone_ref()))
                        .collect()
                })
                .unwrap_or_default()
        });
        let dead: Vec<u64> = candidates
            .into_iter()
            .filter_map(|(ck, w)| resolve_weak_inspectable(&w).is_none().then_some(ck))
            .collect();
        if !dead.is_empty() {
            let removed: Vec<(TargetKind, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
                let Ok(mut s) = cell.try_borrow_mut() else {
                    return Vec::new();
                };
                dead.iter()
                    .filter_map(|k| s.general_modified_targets.remove(k))
                    .collect()
            });
            drop(removed);
            purge_saved_brushes_for_current_thread(&dead);
        }
    }
    let Some(weak_ref) = make_weak_ref_com(insp_ptr) else {
        return;
    };
    let displaced = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if let std::collections::hash_map::Entry::Vacant(e) =
                s.general_modified_targets.entry(key)
            {
                e.insert((kind, weak_ref));
                return None;
            }
        }
        Some(weak_ref)
    });
    drop(displaced);
}

pub(in super::super) unsafe fn track_modified_shape(
    key: u64,
    insp_ptr: *mut c_void,
    orig_opacity: f64,
    orig_visibility: i32,
) {
    if key == 0 || insp_ptr.is_null() {
        return;
    }
    let (existing_weak, need_prune) = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .map(|s| {
                let existing = s.modified_shapes.get(&key).map(|(w, _, _)| w.clone_ref());
                let prune = existing.is_none() && s.modified_shapes.len() >= 128;
                (existing, prune)
            })
            .unwrap_or((None, false))
    });
    if let Some(ew) = existing_weak {
        if weak_matches_live_object(&ew, insp_ptr) {
            return;
        }
        purge_saved_brushes_for_current_thread(&[key]);
    }
    if need_prune {
        let candidates: Vec<(u64, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .map(|s| {
                    s.modified_shapes
                        .iter()
                        .map(|(&k, (w, _, _))| (k, w.clone_ref()))
                        .collect()
                })
                .unwrap_or_default()
        });
        let dead: Vec<u64> = candidates
            .into_iter()
            .filter_map(|(ck, w)| resolve_weak_inspectable(&w).is_none().then_some(ck))
            .collect();
        if !dead.is_empty() {
            let removed: Vec<(ComRaw, f64, i32)> = UI_BRUSH_STORE.with(|cell| {
                let Ok(mut s) = cell.try_borrow_mut() else {
                    return Vec::new();
                };
                dead.iter()
                    .filter_map(|k| s.modified_shapes.remove(k))
                    .collect()
            });
            drop(removed);
        }
    }
    let Some(weak_ref) = make_weak_ref_com(insp_ptr) else {
        return;
    };
    let displaced = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if let std::collections::hash_map::Entry::Vacant(e) = s.modified_shapes.entry(key) {
                e.insert((weak_ref, orig_opacity, orig_visibility));
                return None;
            }
        }
        Some(weak_ref)
    });
    drop(displaced);
}

pub(in super::super) fn remove_general_modified_target(key: u64) {
    let removed = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut s| s.general_modified_targets.remove(&key))
    });
    drop(removed);
}

pub(in super::super) fn remove_modified_shape(key: u64) -> Option<(f64, i32)> {
    let removed = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut s| s.modified_shapes.remove(&key))
    });
    removed.map(|(_, op, vis)| (op, vis))
}

pub(in super::super) unsafe fn track_shell_card_modified_target(surface_key: u64, surface_insp: *mut c_void) {
    track_paired_modified_target(
        surface_key,
        surface_insp,
        None,
        |s| &mut s.shell_card_modified_targets,
        |s| &s.shell_card_modified_targets,
    );
}

pub(in super::super) fn take_saved_target_brushes(handle: u64) -> SavedTargetBrushes {
    UI_BRUSH_STORE
        .with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.saved.remove(&handle))
        })
        .unwrap_or_default()
}

pub(in super::super) fn register_handle_identity(handle: u64, identity_key: u64) {
    if handle == 0 || identity_key == 0 || handle == identity_key {
        return;
    }
    UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            s.handle_to_identity.insert(handle, identity_key);
        }
    });
}

/// Detaches removed XAML Diagnostics `InstanceHandle`s from `handle_to_identity` without purging
/// live cached COM objects tracked by weak reference in `UI_BRUSH_STORE`.
pub(in super::super) fn detach_handles_for_current_thread(handles: &[u64]) {
    if handles.is_empty() {
        return;
    }
    UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            for &h in handles {
                if h != 0 {
                    s.handle_to_identity.remove(&h);
                }
            }
        }
    });
}

/// Safely purges stale `InstanceHandle`s whose weak reference failed to resolve, skipping any
/// `com_identity_key` that has already been reused (ABA) by a still-live tracked COM object.
pub(in super::super) unsafe fn purge_stale_handles_for_current_thread(handles: &[u64]) {
    if handles.is_empty() {
        return;
    }
    let mut direct_purge: Vec<u64> = Vec::with_capacity(handles.len());
    let mut id_candidates: Vec<(u64, Vec<ComRaw>)> = Vec::new();
    UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return;
        };
        for &h in handles {
            if h == 0 {
                continue;
            }
            direct_purge.push(h);
            if let Some(id_key) = s.handle_to_identity.remove(&h) {
                if id_key != 0 && id_key != h {
                    let mut weaks = Vec::new();
                    if let Some((_, w)) = s.general_modified_targets.get(&id_key) {
                        weaks.push(w.clone_ref());
                    }
                    if let Some((w, _, _)) = s.modified_shapes.get(&id_key) {
                        weaks.push(w.clone_ref());
                    }
                    if let Some((_, w)) = s.alttab_subtargets.get(&id_key) {
                        weaks.push(w.clone_ref());
                    }
                    if let Some((w, _, _)) = s.hover_flyout_shapes.get(&id_key) {
                        weaks.push(w.clone_ref());
                    }
                    if let Some(e) = s.tooltip_modified_targets.get(&id_key) {
                        weaks.push(e.presenter_weak.clone_ref());
                    }
                    if let Some(e) = s.menu_flyout_modified_targets.get(&id_key) {
                        weaks.push(e.presenter_weak.clone_ref());
                    }
                    if let Some(e) = s.shell_card_modified_targets.get(&id_key) {
                        weaks.push(e.presenter_weak.clone_ref());
                    }
                    id_candidates.push((id_key, weaks));
                }
            }
        }
    });
    for (id_key, weaks) in id_candidates {
        let any_alive = weaks
            .iter()
            .any(|w| resolve_weak_inspectable(w).is_some());
        if !any_alive {
            direct_purge.push(id_key);
        }
    }
    purge_saved_brushes_for_current_thread(&direct_purge);
}

pub(in super::super) fn purge_saved_brushes_for_current_thread(handles: &[u64]) {
    if handles.is_empty() {
        return;
    }
    let (
        removed_brushes,
        removed_watches,
        removed_general,
        removed_shapes,
        removed_alttab_sub,
        removed_hover_shapes,
        removed_tooltip_mod,
        removed_menu_mod,
        removed_shell_mod,
    ) = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
        };
        // Resolve InstanceHandle -> com_identity_key so removal/eviction purges both namespaces.
        let mut keys: Vec<u64> = Vec::with_capacity(handles.len() * 2);
        for &h in handles {
            if h != 0 {
                keys.push(h);
                if let Some(id_key) = s.handle_to_identity.remove(&h) {
                    if id_key != 0 && id_key != h {
                        keys.push(id_key);
                    }
                }
            }
        }
        if !s.tooltip_orig_corner_radius.is_empty() {
            for &k in &keys {
                s.tooltip_orig_corner_radius.remove(&k);
            }
        }
        if !s.menu_flyout_orig_corner_radius.is_empty() {
            for &k in &keys {
                s.menu_flyout_orig_corner_radius.remove(&k);
            }
        }
        if !s.shell_card_orig_corner_radius.is_empty() {
            for &k in &keys {
                s.shell_card_orig_corner_radius.remove(&k);
            }
        }
        if !s.shell_card_orig_comp_visible.is_empty() {
            for &k in &keys {
                s.shell_card_orig_comp_visible.remove(&k);
            }
        }
        if !s.start_modal_applied_style.is_empty() {
            for &k in &keys {
                s.start_modal_applied_style.remove(&k);
            }
        }
        let brushes: Vec<SavedTargetBrushes> = if s.saved.is_empty() {
            Vec::new()
        } else {
            keys.iter().filter_map(|k| s.saved.remove(k)).collect()
        };
        if !s.alttab_item_orig_comp_visible.is_empty() {
            for &k in &keys {
                s.alttab_item_orig_comp_visible.remove(&k);
            }
        }
        if !s.alttab_item_orig_focus_thickness.is_empty() {
            for &k in &keys {
                s.alttab_item_orig_focus_thickness.remove(&k);
            }
        }
        let mut watches = Vec::new();
        if !s.watches.is_empty() {
            for &k in &keys {
                for prop_idx in 0u8..BrushPropKind::COUNT {
                    if let Some(entry) = s.watches.remove(&(k, prop_idx)) {
                        watches.push(entry);
                    }
                }
            }
        }
        let general: Vec<(TargetKind, ComRaw)> = if s.general_modified_targets.is_empty() {
            Vec::new()
        } else {
            keys.iter()
                .filter_map(|k| s.general_modified_targets.remove(k))
                .collect()
        };
        let shapes: Vec<(ComRaw, f64, i32)> = if s.modified_shapes.is_empty() {
            Vec::new()
        } else {
            keys.iter()
                .filter_map(|k| s.modified_shapes.remove(k))
                .collect()
        };
        let alttab_sub: Vec<(TargetKind, ComRaw)> = if s.alttab_subtargets.is_empty() {
            Vec::new()
        } else {
            keys.iter()
                .filter_map(|k| s.alttab_subtargets.remove(k))
                .collect()
        };
        let hover_shapes: Vec<(ComRaw, f64, i32)> = if s.hover_flyout_shapes.is_empty() {
            Vec::new()
        } else {
            keys.iter()
                .filter_map(|k| s.hover_flyout_shapes.remove(k))
                .collect()
        };
        let tooltip_mod: Vec<TooltipTrackedEntry> = if s.tooltip_modified_targets.is_empty() {
            Vec::new()
        } else {
            keys.iter()
                .filter_map(|k| s.tooltip_modified_targets.remove(k))
                .collect()
        };
        let menu_mod: Vec<TooltipTrackedEntry> = if s.menu_flyout_modified_targets.is_empty() {
            Vec::new()
        } else {
            keys.iter()
                .filter_map(|k| s.menu_flyout_modified_targets.remove(k))
                .collect()
        };
        let shell_mod: Vec<TooltipTrackedEntry> = if s.shell_card_modified_targets.is_empty() {
            Vec::new()
        } else {
            keys.iter()
                .filter_map(|k| s.shell_card_modified_targets.remove(k))
                .collect()
        };
        (
            brushes,
            watches,
            general,
            shapes,
            alttab_sub,
            hover_shapes,
            tooltip_mod,
            menu_mod,
            shell_mod,
        )
    });
    drop(removed_brushes);
    drop(removed_general);
    drop(removed_shapes);
    drop(removed_alttab_sub);
    drop(removed_hover_shapes);
    let mut orphaned_parent_keys: Vec<u64> = Vec::new();
    for entry in removed_tooltip_mod
        .into_iter()
        .chain(removed_menu_mod)
        .chain(removed_shell_mod)
    {
        if let Some(pk) = entry.tooltip_key {
            let parent_alive = entry
                .tooltip_weak
                .as_ref()
                .is_some_and(|w| unsafe { resolve_weak_inspectable(w).is_some() });
            if !parent_alive {
                orphaned_parent_keys.push(pk);
            }
        }
        drop(entry);
    }
    for entry in removed_watches {
        unsafe {
            unregister_watch_entry(entry);
        }
    }
    if !orphaned_parent_keys.is_empty() {
        purge_saved_brushes_for_current_thread(&orphaned_parent_keys);
    }
}

pub(in super::super) unsafe fn read_orig_brush(
    insp_ptr: *mut c_void,
    dp: Option<&ComRaw>,
    unset_val: Option<&ComRaw>,
    current_brush: &ComRaw,
) -> Option<SavedBrush> {
    let effective_cur = (!current_brush.is_null()
        && !is_transparent_solid_brush(current_brush.as_ptr())
        && !is_cached_tap_brush(current_brush.as_ptr()))
    .then(|| current_brush.clone_ref());
    if let (Some(dp_com), Some(unset_com)) = (dp, unset_val) {
        let dep_com = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        if !dep_com.is_null() {
            let dep = dep_com.as_ptr().cast::<RawIDependencyObject>();
            let mut local_val = ComRaw::null();
            if ((*(*dep).lp_vtbl).read_local_value)(
                dep_com.as_ptr(),
                dp_com.as_ptr(),
                &mut local_val.0,
            )
            .is_ok()
                && !local_val.is_null()
            {
                if com_same_identity(local_val.as_ptr(), unset_com.as_ptr()) {
                    return Some(SavedBrush::Unset(effective_cur));
                }
                let local_brush = qi_interface(local_val.as_ptr(), &IID_IBRUSH);
                if !local_brush.is_null()
                    && !is_transparent_solid_brush(local_brush.as_ptr())
                    && !is_cached_tap_brush(local_brush.as_ptr())
                {
                    if let Some(cur) = effective_cur {
                        if !brushes_equivalent(local_brush.as_ptr(), cur.as_ptr()) {
                            // An active higher-precedence VisualState Setter / Storyboard
                            // (e.g., Win11 ColorPrevalence accent tint on CalendarCenterGrid)
                            // overrides the underlying XAML local brush. Try ClearValue first
                            // on restore and fall back to this active effective brush so exit
                            // never regresses to the stale base local brush.
                            return Some(SavedBrush::Unset(Some(cur)));
                        }
                    }
                    return Some(SavedBrush::Local(local_brush));
                }
            }
        }
    }
    if let Some(cur) = effective_cur {
        Some(SavedBrush::Local(cur))
    } else if current_brush.is_null() {
        Some(SavedBrush::Unset(None))
    } else {
        None
    }
}

pub(in super::super) unsafe fn clear_dp_value(insp_ptr: *mut c_void, dp: Option<&ComRaw>) -> bool {
    let Some(dp_com) = dp else {
        return false;
    };
    let dep_com = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
    if dep_com.is_null() {
        return false;
    }
    let dep = dep_com.as_ptr().cast::<RawIDependencyObject>();
    ((*(*dep).lp_vtbl).clear_value)(dep_com.as_ptr(), dp_com.as_ptr()).is_ok()
}

#[derive(Clone, Copy)]
pub(in super::super) enum BrushPropKind {
    BorderBackground,
    BorderBrush,
    PanelBackground,
    FocusVisualSecondaryBrush,
    ElementShadow,
    FocusVisualSecondaryThickness,
    GridBorderBrush,
    ContentPresenterBackground,
    ContentPresenterBorderBrush,
    ControlBackground,
    ControlBorderBrush,
}

impl BrushPropKind {
    pub(super) const COUNT: u8 = 11;

    #[inline]
    pub(in super::super) fn key(self) -> u8 {
        match self {
            Self::BorderBackground => 0,
            Self::BorderBrush => 1,
            Self::PanelBackground => 2,
            Self::FocusVisualSecondaryBrush => 3,
            Self::ElementShadow => 4,
            Self::FocusVisualSecondaryThickness => 5,
            Self::GridBorderBrush => 6,
            Self::ContentPresenterBackground => 7,
            Self::ContentPresenterBorderBrush => 8,
            Self::ControlBackground => 9,
            Self::ControlBorderBrush => 10,
        }
    }

    #[inline]
    pub(in super::super) fn brush_slot(self) -> BrushSlot {
        match self {
            Self::BorderBrush
            | Self::GridBorderBrush
            | Self::ContentPresenterBorderBrush
            | Self::ControlBorderBrush => BrushSlot::BorderBrush,
            Self::FocusVisualSecondaryBrush | Self::FocusVisualSecondaryThickness => {
                BrushSlot::FocusSecondary
            }
            Self::ElementShadow => BrushSlot::Shadow,
            Self::BorderBackground
            | Self::PanelBackground
            | Self::ContentPresenterBackground
            | Self::ControlBackground => BrushSlot::Background,
        }
    }

    #[inline]
    pub(in super::super) fn allow_null_on_hide(self) -> bool {
        matches!(
            self,
            Self::BorderBackground | Self::FocusVisualSecondaryBrush
        )
    }

    pub(in super::super) unsafe fn get(self, iface: *mut c_void) -> (HRESULT, ComRaw) {
        let mut cur = ComRaw::null();
        let hr = match self {
            Self::BorderBackground => {
                let b = iface.cast::<RawIBorder>();
                ((*(*b).lp_vtbl).get_background)(iface, &mut cur.0)
            }
            Self::BorderBrush => {
                let b = iface.cast::<RawIBorder>();
                ((*(*b).lp_vtbl).get_border_brush)(iface, &mut cur.0)
            }
            Self::PanelBackground => {
                let p = iface.cast::<RawIPanel>();
                ((*(*p).lp_vtbl).get_background)(iface, &mut cur.0)
            }
            Self::GridBorderBrush => {
                let g = iface.cast::<RawIGrid2>();
                ((*(*g).lp_vtbl).get_border_brush)(iface, &mut cur.0)
            }
            Self::ContentPresenterBackground => {
                let cp = iface.cast::<RawIContentPresenter4>();
                ((*(*cp).lp_vtbl).get_background)(iface, &mut cur.0)
            }
            Self::ContentPresenterBorderBrush => {
                let cp = iface.cast::<RawIContentPresenter4>();
                ((*(*cp).lp_vtbl).get_border_brush)(iface, &mut cur.0)
            }
            Self::ControlBackground => {
                let c = iface.cast::<RawIControl>();
                ((*(*c).lp_vtbl).get_background)(iface, &mut cur.0)
            }
            Self::ControlBorderBrush => {
                let c = iface.cast::<RawIControl>();
                ((*(*c).lp_vtbl).get_border_brush)(iface, &mut cur.0)
            }
            Self::FocusVisualSecondaryBrush => {
                let fe4 = iface.cast::<RawIFrameworkElement4>();
                ((*(*fe4).lp_vtbl).get_focus_visual_secondary_brush)(iface, &mut cur.0)
            }
            Self::ElementShadow => {
                let ui10 = iface.cast::<RawIUIElement10>();
                ((*(*ui10).lp_vtbl).get_shadow)(iface, &mut cur.0)
            }
            Self::FocusVisualSecondaryThickness => E_FAIL_HR,
        };
        (hr, cur)
    }

    pub(in super::super) unsafe fn put(self, iface: *mut c_void, brush: *mut c_void) -> HRESULT {
        match self {
            Self::BorderBackground => {
                let b = iface.cast::<RawIBorder>();
                ((*(*b).lp_vtbl).put_background)(iface, brush)
            }
            Self::BorderBrush => {
                let b = iface.cast::<RawIBorder>();
                ((*(*b).lp_vtbl).put_border_brush)(iface, brush)
            }
            Self::PanelBackground => {
                let p = iface.cast::<RawIPanel>();
                ((*(*p).lp_vtbl).put_background)(iface, brush)
            }
            Self::GridBorderBrush => {
                let g = iface.cast::<RawIGrid2>();
                ((*(*g).lp_vtbl).put_border_brush)(iface, brush)
            }
            Self::ContentPresenterBackground => {
                let cp = iface.cast::<RawIContentPresenter4>();
                ((*(*cp).lp_vtbl).put_background)(iface, brush)
            }
            Self::ContentPresenterBorderBrush => {
                let cp = iface.cast::<RawIContentPresenter4>();
                ((*(*cp).lp_vtbl).put_border_brush)(iface, brush)
            }
            Self::ControlBackground => {
                let c = iface.cast::<RawIControl>();
                ((*(*c).lp_vtbl).put_background)(iface, brush)
            }
            Self::ControlBorderBrush => {
                let c = iface.cast::<RawIControl>();
                ((*(*c).lp_vtbl).put_border_brush)(iface, brush)
            }
            Self::FocusVisualSecondaryBrush => {
                let fe4 = iface.cast::<RawIFrameworkElement4>();
                ((*(*fe4).lp_vtbl).put_focus_visual_secondary_brush)(iface, brush)
            }
            Self::ElementShadow => {
                let ui10 = iface.cast::<RawIUIElement10>();
                ((*(*ui10).lp_vtbl).put_shadow)(iface, brush)
            }
            Self::FocusVisualSecondaryThickness => E_FAIL_HR,
        }
    }

    pub(in super::super) unsafe fn dependency_property(self) -> Option<ComRaw> {
        match self {
            Self::BorderBackground => ensure_cached_com(
                |s| s.border_bg_dp.as_ref(),
                |s, v| s.border_bg_dp = Some(v),
                fetch_border_bg_dp,
            ),
            Self::BorderBrush => ensure_cached_com(
                |s| s.border_brush_dp.as_ref(),
                |s, v| s.border_brush_dp = Some(v),
                fetch_border_brush_dp,
            ),
            Self::PanelBackground => ensure_cached_com(
                |s| s.panel_bg_dp.as_ref(),
                |s, v| s.panel_bg_dp = Some(v),
                fetch_panel_bg_dp,
            ),
            Self::GridBorderBrush => ensure_cached_com(
                |s| s.grid_border_brush_dp.as_ref(),
                |s, v| s.grid_border_brush_dp = Some(v),
                fetch_grid_border_brush_dp,
            ),
            Self::ContentPresenterBackground => ensure_cached_com(
                |s| s.content_presenter_bg_dp.as_ref(),
                |s, v| s.content_presenter_bg_dp = Some(v),
                fetch_content_presenter_bg_dp,
            ),
            Self::ContentPresenterBorderBrush => ensure_cached_com(
                |s| s.content_presenter_border_brush_dp.as_ref(),
                |s, v| s.content_presenter_border_brush_dp = Some(v),
                fetch_content_presenter_border_brush_dp,
            ),
            Self::ControlBackground => ensure_cached_com(
                |s| s.control_bg_dp.as_ref(),
                |s, v| s.control_bg_dp = Some(v),
                fetch_control_bg_dp,
            ),
            Self::ControlBorderBrush => ensure_cached_com(
                |s| s.control_border_brush_dp.as_ref(),
                |s, v| s.control_border_brush_dp = Some(v),
                fetch_control_border_brush_dp,
            ),
            Self::FocusVisualSecondaryBrush => ensure_cached_com(
                |s| s.focus_secondary_brush_dp.as_ref(),
                |s, v| s.focus_secondary_brush_dp = Some(v),
                fetch_focus_secondary_brush_dp,
            ),
            Self::ElementShadow => ensure_cached_com(
                |s| s.element_shadow_dp.as_ref(),
                |s, v| s.element_shadow_dp = Some(v),
                fetch_element_shadow_dp,
            ),
            Self::FocusVisualSecondaryThickness => ensure_cached_com(
                |s| s.focus_secondary_thickness_dp.as_ref(),
                |s, v| s.focus_secondary_thickness_dp = Some(v),
                fetch_focus_secondary_thickness_dp,
            ),
        }
    }
}

