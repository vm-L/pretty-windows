use super::alttab::approx_eq_corner_radius;
use super::modal_blur::{attach_start_modal_glass_visual, detach_element_child_visual};
use super::{sync_element_shadow_prop, sync_overflow_brush_prop, sync_overflow_brush_prop_with_mode};
use super::super::super::tree_match::{
    TapHostScope, TargetKind, START_MENU_OVERLAY_NAME, START_MODAL_OVERLAY_NAME,
};
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows::core::{IInspectable, Interface, HSTRING};
use windows::Win32::System::WinRT::RoActivateInstance;
use super::super::brush_store::*;
use super::super::raw_com::*;
use super::super::TapRuntime;

unsafe fn sync_shell_card_border_corner_radius(
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
                .is_some_and(|s| s.shell_card_orig_corner_radius.contains_key(&surface_key))
        });
        if !already_saved
            && ((*(*border).lp_vtbl).get_corner_radius)(border_ptr, &mut cur_cr).is_ok()
        {
            UI_BRUSH_STORE.with(|cell| {
                if let Ok(mut s) = cell.try_borrow_mut() {
                    s.shell_card_orig_corner_radius
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
                .and_then(|mut s| s.shell_card_orig_corner_radius.remove(&surface_key))
        });
        if let Some(SavedCornerRadius::Local(orig_cr)) = orig {
            let _ = ((*(*border).lp_vtbl).put_corner_radius)(border_ptr, orig_cr);
        }
    }
}

fn has_saved_shell_card_comp_visible(key: u64) -> bool {
    UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow()
            .ok()
            .is_some_and(|s| s.shell_card_orig_comp_visible.contains_key(&key))
    })
}

unsafe fn has_zero_visual_children(insp_ptr: *mut c_void) -> bool {
    let Some(vth) = cached_visual_tree_helper() else {
        return false;
    };
    let dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
    if dep.is_null() {
        return false;
    }
    let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
    let mut count = -1i32;
    ((*(*vth_iface).lp_vtbl).get_children_count)(vth.as_ptr(), dep.as_ptr(), &mut count).is_ok()
        && count == 0
}

unsafe fn record_and_hide_shell_card_visual(key: u64, insp_ptr: *mut c_void) -> bool {
    let Some(ecp) = cached_element_composition_preview() else {
        return false;
    };
    let ui_elem = qi_interface(insp_ptr, &IID_IUIELEMENT);
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
            s.shell_card_orig_comp_visible
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

unsafe fn restore_shell_card_visual(key: u64, insp_ptr: *mut c_void) {
    let saved_vis = UI_BRUSH_STORE.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut s| s.shell_card_orig_comp_visible.remove(&key))
    });
    let Some(ecp) = cached_element_composition_preview() else {
        return;
    };
    let ui_elem = qi_interface(insp_ptr, &IID_IUIELEMENT);
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

unsafe fn has_element_child_visual(elem_insp: *mut c_void) -> bool {
    let Some(ecp) = cached_element_composition_preview() else {
        return false;
    };
    let ui_elem = qi_interface(elem_insp, &IID_IUIELEMENT);
    if ui_elem.is_null() {
        return false;
    }
    let ecp_iface = ecp.as_ptr().cast::<RawIElementCompositionPreviewStatics>();
    let mut child_vis = ComRaw::null();
    ((*(*ecp_iface).lp_vtbl).get_element_child_visual)(
        ecp.as_ptr(),
        ui_elem.as_ptr(),
        &mut child_vis.0,
    )
    .is_ok()
        && !child_vis.is_null()
}

