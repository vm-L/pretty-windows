use super::sync_overflow_brush_prop;
use super::super::super::tree_match::{TargetKind, ALTTAB_ITEM_OVERLAY_NAME};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows::core::{IInspectable, Interface, HSTRING};
use windows::Win32::System::WinRT::RoActivateInstance;
use super::super::brush_store::*;
use super::super::raw_com::*;
use super::super::TapRuntime;

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn apply_border_target_appearance(
    rt: &Arc<TapRuntime>,
    handle: u64,
    insp_ptr: *mut c_void,
    kind: TargetKind,
    bg_color: RawUiColor,
    border_color: RawUiColor,
    desired_hide: bool,
    applied_flag: u32,
) {
    let border_com = qi_interface(insp_ptr, &IID_IBORDER);
    if border_com.is_null() {
        return;
    }
    let saved = if desired_hide {
        SavedTargetBrushes::default()
    } else {
        take_saved_target_brushes(handle)
    };
    sync_overflow_brush_prop(
        rt,
        handle,
        insp_ptr,
        border_com.as_ptr(),
        BrushPropKind::BorderBackground,
        bg_color,
        desired_hide,
        saved.background,
        applied_flag,
    );
    sync_overflow_brush_prop(
        rt,
        handle,
        insp_ptr,
        border_com.as_ptr(),
        BrushPropKind::BorderBrush,
        border_color,
        desired_hide,
        saved.border_brush,
        applied_flag,
    );
    if desired_hide {
        rt.diag_flags.fetch_or(applied_flag, Ordering::Relaxed);
        ensure_prop_watch_registered(
            rt,
            handle,
            kind,
            BrushPropKind::BorderBackground,
            insp_ptr,
        );
        ensure_prop_watch_registered(
            rt,
            handle,
            kind,
            BrushPropKind::BorderBrush,
            insp_ptr,
        );
    }
}

pub(in super::super) unsafe fn sync_focus_secondary_thickness_prop(
    rt: &Arc<TapRuntime>,
    key: u64,
    insp_ptr: *mut c_void,
    desired_hide: bool,
    allow_clear_dp: bool,
) {
    let fe4_com = qi_interface(insp_ptr, &IID_IFRAMEWORKELEMENT4);
    if fe4_com.is_null() {
        return;
    }
    let fe4 = fe4_com.as_ptr().cast::<RawIFrameworkElement4>();
    if desired_hide {
        let mut cur_thick = RawThickness::default();
        let hr_get = ((*(*fe4).lp_vtbl).get_focus_visual_secondary_thickness)(
            fe4_com.as_ptr(),
            &mut cur_thick,
        );
        let zero = RawThickness::default();
        if hr_get.is_ok() && !approx_eq_thickness(cur_thick, zero) {
            UI_BRUSH_STORE.with(|cell| {
                if let Ok(mut s) = cell.try_borrow_mut() {
                    s.alttab_item_orig_focus_thickness
                        .entry(key)
                        .or_insert(cur_thick);
                }
            });
        }
        if hr_get.is_err() || !approx_eq_thickness(cur_thick, zero) {
            let _sync_guard = PropSyncGuard::enter();
            if ((*(*fe4).lp_vtbl).put_focus_visual_secondary_thickness)(
                fe4_com.as_ptr(),
                zero,
            )
            .is_ok()
            {
                rt.diag_flags
                    .fetch_or(DIAG_ALTTAB_FOCUS_APPLIED, Ordering::Relaxed);
            }
        }
    } else {
        let saved_thick = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.alttab_item_orig_focus_thickness.remove(&key))
        });
        let _sync_guard = PropSyncGuard::enter();
        let cleared = allow_clear_dp
            && clear_dp_value(
                insp_ptr,
                BrushPropKind::FocusVisualSecondaryThickness
                    .dependency_property()
                    .as_ref(),
            );
        if !cleared {
            if let Some(orig) = saved_thick {
                let _ = ((*(*fe4).lp_vtbl).put_focus_visual_secondary_thickness)(
                    fe4_com.as_ptr(),
                    orig,
                );
            }
        }
    }
}

