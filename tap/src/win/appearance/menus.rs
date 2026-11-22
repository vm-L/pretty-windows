use super::alttab::approx_eq_corner_radius;
use super::{sync_element_shadow_prop, sync_overflow_brush_prop, sync_overflow_brush_prop_with_mode};
use super::super::super::tree_match::TargetKind;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use super::super::brush_store::*;
use super::super::raw_com::*;
use super::super::TapRuntime;

unsafe fn find_parent_tooltip_control(insp_ptr: *mut c_void) -> Option<(u64, ComRaw)> {
    let vth = cached_visual_tree_helper()?;
    let dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
    if dep.is_null() {
        return None;
    }
    let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
    let mut parent_dep = ComRaw::null();
    if ((*(*vth_iface).lp_vtbl).get_parent)(vth.as_ptr(), dep.as_ptr(), &mut parent_dep.0)
        .is_err()
        || parent_dep.is_null()
    {
        return None;
    }
    let tt_com = qi_interface(parent_dep.as_ptr(), &IID_ITOOLTIP);
    let ctrl_com = qi_interface(parent_dep.as_ptr(), &IID_ICONTROL);
    if tt_com.is_null() || ctrl_com.is_null() {
        return None;
    }
    let parent_insp = qi_interface(parent_dep.as_ptr(), &IID_IINSPECTABLE);
    if parent_insp.is_null() {
        return None;
    }
    let parent_key = com_identity_key(parent_insp.as_ptr());
    Some((parent_key, parent_insp))
}

unsafe fn find_parent_menu_flyout_control(insp_ptr: *mut c_void) -> Option<(u64, ComRaw)> {
    let vth = cached_visual_tree_helper()?;
    let mut cur_dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
    if cur_dep.is_null() {
        return None;
    }
    let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
    for _ in 0..4 {
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
        let ctrl_com = qi_interface(parent_dep.as_ptr(), &IID_ICONTROL);
        if !ctrl_com.is_null() {
            let parent_insp = qi_interface(parent_dep.as_ptr(), &IID_IINSPECTABLE);
            if !parent_insp.is_null() {
                if let Some(cls) = element_runtime_class_name(parent_insp.as_ptr()) {
                    if cls.contains("MenuFlyoutPresenter")
                        || ((cls.contains("JumpList") || cls.contains("JumpView"))
                            && !cls.contains("Item"))
                    {
                        let parent_key = com_identity_key(parent_insp.as_ptr());
                        return Some((parent_key, parent_insp));
                    }
                }
            }
        }
        cur_dep = parent_dep;
    }
    None
}