unsafe fn activate_xaml_element(class_name: &str, name: Option<&str>) -> Option<ComRaw> {
    let class_id = HSTRING::from(class_name);
    let insp: IInspectable = RoActivateInstance(&class_id).ok()?;
    let raw = insp.into_raw();
    if raw.is_null() {
        return None;
    }
    let elem = ComRaw(raw);
    if let Some(elem_name) = name {
        let fe_com = qi_interface(elem.as_ptr(), &IID_IFRAMEWORKELEMENT);
        if !fe_com.is_null() {
            let fe = fe_com.as_ptr().cast::<RawIFrameworkElement>();
            let hs = HSTRING::from(elem_name);
            let abi_hs: core::mem::MaybeUninit<HSTRING> = core::mem::transmute_copy(&hs);
            let _ = ((*(*fe).lp_vtbl).put_name)(fe_com.as_ptr(), abi_hs);
            drop(hs);
        }
    }
    let ui_com = qi_interface(elem.as_ptr(), &IID_IUIELEMENT);
    if !ui_com.is_null() {
        let ui = ui_com.as_ptr().cast::<RawIUIElement>();
        let _ = ((*(*ui).lp_vtbl).put_is_hit_test_visible)(ui_com.as_ptr(), 0u8);
    }
    Some(elem)
}

unsafe fn resolve_parent_panel_and_children(insp_ptr: *mut c_void) -> Option<(ComRaw, ComRaw)> {
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
    let panel_com = qi_interface(parent_dep.as_ptr(), &IID_IPANEL);
    if panel_com.is_null() {
        return None;
    }
    let panel = panel_com.as_ptr().cast::<RawIPanel>();
    let mut children_com = ComRaw::null();
    if ((*(*panel).lp_vtbl).get_children)(panel_com.as_ptr(), &mut children_com.0).is_err()
        || children_com.is_null()
    {
        return None;
    }
    Some((panel_com, children_com))
}

unsafe fn clear_overlay_subtree_resources(overlay_insp: *mut c_void) {
    if overlay_insp.is_null() {
        return;
    }
    detach_element_child_visual(overlay_insp);
    let ui10_com = qi_interface(overlay_insp, &IID_IUIELEMENT10);
    if !ui10_com.is_null() {
        let ui10 = ui10_com.as_ptr().cast::<RawIUIElement10>();
        let _ = ((*(*ui10).lp_vtbl).put_shadow)(ui10_com.as_ptr(), null_mut());
    }
    let panel_com = qi_interface(overlay_insp, &IID_IPANEL);
    if !panel_com.is_null() {
        let panel = panel_com.as_ptr().cast::<RawIPanel>();
        let mut children_com = ComRaw::null();
        if ((*(*panel).lp_vtbl).get_children)(panel_com.as_ptr(), &mut children_com.0).is_ok()
            && !children_com.is_null()
        {
            let vec = children_com.as_ptr().cast::<RawIUIElementVector>();
            let mut count = 0u32;
            if ((*(*vec).lp_vtbl).get_size)(children_com.as_ptr(), &mut count).is_ok() {
                for i in 0..count.min(16) {
                    let mut child = ComRaw::null();
                    if ((*(*vec).lp_vtbl).get_at)(children_com.as_ptr(), i, &mut child.0).is_ok()
                        && !child.is_null()
                    {
                        detach_element_child_visual(child.as_ptr());
                        let c_ui10 = qi_interface(child.as_ptr(), &IID_IUIELEMENT10);
                        if !c_ui10.is_null() {
                            let u10 = c_ui10.as_ptr().cast::<RawIUIElement10>();
                            let _ = ((*(*u10).lp_vtbl).put_shadow)(c_ui10.as_ptr(), null_mut());
                        }
                    }
                }
            }
        }
    }
}

unsafe fn remove_named_overlays_in_children(children_ptr: *mut c_void, overlay_name: &str) {
    let vec = children_ptr.cast::<RawIUIElementVector>();
    let mut count = 0u32;
    if ((*(*vec).lp_vtbl).get_size)(children_ptr, &mut count).is_err() {
        return;
    }
    for i in (0..count.min(64)).rev() {
        let mut child = ComRaw::null();
        if ((*(*vec).lp_vtbl).get_at)(children_ptr, i, &mut child.0).is_ok()
            && !child.is_null()
            && element_framework_name(child.as_ptr()).as_deref() == Some(overlay_name)
        {
            clear_overlay_subtree_resources(child.as_ptr());
            let _ = ((*(*vec).lp_vtbl).remove_at)(children_ptr, i);
        }
    }
}