pub(super) unsafe fn apply_focus_secondary_appearance(
    rt: &Arc<TapRuntime>,
    insp_ptr: *mut c_void,
    desired_hide: bool,
) {
    if insp_ptr.is_null() {
        return;
    }
    let key = com_identity_key(insp_ptr);
    let fe4_com = qi_interface(insp_ptr, &IID_IFRAMEWORKELEMENT4);
    if fe4_com.is_null() {
        return;
    }
    if desired_hide {
        track_alttab_subtarget_weak(key, TargetKind::AltTabItemFocus, insp_ptr);
        sync_focus_secondary_thickness_prop(rt, key, insp_ptr, true, true);
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            fe4_com.as_ptr(),
            BrushPropKind::FocusVisualSecondaryBrush,
            RawUiColor::TRANSPARENT,
            true,
            None,
            DIAG_ALTTAB_FOCUS_APPLIED,
        );
        ensure_prop_watch_registered(
            rt,
            key,
            TargetKind::AltTabItemFocus,
            BrushPropKind::FocusVisualSecondaryBrush,
            insp_ptr,
        );
        ensure_prop_watch_registered(
            rt,
            key,
            TargetKind::AltTabItemFocus,
            BrushPropKind::FocusVisualSecondaryThickness,
            insp_ptr,
        );
    } else {
        let saved = take_saved_target_brushes(key);
        sync_focus_secondary_thickness_prop(rt, key, insp_ptr, false, true);
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            fe4_com.as_ptr(),
            BrushPropKind::FocusVisualSecondaryBrush,
            RawUiColor::TRANSPARENT,
            false,
            saved.focus_secondary,
            DIAG_ALTTAB_FOCUS_APPLIED,
        );
    }
}

#[inline]
fn approx_eq_thickness(a: RawThickness, b: RawThickness) -> bool {
    (a.left - b.left).abs() <= 1e-3
        && (a.top - b.top).abs() <= 1e-3
        && (a.right - b.right).abs() <= 1e-3
        && (a.bottom - b.bottom).abs() <= 1e-3
}

#[inline]
pub(super) fn approx_eq_corner_radius(a: RawCornerRadius, b: RawCornerRadius) -> bool {
    (a.top_left - b.top_left).abs() <= 1e-3
        && (a.top_right - b.top_right).abs() <= 1e-3
        && (a.bottom_right - b.bottom_right).abs() <= 1e-3
        && (a.bottom_left - b.bottom_left).abs() <= 1e-3
}

unsafe fn set_element_framework_name(ptr: *mut c_void, name: &str) -> bool {
    let fe_com = qi_interface(ptr, &IID_IFRAMEWORKELEMENT);
    if fe_com.is_null() {
        return false;
    }
    let fe = fe_com.as_ptr().cast::<RawIFrameworkElement>();
    let hs = HSTRING::from(name);
    let abi_hs: core::mem::MaybeUninit<HSTRING> = core::mem::transmute_copy(&hs);
    let ok = ((*(*fe).lp_vtbl).put_name)(fe_com.as_ptr(), abi_hs).is_ok();
    drop(hs);
    ok
}

