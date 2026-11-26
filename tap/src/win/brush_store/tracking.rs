//! Watch registration, cached solid brushes, ABA weak-reference verification, and paired flyout tracking.

use super::super::super::tree_match::TargetKind;
use super::super::raw_com::*;
use super::super::TapRuntime;
use super::*;
use std::ffi::c_void;
use std::sync::Arc;


pub(in super::super) unsafe fn prewarm_overflow_brush_store_for_current_thread() {
    let already = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return true;
        };
        if s.prewarmed {
            true
        } else {
            s.prewarmed = true;
            false
        }
    });
    if already {
        return;
    }
    let _ = ensure_cached_com(
        |s| s.trans_brush.as_ref(),
        |s, v| s.trans_brush = Some(v),
        fetch_transparent_brush,
    );
    let _ = ensure_cached_com(
        |s| s.unset_value.as_ref(),
        |s, v| s.unset_value = Some(v),
        fetch_unset_value,
    );
    let _ = ensure_cached_com(
        |s| s.border_bg_dp.as_ref(),
        |s, v| s.border_bg_dp = Some(v),
        fetch_border_bg_dp,
    );
    let _ = ensure_cached_com(
        |s| s.border_brush_dp.as_ref(),
        |s, v| s.border_brush_dp = Some(v),
        fetch_border_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.panel_bg_dp.as_ref(),
        |s, v| s.panel_bg_dp = Some(v),
        fetch_panel_bg_dp,
    );
    let _ = ensure_cached_com(
        |s| s.grid_border_brush_dp.as_ref(),
        |s, v| s.grid_border_brush_dp = Some(v),
        fetch_grid_border_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.content_presenter_bg_dp.as_ref(),
        |s, v| s.content_presenter_bg_dp = Some(v),
        fetch_content_presenter_bg_dp,
    );
    let _ = ensure_cached_com(
        |s| s.content_presenter_border_brush_dp.as_ref(),
        |s, v| s.content_presenter_border_brush_dp = Some(v),
        fetch_content_presenter_border_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.control_bg_dp.as_ref(),
        |s, v| s.control_bg_dp = Some(v),
        fetch_control_bg_dp,
    );
    let _ = ensure_cached_com(
        |s| s.control_border_brush_dp.as_ref(),
        |s, v| s.control_border_brush_dp = Some(v),
        fetch_control_border_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.control_corner_radius_dp.as_ref(),
        |s, v| s.control_corner_radius_dp = Some(v),
        fetch_control_corner_radius_dp,
    );
    let _ = ensure_cached_com(
        |s| s.focus_secondary_brush_dp.as_ref(),
        |s, v| s.focus_secondary_brush_dp = Some(v),
        fetch_focus_secondary_brush_dp,
    );
    let _ = ensure_cached_com(
        |s| s.focus_secondary_thickness_dp.as_ref(),
        |s, v| s.focus_secondary_thickness_dp = Some(v),
        fetch_focus_secondary_thickness_dp,
    );
    let _ = ensure_cached_com(
        |s| s.element_shadow_dp.as_ref(),
        |s, v| s.element_shadow_dp = Some(v),
        fetch_element_shadow_dp,
    );
    let _ = cached_visual_tree_helper();
    let _ = cached_element_composition_preview();
}

pub(in super::super) unsafe fn unregister_watch_entry(entry: PropWatchEntry) {
    if entry.token != 0 && !entry.weak_obj.is_null() && !entry.dp.is_null() {
        let wr = entry.weak_obj.as_ptr().cast::<RawIWeakReference>();
        let mut insp = ComRaw::null();
        if ((*(*wr).lp_vtbl).resolve)(
            entry.weak_obj.as_ptr(),
            &IID_IINSPECTABLE,
            &mut insp.0,
        )
        .is_ok()
            && !insp.is_null()
        {
            let dep2 = qi_interface(insp.as_ptr(), &IID_IDEPENDENCYOBJECT2);
            if !dep2.is_null() {
                let d2 = dep2.as_ptr().cast::<RawIDependencyObject2>();
                let _ = ((*(*d2).lp_vtbl).unregister_property_changed_callback)(
                    dep2.as_ptr(),
                    entry.dp.as_ptr(),
                    entry.token,
                );
            }
        }
    }
    drop(entry);
}

