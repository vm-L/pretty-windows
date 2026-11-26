mod alttab;
mod flyouts;
mod menus;
mod modal_blur;
mod shell_cards;

pub(super) use alttab::{
    restore_all_alttab_subtargets_for_current_thread, sync_alttab_subtree_items,
    sync_focus_secondary_thickness_prop,
};
pub(super) use flyouts::{
    restore_all_hover_flyout_shapes_for_current_thread, sync_hover_flyout_bg_shapes,
};
pub(super) use menus::{
    restore_all_menu_flyout_targets_for_current_thread,
    restore_all_tooltip_targets_for_current_thread,
};
pub(super) use shell_cards::restore_all_shell_card_targets_for_current_thread;

use alttab::{
    apply_alttab_item_border_appearance, apply_border_target_appearance,
    apply_focus_secondary_appearance,
};
use flyouts::{
    apply_hover_flyout_border_appearance, apply_input_switcher_panel_appearance,
    apply_input_switcher_sub_panel_appearance, apply_osd_panel_appearance,
    apply_virtual_desktop_bar_appearance,
};
use menus::{apply_menu_flyout_surface_appearance, apply_tooltip_presenter_appearance};
use shell_cards::{apply_shell_card_surface_appearance, apply_start_menu_modal_surface_appearance};

use super::super::tree_match::TargetKind;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use super::brush_store::*;
use super::raw_com::*;
use super::TapRuntime;

unsafe fn read_orig_shadow(
    insp_ptr: *mut c_void,
    dp: Option<&ComRaw>,
    unset_val: Option<&ComRaw>,
    current_shadow: &ComRaw,
) -> Option<SavedBrush> {
    let effective_cur = (!current_shadow.is_null()).then(|| current_shadow.clone_ref());
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
                return Some(SavedBrush::Local(local_val));
            }
        }
    }
    if let Some(cur) = effective_cur {
        Some(SavedBrush::Local(cur))
    } else {
        Some(SavedBrush::Unset(None))
    }
}