unsafe fn sync_overlay_border_geometry(src_insp: *mut c_void, dst_insp: *mut c_void) {
    let src_border_com = qi_interface(src_insp, &IID_IBORDER);
    let dst_border_com = qi_interface(dst_insp, &IID_IBORDER);
    if !src_border_com.is_null() && !dst_border_com.is_null() {
        let src_b = src_border_com.as_ptr().cast::<RawIBorder>();
        let dst_b = dst_border_com.as_ptr().cast::<RawIBorder>();

        let mut src_bt = RawThickness::default();
        let mut dst_bt = RawThickness::default();
        if ((*(*src_b).lp_vtbl).get_border_thickness)(src_border_com.as_ptr(), &mut src_bt)
            .is_ok()
            && (((*(*dst_b).lp_vtbl).get_border_thickness)(
                dst_border_com.as_ptr(),
                &mut dst_bt,
            )
            .is_err()
                || !approx_eq_thickness(src_bt, dst_bt))
        {
            let _ = ((*(*dst_b).lp_vtbl).put_border_thickness)(dst_border_com.as_ptr(), src_bt);
        }

        let mut src_cr = RawCornerRadius::default();
        let mut dst_cr = RawCornerRadius::default();
        if ((*(*src_b).lp_vtbl).get_corner_radius)(src_border_com.as_ptr(), &mut src_cr).is_ok()
            && (((*(*dst_b).lp_vtbl).get_corner_radius)(dst_border_com.as_ptr(), &mut dst_cr)
                .is_err()
                || !approx_eq_corner_radius(src_cr, dst_cr))
        {
            let _ = ((*(*dst_b).lp_vtbl).put_corner_radius)(dst_border_com.as_ptr(), src_cr);
        }
    }

    let src_fe_com = qi_interface(src_insp, &IID_IFRAMEWORKELEMENT);
    let dst_fe_com = qi_interface(dst_insp, &IID_IFRAMEWORKELEMENT);
    if !src_fe_com.is_null() && !dst_fe_com.is_null() {
        let src_fe = src_fe_com.as_ptr().cast::<RawIFrameworkElement>();
        let dst_fe = dst_fe_com.as_ptr().cast::<RawIFrameworkElement>();
        let mut src_margin = RawThickness::default();
        let mut dst_margin = RawThickness::default();
        if ((*(*src_fe).lp_vtbl).get_margin)(src_fe_com.as_ptr(), &mut src_margin).is_ok()
            && (((*(*dst_fe).lp_vtbl).get_margin)(dst_fe_com.as_ptr(), &mut dst_margin)
                .is_err()
                || !approx_eq_thickness(src_margin, dst_margin))
        {
            let _ = ((*(*dst_fe).lp_vtbl).put_margin)(dst_fe_com.as_ptr(), src_margin);
        }
    }
}

unsafe fn sync_overlay_border_brushes(
    dst_border_ptr: *mut c_void,
    bg_color: RawUiColor,
    border_color: RawUiColor,
) {
    let dst_b = dst_border_ptr.cast::<RawIBorder>();

    let mut cur_bg = ComRaw::null();
    if ((*(*dst_b).lp_vtbl).get_background)(dst_border_ptr, &mut cur_bg.0).is_err()
        || !is_solid_brush_with_color(cur_bg.as_ptr(), bg_color)
    {
        if let Some(tb) = cached_solid_brush(bg_color) {
            let _ = ((*(*dst_b).lp_vtbl).put_background)(dst_border_ptr, tb.as_ptr());
        }
    }

    let mut cur_bb = ComRaw::null();
    if ((*(*dst_b).lp_vtbl).get_border_brush)(dst_border_ptr, &mut cur_bb.0).is_err()
        || !is_solid_brush_with_color(cur_bb.as_ptr(), border_color)
    {
        if let Some(tb) = cached_solid_brush(border_color) {
            let _ = ((*(*dst_b).lp_vtbl).put_border_brush)(dst_border_ptr, tb.as_ptr());
        }
    }
}

unsafe fn activate_alttab_overlay_border() -> Option<ComRaw> {
    let class_id = HSTRING::from("Windows.UI.Xaml.Controls.Border");
    let insp: IInspectable = RoActivateInstance(&class_id).ok()?;
    let raw = insp.into_raw();
    (!raw.is_null()).then_some(ComRaw(raw))
}

enum ParentPanelLookup {
    Panel(ComRaw),
    Detached,
    NonPanel,
}