pub(in super::super) unsafe fn ensure_prop_watch_registered(
    rt: &Arc<TapRuntime>,
    handle: u64,
    kind: TargetKind,
    prop: BrushPropKind,
    insp_ptr: *mut c_void,
) {
    if handle == 0 || insp_ptr.is_null() {
        return;
    }
    let key = (handle, prop.key());
    let existing: Option<(ComRaw, TargetKind)> = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|s| s.watches.get(&key).map(|e| (e.weak_obj.clone_ref(), e.kind)))
    });
    if let Some((existing_weak, existing_kind)) = existing {
        if weak_matches_live_object(&existing_weak, insp_ptr) {
            if existing_kind == kind {
                return;
            }
            // Target was reclassified (e.g. ShellCardSurface -> ShellCardSubSurface): replace watch
            // so `PropChangedCallback` uses the updated `TargetKind`.
            let old_entry = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| s.watches.remove(&key))
            });
            if let Some(entry) = old_entry {
                unregister_watch_entry(entry);
            }
        } else {
            // Pointer-address reuse (ABA): purge stale saved state & watches for the destroyed object.
            purge_saved_brushes_for_current_thread(&[handle]);
        }
    }

    let Some(dp) = prop.dependency_property() else {
        return;
    };
    let dep2 = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT2);
    if dep2.is_null() {
        return;
    }
    let weak_src = qi_interface(insp_ptr, &IID_IWEAKREFERENCESOURCE);
    if weak_src.is_null() {
        return;
    }
    let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
    let mut weak_obj = ComRaw::null();
    if ((*(*src_iface).lp_vtbl).get_weak_reference)(weak_src.as_ptr(), &mut weak_obj.0)
        .is_err()
        || weak_obj.is_null()
    {
        return;
    }

    let cb = PropChangedCallback::new_com_ptr(Arc::clone(rt), handle, kind, prop);
    let d2 = dep2.as_ptr().cast::<RawIDependencyObject2>();
    let mut token = 0i64;
    let hr = ((*(*d2).lp_vtbl).register_property_changed_callback)(
        dep2.as_ptr(),
        dp.as_ptr(),
        cb.cast(),
        &mut token,
    );
    prop_changed_release(cb);
    if hr.is_err() || token == 0 {
        return;
    }

    let new_entry = PropWatchEntry {
        weak_obj,
        dp,
        token,
        kind,
    };
    let dup = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if let std::collections::hash_map::Entry::Vacant(e) = s.watches.entry(key) {
                e.insert(new_entry);
                return None;
            }
        }
        Some(new_entry)
    });
    if let Some(entry) = dup {
        unregister_watch_entry(entry);
    }
}

pub(in super::super) fn clear_all_watches_for_current_thread() {
    let removed_watches: Vec<PropWatchEntry> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        s.watches.drain().map(|(_, e)| e).collect()
    });
    for entry in removed_watches {
        unsafe {
            unregister_watch_entry(entry);
        }
    }
}

pub(in super::super) unsafe fn ensure_cached_com(
    pick: fn(&UiThreadBrushStore) -> Option<&ComRaw>,
    put: fn(&mut UiThreadBrushStore, ComRaw),
    fetch: unsafe fn() -> Option<ComRaw>,
) -> Option<ComRaw> {
    if let Some(hit) = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|s| pick(&s).map(ComRaw::clone_ref))
    }) {
        return Some(hit);
    }
    let created = fetch()?;
    let to_store = created.clone_ref();
    let rejected = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if pick(&s).is_none() {
                put(&mut s, to_store);
                return None;
            }
        }
        Some(to_store)
    });
    drop(rejected);
    Some(created)
}