unsafe fn remove_named_overlays_in_parent_panel(native_insp: *mut c_void, overlay_name: &str) {
    let Some((_panel, children_com)) = resolve_parent_panel_and_children(native_insp) else {
        return;
    };
    remove_named_overlays_in_children(children_com.as_ptr(), overlay_name);
}

unsafe fn find_child_index_by_key(children_ptr: *mut c_void, target_key: u64) -> Option<u32> {
    let vec = children_ptr.cast::<RawIUIElementVector>();
    let mut count = 0u32;
    if ((*(*vec).lp_vtbl).get_size)(children_ptr, &mut count).is_err() {
        return None;
    }
    for i in 0..count.min(64) {
        let mut child = ComRaw::null();
        if ((*(*vec).lp_vtbl).get_at)(children_ptr, i, &mut child.0).is_ok()
            && !child.is_null()
            && com_identity_key(child.as_ptr()) == target_key
        {
            return Some(i);
        }
    }
    None
}

unsafe fn find_named_child_in_children(
    children_ptr: *mut c_void,
    target_name: &str,
) -> Option<ComRaw> {
    let vec = children_ptr.cast::<RawIUIElementVector>();
    let mut count = 0u32;
    if ((*(*vec).lp_vtbl).get_size)(children_ptr, &mut count).is_err() {
        return None;
    }
    for i in 0..count.min(64) {
        let mut child = ComRaw::null();
        if ((*(*vec).lp_vtbl).get_at)(children_ptr, i, &mut child.0).is_ok()
            && !child.is_null()
            && element_framework_name(child.as_ptr()).as_deref() == Some(target_name)
        {
            return Some(child);
        }
    }
    None
}

unsafe fn copy_grid_attached_layout(src_insp: *mut c_void, dst_insp: *mut c_void) {
    let src_fe = qi_interface(src_insp, &IID_IFRAMEWORKELEMENT);
    let dst_fe = qi_interface(dst_insp, &IID_IFRAMEWORKELEMENT);
    if src_fe.is_null() || dst_fe.is_null() {
        return;
    }
    let sfe = src_fe.as_ptr().cast::<RawIFrameworkElement>();
    let dfe = dst_fe.as_ptr().cast::<RawIFrameworkElement>();
    let mut margin = RawThickness::default();
    if ((*(*sfe).lp_vtbl).get_margin)(src_fe.as_ptr(), &mut margin).is_ok() {
        let _ = ((*(*dfe).lp_vtbl).put_margin)(dst_fe.as_ptr(), margin);
    }

    let Some(gs_com) = cached_grid_statics() else {
        return;
    };
    let gs = gs_com.as_ptr().cast::<RawIGridStatics>();
    let mut val = 0i32;
    if ((*(*gs).lp_vtbl).get_row)(gs_com.as_ptr(), src_fe.as_ptr(), &mut val).is_ok() {
        let _ = ((*(*gs).lp_vtbl).set_row)(gs_com.as_ptr(), dst_fe.as_ptr(), val);
    }
    if ((*(*gs).lp_vtbl).get_column)(gs_com.as_ptr(), src_fe.as_ptr(), &mut val).is_ok() {
        let _ = ((*(*gs).lp_vtbl).set_column)(gs_com.as_ptr(), dst_fe.as_ptr(), val);
    }
    if ((*(*gs).lp_vtbl).get_row_span)(gs_com.as_ptr(), src_fe.as_ptr(), &mut val).is_ok() {
        let _ = ((*(*gs).lp_vtbl).set_row_span)(gs_com.as_ptr(), dst_fe.as_ptr(), val.max(1));
    }
    if ((*(*gs).lp_vtbl).get_column_span)(gs_com.as_ptr(), src_fe.as_ptr(), &mut val).is_ok() {
        let _ =
            ((*(*gs).lp_vtbl).set_column_span)(gs_com.as_ptr(), dst_fe.as_ptr(), val.max(1));
    }
}

