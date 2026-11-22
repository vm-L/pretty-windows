use super::alttab::approx_eq_corner_radius;
use super::{sync_element_shadow_prop, sync_overflow_brush_prop, sync_overflow_brush_prop_with_mode};
use super::super::super::tree_match::TargetKind;
use std::ffi::c_void;
use std::sync::Arc;
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

unsafe fn restore_shell_card_target(
    rt: &Arc<TapRuntime>,
    surface_key: u64,
    surface_insp: Option<*mut c_void>,
) {
    if surface_key != 0 {
        unregister_watches_for_keys(&[surface_key]);
    }
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
                    DIAG_SHELL_CARD_APPLIED,
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
                    DIAG_SHELL_CARD_APPLIED,
                    true,
                );
            }
            sync_shell_card_border_corner_radius(surface_key, border_com.as_ptr(), false);
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
                        DIAG_SHELL_CARD_APPLIED,
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
                        DIAG_SHELL_CARD_APPLIED,
                    );
                }
            }
        }
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

        let border_com = qi_interface(insp_ptr, &IID_IBORDER);
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
            || has_saved_brush_slot(surface_key, BrushSlot::Background)
            || has_saved_brush_slot(surface_key, BrushSlot::BorderBrush)
            || has_saved_brush_slot(surface_key, BrushSlot::Shadow)
        {
            restore_shell_card_target(rt, surface_key, Some(insp_ptr));
        }
    }
}