pub(in super::super) unsafe fn cached_solid_brush(color: RawUiColor) -> Option<ComRaw> {
    if color.a == 0 {
        return ensure_cached_com(
            |s| s.trans_brush.as_ref(),
            |s, v| s.trans_brush = Some(v),
            fetch_transparent_brush,
        );
    }
    let key = color.to_argb();
    if let Some(hit) = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|s| s.tinted_brushes.get(&key).map(ComRaw::clone_ref))
    }) {
        return Some(hit);
    }
    let created = fetch_solid_brush(color)?;
    let to_store = created.clone_ref();
    let (rejected, evicted) = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if s.tinted_brushes.contains_key(&key) {
                return (Some(to_store), None);
            }
            const MAX_TINTED_BRUSHES: usize = 128;
            let mut evicted_old = None;
            if s.tinted_brushes.len() >= MAX_TINTED_BRUSHES {
                while let Some(oldest) = s.tinted_brush_order.pop_front() {
                    if let Some(old_brush) = s.tinted_brushes.remove(&oldest) {
                        evicted_old = Some(old_brush);
                        break;
                    }
                }
            }
            s.tinted_brushes.insert(key, to_store);
            s.tinted_brush_order.push_back(key);
            return (None, evicted_old);
        }
        (Some(to_store), None)
    });
    drop(rejected);
    drop(evicted);
    Some(created)
}

pub(in super::super) fn has_saved_brush_slot(handle: u64, slot: BrushSlot) -> bool {
    UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow().ok().is_some_and(|s| {
            s.saved.get(&handle).is_some_and(|entry| match slot {
                BrushSlot::Background => entry.background.is_some(),
                BrushSlot::BorderBrush => entry.border_brush.is_some(),
                BrushSlot::FocusSecondary => entry.focus_secondary.is_some(),
                BrushSlot::Shadow => entry.shadow.is_some(),
            })
        })
    })
}

pub(in super::super) fn save_brush_slot_if_absent(handle: u64, slot: BrushSlot, val: SavedBrush) {
    let displaced = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            let entry = s.saved.entry(handle).or_default();
            let target = match slot {
                BrushSlot::Background => &mut entry.background,
                BrushSlot::BorderBrush => &mut entry.border_brush,
                BrushSlot::FocusSecondary => &mut entry.focus_secondary,
                BrushSlot::Shadow => &mut entry.shadow,
            };
            if target.is_none() {
                *target = Some(val);
                return None;
            }
        }
        Some(val)
    });
    drop(displaced);
}

/// Updates an existing saved brush slot when Windows itself mutates the element to a new genuine
/// non-TAP native brush (e.g., `CalendarCenterGrid` applying its `ColorPrevalence` accent brush
/// after initial tree insertion, or a live system theme/accent switch while TAP is active).
pub(in super::super) fn update_saved_brush_with_latest_native(
    handle: u64,
    slot: BrushSlot,
    latest_native: ComRaw,
) {
    let displaced = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if let Some(entry) = s.saved.get_mut(&handle) {
                let target = match slot {
                    BrushSlot::Background => &mut entry.background,
                    BrushSlot::BorderBrush => &mut entry.border_brush,
                    BrushSlot::FocusSecondary => &mut entry.focus_secondary,
                    BrushSlot::Shadow => &mut entry.shadow,
                };
                match target {
                    Some(SavedBrush::Unset(prev)) => {
                        return prev.replace(latest_native);
                    }
                    Some(SavedBrush::Local(prev)) => {
                        return Some(std::mem::replace(prev, latest_native));
                    }
                    None => {
                        *target = Some(SavedBrush::Local(latest_native));
                        return None;
                    }
                }
            }
        }
        Some(latest_native)
    });
    drop(displaced);
}

pub(in super::super) unsafe fn weak_matches_live_object(existing_weak: &ComRaw, insp_ptr: *mut c_void) -> bool {
    resolve_weak_inspectable(existing_weak)
        .is_some_and(|resolved| com_same_identity(resolved.as_ptr(), insp_ptr))
}