unsafe fn copy_border_geometry(
    src_border_ptr: *mut c_void,
    dst_border_ptr: *mut c_void,
    fallback_corner_dip: Option<f64>,
    fallback_thickness_dip: Option<f64>,
) -> RawCornerRadius {
    let sb = src_border_ptr.cast::<RawIBorder>();
    let db = dst_border_ptr.cast::<RawIBorder>();

    let mut target_cr = RawCornerRadius::default();
    let got_cr = ((*(*sb).lp_vtbl).get_corner_radius)(src_border_ptr, &mut target_cr).is_ok();
    if !got_cr
        || (target_cr.top_left < 1.0
            && target_cr.top_right < 1.0
            && target_cr.bottom_right < 1.0
            && target_cr.bottom_left < 1.0)
    {
        if let Some(r) = fallback_corner_dip {
            target_cr = RawCornerRadius {
                top_left: r,
                top_right: r,
                bottom_right: r,
                bottom_left: r,
            };
        }
    }
    let _ = ((*(*db).lp_vtbl).put_corner_radius)(dst_border_ptr, target_cr);

    let mut target_bt = RawThickness::default();
    let got_bt = ((*(*sb).lp_vtbl).get_border_thickness)(src_border_ptr, &mut target_bt).is_ok();
    if !got_bt
        || (target_bt.left + target_bt.top + target_bt.right + target_bt.bottom) < 0.5
    {
        if let Some(t) = fallback_thickness_dip {
            target_bt = RawThickness {
                left: t,
                top: t,
                right: t,
                bottom: t,
            };
        }
    }
    let _ = ((*(*db).lp_vtbl).put_border_thickness)(dst_border_ptr, target_bt);
    target_cr
}

unsafe fn ensure_start_menu_suppressed_overlay(
    native_insp: *mut c_void,
    native_border_ptr: *mut c_void,
    bg_color: RawUiColor,
    border_color: RawUiColor,
) {
    let Some((_panel, children_com)) = resolve_parent_panel_and_children(native_insp) else {
        return;
    };
    let ov_com = if let Some(existing) =
        find_named_child_in_children(children_com.as_ptr(), START_MENU_OVERLAY_NAME)
    {
        existing
    } else {
        let Some(created) =
            activate_xaml_element("Windows.UI.Xaml.Controls.Border", Some(START_MENU_OVERLAY_NAME))
        else {
            return;
        };
        let ui_elem = qi_interface(created.as_ptr(), &IID_IUIELEMENT);
        if ui_elem.is_null() {
            return;
        }
        let native_key = com_identity_key(native_insp);
        let insert_idx = find_child_index_by_key(children_com.as_ptr(), native_key).unwrap_or(0);
        let vec = children_com.as_ptr().cast::<RawIUIElementVector>();
        if ((*(*vec).lp_vtbl).insert_at)(children_com.as_ptr(), insert_idx, ui_elem.as_ptr())
            .is_err()
        {
            return;
        }
        created
    };

    copy_grid_attached_layout(native_insp, ov_com.as_ptr());
    let ov_border_com = qi_interface(ov_com.as_ptr(), &IID_IBORDER);
    if ov_border_com.is_null() {
        return;
    }
    let _ = copy_border_geometry(native_border_ptr, ov_border_com.as_ptr(), Some(8.0), Some(1.0));
    let ob = ov_border_com.as_ptr().cast::<RawIBorder>();
    if let Some(bg_brush) = cached_solid_brush(bg_color) {
        let _ = ((*(*ob).lp_vtbl).put_background)(ov_border_com.as_ptr(), bg_brush.as_ptr());
    }
    if let Some(bd_brush) = cached_solid_brush(border_color) {
        let _ = ((*(*ob).lp_vtbl).put_border_brush)(ov_border_com.as_ptr(), bd_brush.as_ptr());
    }
}