unsafe fn resolve_parent_panel(
    bg_border_insp: *mut c_void,
    parent_hint: Option<*mut c_void>,
) -> ParentPanelLookup {
    if let Some(p) = parent_hint.filter(|p| !p.is_null()) {
        let panel_com = qi_interface(p, &IID_IPANEL);
        return if panel_com.is_null() {
            ParentPanelLookup::NonPanel
        } else {
            ParentPanelLookup::Panel(panel_com)
        };
    }
    let Some(vth) = cached_visual_tree_helper() else {
        return ParentPanelLookup::Detached;
    };
    let dep = qi_interface(bg_border_insp, &IID_IDEPENDENCYOBJECT);
    if dep.is_null() {
        return ParentPanelLookup::Detached;
    }
    let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
    let mut parent_dep = ComRaw::null();
    if ((*(*vth_iface).lp_vtbl).get_parent)(vth.as_ptr(), dep.as_ptr(), &mut parent_dep.0)
        .is_err()
        || parent_dep.is_null()
    {
        return ParentPanelLookup::Detached;
    }
    let panel_com = qi_interface(parent_dep.as_ptr(), &IID_IPANEL);
    if panel_com.is_null() {
        ParentPanelLookup::NonPanel
    } else {
        ParentPanelLookup::Panel(panel_com)
    }
}

unsafe fn find_overlay_in_panel_children(children_ptr: *mut c_void) -> Option<ComRaw> {
    let vec = children_ptr.cast::<RawIUIElementVector>();
    let mut count = 0u32;
    if ((*(*vec).lp_vtbl).get_size)(children_ptr, &mut count).is_err() {
        return None;
    }
    for i in 0..count.min(64) {
        let mut child = ComRaw::null();
        if ((*(*vec).lp_vtbl).get_at)(children_ptr, i, &mut child.0).is_ok()
            && !child.is_null()
            && element_framework_name(child.as_ptr()).as_deref()
                == Some(ALTTAB_ITEM_OVERLAY_NAME)
        {
            return Some(child);
        }
    }
    None
}

unsafe fn remove_all_overlays_in_panel_children(children_ptr: *mut c_void) {
    let vec = children_ptr.cast::<RawIUIElementVector>();
    let mut count = 0u32;
    if ((*(*vec).lp_vtbl).get_size)(children_ptr, &mut count).is_err() {
        return;
    }
    for i in (0..count.min(64)).rev() {
        let mut child = ComRaw::null();
        if ((*(*vec).lp_vtbl).get_at)(children_ptr, i, &mut child.0).is_ok()
            && !child.is_null()
            && element_framework_name(child.as_ptr()).as_deref()
                == Some(ALTTAB_ITEM_OVERLAY_NAME)
        {
            let _ = ((*(*vec).lp_vtbl).remove_at)(children_ptr, i);
        }
    }
}

unsafe fn record_and_hide_alttab_item_visual(key: u64, bg_border_insp: *mut c_void) -> bool {
    let Some(ecp) = cached_element_composition_preview() else {
        return false;
    };
    let ui_elem = qi_interface(bg_border_insp, &IID_IUIELEMENT);
    if ui_elem.is_null() {
        return false;
    }
    let ecp_iface = ecp.as_ptr().cast::<RawIElementCompositionPreviewStatics>();
    let mut vis_com = ComRaw::null();
    if ((*(*ecp_iface).lp_vtbl).get_element_visual)(
        ecp.as_ptr(),
        ui_elem.as_ptr(),
        &mut vis_com.0,
    )
    .is_err()
        || vis_com.is_null()
    {
        return false;
    }
    let vis = vis_com.as_ptr().cast::<RawIVisual>();
    let mut cur_vis = 1u8;
    let hr_get = ((*(*vis).lp_vtbl).get_is_visible)(vis_com.as_ptr(), &mut cur_vis);

    UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            let orig_vis = if hr_get.is_ok() && cur_vis != 0 {
                cur_vis
            } else {
                1u8
            };
            s.alttab_item_orig_comp_visible
                .entry(key)
                .or_insert(orig_vis);
        }
    });

    if hr_get.is_err() || cur_vis != 0 {
        ((*(*vis).lp_vtbl).put_is_visible)(vis_com.as_ptr(), 0u8).is_ok()
    } else {
        true
    }
}