pub(in super::super) unsafe fn is_alttab_subtarget_tracked_live(key: u64, insp_ptr: *mut c_void) -> bool {
    let existing = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|s| s.alttab_subtargets.get(&key).map(|(_, w)| w.clone_ref()))
    });
    existing.is_some_and(|w| weak_matches_live_object(&w, insp_ptr))
}

pub(in super::super) unsafe fn track_alttab_subtarget_weak(key: u64, kind: TargetKind, insp_ptr: *mut c_void) {
    if key == 0 || insp_ptr.is_null() {
        return;
    }
    let existing_weak: Option<ComRaw> = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|s| s.alttab_subtargets.get(&key).map(|(_, w)| w.clone_ref()))
    });
    if let Some(ew) = existing_weak {
        if weak_matches_live_object(&ew, insp_ptr) {
            return;
        }
        purge_saved_brushes_for_current_thread(&[key]);
    }
    let weak_src = qi_interface(insp_ptr, &IID_IWEAKREFERENCESOURCE);
    if weak_src.is_null() {
        return;
    }
    let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
    let mut weak_ref = ComRaw::null();
    if ((*(*src_iface).lp_vtbl).get_weak_reference)(weak_src.as_ptr(), &mut weak_ref.0).is_err()
        || weak_ref.is_null()
    {
        return;
    }

    let need_prune = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .is_some_and(|s| s.alttab_subtargets.len() >= 256)
    });
    if need_prune {
        let candidates: Vec<(u64, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .map(|s| {
                    s.alttab_subtargets
                        .iter()
                        .map(|(&k, (_, w))| (k, w.clone_ref()))
                        .collect()
                })
                .unwrap_or_default()
        });
        let mut dead = Vec::new();
        for (ck, wr_com) in candidates {
            let wr = wr_com.as_ptr().cast::<RawIWeakReference>();
            let mut resolved = ComRaw::null();
            if ((*(*wr).lp_vtbl).resolve)(wr_com.as_ptr(), &IID_IINSPECTABLE, &mut resolved.0)
                .is_err()
                || resolved.is_null()
            {
                dead.push(ck);
            }
        }
        if !dead.is_empty() {
            let removed = UI_BRUSH_STORE.with(|cell| {
                let Ok(mut s) = cell.try_borrow_mut() else {
                    return Vec::new();
                };
                dead.iter()
                    .filter_map(|k| s.alttab_subtargets.remove(k))
                    .collect::<Vec<_>>()
            });
            drop(removed);
            purge_saved_brushes_for_current_thread(&dead);
        }
    }

    let displaced = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if let std::collections::hash_map::Entry::Vacant(e) =
                s.alttab_subtargets.entry(key)
            {
                e.insert((kind, weak_ref));
                return None;
            }
        }
        Some(weak_ref)
    });
    drop(displaced);
}

pub(in super::super) fn unregister_watches_for_keys(keys: &[u64]) {
    if keys.is_empty() {
        return;
    }
    let removed_watches: Vec<PropWatchEntry> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        let mut watches = Vec::new();
        if !s.watches.is_empty() {
            for &k in keys {
                for prop_idx in 0u8..BrushPropKind::COUNT {
                    if let Some(entry) = s.watches.remove(&(k, prop_idx)) {
                        watches.push(entry);
                    }
                }
            }
        }
        watches
    });
    for entry in removed_watches {
        unsafe {
            unregister_watch_entry(entry);
        }
    }
}

pub(in super::super) unsafe fn make_weak_ref_com(insp_ptr: *mut c_void) -> Option<ComRaw> {
    if insp_ptr.is_null() {
        return None;
    }
    let weak_src = qi_interface(insp_ptr, &IID_IWEAKREFERENCESOURCE);
    if weak_src.is_null() {
        return None;
    }
    let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
    let mut weak_ref = ComRaw::null();
    if ((*(*src_iface).lp_vtbl).get_weak_reference)(weak_src.as_ptr(), &mut weak_ref.0).is_ok()
        && !weak_ref.is_null()
    {
        Some(weak_ref)
    } else {
        None
    }
}