unsafe fn ensure_start_modal_overlay_host(native_insp: *mut c_void) -> Option<(ComRaw, bool)> {
    let (_panel, children_com) = resolve_parent_panel_and_children(native_insp)?;
    if let Some(existing) =
        find_named_child_in_children(children_com.as_ptr(), START_MODAL_OVERLAY_NAME)
    {
        if !qi_interface(existing.as_ptr(), &IID_IBORDER).is_null() {
            return Some((existing, false));
        }
        remove_named_overlays_in_children(children_com.as_ptr(), START_MODAL_OVERLAY_NAME);
    }

    let modal_host =
        activate_xaml_element("Windows.UI.Xaml.Controls.Border", Some(START_MODAL_OVERLAY_NAME))?;
    let host_ui = qi_interface(modal_host.as_ptr(), &IID_IUIELEMENT);
    if host_ui.is_null() {
        return None;
    }
    let native_key = com_identity_key(native_insp);
    let insert_idx = find_child_index_by_key(children_com.as_ptr(), native_key).unwrap_or(1);
    let parent_vec = children_com.as_ptr().cast::<RawIUIElementVector>();
    if ((*(*parent_vec).lp_vtbl).insert_at)(children_com.as_ptr(), insert_idx, host_ui.as_ptr())
        .is_err()
    {
        return None;
    }

    Some((modal_host, true))
}

unsafe fn restore_shell_card_target(
    rt: &Arc<TapRuntime>,
    surface_key: u64,
    surface_insp: Option<*mut c_void>,
) {
    if surface_key != 0 {
        unregister_watches_for_keys(&[surface_key]);
    }
    UI_BRUSH_STORE.with(|cell| {
        if let Ok(mut s) = cell.try_borrow_mut() {
            s.start_modal_applied_style.remove(&surface_key);
        }
    });
    let surf_saved = take_saved_target_brushes(surface_key);
    let (target_bg, target_border) = rt.menu_flyout_target_colors();

    if let Some(surf_ptr) = surface_insp {
        if surf_saved.shadow.is_some() {
            sync_element_shadow_prop(
                rt,
                surface_key,
                surf_ptr,
                false,
                surf_saved.shadow,
                DIAG_SHELL_CARD_SHADOW_APPLIED,
                false,
            );
        }
        let border_com = qi_interface(surf_ptr, &IID_IBORDER);
        if !border_com.is_null() {
            let _ = sync_overflow_brush_prop_with_mode(
                rt,
                surface_key,
                surf_ptr,
                border_com.as_ptr(),
                BrushPropKind::BorderBackground,
                target_bg,
                false,
                surf_saved.background,
                DIAG_SHELL_CARD_APPLIED,
                true,
            );
            let _ = sync_overflow_brush_prop_with_mode(
                rt,
                surface_key,
                surf_ptr,
                border_com.as_ptr(),
                BrushPropKind::BorderBrush,
                target_border,
                false,
                surf_saved.border_brush,
                DIAG_SHELL_CARD_APPLIED,
                true,
            );
            sync_shell_card_border_corner_radius(surface_key, border_com.as_ptr(), false);
        } else {
            let panel_com = qi_interface(surf_ptr, &IID_IPANEL);
            if !panel_com.is_null() {
                sync_overflow_brush_prop(
                    rt,
                    surface_key,
                    surf_ptr,
                    panel_com.as_ptr(),
                    BrushPropKind::PanelBackground,
                    target_bg,
                    false,
                    surf_saved.background,
                    DIAG_SHELL_CARD_APPLIED,
                );
            }
            let grid2_com = qi_interface(surf_ptr, &IID_IGRID2);
            if !grid2_com.is_null() {
                sync_overflow_brush_prop(
                    rt,
                    surface_key,
                    surf_ptr,
                    grid2_com.as_ptr(),
                    BrushPropKind::GridBorderBrush,
                    target_border,
                    false,
                    surf_saved.border_brush,
                    DIAG_SHELL_CARD_APPLIED,
                );
            }
        }
        remove_named_overlays_in_parent_panel(surf_ptr, START_MENU_OVERLAY_NAME);
        remove_named_overlays_in_parent_panel(surf_ptr, START_MODAL_OVERLAY_NAME);
        restore_shell_card_visual(surface_key, surf_ptr);
    }
    if surface_key != 0 {
        purge_saved_brushes_for_current_thread(&[surface_key]);
    }
}