unsafe fn sync_control_corner_radius_in_map(
    ctrl_key: u64,
    ctrl_ptr: *mut c_void,
    apply_round: bool,
    get_map: fn(&mut UiThreadBrushStore) -> &mut HashMap<u64, SavedCornerRadius>,
    read_map: fn(&UiThreadBrushStore) -> &HashMap<u64, SavedCornerRadius>,
) {
    if ctrl_ptr.is_null() {
        return;
    }
    let ctrl7_com = qi_interface(ctrl_ptr, &IID_ICONTROL7);
    if ctrl7_com.is_null() {
        return;
    }
    let ctrl7 = ctrl7_com.as_ptr().cast::<RawIControl7>();
    if apply_round {
        let already_saved = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .is_some_and(|s| read_map(&s).contains_key(&ctrl_key))
        });
        if !already_saved {
            let mut saved_cr: Option<SavedCornerRadius> = None;
            let dp = ensure_cached_com(
                |s| s.control_corner_radius_dp.as_ref(),
                |s, v| s.control_corner_radius_dp = Some(v),
                fetch_control_corner_radius_dp,
            );
            let unset = ensure_cached_com(
                |s| s.unset_value.as_ref(),
                |s, v| s.unset_value = Some(v),
                fetch_unset_value,
            );
            if let (Some(dp_com), Some(unset_com)) = (dp, unset) {
                let dep_com = qi_interface(ctrl_ptr, &IID_IDEPENDENCYOBJECT);
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
                        && com_same_identity(local_val.as_ptr(), unset_com.as_ptr())
                    {
                        saved_cr = Some(SavedCornerRadius::Unset);
                    }
                }
            }
            if saved_cr.is_none() {
                let mut cur_cr = RawCornerRadius::default();
                if ((*(*ctrl7).lp_vtbl).get_corner_radius)(ctrl7_com.as_ptr(), &mut cur_cr)
                    .is_ok()
                {
                    saved_cr = Some(SavedCornerRadius::Local(cur_cr));
                }
            }
            if let Some(orig) = saved_cr {
                UI_BRUSH_STORE.with(|cell| {
                    if let Ok(mut s) = cell.try_borrow_mut() {
                        get_map(&mut s).entry(ctrl_key).or_insert(orig);
                    }
                });
            }
        }
        let target_cr = RawCornerRadius {
            top_left: 8.0,
            top_right: 8.0,
            bottom_right: 8.0,
            bottom_left: 8.0,
        };
        let mut cur_cr = RawCornerRadius::default();
        if ((*(*ctrl7).lp_vtbl).get_corner_radius)(ctrl7_com.as_ptr(), &mut cur_cr).is_err()
            || !approx_eq_corner_radius(cur_cr, target_cr)
        {
            let _ = ((*(*ctrl7).lp_vtbl).put_corner_radius)(ctrl7_com.as_ptr(), target_cr);
        }
    } else {
        let orig = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| get_map(&mut s).remove(&ctrl_key))
        });
        match orig {
            Some(SavedCornerRadius::Unset) => {
                let dp = ensure_cached_com(
                    |s| s.control_corner_radius_dp.as_ref(),
                    |s, v| s.control_corner_radius_dp = Some(v),
                    fetch_control_corner_radius_dp,
                );
                let _ = clear_dp_value(ctrl_ptr, dp.as_ref());
            }
            Some(SavedCornerRadius::Local(orig_cr)) => {
                let _ = ((*(*ctrl7).lp_vtbl).put_corner_radius)(ctrl7_com.as_ptr(), orig_cr);
            }
            None => {}
        }
    }
}

unsafe fn sync_tooltip_control_corner_radius(
    tooltip_key: u64,
    tt_ptr: *mut c_void,
    apply_round: bool,
) {
    sync_control_corner_radius_in_map(
        tooltip_key,
        tt_ptr,
        apply_round,
        |s| &mut s.tooltip_orig_corner_radius,
        |s| &s.tooltip_orig_corner_radius,
    );
}

unsafe fn sync_menu_flyout_control_corner_radius(
    ctrl_key: u64,
    ctrl_ptr: *mut c_void,
    apply_round: bool,
) {
    sync_control_corner_radius_in_map(
        ctrl_key,
        ctrl_ptr,
        apply_round,
        |s| &mut s.menu_flyout_orig_corner_radius,
        |s| &s.menu_flyout_orig_corner_radius,
    );
}

unsafe fn sync_tooltip_presenter_corner_radius_fallback(
    presenter_key: u64,
    cp_ptr: *mut c_void,
    apply_round: bool,
) {
    if cp_ptr.is_null() {
        return;
    }
    let cp = cp_ptr.cast::<RawIContentPresenter4>();
    if apply_round {
        let target_cr = RawCornerRadius {
            top_left: 8.0,
            top_right: 8.0,
            bottom_right: 8.0,
            bottom_left: 8.0,
        };
        let mut cur_cr = RawCornerRadius::default();
        if ((*(*cp).lp_vtbl).get_corner_radius)(cp_ptr, &mut cur_cr).is_ok()
            && approx_eq_corner_radius(cur_cr, target_cr)
        {
            return;
        }
        let already_saved = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .is_some_and(|s| s.tooltip_orig_corner_radius.contains_key(&presenter_key))
        });
        if !already_saved
            && ((*(*cp).lp_vtbl).get_corner_radius)(cp_ptr, &mut cur_cr).is_ok()
        {
            UI_BRUSH_STORE.with(|cell| {
                if let Ok(mut s) = cell.try_borrow_mut() {
                    s.tooltip_orig_corner_radius
                        .entry(presenter_key)
                        .or_insert(SavedCornerRadius::Local(cur_cr));
                }
            });
        }
        let _ = ((*(*cp).lp_vtbl).put_corner_radius)(cp_ptr, target_cr);
    } else {
        let orig = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.tooltip_orig_corner_radius.remove(&presenter_key))
        });
        if let Some(SavedCornerRadius::Local(orig_cr)) = orig {
            let _ = ((*(*cp).lp_vtbl).put_corner_radius)(cp_ptr, orig_cr);
        }
    }
}