pub(in super::super) unsafe fn resolve_weak_inspectable(weak_com: &ComRaw) -> Option<ComRaw> {
    if weak_com.is_null() {
        return None;
    }
    let wr = weak_com.as_ptr().cast::<RawIWeakReference>();
    let mut insp = ComRaw::null();
    if ((*(*wr).lp_vtbl).resolve)(weak_com.as_ptr(), &IID_IINSPECTABLE, &mut insp.0).is_ok()
        && !insp.is_null()
    {
        Some(insp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn resolve_weak_raw_inspectable(weak_raw: usize) -> Option<ComRaw> {
    if weak_raw == 0 {
        return None;
    }
    add_ref_raw_com_addr(weak_raw);
    let weak_com = ComRaw(weak_raw as *mut c_void);
    resolve_weak_inspectable(&weak_com)
}

pub(in super::super) unsafe fn track_paired_modified_target(
    primary_key: u64,
    primary_insp: *mut c_void,
    parent_pair: Option<(u64, *mut c_void)>,
    get_map: fn(&mut UiThreadBrushStore) -> &mut HashMap<u64, TooltipTrackedEntry>,
    read_map: fn(&UiThreadBrushStore) -> &HashMap<u64, TooltipTrackedEntry>,
) {
    if primary_key == 0 || primary_insp.is_null() {
        return;
    }
    let (existing_entry, need_prune) = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .map(|s| {
                let map = read_map(&s);
                let exists = map.get(&primary_key).map(|e| {
                    (
                        e.presenter_weak.clone_ref(),
                        e.tooltip_key,
                        e.tooltip_weak.as_ref().map(ComRaw::clone_ref),
                    )
                });
                let prune = exists.is_none() && map.len() >= 128;
                (exists, prune)
            })
            .unwrap_or((None, false))
    });

    if let Some((existing_primary_weak, existing_parent_key, existing_parent_weak)) = existing_entry
    {
        if weak_matches_live_object(&existing_primary_weak, primary_insp) {
            let parent_needs_refresh = match (&existing_parent_weak, parent_pair) {
                (None, Some(_)) => true,
                (Some(ew), Some((_, p_ptr))) => !weak_matches_live_object(ew, p_ptr),
                _ => false,
            };
            if parent_needs_refresh {
                if let Some((pk, p_ptr)) = parent_pair {
                    if let Some(old_pk) = existing_parent_key {
                        if old_pk != pk
                            || existing_parent_weak
                                .as_ref()
                                .is_some_and(|ew| !weak_matches_live_object(ew, p_ptr))
                        {
                            purge_saved_brushes_for_current_thread(&[old_pk]);
                        }
                    }
                    if let Some(p_weak) = make_weak_ref_com(p_ptr) {
                        let rejected = UI_BRUSH_STORE.with(|cell| {
                            if let Ok(mut s) = cell.try_borrow_mut() {
                                if let Some(entry) = get_map(&mut s).get_mut(&primary_key) {
                                    entry.tooltip_key = Some(pk);
                                    return entry.tooltip_weak.replace(p_weak);
                                }
                            }
                            Some(p_weak)
                        });
                        drop(rejected);
                    }
                }
            }
            return;
        }
        // ABA collision: previous primary COM object at this address is dead or different.
        let mut stale_keys = vec![primary_key];
        if let Some(old_pk) = existing_parent_key {
            let parent_alive = existing_parent_weak
                .as_ref()
                .is_some_and(|ew| resolve_weak_inspectable(ew).is_some());
            if !parent_alive {
                stale_keys.push(old_pk);
            }
        }
        purge_saved_brushes_for_current_thread(&stale_keys);
    }

    if need_prune {
        let candidates: Vec<(u64, ComRaw, Option<u64>, Option<ComRaw>)> = UI_BRUSH_STORE
            .with(|cell| {
                cell.try_borrow()
                    .ok()
                    .map(|s| {
                        read_map(&s)
                            .iter()
                            .map(|(&k, e)| {
                                (
                                    k,
                                    e.presenter_weak.clone_ref(),
                                    e.tooltip_key,
                                    e.tooltip_weak.as_ref().map(ComRaw::clone_ref),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            });
        let mut dead_entry_keys = Vec::new();
        let mut dead_purge_keys = Vec::new();
        for (pk, p_weak, tk_opt, t_weak_opt) in candidates {
            let p_alive = resolve_weak_inspectable(&p_weak).is_some();
            let t_alive = t_weak_opt
                .as_ref()
                .and_then(|tw| resolve_weak_inspectable(tw))
                .is_some();
            if !p_alive && !t_alive {
                dead_entry_keys.push(pk);
                dead_purge_keys.push(pk);
                if let Some(tk) = tk_opt {
                    dead_purge_keys.push(tk);
                }
            }
        }
        if !dead_entry_keys.is_empty() {
            let removed: Vec<TooltipTrackedEntry> = UI_BRUSH_STORE.with(|cell| {
                let Ok(mut s) = cell.try_borrow_mut() else {
                    return Vec::new();
                };
                let map = get_map(&mut s);
                dead_entry_keys.iter().filter_map(|k| map.remove(k)).collect()
            });
            drop(removed);
            purge_saved_brushes_for_current_thread(&dead_purge_keys);
        }
    }

    let Some(presenter_weak) = make_weak_ref_com(primary_insp) else {
        return;
    };
    let (tooltip_key, tooltip_weak) = match parent_pair {
        Some((tk, t_ptr)) => match make_weak_ref_com(t_ptr) {
            Some(tw) => (Some(tk), Some(tw)),
            None => (Some(tk), None),
        },
        None => (None, None),
    };
    let displaced = UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            if let std::collections::hash_map::Entry::Vacant(e) = get_map(&mut s).entry(primary_key) {
                e.insert(TooltipTrackedEntry {
                    presenter_weak,
                    tooltip_key,
                    tooltip_weak,
                });
                return None;
            }
        }
        Some((presenter_weak, tooltip_weak))
    });
    drop(displaced);
}

pub(in super::super) unsafe fn track_tooltip_modified_target(
    presenter_key: u64,
    presenter_insp: *mut c_void,
    parent_tooltip: Option<(u64, *mut c_void)>,
) {
    track_paired_modified_target(
        presenter_key,
        presenter_insp,
        parent_tooltip,
        |s| &mut s.tooltip_modified_targets,
        |s| &s.tooltip_modified_targets,
    );
}

pub(in super::super) unsafe fn track_menu_flyout_modified_target(
    surface_key: u64,
    surface_insp: *mut c_void,
    parent_ctrl: Option<(u64, *mut c_void)>,
) {
    track_paired_modified_target(
        surface_key,
        surface_insp,
        parent_ctrl,
        |s| &mut s.menu_flyout_modified_targets,
        |s| &s.menu_flyout_modified_targets,
    );
}

pub(in super::super) unsafe fn is_cached_tap_brush(brush_ptr: *mut c_void) -> bool {
    if brush_ptr.is_null() {
        return false;
    }
    let candidates: Vec<ComRaw> = UI_BRUSH_STORE.with(|cell| {
        let Ok(s) = cell.try_borrow() else {
            return Vec::new();
        };
        let mut list = Vec::with_capacity(1 + s.tinted_brushes.len());
        if let Some(ref tb) = s.trans_brush {
            list.push(tb.clone_ref());
        }
        for b in s.tinted_brushes.values() {
            list.push(b.clone_ref());
        }
        list
    });
    candidates
        .iter()
        .any(|c| com_same_identity(brush_ptr, c.as_ptr()))
}