pub(in super::super) unsafe fn restore_all_shell_card_targets_for_current_thread(rt: &Arc<TapRuntime>) {
    let entries: Vec<(u64, TooltipTrackedEntry)> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        s.shell_card_modified_targets.drain().collect()
    });
    for (surface_key, entry) in entries {
        let surf_insp = resolve_weak_inspectable(&entry.presenter_weak);
        restore_shell_card_target(
            rt,
            surface_key,
            surf_insp.as_ref().map(ComRaw::as_ptr),
        );
    }
}

pub(super) unsafe fn apply_shell_card_surface_appearance(
    rt: &Arc<TapRuntime>,
    _handle: u64,
    insp_ptr: *mut c_void,
    kind: TargetKind,
    desired_hide: bool,
) {
    if insp_ptr.is_null() {
        return;
    }
    let surface_key = com_identity_key(insp_ptr);
    if desired_hide {
        let border_com = qi_interface(insp_ptr, &IID_IBORDER);
        if rt.host_scope == TapHostScope::StartMenu
            && !border_com.is_null()
            && has_zero_visual_children(insp_ptr)
        {
            track_shell_card_modified_target(surface_key, insp_ptr);
            if !record_and_hide_shell_card_visual(surface_key, insp_ptr) {
                return;
            }
            if kind == TargetKind::ShellCardSurface && rt.is_menu_flyout_suppressed() {
                let (target_bg, target_border) = rt.menu_flyout_target_colors();
                ensure_start_menu_suppressed_overlay(
                    insp_ptr,
                    border_com.as_ptr(),
                    target_bg,
                    target_border,
                );
            } else {
                remove_named_overlays_in_parent_panel(insp_ptr, START_MENU_OVERLAY_NAME);
            }
            rt.diag_flags
                .fetch_or(DIAG_SHELL_CARD_APPLIED, Ordering::Relaxed);
            return;
        }

        track_shell_card_modified_target(surface_key, insp_ptr);
        let is_subsurface = kind == TargetKind::ShellCardSubSurface;
        let (target_bg, target_border) = if is_subsurface {
            (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
        } else {
            rt.menu_flyout_target_colors()
        };
        let apply_round = !is_subsurface && rt.is_menu_flyout_suppressed();

        sync_element_shadow_prop(
            rt,
            surface_key,
            insp_ptr,
            true,
            None,
            DIAG_SHELL_CARD_SHADOW_APPLIED,
            false,
        );
        ensure_prop_watch_registered(
            rt,
            surface_key,
            kind,
            BrushPropKind::ElementShadow,
            insp_ptr,
        );

        if !border_com.is_null() {
            let _ = sync_overflow_brush_prop_with_mode(
                rt,
                surface_key,
                insp_ptr,
                border_com.as_ptr(),
                BrushPropKind::BorderBackground,
                target_bg,
                true,
                None,
                DIAG_SHELL_CARD_APPLIED,
                true,
            );
            let _ = sync_overflow_brush_prop_with_mode(
                rt,
                surface_key,
                insp_ptr,
                border_com.as_ptr(),
                BrushPropKind::BorderBrush,
                target_border,
                true,
                None,
                DIAG_SHELL_CARD_APPLIED,
                true,
            );
            ensure_prop_watch_registered(
                rt,
                surface_key,
                kind,
                BrushPropKind::BorderBackground,
                insp_ptr,
            );
            ensure_prop_watch_registered(
                rt,
                surface_key,
                kind,
                BrushPropKind::BorderBrush,
                insp_ptr,
            );
            sync_shell_card_border_corner_radius(surface_key, border_com.as_ptr(), apply_round);
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
                    DIAG_SHELL_CARD_APPLIED,
                );
                ensure_prop_watch_registered(
                    rt,
                    surface_key,
                    kind,
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
                    DIAG_SHELL_CARD_APPLIED,
                );
                ensure_prop_watch_registered(
                    rt,
                    surface_key,
                    kind,
                    BrushPropKind::GridBorderBrush,
                    insp_ptr,
                );
            }
        }
    } else {
        let removed = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.shell_card_modified_targets.remove(&surface_key))
        });
        if removed.is_some()
            || has_saved_shell_card_comp_visible(surface_key)
            || has_saved_brush_slot(surface_key, BrushSlot::Background)
            || has_saved_brush_slot(surface_key, BrushSlot::BorderBrush)
            || has_saved_brush_slot(surface_key, BrushSlot::Shadow)
        {
            restore_shell_card_target(rt, surface_key, Some(insp_ptr));
        }
    }
}