unsafe fn restore_tooltip_pair(
    rt: &Arc<TapRuntime>,
    presenter_key: u64,
    presenter_insp: Option<*mut c_void>,
    tooltip_key_opt: Option<u64>,
    tooltip_insp: Option<*mut c_void>,
) {
    let tk_eff = tooltip_insp
        .map(|p| com_identity_key(p))
        .or(tooltip_key_opt);

    let mut keys_to_clean = Vec::with_capacity(2);
    if presenter_key != 0 {
        keys_to_clean.push(presenter_key);
    }
    if let Some(tk) = tk_eff.filter(|&k| k != 0 && !keys_to_clean.contains(&k)) {
        keys_to_clean.push(tk);
    }
    unregister_watches_for_keys(&keys_to_clean);

    let (target_bg, target_border) = rt.tooltip_target_colors();

    if let (Some(tk), Some(tt_ptr)) = (tk_eff, tooltip_insp) {
        let tt_saved = take_saved_target_brushes(tk);
        let ctrl_com = qi_interface(tt_ptr, &IID_ICONTROL);
        if !ctrl_com.is_null() {
            sync_overflow_brush_prop(
                rt,
                tk,
                tt_ptr,
                ctrl_com.as_ptr(),
                BrushPropKind::ControlBackground,
                target_bg,
                false,
                tt_saved.background,
                DIAG_TOOLTIP_APPLIED,
            );
            sync_overflow_brush_prop(
                rt,
                tk,
                tt_ptr,
                ctrl_com.as_ptr(),
                BrushPropKind::ControlBorderBrush,
                target_border,
                false,
                tt_saved.border_brush,
                DIAG_TOOLTIP_APPLIED,
            );
        }
        sync_tooltip_control_corner_radius(tk, tt_ptr, false);
        if tt_saved.shadow.is_some() {
            sync_element_shadow_prop(
                rt,
                tk,
                tt_ptr,
                false,
                tt_saved.shadow,
                DIAG_TOOLTIP_SHADOW_APPLIED,
                false,
            );
        }
    }

    if let Some(cp_ptr) = presenter_insp {
        let cp_saved = take_saved_target_brushes(presenter_key);
        if cp_saved.shadow.is_some() {
            sync_element_shadow_prop(
                rt,
                presenter_key,
                cp_ptr,
                false,
                cp_saved.shadow,
                DIAG_TOOLTIP_SHADOW_APPLIED,
                true,
            );
        }
        let cp_com = qi_interface(cp_ptr, &IID_ICONTENTPRESENTER4);
        if !cp_com.is_null() {
            if let Some(sb_bg) = cp_saved.background {
                let _ = sync_overflow_brush_prop_with_mode(
                    rt,
                    presenter_key,
                    cp_ptr,
                    cp_com.as_ptr(),
                    BrushPropKind::ContentPresenterBackground,
                    target_bg,
                    false,
                    Some(sb_bg),
                    DIAG_TOOLTIP_APPLIED,
                    true,
                );
            }
            if let Some(sb_bb) = cp_saved.border_brush {
                let _ = sync_overflow_brush_prop_with_mode(
                    rt,
                    presenter_key,
                    cp_ptr,
                    cp_com.as_ptr(),
                    BrushPropKind::ContentPresenterBorderBrush,
                    target_border,
                    false,
                    Some(sb_bb),
                    DIAG_TOOLTIP_APPLIED,
                    true,
                );
            }
            sync_tooltip_presenter_corner_radius_fallback(
                presenter_key,
                cp_com.as_ptr(),
                false,
            );
        }
    }

    purge_saved_brushes_for_current_thread(&keys_to_clean);
}