pub(super) unsafe fn sync_element_shadow_prop(
    rt: &Arc<TapRuntime>,
    handle: u64,
    insp_ptr: *mut c_void,
    desired_hide: bool,
    restored_saved: Option<SavedBrush>,
    applied_flag: u32,
    prefer_local_on_non_null: bool,
) {
    let ui10_com = qi_interface(insp_ptr, &IID_IUIELEMENT10);
    if ui10_com.is_null() {
        return;
    }
    let ui10 = ui10_com.as_ptr().cast::<RawIUIElement10>();
    if desired_hide {
        let mut cur_shadow = ComRaw::null();
        let hr_get = ((*(*ui10).lp_vtbl).get_shadow)(ui10_com.as_ptr(), &mut cur_shadow.0);
        if hr_get.is_err() || cur_shadow.is_null() {
            return;
        }
        let captured = if !has_saved_brush_slot(handle, BrushSlot::Shadow) {
            if prefer_local_on_non_null {
                Some(SavedBrush::Local(cur_shadow.clone_ref()))
            } else {
                let dp = BrushPropKind::ElementShadow.dependency_property();
                let unset = ensure_cached_com(
                    |s| s.unset_value.as_ref(),
                    |s, v| s.unset_value = Some(v),
                    fetch_unset_value,
                );
                read_orig_shadow(insp_ptr, dp.as_ref(), unset.as_ref(), &cur_shadow)
            }
        } else {
            None
        };
        let put_ok = {
            let _sync_guard = PropSyncGuard::enter();
            ((*(*ui10).lp_vtbl).put_shadow)(ui10_com.as_ptr(), null_mut()).is_ok()
        };
        if put_ok {
            if let Some(orig) = captured {
                save_brush_slot_if_absent(handle, BrushSlot::Shadow, orig);
            }
            rt.diag_flags.fetch_or(applied_flag, Ordering::Relaxed);
        }
    } else {
        let _sync_guard = PropSyncGuard::enter();
        match restored_saved {
            Some(SavedBrush::Local(orig)) => {
                let _ = ((*(*ui10).lp_vtbl).put_shadow)(ui10_com.as_ptr(), orig.as_ptr());
            }
            Some(SavedBrush::Unset(orig_cur)) => {
                let dp = BrushPropKind::ElementShadow.dependency_property();
                let cleared = clear_dp_value(insp_ptr, dp.as_ref());
                if let Some(orig) = orig_cur {
                    let mut after = ComRaw::null();
                    let restored_ok = cleared
                        && ((*(*ui10).lp_vtbl).get_shadow)(ui10_com.as_ptr(), &mut after.0).is_ok()
                        && !after.is_null();
                    if !restored_ok {
                        let _ = ((*(*ui10).lp_vtbl).put_shadow)(ui10_com.as_ptr(), orig.as_ptr());
                    }
                } else if !cleared {
                    let _ = ((*(*ui10).lp_vtbl).put_shadow)(ui10_com.as_ptr(), null_mut());
                }
            }
            None => {
                if !prefer_local_on_non_null {
                    // Only clear a local null override if the element currently lacks a shadow;
                    // never strip an intact native ThemeShadow that TAP never saved/modified.
                    let mut cur_shadow = ComRaw::null();
                    let is_nulled = ((*(*ui10).lp_vtbl).get_shadow)(
                        ui10_com.as_ptr(),
                        &mut cur_shadow.0,
                    )
                    .is_ok()
                        && cur_shadow.is_null();
                    if is_nulled {
                        let dp = BrushPropKind::ElementShadow.dependency_property();
                        let _ = clear_dp_value(insp_ptr, dp.as_ref());
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn sync_overflow_brush_prop_with_mode(
    rt: &Arc<TapRuntime>,
    handle: u64,
    insp_ptr: *mut c_void,
    iface: *mut c_void,
    prop: BrushPropKind,
    target_color: RawUiColor,
    desired_hide: bool,
    restored_saved: Option<SavedBrush>,
    applied_flag: u32,
    prefer_local_on_non_null: bool,
) -> bool {
    if desired_hide {
        let (hr_get, cur) = prop.get(iface);
        if hr_get.is_err() || is_solid_brush_with_color(cur.as_ptr(), target_color) {
            return false;
        }
        if !prop.allow_null_on_hide() && cur.is_null() && target_color.a == 0 {
            return false;
        }
        let Some(tb) = cached_solid_brush(target_color) else {
            return false;
        };
        let is_genuine_native = !cur.is_null()
            && !is_transparent_solid_brush(cur.as_ptr())
            && !is_cached_tap_brush(cur.as_ptr());
        let had_saved = has_saved_brush_slot(handle, prop.brush_slot());
        let captured = if !had_saved {
            if prefer_local_on_non_null && is_genuine_native {
                Some(SavedBrush::Local(cur.clone_ref()))
            } else {
                let dp = prop.dependency_property();
                let unset = ensure_cached_com(
                    |s| s.unset_value.as_ref(),
                    |s, v| s.unset_value = Some(v),
                    fetch_unset_value,
                );
                read_orig_brush(insp_ptr, dp.as_ref(), unset.as_ref(), &cur)
            }
        } else {
            None
        };
        let updated_native = (had_saved && is_genuine_native).then(|| cur.clone_ref());
        let put_ok = {
            let _sync_guard = PropSyncGuard::enter();
            prop.put(iface, tb.as_ptr()).is_ok()
        };
        if put_ok {
            if let Some(orig) = captured {
                save_brush_slot_if_absent(handle, prop.brush_slot(), orig);
            } else if let Some(latest) = updated_native {
                update_saved_brush_with_latest_native(handle, prop.brush_slot(), latest);
            }
            rt.diag_flags.fetch_or(applied_flag, Ordering::Relaxed);
            return true;
        }
        false
    } else {
        let _sync_guard = PropSyncGuard::enter();
        match restored_saved {
            Some(SavedBrush::Local(orig)) => {
                let _ = prop.put(iface, orig.as_ptr());
                true
            }
            Some(SavedBrush::Unset(orig_cur)) => {
                let dp = prop.dependency_property();
                let cleared = clear_dp_value(insp_ptr, dp.as_ref());
                if let Some(orig) = orig_cur {
                    // Respect any genuine live theme/style brush restored by `ClearValue` so a
                    // Light/Dark or accent switch while TAP was active is not overwritten with a
                    // stale pre-switch brush. Fall back to `orig` only if `ClearValue` failed or
                    // left a null/transparent/TAP brush.
                    let restored_ok = if cleared {
                        let (hr_after, after) = prop.get(iface);
                        hr_after.is_ok()
                            && !after.is_null()
                            && !is_transparent_solid_brush(after.as_ptr())
                            && !is_cached_tap_brush(after.as_ptr())
                    } else {
                        false
                    };
                    if !restored_ok {
                        let _ = prop.put(iface, orig.as_ptr());
                    }
                } else if !cleared {
                    let _ = prop.put(iface, null_mut());
                }
                true
            }
            None => {
                let (hr_get, cur) = prop.get(iface);
                let is_our_brush = hr_get.is_ok() && is_cached_tap_brush(cur.as_ptr());
                if is_our_brush {
                    let dp = prop.dependency_property();
                    if !clear_dp_value(insp_ptr, dp.as_ref()) {
                        let _ = prop.put(iface, null_mut());
                    }
                    true
                } else {
                    false
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn sync_overflow_brush_prop(
    rt: &Arc<TapRuntime>,
    handle: u64,
    insp_ptr: *mut c_void,
    iface: *mut c_void,
    prop: BrushPropKind,
    target_color: RawUiColor,
    desired_hide: bool,
    restored_saved: Option<SavedBrush>,
    applied_flag: u32,
) {
    let _ = sync_overflow_brush_prop_with_mode(
        rt,
        handle,
        insp_ptr,
        iface,
        prop,
        target_color,
        desired_hide,
        restored_saved,
        applied_flag,
        false,
    );
}

pub(super) unsafe fn apply_shape_appearance(
    rt: &Arc<TapRuntime>,
    insp_ptr: *mut c_void,
    desired_hide: bool,
    orig_opacity: f64,
    orig_visibility: i32,
) {
    if insp_ptr.is_null() {
        return;
    }
    let key = com_identity_key(insp_ptr);
    let (eff_orig_opacity, eff_orig_visibility) = if desired_hide {
        track_modified_shape(key, insp_ptr, orig_opacity, orig_visibility);
        (orig_opacity, orig_visibility)
    } else {
        remove_modified_shape(key).unwrap_or((orig_opacity, orig_visibility))
    };
    let ui_elem = qi_interface(insp_ptr, &IID_IUIELEMENT);
    if ui_elem.is_null() {
        return;
    }
    let ui = ui_elem.as_ptr().cast::<RawIUIElement>();
    let target_op = if desired_hide { 0.0 } else { eff_orig_opacity };
    let mut current_val = -1.0f64;
    let hr_get = ((*(*ui).lp_vtbl).get_opacity)(ui_elem.as_ptr(), &mut current_val);
    if hr_get.is_err() || (current_val - target_op).abs() > 0.001 {
        let hr_op = ((*(*ui).lp_vtbl).put_opacity)(ui_elem.as_ptr(), target_op);
        rt.last_put_opacity_hr.store(hr_op.0, Ordering::Release);
    }

    let target_vis = if desired_hide {
        VISIBILITY_COLLAPSED
    } else {
        eff_orig_visibility
    };
    let mut current_vis = -1i32;
    let hr_gv = ((*(*ui).lp_vtbl).get_visibility)(ui_elem.as_ptr(), &mut current_vis);
    if hr_gv.is_err() || current_vis != target_vis {
        let hr_vis = ((*(*ui).lp_vtbl).put_visibility)(ui_elem.as_ptr(), target_vis);
        rt.last_put_visibility_hr.store(hr_vis.0, Ordering::Release);
    }
}

pub(super) unsafe fn restore_all_general_targets_for_current_thread(rt: &Arc<TapRuntime>) {
    let entries: Vec<(u64, TargetKind, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        s.general_modified_targets
            .drain()
            .map(|(k, (kind, weak_obj))| (k, kind, weak_obj))
            .collect()
    });
    if entries.is_empty() {
        return;
    }
    let mut keys = Vec::with_capacity(entries.len());
    for (key, kind, weak_com) in entries {
        keys.push(key);
        if let Some(insp) = resolve_weak_inspectable(&weak_com) {
            apply_target_appearance(rt, key, insp.as_ptr(), kind, false, 1.0, VISIBILITY_VISIBLE);
        }
    }
    purge_saved_brushes_for_current_thread(&keys);
}

pub(super) unsafe fn restore_all_modified_shapes_for_current_thread(rt: &Arc<TapRuntime>) {
    let entries: Vec<(ComRaw, f64, i32)> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        s.modified_shapes.drain().map(|(_, entry)| entry).collect()
    });
    for (weak_com, orig_opacity, orig_visibility) in entries {
        if let Some(insp) = resolve_weak_inspectable(&weak_com) {
            apply_shape_appearance(rt, insp.as_ptr(), false, orig_opacity, orig_visibility);
        }
    }
}

pub(super) unsafe fn measure_element_bounds_dip(
    insp_ptr: *mut c_void,
    min_w: f32,
    min_h: f32,
    min_opacity: f64,
) -> Option<[f32; 4]> {
    if insp_ptr.is_null() {
        return None;
    }
    let ui_elem = qi_interface(insp_ptr, &IID_IUIELEMENT);
    if ui_elem.is_null() {
        return None;
    }
    let ui = ui_elem.as_ptr().cast::<RawIUIElement>();
    let mut vis = VISIBILITY_COLLAPSED;
    if ((*(*ui).lp_vtbl).get_visibility)(ui_elem.as_ptr(), &mut vis).is_err()
        || vis != VISIBILITY_VISIBLE
    {
        return None;
    }
    let mut op = 0.0f64;
    if ((*(*ui).lp_vtbl).get_opacity)(ui_elem.as_ptr(), &mut op).is_err()
        || !op.is_finite()
        || op <= min_opacity
    {
        return None;
    }
    if let Some(vth) = cached_visual_tree_helper() {
        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
        let mut cur_dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        for hop in 0..32 {
            if cur_dep.is_null() {
                return None;
            }
            let mut parent_dep = ComRaw::null();
            if ((*(*vth_iface).lp_vtbl).get_parent)(
                vth.as_ptr(),
                cur_dep.as_ptr(),
                &mut parent_dep.0,
            )
            .is_err()
            {
                return None;
            }
            if parent_dep.is_null() {
                if hop == 0 {
                    return None;
                }
                break;
            }
            let p_ui = qi_interface(parent_dep.as_ptr(), &IID_IUIELEMENT);
            if !p_ui.is_null() {
                let p_ui_iface = p_ui.as_ptr().cast::<RawIUIElement>();
                let mut p_vis = VISIBILITY_COLLAPSED;
                if ((*(*p_ui_iface).lp_vtbl).get_visibility)(p_ui.as_ptr(), &mut p_vis)
                    .is_err()
                    || p_vis != VISIBILITY_VISIBLE
                {
                    return None;
                }
                let mut p_op = 0.0f64;
                if ((*(*p_ui_iface).lp_vtbl).get_opacity)(p_ui.as_ptr(), &mut p_op).is_err()
                    || !p_op.is_finite()
                    || p_op <= min_opacity
                {
                    return None;
                }
            }
            cur_dep = parent_dep;
        }
    }
    let mut sz = RawSize {
        width: 0.0,
        height: 0.0,
    };
    if ((*(*ui).lp_vtbl).get_render_size)(ui_elem.as_ptr(), &mut sz).is_err()
        || !sz.width.is_finite()
        || !sz.height.is_finite()
        || sz.width < min_w
        || sz.height < min_h
    {
        return None;
    }
    let mut gt_com = ComRaw::null();
    if ((*(*ui).lp_vtbl).transform_to_visual)(ui_elem.as_ptr(), null_mut(), &mut gt_com.0)
        .is_err()
        || gt_com.is_null()
    {
        return None;
    }
    let gt = gt_com.as_ptr().cast::<RawIGeneralTransform>();
    let mut p0 = RawPoint { x: 0.0, y: 0.0 };
    let mut p1 = RawPoint { x: 0.0, y: 0.0 };
    if ((*(*gt).lp_vtbl).transform_point)(
        gt_com.as_ptr(),
        RawPoint { x: 0.0, y: 0.0 },
        &mut p0,
    )
    .is_err()
        || ((*(*gt).lp_vtbl).transform_point)(
            gt_com.as_ptr(),
            RawPoint {
                x: sz.width,
                y: sz.height,
            },
            &mut p1,
        )
        .is_err()
    {
        return None;
    }
    if !p0.x.is_finite()
        || !p0.y.is_finite()
        || !p1.x.is_finite()
        || !p1.y.is_finite()
        || (p1.x - p0.x) < min_w
        || (p1.y - p0.y) < min_h
    {
        return None;
    }
    Some([p0.x, p0.y, p1.x, p1.y])
}

pub(super) unsafe fn measure_hover_flyout_bounds_dip(insp_ptr: *mut c_void) -> Option<[f32; 4]> {
    if let Some(bounds) = measure_element_bounds_dip(insp_ptr, 48.0, 36.0, 0.05) {
        return Some(bounds);
    }
    let vth = cached_visual_tree_helper()?;
    let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
    let dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
    if dep.is_null() {
        return None;
    }
    let mut parent_dep = ComRaw::null();
    if ((*(*vth_iface).lp_vtbl).get_parent)(vth.as_ptr(), dep.as_ptr(), &mut parent_dep.0)
        .is_err()
        || parent_dep.is_null()
    {
        return None;
    }
    if element_framework_name(parent_dep.as_ptr()).as_deref() == Some("HoverFlyoutGrid") {
        return measure_element_bounds_dip(parent_dep.as_ptr(), 48.0, 36.0, 0.05);
    }
    None
}

pub(super) unsafe fn measure_tooltip_bounds_dip(insp_ptr: *mut c_void) -> Option<[f32; 4]> {
    if let Some(vth) = cached_visual_tree_helper() {
        let dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        if !dep.is_null() {
            let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
            let mut parent_dep = ComRaw::null();
            if ((*(*vth_iface).lp_vtbl).get_parent)(
                vth.as_ptr(),
                dep.as_ptr(),
                &mut parent_dep.0,
            )
            .is_ok()
                && !parent_dep.is_null()
            {
                let tt_com = qi_interface(parent_dep.as_ptr(), &IID_ITOOLTIP);
                if !tt_com.is_null() {
                    let tt = tt_com.as_ptr().cast::<RawIToolTip>();
                    let mut is_open = 1u8;
                    if ((*(*tt).lp_vtbl).get_is_open)(tt_com.as_ptr(), &mut is_open).is_ok()
                        && is_open == 0
                    {
                        return None;
                    }
                }
            }
        }
    }
    measure_element_bounds_dip(insp_ptr, 20.0, 16.0, 0.05)
}

pub(super) unsafe fn measure_input_switcher_bounds_dip(
    insp_ptr: *mut c_void,
) -> Option<[f32; 4]> {
    if insp_ptr.is_null() {
        return None;
    }
    if let Some(vth) = cached_visual_tree_helper() {
        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
        let mut cur_dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        for _ in 0..6 {
            if cur_dep.is_null() {
                break;
            }
            let mut parent_dep = ComRaw::null();
            if ((*(*vth_iface).lp_vtbl).get_parent)(
                vth.as_ptr(),
                cur_dep.as_ptr(),
                &mut parent_dep.0,
            )
            .is_err()
                || parent_dep.is_null()
            {
                break;
            }
            let p_ui = qi_interface(parent_dep.as_ptr(), &IID_IUIELEMENT);
            if !p_ui.is_null() {
                let p_ui_iface = p_ui.as_ptr().cast::<RawIUIElement>();
                let mut p_vis = VISIBILITY_COLLAPSED;
                if ((*(*p_ui_iface).lp_vtbl).get_visibility)(p_ui.as_ptr(), &mut p_vis).is_err()
                    || p_vis != VISIBILITY_VISIBLE
                {
                    return None;
                }
                let mut p_op = 0.0f64;
                if ((*(*p_ui_iface).lp_vtbl).get_opacity)(p_ui.as_ptr(), &mut p_op).is_err()
                    || !p_op.is_finite()
                    || p_op < 0.05
                {
                    return None;
                }
            }
            cur_dep = parent_dep;
        }
    }
    measure_element_bounds_dip(insp_ptr, 120.0, 60.0, 0.05)
}

pub(super) unsafe fn measure_menu_flyout_bounds_dip(insp_ptr: *mut c_void) -> Option<[f32; 4]> {
    let vth = cached_visual_tree_helper()?;
    let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
    let mut cur_dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
    let mut ancestor_hops = 0usize;
    for _ in 0..8 {
        if cur_dep.is_null() {
            break;
        }
        let mut parent_dep = ComRaw::null();
        if ((*(*vth_iface).lp_vtbl).get_parent)(
            vth.as_ptr(),
            cur_dep.as_ptr(),
            &mut parent_dep.0,
        )
        .is_err()
            || parent_dep.is_null()
        {
            break;
        }
        let p_ui = qi_interface(parent_dep.as_ptr(), &IID_IUIELEMENT);
        if !p_ui.is_null() {
            let p_ui_iface = p_ui.as_ptr().cast::<RawIUIElement>();
            let mut p_vis = VISIBILITY_COLLAPSED;
            if ((*(*p_ui_iface).lp_vtbl).get_visibility)(p_ui.as_ptr(), &mut p_vis).is_err()
                || p_vis != VISIBILITY_VISIBLE
            {
                return None;
            }
            let mut p_op = 0.0f64;
            if ((*(*p_ui_iface).lp_vtbl).get_opacity)(p_ui.as_ptr(), &mut p_op).is_err()
                || !p_op.is_finite()
                || p_op < 0.05
            {
                return None;
            }
        }
        ancestor_hops += 1;
        cur_dep = parent_dep;
    }
    if ancestor_hops < 2 {
        return None;
    }
    measure_element_bounds_dip(insp_ptr, 64.0, 28.0, 0.05)
}

pub(super) unsafe fn apply_target_appearance(
    rt: &Arc<TapRuntime>,
    handle: u64,
    insp_ptr: *mut c_void,
    kind: TargetKind,
    desired_hide: bool,
    orig_opacity: f64,
    orig_visibility: i32,
) {
    if insp_ptr.is_null() {
        return;
    }
    register_handle_identity(handle, com_identity_key(insp_ptr));
    match kind {
        TargetKind::ShapeElement => {
            apply_shape_appearance(rt, insp_ptr, desired_hide, orig_opacity, orig_visibility);
        }
        TargetKind::HoverFlyoutShape => {
            apply_shape_appearance(rt, insp_ptr, desired_hide, orig_opacity, orig_visibility);
            if desired_hide {
                rt.diag_flags
                    .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
            }
        }
        TargetKind::HoverFlyoutBorder => {
            apply_hover_flyout_border_appearance(rt, handle, insp_ptr, desired_hide);
        }
        TargetKind::OverflowBorder => {
            apply_border_target_appearance(
                rt,
                handle,
                insp_ptr,
                TargetKind::OverflowBorder,
                RawUiColor::TRANSPARENT,
                RawUiColor::TRANSPARENT,
                desired_hide,
                DIAG_OVERFLOW_APPLIED,
            );
        }
        TargetKind::AltTabBorder => {
            apply_border_target_appearance(
                rt,
                handle,
                insp_ptr,
                TargetKind::AltTabBorder,
                RawUiColor::TRANSPARENT,
                RawUiColor::TRANSPARENT,
                desired_hide,
                DIAG_ALTTAB_APPLIED,
            );
        }
        TargetKind::AltTabItemBorder => {
            let bg_color =
                RawUiColor::from_argb(rt.alt_tab_item_bg_argb.load(Ordering::Acquire));
            let border_color =
                RawUiColor::from_argb(rt.alt_tab_item_border_argb.load(Ordering::Acquire));
            apply_alttab_item_border_appearance(
                rt,
                insp_ptr,
                None,
                bg_color,
                border_color,
                desired_hide,
            );
        }
        TargetKind::AltTabItemFocus => {
            apply_focus_secondary_appearance(rt, insp_ptr, desired_hide);
        }
        TargetKind::OsdPanel => {
            apply_osd_panel_appearance(rt, handle, insp_ptr, desired_hide);
        }
        TargetKind::ToolTipPresenter => {
            apply_tooltip_presenter_appearance(rt, handle, insp_ptr, desired_hide);
        }
        TargetKind::MenuFlyoutSurface => {
            apply_menu_flyout_surface_appearance(rt, handle, insp_ptr, desired_hide);
        }
        TargetKind::ShellCardSurface | TargetKind::ShellCardSubSurface => {
            apply_shell_card_surface_appearance(rt, handle, insp_ptr, kind, desired_hide);
        }
        TargetKind::StartMenuModalSurface => {
            apply_start_menu_modal_surface_appearance(rt, handle, insp_ptr, desired_hide);
        }
        TargetKind::VirtualDesktopBarSurface => {
            apply_virtual_desktop_bar_appearance(rt, handle, insp_ptr, kind, desired_hide);
        }
        TargetKind::InputSwitcherPanel => {
            apply_input_switcher_panel_appearance(rt, handle, insp_ptr, desired_hide);
        }
        TargetKind::InputSwitcherSubPanel => {
            apply_input_switcher_sub_panel_appearance(rt, handle, insp_ptr, desired_hide);
        }
        TargetKind::OverflowPanel => {
            let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
            if panel_com.is_null() {
                return;
            }
            let key = com_identity_key(insp_ptr);
            let saved = if desired_hide {
                track_general_modified_target(key, TargetKind::OverflowPanel, insp_ptr);
                SavedTargetBrushes::default()
            } else {
                remove_general_modified_target(key);
                unregister_watches_for_keys(&[key]);
                take_saved_target_brushes(key)
            };
            sync_overflow_brush_prop(
                rt,
                key,
                insp_ptr,
                panel_com.as_ptr(),
                BrushPropKind::PanelBackground,
                RawUiColor::TRANSPARENT,
                desired_hide,
                saved.background,
                DIAG_OVERFLOW_APPLIED,
            );
            if desired_hide {
                ensure_prop_watch_registered(
                    rt,
                    key,
                    TargetKind::OverflowPanel,
                    BrushPropKind::PanelBackground,
                    insp_ptr,
                );
            }
        }
    }
}