unsafe fn restore_alttab_item_visual(key: u64, bg_border_insp: *mut c_void) {
    let saved_vis = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut s| s.alttab_item_orig_comp_visible.remove(&key))
    });
    let Some(ecp) = cached_element_composition_preview() else {
        return;
    };
    let ui_elem = qi_interface(bg_border_insp, &IID_IUIELEMENT);
    if ui_elem.is_null() {
        return;
    }
    let ecp_iface = ecp.as_ptr().cast::<RawIElementCompositionPreviewStatics>();
    let mut vis_com = ComRaw::null();
    if ((*(*ecp_iface).lp_vtbl).get_element_visual)(
        ecp.as_ptr(),
        ui_elem.as_ptr(),
        &mut vis_com.0,
    )
    .is_err()
        || vis_com.is_null()
    {
        return;
    }
    let vis = vis_com.as_ptr().cast::<RawIVisual>();
    let mut cur_vis = 1u8;
    let hr_get = ((*(*vis).lp_vtbl).get_is_visible)(vis_com.as_ptr(), &mut cur_vis);
    if saved_vis.is_some() || (hr_get.is_ok() && cur_vis == 0) {
        let target_vis = saved_vis.unwrap_or(1u8);
        if hr_get.is_err() || cur_vis != target_vis {
            let _ = ((*(*vis).lp_vtbl).put_is_visible)(vis_com.as_ptr(), target_vis);
        }
    }
}

fn has_saved_fallback_brushes(key: u64) -> bool {
    UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow().ok().is_some_and(|s| {
            s.saved.contains_key(&key)
                || s.watches.contains_key(&(key, 0))
                || s.watches.contains_key(&(key, 1))
        })
    })
}