pub(in super::super) unsafe fn restore_all_tooltip_targets_for_current_thread(rt: &Arc<TapRuntime>) {
    let entries: Vec<(u64, TooltipTrackedEntry)> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        s.tooltip_modified_targets.drain().collect()
    });
    for (presenter_key, entry) in entries {
        let cp_insp = resolve_weak_inspectable(&entry.presenter_weak);
        let tt_insp = entry
            .tooltip_weak
            .as_ref()
            .and_then(|tw| resolve_weak_inspectable(tw));
        restore_tooltip_pair(
            rt,
            presenter_key,
            cp_insp.as_ref().map(ComRaw::as_ptr),
            entry.tooltip_key,
            tt_insp.as_ref().map(ComRaw::as_ptr),
        );
    }
}

pub(super) unsafe fn apply_tooltip_presenter_appearance(
    rt: &Arc<TapRuntime>,
    _handle: u64,
    insp_ptr: *mut c_void,
    desired_hide: bool,
) {
    if insp_ptr.is_null() {
        return;
    }
    let presenter_key = com_identity_key(insp_ptr);
    let parent_tooltip = find_parent_tooltip_control(insp_ptr);

    if desired_hide {
        let parent_pair = parent_tooltip
            .as_ref()
            .map(|(tk, insp)| (*tk, insp.as_ptr()));
        track_tooltip_modified_target(presenter_key, insp_ptr, parent_pair);

        let (target_bg, target_border) = rt.tooltip_target_colors();
        let apply_round = rt.is_tooltip_suppressed();

        if let Some((tooltip_key, ref tt_insp)) = parent_tooltip {
            let tt_ptr = tt_insp.as_ptr();
            let ctrl_com = qi_interface(tt_ptr, &IID_ICONTROL);
            if !ctrl_com.is_null() {
                sync_overflow_brush_prop(
                    rt,
                    tooltip_key,
                    tt_ptr,
                    ctrl_com.as_ptr(),
                    BrushPropKind::ControlBackground,
                    target_bg,
                    true,
                    None,
                    DIAG_TOOLTIP_APPLIED,
                );
                sync_overflow_brush_prop(
                    rt,
                    tooltip_key,
                    tt_ptr,
                    ctrl_com.as_ptr(),
                    BrushPropKind::ControlBorderBrush,
                    target_border,
                    true,
                    None,
                    DIAG_TOOLTIP_APPLIED,
                );
                ensure_prop_watch_registered(
                    rt,
                    tooltip_key,
                    TargetKind::ToolTipPresenter,
                    BrushPropKind::ControlBackground,
                    tt_ptr,
                );
                ensure_prop_watch_registered(
                    rt,
                    tooltip_key,
                    TargetKind::ToolTipPresenter,
                    BrushPropKind::ControlBorderBrush,
                    tt_ptr,
                );
            }
            sync_tooltip_control_corner_radius(tooltip_key, tt_ptr, apply_round);
            sync_element_shadow_prop(
                rt,
                tooltip_key,
                tt_ptr,
                true,
                None,
                DIAG_TOOLTIP_SHADOW_APPLIED,
                false,
            );
            ensure_prop_watch_registered(
                rt,
                tooltip_key,
                TargetKind::ToolTipPresenter,
                BrushPropKind::ElementShadow,
                tt_ptr,
            );
        }

        sync_element_shadow_prop(
            rt,
            presenter_key,
            insp_ptr,
            true,
            None,
            DIAG_TOOLTIP_SHADOW_APPLIED,
            true,
        );
        ensure_prop_watch_registered(
            rt,
            presenter_key,
            TargetKind::ToolTipPresenter,
            BrushPropKind::ElementShadow,
            insp_ptr,
        );

        // Preserve {TemplateBinding Background/BorderBrush/CornerRadius} on ContentPresenter#LayoutRoot:
        // only fall back to local mutation if LayoutRoot did not already inherit ToolTip's values.
        let cp_com = qi_interface(insp_ptr, &IID_ICONTENTPRESENTER4);
        if !cp_com.is_null() {
            let wrote_cp_bg = sync_overflow_brush_prop_with_mode(
                rt,
                presenter_key,
                insp_ptr,
                cp_com.as_ptr(),
                BrushPropKind::ContentPresenterBackground,
                target_bg,
                true,
                None,
                DIAG_TOOLTIP_APPLIED,
                true,
            );
            if wrote_cp_bg
                || parent_tooltip.is_none()
                || has_saved_brush_slot(presenter_key, BrushSlot::Background)
            {
                ensure_prop_watch_registered(
                    rt,
                    presenter_key,
                    TargetKind::ToolTipPresenter,
                    BrushPropKind::ContentPresenterBackground,
                    insp_ptr,
                );
            }
            let wrote_cp_bb = sync_overflow_brush_prop_with_mode(
                rt,
                presenter_key,
                insp_ptr,
                cp_com.as_ptr(),
                BrushPropKind::ContentPresenterBorderBrush,
                target_border,
                true,
                None,
                DIAG_TOOLTIP_APPLIED,
                true,
            );
            if wrote_cp_bb
                || parent_tooltip.is_none()
                || has_saved_brush_slot(presenter_key, BrushSlot::BorderBrush)
            {
                ensure_prop_watch_registered(
                    rt,
                    presenter_key,
                    TargetKind::ToolTipPresenter,
                    BrushPropKind::ContentPresenterBorderBrush,
                    insp_ptr,
                );
            }
            if apply_round
                || UI_BRUSH_STORE.with(|cell| {
                    cell.try_borrow()
                        .ok()
                        .is_some_and(|s| s.tooltip_orig_corner_radius.contains_key(&presenter_key))
                })
            {
                sync_tooltip_presenter_corner_radius_fallback(
                    presenter_key,
                    cp_com.as_ptr(),
                    apply_round,
                );
            }
        }
    } else {
        let tracked = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.tooltip_modified_targets.remove(&presenter_key))
        });
        let resolved_parent = if parent_tooltip.is_some() {
            parent_tooltip
        } else {
            tracked.as_ref().and_then(|e| {
                e.tooltip_weak
                    .as_ref()
                    .and_then(|tw| resolve_weak_inspectable(tw))
                    .map(|insp| (e.tooltip_key.unwrap_or_else(|| com_identity_key(insp.as_ptr())), insp))
            })
        };
        let tk_opt = resolved_parent
            .as_ref()
            .map(|(tk, _)| *tk)
            .or_else(|| tracked.as_ref().and_then(|e| e.tooltip_key));
        restore_tooltip_pair(
            rt,
            presenter_key,
            Some(insp_ptr),
            tk_opt,
            resolved_parent.as_ref().map(|(_, insp)| insp.as_ptr()),
        );
    }
}