pub(super) unsafe fn apply_start_menu_modal_surface_appearance(
    rt: &Arc<TapRuntime>,
    _handle: u64,
    insp_ptr: *mut c_void,
    desired_hide: bool,
) {
    if insp_ptr.is_null() {
        return;
    }
    let surface_key = com_identity_key(insp_ptr);
    if desired_hide {
        let native_border = qi_interface(insp_ptr, &IID_IBORDER);
        if native_border.is_null() || !has_zero_visual_children(insp_ptr) {
            return;
        }
        let Some((modal_host, is_new)) = ensure_start_modal_overlay_host(insp_ptr) else {
            return;
        };
        copy_grid_attached_layout(insp_ptr, modal_host.as_ptr());

        let nb = native_border.as_ptr().cast::<RawIBorder>();
        let mut cr = RawCornerRadius::default();
        let corner_radius =
            if ((*(*nb).lp_vtbl).get_corner_radius)(native_border.as_ptr(), &mut cr).is_ok()
                && cr.top_left >= 1.0
            {
                cr.top_left as f32
            } else {
                8.0
            };

        let style_spec = rt.start_modal_style_spec();
        let prev_spec = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .and_then(|s| s.start_modal_applied_style.get(&surface_key).copied())
        });
        let need_restyle = is_new
            || prev_spec != Some(style_spec)
            || !has_element_child_visual(modal_host.as_ptr());

        if need_restyle {
            if !attach_start_modal_glass_visual(
                modal_host.as_ptr(),
                corner_radius,
                style_spec.0,
                style_spec.1,
                style_spec.2,
                style_spec.3,
            ) {
                remove_named_overlays_in_parent_panel(insp_ptr, START_MODAL_OVERLAY_NAME);
                restore_shell_card_visual(surface_key, insp_ptr);
                return;
            }
            UI_BRUSH_STORE.with(|cell| {
                if let Ok(mut s) = cell.try_borrow_mut() {
                    s.start_modal_applied_style.insert(surface_key, style_spec);
                }
            });
        }

        track_shell_card_modified_target(surface_key, insp_ptr);
        if !record_and_hide_shell_card_visual(surface_key, insp_ptr) {
            remove_named_overlays_in_parent_panel(insp_ptr, START_MODAL_OVERLAY_NAME);
            return;
        }

        rt.diag_flags
            .fetch_or(DIAG_SHELL_CARD_APPLIED, Ordering::Relaxed);
    } else {
        let removed = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.shell_card_modified_targets.remove(&surface_key))
        });
        if removed.is_some()
            || has_saved_shell_card_comp_visible(surface_key)
            || has_saved_brush_slot(surface_key, BrushSlot::Background)
            || has_saved_brush_slot(surface_key, BrushSlot::BorderBrush)
        {
            restore_shell_card_target(rt, surface_key, Some(insp_ptr));
        }
    }
}
