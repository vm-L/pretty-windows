//! UI-thread appearance application/restoration loop and newly matched visual-tree target onboarding.

use super::appearance::{
    apply_shape_appearance, apply_target_appearance, measure_element_bounds_dip,
    measure_hover_flyout_bounds_dip, measure_input_switcher_bounds_dip,
    measure_menu_flyout_bounds_dip, measure_tooltip_bounds_dip,
    restore_all_alttab_subtargets_for_current_thread,
    restore_all_general_targets_for_current_thread,
    restore_all_hover_flyout_shapes_for_current_thread,
    restore_all_menu_flyout_targets_for_current_thread,
    restore_all_modified_shapes_for_current_thread,
    restore_all_shell_card_targets_for_current_thread,
    restore_all_tooltip_targets_for_current_thread, sync_alttab_subtree_items,
    sync_focus_secondary_thickness_prop, sync_hover_flyout_bg_shapes,
};
use super::brush_store::{
    clear_all_watches_for_current_thread, prewarm_overflow_brush_store_for_current_thread,
    purge_saved_brushes_for_current_thread, purge_stale_handles_for_current_thread,
    register_handle_identity, resolve_weak_raw_inspectable, UI_BRUSH_STORE,
};
use super::raw_com::*;
use super::super::tree_match::{TargetKind, MAX_MENU_FLYOUT_RECTS};
use super::{release_owned_dispatcher, release_target_resources, TapRuntime};
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows::core::{IUnknown, Interface};
use windows::System::DispatcherQueue;
use windows::Win32::System::Threading::GetCurrentThreadId;

