use super::{apply_shape_appearance, sync_element_shadow_prop, sync_overflow_brush_prop, sync_overflow_brush_prop_with_mode};
use super::super::super::tree_match::TargetKind;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use super::super::brush_store::*;
use super::super::raw_com::*;
use super::super::TapRuntime;

pub(in super::super) unsafe fn sync_hover_flyout_bg_shapes(
    rt: &Arc<TapRuntime>,
    bg_border_ptr: *mut c_void,
    desired_hide: bool,
) {
    if bg_border_ptr.is_null() {
        return;
    }
    let Some(vth) = cached_visual_tree_helper() else {
        return;
    };
    let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
    let root_insp = qi_interface(bg_border_ptr, &IID_IINSPECTABLE);
    if root_insp.is_null() {
        return;
    }

    let mut queue: VecDeque<ComRaw> = VecDeque::new();
    queue.push_back(root_insp);
    let mut visited = 0usize;

    while let Some(node_com) = queue.pop_front() {
        if visited >= 32 {
            break;
        }
        visited += 1;

        let elem_name = element_framework_name(node_com.as_ptr());
        if elem_name
            .as_deref()
            .is_some_and(|n| matches!(n, "BackgroundFill" | "BackgroundStroke" | "ScreenEdgeStroke"))
        {
            let key = com_identity_key(node_com.as_ptr());
            let cached_entry = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow().ok().and_then(|s| {
                    s.hover_flyout_shapes
                        .get(&key)
                        .map(|(w, op, vis)| (w.clone_ref(), *op, *vis))
                })
            });
            let cached_orig = match cached_entry {
                Some((w, op, vis)) if weak_matches_live_object(&w, node_com.as_ptr()) => {
                    Some((op, vis))
                }
                Some(_) => {
                    let displaced = UI_BRUSH_STORE.with(|cell| {
                        cell.try_borrow_mut()
                            .ok()
                            .and_then(|mut s| s.hover_flyout_shapes.remove(&key))
                    });
                    drop(displaced);
                    None
                }
                None => None,
            };
            let (orig_opacity, orig_visibility) = if let Some(pair) = cached_orig {
                pair
            } else {
                let mut orig_op = 1.0f64;
                let mut orig_vis = VISIBILITY_VISIBLE;
                let ui_elem = qi_interface(node_com.as_ptr(), &IID_IUIELEMENT);
                if !ui_elem.is_null() {
                    let ui = ui_elem.as_ptr().cast::<RawIUIElement>();
                    if ((*(*ui).lp_vtbl).get_opacity)(ui_elem.as_ptr(), &mut orig_op).is_err()
                        || !orig_op.is_finite()
                        || orig_op <= 0.01
                    {
                        orig_op = 1.0;
                    }
                    if ((*(*ui).lp_vtbl).get_visibility)(ui_elem.as_ptr(), &mut orig_vis).is_err() {
                        orig_vis = VISIBILITY_VISIBLE;
                    }
                }
                if desired_hide {
                    let weak_src = qi_interface(node_com.as_ptr(), &IID_IWEAKREFERENCESOURCE);
                    if !weak_src.is_null() {
                        let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
                        let mut weak_ref = ComRaw::null();
                        if ((*(*src_iface).lp_vtbl).get_weak_reference)(
                            weak_src.as_ptr(),
                            &mut weak_ref.0,
                        )
                        .is_ok()
                            && !weak_ref.is_null()
                        {
                            let need_prune = UI_BRUSH_STORE.with(|cell| {
                                cell.try_borrow().ok().is_some_and(|s| {
                                    !s.hover_flyout_shapes.contains_key(&key)
                                        && s.hover_flyout_shapes.len() >= 64
                                })
                            });
                            if need_prune {
                                let candidates: Vec<(u64, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
                                    cell.try_borrow()
                                        .ok()
                                        .map(|s| {
                                            s.hover_flyout_shapes
                                                .iter()
                                                .map(|(&k, (w, _, _))| (k, w.clone_ref()))
                                                .collect()
                                        })
                                        .unwrap_or_default()
                                });
                                let dead: Vec<u64> = candidates
                                    .into_iter()
                                    .filter_map(|(ck, w)| {
                                        resolve_weak_inspectable(&w).is_none().then_some(ck)
                                    })
                                    .collect();
                                if !dead.is_empty() {
                                    let removed: Vec<(ComRaw, f64, i32)> =
                                        UI_BRUSH_STORE.with(|cell| {
                                            let Ok(mut s) = cell.try_borrow_mut() else {
                                                return Vec::new();
                                            };
                                            dead.iter()
                                                .filter_map(|dk| s.hover_flyout_shapes.remove(dk))
                                                .collect()
                                        });
                                    drop(removed);
                                }
                            }
                            let displaced = UI_BRUSH_STORE.with(|cell| {
                                if let Ok(mut s) = cell.try_borrow_mut() {
                                    if let std::collections::hash_map::Entry::Vacant(e) =
                                        s.hover_flyout_shapes.entry(key)
                                    {
                                        e.insert((weak_ref, orig_op, orig_vis));
                                        return None;
                                    }
                                }
                                Some(weak_ref)
                            });
                            drop(displaced);
                        }
                    }
                }
                (orig_op, orig_vis)
            };
            apply_shape_appearance(rt, node_com.as_ptr(), desired_hide, orig_opacity, orig_visibility);
            if desired_hide {
                rt.diag_flags
                    .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
            }
            continue;
        }

        if elem_name
            .as_deref()
            .is_some_and(|n| n.contains("HoverFlyoutContent") || n.contains("Thumbnail"))
        {
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
        for i in 0..count.min(32) {
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
                queue.push_back(child_dep);
            }
        }
    }
}