pub(super) unsafe fn apply_alttab_item_border_appearance(
    rt: &Arc<TapRuntime>,
    bg_border_insp: *mut c_void,
    parent_hint: Option<*mut c_void>,
    bg_color: RawUiColor,
    border_color: RawUiColor,
    desired_hide: bool,
) {
    if bg_border_insp.is_null() {
        return;
    }
    let key = com_identity_key(bg_border_insp);
    if desired_hide {
        track_alttab_subtarget_weak(key, TargetKind::AltTabItemBorder, bg_border_insp);
        match resolve_parent_panel(bg_border_insp, parent_hint) {
            ParentPanelLookup::Detached => return,
            ParentPanelLookup::Panel(panel_com) => {
                if let Some(vth) = cached_visual_tree_helper() {
                    let root_dep = qi_interface(panel_com.as_ptr(), &IID_IDEPENDENCYOBJECT);
                    if !root_dep.is_null() {
                        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
                        let mut lv_dep = ComRaw::null();
                        if ((*(*vth_iface).lp_vtbl).get_parent)(
                            vth.as_ptr(),
                            root_dep.as_ptr(),
                            &mut lv_dep.0,
                        )
                        .is_ok()
                            && !lv_dep.is_null()
                        {
                            apply_focus_secondary_appearance(rt, lv_dep.as_ptr(), true);
                        }
                    }
                }
                let panel = panel_com.as_ptr().cast::<RawIPanel>();
                let mut children_com = ComRaw::null();
                if ((*(*panel).lp_vtbl).get_children)(panel_com.as_ptr(), &mut children_com.0)
                    .is_ok()
                    && !children_com.is_null()
                {
                    let overlay_ready = if let Some(existing) =
                        find_overlay_in_panel_children(children_com.as_ptr())
                    {
                        sync_overlay_border_geometry(bg_border_insp, existing.as_ptr());
                        let ov_border = qi_interface(existing.as_ptr(), &IID_IBORDER);
                        if !ov_border.is_null() {
                            sync_overlay_border_brushes(
                                ov_border.as_ptr(),
                                bg_color,
                                border_color,
                            );
                        }
                        true
                    } else if let Some(overlay) = activate_alttab_overlay_border() {
                        let _ = set_element_framework_name(
                            overlay.as_ptr(),
                            ALTTAB_ITEM_OVERLAY_NAME,
                        );
                        let ov_ui = qi_interface(overlay.as_ptr(), &IID_IUIELEMENT);
                        if !ov_ui.is_null() {
                            let ui_iface = ov_ui.as_ptr().cast::<RawIUIElement>();
                            let _ = ((*(*ui_iface).lp_vtbl).put_is_hit_test_visible)(
                                ov_ui.as_ptr(),
                                0u8,
                            );
                        }
                        sync_overlay_border_geometry(bg_border_insp, overlay.as_ptr());
                        let ov_border = qi_interface(overlay.as_ptr(), &IID_IBORDER);
                        if !ov_border.is_null() {
                            sync_overlay_border_brushes(
                                ov_border.as_ptr(),
                                bg_color,
                                border_color,
                            );
                        }
                        if !ov_ui.is_null() {
                            let vec = children_com.as_ptr().cast::<RawIUIElementVector>();
                            ((*(*vec).lp_vtbl).insert_at)(
                                children_com.as_ptr(),
                                0,
                                ov_ui.as_ptr(),
                            )
                            .is_ok()
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if overlay_ready {
                        if record_and_hide_alttab_item_visual(key, bg_border_insp) {
                            rt.diag_flags
                                .fetch_or(DIAG_ALTTAB_ITEM_APPLIED, Ordering::Relaxed);
                            return;
                        }
                        remove_all_overlays_in_panel_children(children_com.as_ptr());
                    } else {
                        return;
                    }
                }
            }
            ParentPanelLookup::NonPanel => {}
        }

        apply_border_target_appearance(
            rt,
            key,
            bg_border_insp,
            TargetKind::AltTabItemBorder,
            bg_color,
            border_color,
            true,
            DIAG_ALTTAB_ITEM_APPLIED,
        );
    } else {
        if let ParentPanelLookup::Panel(panel_com) =
            resolve_parent_panel(bg_border_insp, parent_hint)
        {
            let panel = panel_com.as_ptr().cast::<RawIPanel>();
            let mut children_com = ComRaw::null();
            if ((*(*panel).lp_vtbl).get_children)(panel_com.as_ptr(), &mut children_com.0)
                .is_ok()
                && !children_com.is_null()
            {
                remove_all_overlays_in_panel_children(children_com.as_ptr());
            }
        }
        restore_alttab_item_visual(key, bg_border_insp);
        if has_saved_fallback_brushes(key) {
            apply_border_target_appearance(
                rt,
                key,
                bg_border_insp,
                TargetKind::AltTabItemBorder,
                bg_color,
                border_color,
                false,
                DIAG_ALTTAB_ITEM_APPLIED,
            );
        }
    }
}

pub(in super::super) unsafe fn sync_alttab_subtree_items(
    rt: &Arc<TapRuntime>,
    bg_element_ptr: *mut c_void,
    desired_hide: bool,
) {
    if bg_element_ptr.is_null() {
        return;
    }
    let Some(vth) = cached_visual_tree_helper() else {
        return;
    };
    let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
    let root_insp = qi_interface(bg_element_ptr, &IID_IINSPECTABLE);
    if root_insp.is_null() {
        return;
    }

    let bg_color = RawUiColor::from_argb(rt.alt_tab_item_bg_argb.load(Ordering::Acquire));
    let border_color =
        RawUiColor::from_argb(rt.alt_tab_item_border_argb.load(Ordering::Acquire));

    let mut queue: VecDeque<(ComRaw, Option<ComRaw>, Option<ComRaw>)> = VecDeque::new();
    queue.push_back((root_insp, None, None));
    let mut visited = 0usize;

    while let Some((node_com, parent_com, gp_com)) = queue.pop_front() {
        if visited >= 600 {
            break;
        }
        visited += 1;

        let cls_name = element_runtime_class_name(node_com.as_ptr());
        let elem_name = element_framework_name(node_com.as_ptr());

        if elem_name.as_deref() == Some(ALTTAB_ITEM_OVERLAY_NAME) {
            continue;
        }

        if cls_name.as_deref().is_some_and(|c| c.contains("ListViewItem")) {
            apply_focus_secondary_appearance(rt, node_com.as_ptr(), desired_hide);
        }

        if elem_name.as_deref() == Some("BackgroundBorder")
            && !qi_interface(node_com.as_ptr(), &IID_IBORDER).is_null()
        {
            apply_alttab_item_border_appearance(
                rt,
                node_com.as_ptr(),
                parent_com.as_ref().map(ComRaw::as_ptr),
                bg_color,
                border_color,
                desired_hide,
            );
            if let Some(ref gp) = gp_com {
                let gp_key = com_identity_key(gp.as_ptr());
                if !is_alttab_subtarget_tracked(gp_key) {
                    apply_focus_secondary_appearance(rt, gp.as_ptr(), desired_hide);
                }
            }
            continue;
        }

        let skip_children = elem_name
            .as_deref()
            .is_some_and(|n| matches!(n, "ThumbnailHost" | "CloseButton"))
            || cls_name.as_deref().is_some_and(|c| {
                c.contains("SwitchItemControl")
                    || c.contains("SwitchItemElement")
                    || c.contains("Button")
                    || c.contains("ScrollBar")
            });
        if skip_children {
            continue;
        }

        let dep = qi_interface(node_com.as_ptr(), &IID_IDEPENDENCYOBJECT);
        if dep.is_null() {
            continue;
        }
        let mut count = 0i32;
        if ((*(*vth_iface).lp_vtbl).get_children_count)(vth.as_ptr(), dep.as_ptr(), &mut count)
            .is_err()
            || count <= 0
        {
            continue;
        }
        for i in 0..count.min(128) {
            let mut child_dep = ComRaw::null();
            if ((*(*vth_iface).lp_vtbl).get_child)(
                vth.as_ptr(),
                dep.as_ptr(),
                i,
                &mut child_dep.0,
            )
            .is_ok()
                && !child_dep.is_null()
            {
                queue.push_back((
                    child_dep,
                    Some(node_com.clone_ref()),
                    parent_com.as_ref().map(ComRaw::clone_ref),
                ));
            }
        }
    }
}

pub(in super::super) unsafe fn restore_all_alttab_subtargets_for_current_thread(rt: &Arc<TapRuntime>) {
    let entries: Vec<(u64, TargetKind, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        s.alttab_subtargets
            .drain()
            .map(|(k, (kind, weak_obj))| (k, kind, weak_obj))
            .collect()
    });
    if entries.is_empty() {
        return;
    }
    let bg_color = RawUiColor::from_argb(rt.alt_tab_item_bg_argb.load(Ordering::Acquire));
    let border_color =
        RawUiColor::from_argb(rt.alt_tab_item_border_argb.load(Ordering::Acquire));
    let mut keys = Vec::with_capacity(entries.len());
    for (key, kind, weak_com) in entries {
        keys.push(key);
        let wr = weak_com.as_ptr().cast::<RawIWeakReference>();
        let mut insp = ComRaw::null();
        if ((*(*wr).lp_vtbl).resolve)(weak_com.as_ptr(), &IID_IINSPECTABLE, &mut insp.0)
            .is_ok()
            && !insp.is_null()
        {
            match kind {
                TargetKind::AltTabItemBorder => {
                    apply_alttab_item_border_appearance(
                        rt,
                        insp.as_ptr(),
                        None,
                        bg_color,
                        border_color,
                        false,
                    );
                }
                TargetKind::AltTabItemFocus => {
                    apply_focus_secondary_appearance(rt, insp.as_ptr(), false);
                }
                _ => {}
            }
        }
    }
    purge_saved_brushes_for_current_thread(&keys);
}