unsafe fn sync_menu_flyout_border_corner_radius_fallback(
    surface_key: u64,
    border_ptr: *mut c_void,
    apply_round: bool,
) {
    if border_ptr.is_null() {
        return;
    }
    let border = border_ptr.cast::<RawIBorder>();
    if apply_round {
        let target_cr = RawCornerRadius {
            top_left: 8.0,
            top_right: 8.0,
            bottom_right: 8.0,
            bottom_left: 8.0,
        };
        let mut cur_cr = RawCornerRadius::default();
        if ((*(*border).lp_vtbl).get_corner_radius)(border_ptr, &mut cur_cr).is_ok()
            && approx_eq_corner_radius(cur_cr, target_cr)
        {
            return;
        }
        let already_saved = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .is_some_and(|s| s.menu_flyout_orig_corner_radius.contains_key(&surface_key))
        });
        if !already_saved
            && ((*(*border).lp_vtbl).get_corner_radius)(border_ptr, &mut cur_cr).is_ok()
        {
            UI_BRUSH_STORE.with(|cell| {
                if let Ok(mut s) = cell.try_borrow_mut() {
                    s.menu_flyout_orig_corner_radius
                        .entry(surface_key)
                        .or_insert(SavedCornerRadius::Local(cur_cr));
                }
            });
        }
        let _ = ((*(*border).lp_vtbl).put_corner_radius)(border_ptr, target_cr);
    } else {
        let orig = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.menu_flyout_orig_corner_radius.remove(&surface_key))
        });
        if let Some(SavedCornerRadius::Local(orig_cr)) = orig {
            let _ = ((*(*border).lp_vtbl).put_corner_radius)(border_ptr, orig_cr);
        }
    }
}