pub(in super::super) unsafe fn restore_all_hover_flyout_shapes_for_current_thread(rt: &Arc<TapRuntime>) {
    let entries: Vec<(ComRaw, f64, i32)> = UI_BRUSH_STORE.with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else {
            return Vec::new();
        };
        s.hover_flyout_shapes
            .drain()
            .map(|(_, entry)| entry)
            .collect()
    });
    for (weak_com, orig_opacity, orig_visibility) in entries {
        let wr = weak_com.as_ptr().cast::<RawIWeakReference>();
        let mut insp = ComRaw::null();
        if ((*(*wr).lp_vtbl).resolve)(weak_com.as_ptr(), &IID_IINSPECTABLE, &mut insp.0)
            .is_ok()
            && !insp.is_null()
        {
            apply_shape_appearance(rt, insp.as_ptr(), false, orig_opacity, orig_visibility);
        }
    }
}

pub(super) unsafe fn apply_hover_flyout_border_appearance(
    rt: &Arc<TapRuntime>,
    _handle: u64,
    insp_ptr: *mut c_void,
    desired_hide: bool,
) {
    let key = com_identity_key(insp_ptr);
    let border_com = qi_interface(insp_ptr, &IID_IBORDER);
    let saved = if desired_hide {
        track_general_modified_target(key, TargetKind::HoverFlyoutBorder, insp_ptr);
        SavedTargetBrushes::default()
    } else {
        remove_general_modified_target(key);
        unregister_watches_for_keys(&[key]);
        take_saved_target_brushes(key)
    };
    if !border_com.is_null() {
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            border_com.as_ptr(),
            BrushPropKind::BorderBackground,
            RawUiColor::TRANSPARENT,
            desired_hide,
            saved.background,
            DIAG_HOVER_FLYOUT_APPLIED,
        );
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            border_com.as_ptr(),
            BrushPropKind::BorderBrush,
            RawUiColor::TRANSPARENT,
            desired_hide,
            saved.border_brush,
            DIAG_HOVER_FLYOUT_APPLIED,
        );
    }
    sync_element_shadow_prop(
        rt,
        key,
        insp_ptr,
        desired_hide,
        saved.shadow,
        DIAG_HOVER_FLYOUT_SHADOW_APPLIED,
        false,
    );
    if desired_hide {
        if !border_com.is_null() {
            rt.diag_flags
                .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::HoverFlyoutBorder,
                BrushPropKind::BorderBackground,
                insp_ptr,
            );
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::HoverFlyoutBorder,
                BrushPropKind::BorderBrush,
                insp_ptr,
            );
        }
        ensure_prop_watch_registered(
            rt,
            key,
            TargetKind::HoverFlyoutBorder,
            BrushPropKind::ElementShadow,
            insp_ptr,
        );
    }
}