/// Runs strictly on an owning XAML UI thread via `DispatcherQueue` or `ICoreDispatcher`.
pub(super) fn apply_for_ui_thread(rt: &Arc<TapRuntime>, filter: DispatchFilter) {
    struct SnapshotItem {
        handle: u64,
        weak_com: ComRaw,
        kind: TargetKind,
        orig_opacity: f64,
        orig_visibility: i32,
    }

    let (desired_hide, items, pending_restores): (bool, Vec<SnapshotItem>, Vec<(u64, TrackedTarget)>) = {
        let Ok(mut g) = rt.state.lock() else {
            return;
        };
        let target_hide = g.desired_hide && !g.shutdown;
        let matches_filter = |t: &TrackedTarget| match (filter, t.thread_id, t.dispatcher) {
            (DispatchFilter::QueueTid(tid), Some(t_tid), OwnedDispatcher::Queue(_)) => t_tid == tid,
            (DispatchFilter::CoreTid(tid), Some(t_tid), OwnedDispatcher::Core(_)) => t_tid == tid,
            _ => false,
        };
        let list = g
            .targets
            .iter()
            .filter(|(_, t)| matches_filter(t))
            .map(|(&handle, t)| {
                add_ref_raw_com_addr(t.weak_ref_raw);
                SnapshotItem {
                    handle,
                    weak_com: ComRaw(t.weak_ref_raw as *mut c_void),
                    kind: t.kind,
                    orig_opacity: t.orig_opacity,
                    orig_visibility: t.orig_visibility,
                }
            })
            .collect();
        let mut restores = Vec::new();
        let mut i = 0usize;
        while i < g.pending_restores.len() {
            if matches_filter(&g.pending_restores[i].1) {
                restores.push(g.pending_restores.swap_remove(i));
            } else {
                i += 1;
            }
        }
        (target_hide, list, restores)
    };

    if !pending_restores.is_empty() {
        let git = unsafe { create_git().ok() };
        let mut restored_handles = Vec::with_capacity(pending_restores.len());
        for (eh, old) in pending_restores {
            restored_handles.push(eh);
            unsafe {
                if let Some(insp) = resolve_weak_raw_inspectable(old.weak_ref_raw) {
                    apply_target_appearance(
                        rt,
                        eh,
                        insp.as_ptr(),
                        old.kind,
                        false,
                        old.orig_opacity,
                        old.orig_visibility,
                    );
                }
            }
            release_target_resources(old, git.as_ref());
        }
        purge_saved_brushes_for_current_thread(&restored_handles);
    }

    if desired_hide && items.is_empty() {
        return;
    }
    if !items.is_empty() {
        rt.diag_flags.fetch_or(DIAG_APPLY_RAN, Ordering::Relaxed);
    }
    if desired_hide {
        unsafe {
            prewarm_overflow_brush_store_for_current_thread();
        }
    }

    let mut stale_handles = Vec::new();
    let mut saw_alt_tab = false;
    let mut measured_alt_tab: Option<[f32; 4]> = None;
    let mut saw_hover_flyout = false;
    let mut measured_hover_flyout: Option<([f32; 4], f32)> = None;
    let mut saw_osd = false;
    let mut measured_osd: Option<([f32; 4], f32)> = None;
    let osd_suppressed = rt.is_osd_suppressed();
    let mut saw_tooltip = false;
    let mut measured_tooltip: Option<([f32; 4], f32)> = None;
    let tooltip_suppressed = rt.is_tooltip_suppressed();
    let mut saw_input_switcher = false;
    let mut measured_input_switcher: Option<([f32; 4], f32)> = None;
    let input_switcher_suppressed = rt.is_input_switcher_suppressed();
    let mut saw_menu_flyout = false;
    let mut measured_menu_flyouts: Vec<[f32; 4]> = Vec::new();
    let menu_flyout_suppressed = rt.is_menu_flyout_suppressed();
    for item in items {
        let weak_com = item.weak_com;
        if weak_com.is_null() {
            stale_handles.push(item.handle);
            continue;
        }
        unsafe {
            let wr = weak_com.as_ptr().cast::<RawIWeakReference>();
            let mut insp = ComRaw::null();
            let hr_res = ((*(*wr).lp_vtbl).resolve)(
                weak_com.as_ptr(),
                &IID_IINSPECTABLE,
                &mut insp.0,
            );
            if hr_res.is_err() || insp.is_null() {
                if hr_res.is_ok() {
                    stale_handles.push(item.handle);
                }
                continue;
            }

            apply_target_appearance(
                rt,
                item.handle,
                insp.as_ptr(),
                item.kind,
                desired_hide,
                item.orig_opacity,
                item.orig_visibility,
            );

            if item.kind == TargetKind::AltTabBorder {
                saw_alt_tab = true;
                sync_alttab_subtree_items(rt, insp.as_ptr(), desired_hide);
                if desired_hide {
                    if let Some(bounds) =
                        measure_element_bounds_dip(insp.as_ptr(), 80.0, 60.0, 0.01)
                    {
                        measured_alt_tab = Some(bounds);
                    }
                }
            } else if item.kind == TargetKind::HoverFlyoutBorder {
                saw_hover_flyout = true;
                sync_hover_flyout_bg_shapes(rt, insp.as_ptr(), desired_hide);
                if desired_hide {
                    if let Some(bounds) = measure_hover_flyout_bounds_dip(insp.as_ptr()) {
                        let area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
                        if measured_hover_flyout
                            .is_none_or(|(_, best_area)| area > best_area)
                        {
                            measured_hover_flyout = Some((bounds, area));
                        }
                    }
                }
            } else if item.kind == TargetKind::OsdPanel {
                saw_osd = true;
                if desired_hide && !osd_suppressed {
                    if let Some(bounds) =
                        measure_element_bounds_dip(insp.as_ptr(), 80.0, 32.0, 0.05)
                    {
                        let area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
                        if measured_osd.is_none_or(|(_, best_area)| area > best_area) {
                            measured_osd = Some((bounds, area));
                        }
                    }
                }
            } else if item.kind == TargetKind::ToolTipPresenter {
                saw_tooltip = true;
                if desired_hide && !tooltip_suppressed {
                    if let Some(bounds) = measure_tooltip_bounds_dip(insp.as_ptr()) {
                        let area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
                        if measured_tooltip.is_none_or(|(_, best_area)| area > best_area) {
                            measured_tooltip = Some((bounds, area));
                        }
                    }
                }
            } else if item.kind == TargetKind::InputSwitcherPanel {
                saw_input_switcher = true;
                if desired_hide && !input_switcher_suppressed {
                    if let Some(bounds) = measure_input_switcher_bounds_dip(insp.as_ptr()) {
                        let area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
                        if measured_input_switcher
                            .is_none_or(|(_, best_area)| area > best_area)
                        {
                            measured_input_switcher = Some((bounds, area));
                        }
                    }
                }
            } else if item.kind == TargetKind::MenuFlyoutSurface {
                saw_menu_flyout = true;
                if desired_hide && !menu_flyout_suppressed {
                    if let Some(bounds) = measure_menu_flyout_bounds_dip(insp.as_ptr()) {
                        let is_dup = measured_menu_flyouts.iter().any(|prev| {
                            (prev[0] - bounds[0]).abs() <= 4.0
                                && (prev[1] - bounds[1]).abs() <= 4.0
                                && (prev[2] - bounds[2]).abs() <= 4.0
                                && (prev[3] - bounds[3]).abs() <= 4.0
                        });
                        if !is_dup && measured_menu_flyouts.len() < MAX_MENU_FLYOUT_RECTS {
                            measured_menu_flyouts.push(bounds);
                        }
                    }
                }
            } else if item.kind == TargetKind::ShellCardSurface {
                let cur_tid = GetCurrentThreadId();
                let bounds_opt = if desired_hide && !menu_flyout_suppressed {
                    measure_element_bounds_dip(insp.as_ptr(), 64.0, 28.0, 0.05)
                } else {
                    None
                };
                if let Ok(mut b) = rt.shell_card_bounds_by_handle.lock() {
                    if let Some(bounds) = bounds_opt {
                        b.insert(item.handle, (cur_tid, bounds));
                    } else {
                        b.remove(&item.handle);
                    }
                }
            }
        }
    }

    if saw_alt_tab {
        let cur_tid = unsafe { GetCurrentThreadId() };
        if let Ok(mut b) = rt.alt_tab_bounds_dip.lock() {
            *b = if desired_hide {
                measured_alt_tab.map(|r| (cur_tid, r))
            } else {
                None
            };
        }
    }
    if saw_hover_flyout {
        let cur_tid = unsafe { GetCurrentThreadId() };
        if let Ok(mut b) = rt.hover_flyout_bounds_dip.lock() {
            *b = if desired_hide {
                measured_hover_flyout.map(|(r, _)| (cur_tid, r))
            } else {
                None
            };
        }
    }
    if saw_osd {
        let cur_tid = unsafe { GetCurrentThreadId() };
        if let Ok(mut b) = rt.osd_bounds_dip.lock() {
            *b = if desired_hide && !osd_suppressed {
                measured_osd.map(|(r, _)| (cur_tid, r))
            } else {
                None
            };
        }
    }
    if saw_tooltip {
        let cur_tid = unsafe { GetCurrentThreadId() };
        if let Ok(mut b) = rt.tooltip_bounds_dip.lock() {
            *b = if desired_hide && !tooltip_suppressed {
                measured_tooltip.map(|(r, _)| (cur_tid, r))
            } else {
                None
            };
        }
    }
    if saw_input_switcher {
        let cur_tid = unsafe { GetCurrentThreadId() };
        if let Ok(mut b) = rt.input_switcher_bounds_dip.lock() {
            *b = if desired_hide && !input_switcher_suppressed {
                measured_input_switcher.map(|(r, _)| (cur_tid, r))
            } else {
                None
            };
        }
    }
    if saw_menu_flyout {
        let cur_tid = unsafe { GetCurrentThreadId() };
        if let Ok(mut b) = rt.menu_flyout_bounds_by_tid.lock() {
            if desired_hide && !menu_flyout_suppressed && !measured_menu_flyouts.is_empty() {
                b.insert(cur_tid, measured_menu_flyouts);
            } else {
                b.remove(&cur_tid);
            }
        }
    }

    if !stale_handles.is_empty() {
        unsafe {
            purge_stale_handles_for_current_thread(&stale_handles);
        }
        if let Ok(mut b) = rt.shell_card_bounds_by_handle.lock() {
            for &h in &stale_handles {
                b.remove(&h);
            }
        }
        let mut evicted = Vec::new();
        if let Ok(mut g) = rt.state.lock() {
            for h in stale_handles {
                if let Some(old) = g.targets.remove(&h) {
                    evicted.push(old);
                }
            }
        }
        if !evicted.is_empty() {
            let git = unsafe { create_git().ok() };
            for old in evicted {
                release_target_resources(old, git.as_ref());
            }
        }
    }

    if !desired_hide {
        unsafe {
            restore_all_modified_shapes_for_current_thread(rt);
            restore_all_general_targets_for_current_thread(rt);
            restore_all_alttab_subtargets_for_current_thread(rt);
            restore_all_hover_flyout_shapes_for_current_thread(rt);
            restore_all_tooltip_targets_for_current_thread(rt);
            restore_all_menu_flyout_targets_for_current_thread(rt);
            restore_all_shell_card_targets_for_current_thread(rt);
        }
    }

    if rt.is_shutdown() {
        clear_all_watches_for_current_thread();
        let old_store = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .map(|mut s| std::mem::take(&mut *s))
        });
        drop(old_store);
    }
}