unsafe fn restore_menu_flyout_pair(
    rt: &Arc<TapRuntime>,
    surface_key: u64,
    surface_insp: Option<*mut c_void>,
    ctrl_key_opt: Option<u64>,
    ctrl_insp: Option<*mut c_void>,
) {
    let ck_eff = ctrl_insp.map(|p| com_identity_key(p)).or(ctrl_key_opt);

    let mut keys_to_clean = Vec::with_capacity(2);
    if surface_key != 0 {
        keys_to_clean.push(surface_key);
    }
    if let Some(ck) = ck_eff.filter(|&k| k != 0 && !keys_to_clean.contains(&k)) {
        keys_to_clean.push(ck);
    }
    unregister_watches_for_keys(&keys_to_clean);

    let (target_bg, target_border) = rt.menu_flyout_target_colors();

    if let (Some(ck), Some(ctrl_ptr)) = (ck_eff, ctrl_insp) {
        let ctrl_saved = take_saved_target_brushes(ck);
        let ctrl_com = qi_interface(ctrl_ptr, &IID_ICONTROL);
        if !ctrl_com.is_null() {
            sync_overflow_brush_prop(
                rt,
                ck,
                ctrl_ptr,
                ctrl_com.as_ptr(),
                BrushPropKind::ControlBackground,
                target_bg,
                false,
                ctrl_saved.background,
                DIAG_MENU_FLYOUT_APPLIED,
            );
            sync_overflow_brush_prop(
                rt,
                ck,
                ctrl_ptr,
                ctrl_com.as_ptr(),
                BrushPropKind::ControlBorderBrush,
                target_border,
                false,
                ctrl_saved.border_brush,
                DIAG_MENU_FLYOUT_APPLIED,
            );
        }
        sync_menu_flyout_control_corner_radius(ck, ctrl_ptr, false);
        if ctrl_saved.shadow.is_some() {
            sync_element_shadow_prop(
                rt,
                ck,
                ctrl_ptr,
                false,
                ctrl_saved.shadow,
                DIAG_MENU_FLYOUT_SHADOW_APPLIED,
                false,
            );
        }
    }

    if let Some(surf_ptr) = surface_insp {
        let surf_saved = take_saved_target_brushes(surface_key);
        if surf_saved.shadow.is_some() {
            sync_element_shadow_prop(
                rt,
                surface_key,
                surf_ptr,
                false,
                surf_saved.shadow,
                DIAG_MENU_FLYOUT_SHADOW_APPLIED,
                false,
            );
        }
        let border_com = qi_interface(surf_ptr, &IID_IBORDER);
        if !border_com.is_null() {
            if let Some(sb_bg) = surf_saved.background {
                let _ = sync_overflow_brush_prop_with_mode(
                    rt,
                    surface_key,
                    surf_ptr,
                    border_com.as_ptr(),
                    BrushPropKind::BorderBackground,
                    target_bg,
                    false,
                    Some(sb_bg),
                    DIAG_MENU_FLYOUT_APPLIED,
                    true,
                );
            }
            if let Some(sb_bb) = surf_saved.border_brush {
                let _ = sync_overflow_brush_prop_with_mode(
                    rt,
                    surface_key,
                    surf_ptr,
                    border_com.as_ptr(),
                    BrushPropKind::BorderBrush,
                    target_border,
                    false,
                    Some(sb_bb),
                    DIAG_MENU_FLYOUT_APPLIED,
                    true,
                );
            }
            sync_menu_flyout_border_corner_radius_fallback(
                surface_key,
                border_com.as_ptr(),
                false,
            );
        } else {
            let panel_com = qi_interface(surf_ptr, &IID_IPANEL);
            if !panel_com.is_null() {
                if let Some(sb_bg) = surf_saved.background {
                    sync_overflow_brush_prop(
                        rt,
                        surface_key,
                        surf_ptr,
                        panel_com.as_ptr(),
                        BrushPropKind::PanelBackground,
                        target_bg,
                        false,
                        Some(sb_bg),
                        DIAG_MENU_FLYOUT_APPLIED,
                    );
                }
            }
            let grid2_com = qi_interface(surf_ptr, &IID_IGRID2);
            if !grid2_com.is_null() {
                if let Some(sb_bb) = surf_saved.border_brush {
                    sync_overflow_brush_prop(
                        rt,
                        surface_key,
                        surf_ptr,
                        grid2_com.as_ptr(),
                        BrushPropKind::GridBorderBrush,
                        target_border,
                        false,
                        Some(sb_bb),
                        DIAG_MENU_FLYOUT_APPLIED,
                    );
                }
            }
        }
    }

    purge_saved_brushes_for_current_thread(&keys_to_clean);
}