pub(super) unsafe fn apply_osd_panel_appearance(
    rt: &Arc<TapRuntime>,
    _handle: u64,
    insp_ptr: *mut c_void,
    desired_hide: bool,
) {
    let key = com_identity_key(insp_ptr);
    let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
    let grid2_com = qi_interface(insp_ptr, &IID_IGRID2);
    let saved = if desired_hide {
        track_general_modified_target(key, TargetKind::OsdPanel, insp_ptr);
        SavedTargetBrushes::default()
    } else {
        remove_general_modified_target(key);
        unregister_watches_for_keys(&[key]);
        take_saved_target_brushes(key)
    };
    let (target_bg, target_border) = rt.osd_target_colors();
    if !panel_com.is_null() {
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            panel_com.as_ptr(),
            BrushPropKind::PanelBackground,
            target_bg,
            desired_hide,
            saved.background,
            DIAG_OSD_APPLIED,
        );
    }
    if !grid2_com.is_null() {
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            grid2_com.as_ptr(),
            BrushPropKind::GridBorderBrush,
            target_border,
            desired_hide,
            saved.border_brush,
            DIAG_OSD_APPLIED,
        );
    }
    sync_element_shadow_prop(
        rt,
        key,
        insp_ptr,
        desired_hide,
        saved.shadow,
        DIAG_OSD_SHADOW_APPLIED,
        false,
    );
    if desired_hide {
        if !panel_com.is_null() {
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::OsdPanel,
                BrushPropKind::PanelBackground,
                insp_ptr,
            );
        }
        if !grid2_com.is_null() {
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::OsdPanel,
                BrushPropKind::GridBorderBrush,
                insp_ptr,
            );
        }
        ensure_prop_watch_registered(
            rt,
            key,
            TargetKind::OsdPanel,
            BrushPropKind::ElementShadow,
            insp_ptr,
        );
    }
}


pub(super) unsafe fn apply_virtual_desktop_bar_appearance(
    rt: &Arc<TapRuntime>,
    _handle: u64,
    insp_ptr: *mut c_void,
    kind: TargetKind,
    desired_hide: bool,
) {
    if insp_ptr.is_null() {
        return;
    }
    let key = com_identity_key(insp_ptr);
    let (target_bg, target_border) = rt.virtual_desktop_bar_target_colors();
    let saved = if desired_hide {
        track_general_modified_target(key, kind, insp_ptr);
        SavedTargetBrushes::default()
    } else {
        remove_general_modified_target(key);
        unregister_watches_for_keys(&[key]);
        take_saved_target_brushes(key)
    };

    let border_com = qi_interface(insp_ptr, &IID_IBORDER);
    if !border_com.is_null() {
        let _ = sync_overflow_brush_prop_with_mode(
            rt,
            key,
            insp_ptr,
            border_com.as_ptr(),
            BrushPropKind::BorderBackground,
            target_bg,
            desired_hide,
            saved.background,
            DIAG_VIRTUAL_DESKTOP_BAR_APPLIED,
            true,
        );
        let _ = sync_overflow_brush_prop_with_mode(
            rt,
            key,
            insp_ptr,
            border_com.as_ptr(),
            BrushPropKind::BorderBrush,
            target_border,
            desired_hide,
            saved.border_brush,
            DIAG_VIRTUAL_DESKTOP_BAR_APPLIED,
            true,
        );
        if desired_hide {
            ensure_prop_watch_registered(
                rt,
                key,
                kind,
                BrushPropKind::BorderBackground,
                insp_ptr,
            );
            ensure_prop_watch_registered(
                rt,
                key,
                kind,
                BrushPropKind::BorderBrush,
                insp_ptr,
            );
        }
        return;
    }

    let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
    if !panel_com.is_null() {
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            panel_com.as_ptr(),
            BrushPropKind::PanelBackground,
            target_bg,
            desired_hide,
            saved.background,
            DIAG_VIRTUAL_DESKTOP_BAR_APPLIED,
        );
        if desired_hide {
            ensure_prop_watch_registered(
                rt,
                key,
                kind,
                BrushPropKind::PanelBackground,
                insp_ptr,
            );
        }
    }
    let grid2_com = qi_interface(insp_ptr, &IID_IGRID2);
    if !grid2_com.is_null() {
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            grid2_com.as_ptr(),
            BrushPropKind::GridBorderBrush,
            target_border,
            desired_hide,
            saved.border_brush,
            DIAG_VIRTUAL_DESKTOP_BAR_APPLIED,
        );
        if desired_hide {
            ensure_prop_watch_registered(
                rt,
                key,
                kind,
                BrushPropKind::GridBorderBrush,
                insp_ptr,
            );
        }
    }
}