/// Runs on the XAML UI thread from `OnVisualTreeChange(Add)` for matching background elements.
/// Overflow elements must NEVER call `RoGetActivationFactory` or mutate XAML brushes here; they
/// only record weak references and enqueue async UI-thread work after the callback unwinds.
pub(super) unsafe fn on_target_added(rt: &Arc<TapRuntime>, handle: u64, kind: TargetKind) {
    if handle == 0 || rt.is_shutdown() {
        return;
    }
    rt.diag_flags
        .fetch_or(DIAG_ENTER_ON_TARGET, Ordering::Relaxed);
    let diag_cookie = rt.diag_cookie.load(Ordering::Acquire);
    if diag_cookie == 0 {
        return;
    }

    let Ok(git) = create_git() else {
        return;
    };

    let mut diag = ComRaw::null();
    if git
        .GetInterfaceFromGlobal(diag_cookie, &IID_IXAMLDIAGNOSTICS, &mut diag.0)
        .is_err()
        || diag.is_null()
    {
        return;
    }
    rt.diag_flags
        .fetch_or(DIAG_DIAG_FROM_GIT_OK, Ordering::Relaxed);

    let diag_iface = diag.as_ptr().cast::<RawIXamlDiagnostics>();
    let mut insp = ComRaw::null();
    let hr_insp = ((*(*diag_iface).lp_vtbl).get_iinspectable_from_handle)(
        diag.as_ptr(),
        handle,
        &mut insp.0,
    );
    if hr_insp.is_err() || insp.is_null() {
        return;
    }
    rt.diag_flags.fetch_or(DIAG_GET_INSP_OK, Ordering::Relaxed);
    register_handle_identity(handle, com_identity_key(insp.as_ptr()));

    let mut orig = 1.0f64;
    let mut orig_vis = VISIBILITY_VISIBLE;
    if matches!(kind, TargetKind::ShapeElement | TargetKind::HoverFlyoutShape) {
        let ui_elem = qi_interface(insp.as_ptr(), &IID_IUIELEMENT);
        if ui_elem.is_null() {
            return;
        }
        rt.diag_flags.fetch_or(DIAG_QI_UI_OK, Ordering::Relaxed);
        let ui = ui_elem.as_ptr().cast::<RawIUIElement>();
        if ((*(*ui).lp_vtbl).get_opacity)(ui_elem.as_ptr(), &mut orig).is_err()
            || !orig.is_finite()
            || orig <= 0.01
        {
            orig = 1.0;
        }
        if ((*(*ui).lp_vtbl).get_visibility)(ui_elem.as_ptr(), &mut orig_vis).is_err() {
            orig_vis = VISIBILITY_VISIBLE;
        }
    }

    let weak_src = qi_interface(insp.as_ptr(), &IID_IWEAKREFERENCESOURCE);
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
    rt.diag_flags.fetch_or(DIAG_QI_WEAK_OK, Ordering::Relaxed);

    let want_shape_hide_initial =
        matches!(kind, TargetKind::ShapeElement | TargetKind::HoverFlyoutShape)
            && rt.desired_hide();
    let want_focus_hide_initial = kind == TargetKind::AltTabItemFocus && rt.desired_hide();
    if want_shape_hide_initial {
        apply_shape_appearance(rt, insp.as_ptr(), true, orig, orig_vis);
        if kind == TargetKind::HoverFlyoutShape {
            rt.diag_flags
                .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
        }
    } else if want_focus_hide_initial {
        let key = com_identity_key(insp.as_ptr());
        sync_focus_secondary_thickness_prop(rt, key, insp.as_ptr(), true, false);
    }

    let cur_tid = GetCurrentThreadId();
    let need_ui_disp = rt
        .state
        .lock()
        .map(|g| {
            !g.shutdown
                && g.ui_dispatchers.len() < 64
                && !g.ui_dispatchers.iter().any(|(tid, _)| *tid == cur_tid)
        })
        .unwrap_or(false);
    let mut extra_ui_disp: Option<OwnedDispatcher> = None;
    let (thread_id, dispatcher, async_dispatch) =
        if let Ok(dq) = DispatcherQueue::GetForCurrentThread() {
            let dq_for_enqueue = dq.clone();
            if need_ui_disp {
                extra_ui_disp = Some(OwnedDispatcher::Queue(dq.clone().into_raw() as usize));
            }
            (
                Some(cur_tid),
                OwnedDispatcher::Queue(dq.into_raw() as usize),
                DispatchTarget::Queue {
                    thread_id: cur_tid,
                    queue: dq_for_enqueue,
                },
            )
        } else {
            let dep = qi_interface(insp.as_ptr(), &IID_IDEPENDENCYOBJECT);
            if dep.is_null() {
                return;
            }
            let dep_iface = dep.as_ptr().cast::<RawIDependencyObject>();
            let mut cd = ComRaw::null();
            if ((*(*dep_iface).lp_vtbl).get_dispatcher)(dep.as_ptr(), &mut cd.0).is_err()
                || cd.is_null()
            {
                return;
            }
            let Some(cd_unk) = IUnknown::from_raw_borrowed(&cd.0) else {
                return;
            };
            let Ok(cookie) = git.RegisterInterfaceInGlobal(cd_unk, &IID_ICOREDISPATCHER) else {
                return;
            };
            if need_ui_disp {
                if let Ok(extra_cookie) =
                    git.RegisterInterfaceInGlobal(cd_unk, &IID_ICOREDISPATCHER)
                {
                    extra_ui_disp = Some(OwnedDispatcher::Core(extra_cookie));
                }
            }
            (
                Some(cur_tid),
                OwnedDispatcher::Core(cookie),
                DispatchTarget::Core {
                    thread_id: cur_tid,
                    cookie,
                },
            )
        };
    rt.diag_flags
        .fetch_or(DIAG_DISPATCHER_OK, Ordering::Relaxed);

    let evicted: Option<TrackedTarget>;
    let mut want_hide_after = false;
    let mut shutdown_after = true;

    if let Ok(mut g) = rt.state.lock() {
        shutdown_after = g.shutdown;
        if !g.shutdown {
            evicted = g.targets.insert(
                handle,
                TrackedTarget {
                    thread_id,
                    weak_ref_raw: weak_ref.into_raw() as usize,
                    kind,
                    orig_opacity: orig,
                    orig_visibility: orig_vis,
                    dispatcher,
                },
            );
            if let Some(extra) = extra_ui_disp.take() {
                if g.ui_dispatchers.len() < 64
                    && !g.ui_dispatchers.iter().any(|(tid, _)| *tid == cur_tid)
                {
                    g.ui_dispatchers.push((cur_tid, extra));
                } else {
                    extra_ui_disp = Some(extra);
                }
            }
            want_hide_after = g.desired_hide;
            rt.diag_flags
                .fetch_or(DIAG_INSERTED_TARGET, Ordering::Relaxed);
        } else {
            evicted = Some(TrackedTarget {
                thread_id,
                weak_ref_raw: weak_ref.into_raw() as usize,
                kind,
                orig_opacity: orig,
                orig_visibility: orig_vis,
                dispatcher,
            });
        }
    } else {
        evicted = Some(TrackedTarget {
            thread_id,
            weak_ref_raw: weak_ref.into_raw() as usize,
            kind,
            orig_opacity: orig,
            orig_visibility: orig_vis,
            dispatcher,
        });
    }

    if let Some(unused_extra) = extra_ui_disp {
        release_owned_dispatcher(unused_extra, Some(&git));
    }
    if let Some(old) = evicted {
        release_target_resources(old, Some(&git));
    }

    if shutdown_after {
        if want_shape_hide_initial {
            apply_shape_appearance(rt, insp.as_ptr(), false, orig, orig_vis);
        } else if want_focus_hide_initial {
            let key = com_identity_key(insp.as_ptr());
            sync_focus_secondary_thickness_prop(rt, key, insp.as_ptr(), false, false);
        }
    } else if matches!(kind, TargetKind::ShapeElement | TargetKind::HoverFlyoutShape) {
        if want_hide_after != want_shape_hide_initial {
            apply_shape_appearance(rt, insp.as_ptr(), want_hide_after, orig, orig_vis);
            if want_hide_after && kind == TargetKind::HoverFlyoutShape {
                rt.diag_flags
                    .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
            }
        }
    } else if want_hide_after {
        let core_git = matches!(async_dispatch, DispatchTarget::Core { .. }).then_some(&git);
        rt.enqueue_target_dispatch(async_dispatch, core_git, None);
    } else if want_focus_hide_initial {
        let key = com_identity_key(insp.as_ptr());
        sync_focus_secondary_thickness_prop(rt, key, insp.as_ptr(), false, false);
    }
}