pub(in super::super) unsafe fn restore_all_menu_flyout_targets_for_current_thread(rt: &Arc<TapRuntime>) {
    let entries: Vec<(u64, TooltipTrackedEntry)> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        s.menu_flyout_modified_targets.drain().collect()
    });
    for (surface_key, entry) in entries {
        let surf_insp = resolve_weak_inspectable(&entry.presenter_weak);
        let ctrl_insp = entry
            .tooltip_weak
            .as_ref()
            .and_then(|cw| resolve_weak_inspectable(cw));
        restore_menu_flyout_pair(
            rt,
            surface_key,
            surf_insp.as_ref().map(ComRaw::as_ptr),
            entry.tooltip_key,
            ctrl_insp.as_ref().map(ComRaw::as_ptr),
        );
    }
}

pub(super) unsafe fn apply_menu_flyout_surface_appearance(
    rt: &Arc<TapRuntime>,
    _handle: u64,
    insp_ptr: *mut c_void,
    desired_hide: bool,
) {
    if insp_ptr.is_null() {
        return;
    }
    let surface_key = com_identity_key(insp_ptr);
    let parent_ctrl = find_parent_menu_flyout_control(insp_ptr);

    if desired_hide {
        let parent_pair = parent_ctrl
            .as_ref()
            .map(|(ck, insp)| (*ck, insp.as_ptr()));
        track_menu_flyout_modified_target(surface_key, insp_ptr, parent_pair);

        let (target_bg, target_border) = rt.menu_flyout_target_colors();
        let apply_round = rt.is_menu_flyout_suppressed();

        if let Some((ctrl_key, ref ctrl_insp)) = parent_ctrl {
            let ctrl_ptr = ctrl_insp.as_ptr();
            let ctrl_com = qi_interface(ctrl_ptr, &IID_ICONTROL);
            if !ctrl_com.is_null() {
                sync_overflow_brush_prop(
                    rt,
                    ctrl_key,
                    ctrl_ptr,
                    ctrl_com.as_ptr(),
                    BrushPropKind::ControlBackground,
                    target_bg,
                    true,
                    None,
                    DIAG_MENU_FLYOUT_APPLIED,
                );
                sync_overflow_brush_prop(
                    rt,
                    ctrl_key,
                    ctrl_ptr,
                    ctrl_com.as_ptr(),
                    BrushPropKind::ControlBorderBrush,
                    target_border,
                    true,
                    None,
                    DIAG_MENU_FLYOUT_APPLIED,
                );
                ensure_prop_watch_registered(
                    rt,
                    ctrl_key,
                    TargetKind::MenuFlyoutSurface,
                    BrushPropKind::ControlBackground,
                    ctrl_ptr,
                );
                ensure_prop_watch_registered(
                    rt,
                    ctrl_key,
                    TargetKind::MenuFlyoutSurface,
                    BrushPropKind::ControlBorderBrush,
                    ctrl_ptr,
                );
            }
            sync_menu_flyout_control_corner_radius(ctrl_key, ctrl_ptr, apply_round);
            sync_element_shadow_prop(
                rt,
                ctrl_key,
                ctrl_ptr,
                true,
                None,
                DIAG_MENU_FLYOUT_SHADOW_APPLIED,
                false,
            );
            ensure_prop_watch_registered(
                rt,
                ctrl_key,
                TargetKind::MenuFlyoutSurface,
                BrushPropKind::ElementShadow,
                ctrl_ptr,
            );
        }

        sync_element_shadow_prop(
            rt,
            surface_key,
            insp_ptr,
            true,
            None,
            DIAG_MENU_FLYOUT_SHADOW_APPLIED,
            false,
        );
        ensure_prop_watch_registered(
            rt,
            surface_key,
            TargetKind::MenuFlyoutSurface,
            BrushPropKind::ElementShadow,
            insp_ptr,
        );

        // Preserve {TemplateBinding Background/BorderBrush/CornerRadius} on child Border
        // (e.g. MenuFlyoutPresenterRoot) when inherited from parent Control.
        let border_com = qi_interface(insp_ptr, &IID_IBORDER);
        if !border_com.is_null() {
            let wrote_b_bg = sync_overflow_brush_prop_with_mode(
                rt,
                surface_key,
                insp_ptr,
                border_com.as_ptr(),
                BrushPropKind::BorderBackground,
                target_bg,
                true,
                None,
                DIAG_MENU_FLYOUT_APPLIED,
                true,
            );
            if wrote_b_bg
                || parent_ctrl.is_none()
                || has_saved_brush_slot(surface_key, BrushSlot::Background)
            {
                ensure_prop_watch_registered(
                    rt,
                    surface_key,
                    TargetKind::MenuFlyoutSurface,
                    BrushPropKind::BorderBackground,
                    insp_ptr,
                );
            }
            let wrote_b_bb = sync_overflow_brush_prop_with_mode(
                rt,
                surface_key,
                insp_ptr,
                border_com.as_ptr(),
                BrushPropKind::BorderBrush,
                target_border,
                true,
                None,
                DIAG_MENU_FLYOUT_APPLIED,
                true,
            );
            if wrote_b_bb
                || parent_ctrl.is_none()
                || has_saved_brush_slot(surface_key, BrushSlot::BorderBrush)
            {
                ensure_prop_watch_registered(
                    rt,
                    surface_key,
                    TargetKind::MenuFlyoutSurface,
                    BrushPropKind::BorderBrush,
                    insp_ptr,
                );
            }
            if apply_round
                || UI_BRUSH_STORE.with(|cell| {
                    cell.try_borrow()
                        .ok()
                        .is_some_and(|s| s.menu_flyout_orig_corner_radius.contains_key(&surface_key))
                })
            {
                sync_menu_flyout_border_corner_radius_fallback(
                    surface_key,
                    border_com.as_ptr(),
                    apply_round,
                );
            }
        } else {
            let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
            if !panel_com.is_null() {
                sync_overflow_brush_prop(
                    rt,
                    surface_key,
                    insp_ptr,
                    panel_com.as_ptr(),
                    BrushPropKind::PanelBackground,
                    target_bg,
                    true,
                    None,
                    DIAG_MENU_FLYOUT_APPLIED,
                );
                ensure_prop_watch_registered(
                    rt,
                    surface_key,
                    TargetKind::MenuFlyoutSurface,
                    BrushPropKind::PanelBackground,
                    insp_ptr,
                );
            }
            let grid2_com = qi_interface(insp_ptr, &IID_IGRID2);
            if !grid2_com.is_null() {
                sync_overflow_brush_prop(
                    rt,
                    surface_key,
                    insp_ptr,
                    grid2_com.as_ptr(),
                    BrushPropKind::GridBorderBrush,
                    target_border,
                    true,
                    None,
                    DIAG_MENU_FLYOUT_APPLIED,
                );
                ensure_prop_watch_registered(
                    rt,
                    surface_key,
                    TargetKind::MenuFlyoutSurface,
                    BrushPropKind::GridBorderBrush,
                    insp_ptr,
                );
            }
        }
    } else {
        let tracked = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.menu_flyout_modified_targets.remove(&surface_key))
        });
        let resolved_parent = if parent_ctrl.is_some() {
            parent_ctrl
        } else {
            tracked.as_ref().and_then(|e| {
                e.tooltip_weak
                    .as_ref()
                    .and_then(|cw| resolve_weak_inspectable(cw))
                    .map(|insp| (e.tooltip_key.unwrap_or_else(|| com_identity_key(insp.as_ptr())), insp))
            })
        };
        let ck_opt = resolved_parent
            .as_ref()
            .map(|(ck, _)| *ck)
            .or_else(|| tracked.as_ref().and_then(|e| e.tooltip_key));
        restore_menu_flyout_pair(
            rt,
            surface_key,
            Some(insp_ptr),
            ck_opt,
            resolved_parent.as_ref().map(|(_, insp)| insp.as_ptr()),
        );
    }
}