pub(super) unsafe fn apply_input_switcher_panel_appearance(
    rt: &Arc<TapRuntime>,
    _handle: u64,
    insp_ptr: *mut c_void,
    desired_hide: bool,
) {
    if insp_ptr.is_null() {
        return;
    }
    let key = com_identity_key(insp_ptr);
    let (target_bg, target_border) = rt.input_switcher_target_colors();
    let saved = if desired_hide {
        track_general_modified_target(key, TargetKind::InputSwitcherPanel, insp_ptr);
        SavedTargetBrushes::default()
    } else {
        remove_general_modified_target(key);
        unregister_watches_for_keys(&[key]);
        take_saved_target_brushes(key)
    };

    let border_com = qi_interface(insp_ptr, &IID_IBORDER);
    if !border_com.is_null() {
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            border_com.as_ptr(),
            BrushPropKind::BorderBackground,
            target_bg,
            desired_hide,
            saved.background,
            DIAG_INPUT_SWITCHER_APPLIED,
        );
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            border_com.as_ptr(),
            BrushPropKind::BorderBrush,
            target_border,
            desired_hide,
            saved.border_brush,
            DIAG_INPUT_SWITCHER_APPLIED,
        );
        if desired_hide {
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::InputSwitcherPanel,
                BrushPropKind::BorderBackground,
                insp_ptr,
            );
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::InputSwitcherPanel,
                BrushPropKind::BorderBrush,
                insp_ptr,
            );
        }
    } else {
        let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
        if !panel_com.is_null() {
            sync_overflow_brush_prop(
                rt,
                key,
                insp_ptr,
                panel_com.as_ptr(),
                BrushPropKind::PanelBackground,
                target_bg,
                desired_hide,
                saved.background,
                DIAG_INPUT_SWITCHER_APPLIED,
            );
            if desired_hide {
                ensure_prop_watch_registered(
                    rt,
                    key,
                    TargetKind::InputSwitcherPanel,
                    BrushPropKind::PanelBackground,
                    insp_ptr,
                );
            }
        }
        let grid2_com = qi_interface(insp_ptr, &IID_IGRID2);
        if !grid2_com.is_null() {
            sync_overflow_brush_prop(
                rt,
                key,
                insp_ptr,
                grid2_com.as_ptr(),
                BrushPropKind::GridBorderBrush,
                target_border,
                desired_hide,
                saved.border_brush,
                DIAG_INPUT_SWITCHER_APPLIED,
            );
            if desired_hide {
                ensure_prop_watch_registered(
                    rt,
                    key,
                    TargetKind::InputSwitcherPanel,
                    BrushPropKind::GridBorderBrush,
                    insp_ptr,
                );
            }
        }
    }

    sync_element_shadow_prop(
        rt,
        key,
        insp_ptr,
        desired_hide,
        saved.shadow,
        DIAG_INPUT_SWITCHER_SHADOW_APPLIED,
        false,
    );
    if desired_hide {
        ensure_prop_watch_registered(
            rt,
            key,
            TargetKind::InputSwitcherPanel,
            BrushPropKind::ElementShadow,
            insp_ptr,
        );
    }
}

pub(super) unsafe fn apply_input_switcher_sub_panel_appearance(
    rt: &Arc<TapRuntime>,
    _handle: u64,
    insp_ptr: *mut c_void,
    desired_hide: bool,
) {
    if insp_ptr.is_null() {
        return;
    }
    let key = com_identity_key(insp_ptr);
    let saved = if desired_hide {
        track_general_modified_target(key, TargetKind::InputSwitcherSubPanel, insp_ptr);
        SavedTargetBrushes::default()
    } else {
        remove_general_modified_target(key);
        unregister_watches_for_keys(&[key]);
        take_saved_target_brushes(key)
    };
    let border_com = qi_interface(insp_ptr, &IID_IBORDER);
    if !border_com.is_null() {
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            border_com.as_ptr(),
            BrushPropKind::BorderBackground,
            RawUiColor::TRANSPARENT,
            desired_hide,
            saved.background,
            DIAG_INPUT_SWITCHER_APPLIED,
        );
        if desired_hide {
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::InputSwitcherSubPanel,
                BrushPropKind::BorderBackground,
                insp_ptr,
            );
        }
        return;
    }
    let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
    if !panel_com.is_null() {
        sync_overflow_brush_prop(
            rt,
            key,
            insp_ptr,
            panel_com.as_ptr(),
            BrushPropKind::PanelBackground,
            RawUiColor::TRANSPARENT,
            desired_hide,
            saved.background,
            DIAG_INPUT_SWITCHER_APPLIED,
        );
        if desired_hide {
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::InputSwitcherSubPanel,
                BrushPropKind::PanelBackground,
                insp_ptr,
            );
        }
    }
}

