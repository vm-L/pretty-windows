use super::super::tree_match::{TapHostScope, TargetKind};
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::{IUnknown, Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{BOOL, CLASS_E_CLASSNOTAVAILABLE, E_NOINTERFACE, E_POINTER, HINSTANCE, HMODULE, S_FALSE, S_OK, TRUE};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::{GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN};
use windows::Win32::System::Threading::{GetCurrentThreadId, SetEvent, WaitForSingleObject, INFINITE};
use super::appearance::apply_target_appearance;
use super::brush_store::{
    purge_saved_brushes_for_current_thread, resolve_weak_inspectable,
    resolve_weak_raw_inspectable,
};
use super::pipe::{run_pipe_server_loop, run_reverse_pipe_client_loop};
use super::raw_com::*;
use super::{
    on_target_added, release_owned_dispatcher, release_target_resources, TapRuntime,
};

unsafe fn bstr_ptr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while len < 512 && *ptr.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
}

// ---------- COM Object: VisualTreeWatcher (IVisualTreeServiceCallback2 + FTM + IAgileObject) ----------

#[repr(C)]
struct RawCallback2Vtbl {
    query_interface: unsafe extern "system" fn(
        *mut VisualTreeWatcher,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut VisualTreeWatcher) -> u32,
    release: unsafe extern "system" fn(*mut VisualTreeWatcher) -> u32,
    on_visual_tree_change: unsafe extern "system" fn(
        *mut VisualTreeWatcher,
        RawParentChildRelation,
        RawVisualElement,
        i32,
    ) -> HRESULT,
    on_element_state_changed:
        unsafe extern "system" fn(*mut VisualTreeWatcher, u64, i32, PCWSTR) -> HRESULT,
}

static WATCHER_VTBL: RawCallback2Vtbl = RawCallback2Vtbl {
    query_interface: watcher_qi,
    add_ref: watcher_add_ref,
    release: watcher_release,
    on_visual_tree_change: watcher_on_visual_tree_change,
    on_element_state_changed: watcher_on_element_state_changed,
};

#[repr(C)]
struct VisualTreeWatcher {
    lp_vtbl: *const RawCallback2Vtbl,
    ref_count: AtomicU32,
    destroying: AtomicBool,
    ftm: Mutex<Option<usize>>,
    runtime: Arc<TapRuntime>,
}

impl VisualTreeWatcher {
    fn new_com_ptr(runtime: Arc<TapRuntime>) -> *mut Self {
        ACTIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
        Box::into_raw(Box::new(Self {
            lp_vtbl: &WATCHER_VTBL,
            ref_count: AtomicU32::new(1),
            destroying: AtomicBool::new(false),
            ftm: Mutex::new(None),
            runtime,
        }))
    }
}

unsafe extern "system" fn watcher_qi(
    this: *mut VisualTreeWatcher,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if (*this).destroying.load(Ordering::Acquire) {
        return E_NOINTERFACE;
    }
    let iid = *riid;
    if iid == IID_IUNKNOWN
        || iid == IID_IVISUALTREESERVICECALLBACK
        || iid == IID_IVISUALTREESERVICECALLBACK2
        || iid == IID_IAGILEOBJECT
    {
        *ppv = this.cast();
        watcher_add_ref(this);
        S_OK
    } else if iid == IID_IMARSHAL {
        delegate_ftm_qi(this.cast(), &(*this).ftm, riid, ppv)
    } else {
        E_NOINTERFACE
    }
}

unsafe extern "system" fn watcher_add_ref(this: *mut VisualTreeWatcher) -> u32 {
    (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn watcher_release(this: *mut VisualTreeWatcher) -> u32 {
    let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        if (*this).destroying.swap(true, Ordering::AcqRel) {
            return 0;
        }
        std::sync::atomic::fence(Ordering::Acquire);
        (*this).ref_count.store(1, Ordering::Release);
        let ftm_addr = (*this).ftm.lock().ok().and_then(|mut g| g.take());
        if let Some(addr) = ftm_addr {
            release_raw_com_addr(addr);
        }
        drop(Box::from_raw(this));
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
        return 0;
    }
    prev - 1
}

unsafe extern "system" fn watcher_on_visual_tree_change(
    this: *mut VisualTreeWatcher,
    relation: RawParentChildRelation,
    element: RawVisualElement,
    mutation_type: i32,
) -> HRESULT {
    let watcher = &*this;
    let rt = &watcher.runtime;
    if rt.is_shutdown() {
        return S_OK;
    }

    let handle = if element.handle != 0 {
        element.handle
    } else {
        relation.child
    };

    if mutation_type == 0 {
        rt.diag_flags.fetch_or(DIAG_WATCHER_CALLED, Ordering::Relaxed);
        let cur_tid = GetCurrentThreadId();
        let type_name = bstr_ptr_to_string(element.type_name);
        let name = bstr_ptr_to_string(element.name);
        let hint_container_root = type_name.contains("TaskbarFrame")
            || type_name.contains("TaskbarBackground")
            || type_name.contains("FlyoutFrame")
            || matches!(
                name.as_str(),
                "HoverFlyoutCanvas"
                    | "HoverFlyoutGrid"
                    | "HoverFlyoutBackground"
                    | "HoverFlyoutBackgroundControl"
            )
            || name == "OverflowRootGrid"
            || name == "ModalRootGrid"
            || name == "AltTabRoot"
            || type_name.contains("Switcher.AltTab")
            || type_name.contains("Switcher.TaskView")
            || type_name.contains("VirtualDesktopBarElement")
            || matches!(
                name.as_str(),
                "TaskViewTimelineRoot"
                    | "TaskViewRoot"
                    | "VirtualDesktopBar"
                    | "VirtualDesktopSwitcherBackground"
                    | "VirtualDesktopBarBackground"
            )
            || name == "BackgroundElement"
            || type_name.contains("SwitchItemList")
            || name == "CustomFocusItemPanel"
            || type_name.contains("NotificationAreaOverflow")
            || type_name.contains("NotifyIconOverflow")
            || type_name.contains("OverflowView")
            || name == "ConfirmatorFrameCanvas"
            || name == "ConfirmatorMainGrid"
            || type_name.contains("HWConfirmatorUI")
            || type_name.contains("ConfirmatorHostControl")
            || type_name.ends_with(".ToolTip")
            || type_name == "ToolTip"
            || type_name.contains("MenuFlyoutPresenter")
            || type_name.contains("JumpList")
            || type_name.contains("JumpView")
            || name == "JumpListRestyledAcrylic"
            || name == "JumpListGrid"
            || type_name.contains("TaskbarJumpListFrame")
            || type_name.contains("ControlCenter")
            || type_name.contains("NotificationCenter")
            || type_name.contains("CalendarCenter")
            || type_name.contains("ActionCenter")
            || type_name.contains("ToastNotificationView")
            || type_name.contains("FlexibleToastView")
            || type_name.contains("FlexibleNormalToastView")
            || type_name.contains("FlexiblePriorityToastView")
            || type_name.contains("InputSwitcher")
            || type_name.contains("TaskbarSearchPage")
            || type_name.contains("FolderModal")
            || type_name.contains("ExpandedFolderList")
            || (rt.host_scope == TapHostScope::StartMenu && name == "Root")
            || matches!(
                name.as_str(),
                "NotificationCenterGrid"
                    | "NotificationCenterView"
                    | "NotificationCenterPage"
                    | "CalendarCenterGrid"
                    | "CalendarHeaderMinimizedOverlay"
                    | "ControlCenterRegion"
                    | "RootGridBorder"
                    | "L1Grid"
                    | "L2Frame"
                    | "MainGrid"
                    | "MediaTransportControlsRegion"
                    | "MediaTransportControlsGrid"
                    | "BackgroundBlur"
                    | "FooterGrid"
                    | "PaneFooter"
                    | "Footer"
                    | "ToastBackgroundBorder2"
                    | "ToastBackgroundBorder"
                    | "FrameRoot"
                    | "CompanionRoot"
                    | "MainMenu"
                    | "AcrylicBorder"
                    | "AcrylicOverlay"
                    | "StartDropShadow"
                    | "LeftCompanionDropShadow"
                    | "RightCompanionDropShadow"
                    | "StartFolderModal"
                    | "FolderModal"
                    | "ExpandedFolderList"
                    | "OuterBorderGrid"
                    | "BorderGrid"
                    | "AppBorder"
                    | "AccentAppBorder"
                    | "LayerBorder"
                    | "AccentLayerBorder"
                    | "dropshadow"
                    | "FlatContent"
                    | "OverlayPanel"
                    | "HeaderPanel"
            );
        let may_be_alttab_item_container = type_name.contains("ListViewItem");
        let mut is_container_root = hint_container_root;
        if hint_container_root {
            rt.diag_flags.fetch_or(DIAG_SAW_FRAME_OR_BG, Ordering::Relaxed);
        }
        if matches!(
            name.as_str(),
            "BackgroundFill" | "BackgroundStroke" | "ScreenEdgeStroke"
        ) {
            rt.diag_flags.fetch_or(DIAG_SAW_TARGET_NAME, Ordering::Relaxed);
        }
        let existing_flyout_on_tid: Vec<(u64, TargetKind)> = if is_container_root {
            rt.state
                .lock()
                .map(|g| {
                    g.targets
                        .iter()
                        .filter_map(|(&h, t)| {
                            (t.thread_id == Some(cur_tid)
                                && matches!(
                                    t.kind,
                                    TargetKind::MenuFlyoutSurface
                                        | TargetKind::ShellCardSurface
                                        | TargetKind::ShellCardSubSurface
                                        | TargetKind::StartMenuModalSurface
                                ))
                            .then_some((h, t.kind))
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let (matched_kind, extra_candidates, reclass_actions) = if let Ok(mut idx) =
            rt.tree.lock()
        {
            idx.on_add_on_thread(relation.parent, handle, type_name, name, cur_tid);
            if !is_container_root && may_be_alttab_item_container {
                is_container_root = idx.thread_has_alttab_container(cur_tid);
            }
            let matched_kind = idx.classify_target_in_scope(handle, rt.host_scope);
            let extra = if is_container_root {
                idx.collect_classified_targets_for_thread_in_scope(cur_tid, rt.host_scope)
            } else {
                Vec::new()
            };
            let mut reclass = Vec::new();
            for (eh, old_kind) in existing_flyout_on_tid {
                let new_kind = idx.classify_target_in_scope(eh, rt.host_scope);
                if new_kind != Some(old_kind) {
                    reclass.push((eh, old_kind, new_kind));
                }
            }
            (matched_kind, extra, reclass)
        } else {
            (None, Vec::new(), Vec::new())
        };
        if !reclass_actions.is_empty() {
            let mut dropped_targets = Vec::new();
            let mut downgraded_handles = Vec::new();
            let mut clear_bounds_handles = Vec::new();
            if let Ok(mut g) = rt.state.lock() {
                for (eh, old_kind, new_kind) in reclass_actions {
                    if old_kind == TargetKind::ShellCardSurface
                        && new_kind != Some(TargetKind::ShellCardSurface)
                    {
                        clear_bounds_handles.push(eh);
                    }
                    match new_kind {
                        Some(nk) => {
                            if let Some(t) = g.targets.get_mut(&eh) {
                                t.kind = nk;
                                downgraded_handles.push((eh, nk));
                            }
                        }
                        None => {
                            if let Some(old) = g.targets.remove(&eh) {
                                dropped_targets.push((eh, old));
                            }
                        }
                    }
                }
            }
            if !clear_bounds_handles.is_empty() {
                if let Ok(mut b) = rt.shell_card_bounds_by_handle.lock() {
                    for h in clear_bounds_handles {
                        b.remove(&h);
                    }
                }
            }
            for (eh, nk) in downgraded_handles {
                if let Ok(g) = rt.state.lock() {
                    if let Some(t) = g.targets.get(&eh) {
                        let hide_now = g.desired_hide && !g.shutdown;
                        let weak_raw = t.weak_ref_raw;
                        let op = t.orig_opacity;
                        let vis = t.orig_visibility;
                        if weak_raw != 0 {
                            add_ref_raw_com_addr(weak_raw);
                        }
                        let weak_com = ComRaw(weak_raw as *mut c_void);
                        drop(g);
                        if let Some(insp) = resolve_weak_inspectable(&weak_com) {
                            apply_target_appearance(
                                rt,
                                eh,
                                insp.as_ptr(),
                                nk,
                                hide_now,
                                op,
                                vis,
                            );
                        }
                    }
                }
            }
            if !dropped_targets.is_empty() {
                let git = create_git().ok();
                for (eh, old) in dropped_targets {
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
                    release_target_resources(old, git.as_ref());
                }
            }
        }
        if let Some(kind) = matched_kind {
            rt.diag_flags.fetch_or(DIAG_TARGET_MATCHED, Ordering::Relaxed);
            on_target_added(rt, handle, kind);
        }
        if !extra_candidates.is_empty() {
            let pending: Vec<(u64, TargetKind)> = if let Ok(g) = rt.state.lock() {
                extra_candidates
                    .into_iter()
                    .filter(|&(h, _)| h != handle && !g.targets.contains_key(&h))
                    .collect()
            } else {
                Vec::new()
            };
            for (extra_h, extra_kind) in pending {
                rt.diag_flags.fetch_or(DIAG_TARGET_MATCHED, Ordering::Relaxed);
                on_target_added(rt, extra_h, extra_kind);
            }
        }
    } else {
        let removed = if let Ok(mut idx) = rt.tree.lock() {
            idx.on_remove(handle)
        } else {
            Vec::new()
        };
        if !removed.is_empty() {
            purge_saved_brushes_for_current_thread(&removed);
            let mut evicted = Vec::new();
            let mut removed_alt_tab = false;
            let mut removed_hover_flyout = false;
            let mut removed_osd = false;
            let mut removed_tooltip = false;
            let mut removed_menu_flyout = false;
            let mut removed_input_switcher = false;
            let mut remaining_menu_on_tid = false;
            let mut removed_shell_handles: Vec<u64> = Vec::new();
            let rem_tid = GetCurrentThreadId();
            if let Ok(mut g) = rt.state.lock() {
                for rh in removed {
                    if let Some(old) = g.targets.remove(&rh) {
                        if old.kind == TargetKind::AltTabBorder {
                            removed_alt_tab = true;
                        } else if old.kind == TargetKind::HoverFlyoutBorder {
                            removed_hover_flyout = true;
                        } else if old.kind == TargetKind::OsdPanel {
                            removed_osd = true;
                        } else if old.kind == TargetKind::ToolTipPresenter {
                            removed_tooltip = true;
                        } else if old.kind == TargetKind::MenuFlyoutSurface {
                            removed_menu_flyout = true;
                        } else if old.kind == TargetKind::InputSwitcherPanel {
                            removed_input_switcher = true;
                        } else if old.kind == TargetKind::ShellCardSurface {
                            removed_shell_handles.push(rh);
                        }
                        evicted.push(old);
                    }
                }
                if removed_menu_flyout {
                    remaining_menu_on_tid = g.targets.values().any(|t| {
                        t.kind == TargetKind::MenuFlyoutSurface
                            && t.thread_id.is_none_or(|tid| tid == rem_tid)
                    });
                }
            }
            if removed_alt_tab {
                if let Ok(mut b) = rt.alt_tab_bounds_dip.lock() {
                    *b = None;
                }
            }
            if removed_hover_flyout {
                if let Ok(mut b) = rt.hover_flyout_bounds_dip.lock() {
                    *b = None;
                }
            }
            if removed_osd {
                if let Ok(mut b) = rt.osd_bounds_dip.lock() {
                    *b = None;
                }
            }
            if removed_tooltip {
                if let Ok(mut b) = rt.tooltip_bounds_dip.lock() {
                    *b = None;
                }
            }
            if removed_input_switcher {
                if let Ok(mut b) = rt.input_switcher_bounds_dip.lock() {
                    *b = None;
                }
            }
            if removed_menu_flyout && !remaining_menu_on_tid {
                if let Ok(mut b) = rt.menu_flyout_bounds_by_tid.lock() {
                    b.remove(&rem_tid);
                }
            }
            if !removed_shell_handles.is_empty() {
                if let Ok(mut b) = rt.shell_card_bounds_by_handle.lock() {
                    for rh in removed_shell_handles {
                        b.remove(&rh);
                    }
                }
            }
            if !evicted.is_empty() {
                let git = create_git().ok();
                for old in evicted {
                    release_target_resources(old, git.as_ref());
                }
            }
        }
    }

    S_OK
}

unsafe extern "system" fn watcher_on_element_state_changed(
    _this: *mut VisualTreeWatcher,
    _element: u64,
    _element_state: i32,
    _context: PCWSTR,
) -> HRESULT {
    S_OK
}

// ---------- COM Object: TapSite (IObjectWithSite + FTM + IAgileObject) ----------

#[repr(C)]
struct RawObjectWithSiteVtbl {
    query_interface:
        unsafe extern "system" fn(*mut TapSite, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut TapSite) -> u32,
    release: unsafe extern "system" fn(*mut TapSite) -> u32,
    set_site: unsafe extern "system" fn(*mut TapSite, *mut RawIUnknown) -> HRESULT,
    get_site: unsafe extern "system" fn(*mut TapSite, *const GUID, *mut *mut c_void) -> HRESULT,
}

static SITE_VTBL: RawObjectWithSiteVtbl = RawObjectWithSiteVtbl {
    query_interface: site_qi,
    add_ref: site_add_ref,
    release: site_release,
    set_site: site_set_site,
    get_site: site_get_site,
};

#[repr(C)]
struct TapSite {
    lp_vtbl: *const RawObjectWithSiteVtbl,
    ref_count: AtomicU32,
    destroying: AtomicBool,
    ftm: Mutex<Option<usize>>,
    site_cookie: Mutex<u32>,
    runtime: Mutex<Option<Arc<TapRuntime>>>,
}

impl TapSite {
    fn new_com_ptr() -> *mut Self {
        ACTIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
        Box::into_raw(Box::new(Self {
            lp_vtbl: &SITE_VTBL,
            ref_count: AtomicU32::new(1),
            destroying: AtomicBool::new(false),
            ftm: Mutex::new(None),
            site_cookie: Mutex::new(0),
            runtime: Mutex::new(None),
        }))
    }
}

unsafe extern "system" fn site_qi(
    this: *mut TapSite,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if (*this).destroying.load(Ordering::Acquire) {
        return E_NOINTERFACE;
    }
    let iid = *riid;
    if iid == IID_IUNKNOWN || iid == IID_IOBJECTWITHSITE || iid == IID_IAGILEOBJECT {
        *ppv = this.cast();
        site_add_ref(this);
        S_OK
    } else if iid == IID_IMARSHAL {
        delegate_ftm_qi(this.cast(), &(*this).ftm, riid, ppv)
    } else {
        E_NOINTERFACE
    }
}

unsafe extern "system" fn site_add_ref(this: *mut TapSite) -> u32 {
    (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn site_release(this: *mut TapSite) -> u32 {
    let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        if (*this).destroying.swap(true, Ordering::AcqRel) {
            return 0;
        }
        std::sync::atomic::fence(Ordering::Acquire);
        (*this).ref_count.store(1, Ordering::Release);

        // Release COM site & FTM; keep active TapRuntime alive until SetSite(null),
        // CMD_SHUTDOWN, or host crash watchdog triggers shutdown.
        let ftm_addr = (*this).ftm.lock().ok().and_then(|mut g| g.take());
        if let Some(addr) = ftm_addr {
            release_raw_com_addr(addr);
        }
        if let Ok(mut rt_guard) = (*this).runtime.lock() {
            let _ = rt_guard.take();
        }
        let old_cookie = (*this)
            .site_cookie
            .lock()
            .ok()
            .map(|mut sc| {
                let old = *sc;
                *sc = 0;
                old
            })
            .unwrap_or(0);
        if old_cookie != 0 {
            if let Ok(git) = create_git() {
                let _ = git.RevokeInterfaceFromGlobal(old_cookie);
            }
        }
        drop(Box::from_raw(this));
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
        return 0;
    }
    prev - 1
}

unsafe extern "system" fn site_set_site(
    this: *mut TapSite,
    punk_site: *mut RawIUnknown,
) -> HRESULT {
    let site = &*this;
    if punk_site.is_null() {
        let old_rt = site.runtime.lock().ok().and_then(|mut g| g.take());
        if let Some(rt) = old_rt {
            rt.trigger_shutdown(500);
        }
        let old_cookie = site
            .site_cookie
            .lock()
            .ok()
            .map(|mut sc| {
                let old = *sc;
                *sc = 0;
                old
            })
            .unwrap_or(0);
        if old_cookie != 0 {
            if let Ok(git) = create_git() {
                let _ = git.RevokeInterfaceFromGlobal(old_cookie);
            }
        }
        return S_OK;
    }

    let prev_rt = site.runtime.lock().ok().and_then(|mut g| g.take());
    if let Some(old_rt) = prev_rt {
        old_rt.trigger_shutdown(300);
    }

    // Verify both IVisualTreeService and IXamlDiagnostics on punk_site and marshal via GIT.
    let mut vts_com = ComRaw::null();
    let hr_vts = ((*(*punk_site).lp_vtbl).query_interface)(
        punk_site.cast(),
        &IID_IVISUALTREESERVICE,
        &mut vts_com.0,
    );
    if hr_vts.is_err() || vts_com.is_null() {
        return if hr_vts.is_err() {
            hr_vts
        } else {
            E_NOINTERFACE
        };
    }

    let mut diag_com = ComRaw::null();
    let hr_diag = ((*(*punk_site).lp_vtbl).query_interface)(
        punk_site.cast(),
        &IID_IXAMLDIAGNOSTICS,
        &mut diag_com.0,
    );
    if hr_diag.is_err() || diag_com.is_null() {
        return if hr_diag.is_err() {
            hr_diag
        } else {
            E_NOINTERFACE
        };
    }

    let git = match create_git() {
        Ok(g) => g,
        Err(e) => return e.code(),
    };

    let Some(vts_unk) = IUnknown::from_raw_borrowed(&vts_com.0) else {
        return E_FAIL_HR;
    };
    let vts_cookie = match git.RegisterInterfaceInGlobal(vts_unk, &IID_IVISUALTREESERVICE) {
        Ok(c) => c,
        Err(e) => return e.code(),
    };

    let Some(diag_unk) = IUnknown::from_raw_borrowed(&diag_com.0) else {
        let _ = git.RevokeInterfaceFromGlobal(vts_cookie);
        return E_FAIL_HR;
    };
    let diag_cookie = match git.RegisterInterfaceInGlobal(diag_unk, &IID_IXAMLDIAGNOSTICS) {
        Ok(c) => c,
        Err(e) => {
            let _ = git.RevokeInterfaceFromGlobal(vts_cookie);
            return e.code();
        }
    };

    let site_raw = punk_site.cast::<c_void>();
    let Some(site_iunk) = IUnknown::from_raw_borrowed(&site_raw) else {
        let _ = git.RevokeInterfaceFromGlobal(diag_cookie);
        let _ = git.RevokeInterfaceFromGlobal(vts_cookie);
        return E_FAIL_HR;
    };
    let site_cookie = match git.RegisterInterfaceInGlobal(site_iunk, &IID_IUNKNOWN) {
        Ok(c) => c,
        Err(e) => {
            let _ = git.RevokeInterfaceFromGlobal(diag_cookie);
            let _ = git.RevokeInterfaceFromGlobal(vts_cookie);
            return e.code();
        }
    };

    let Ok(runtime) = TapRuntime::new(diag_cookie) else {
        let _ = git.RevokeInterfaceFromGlobal(site_cookie);
        let _ = git.RevokeInterfaceFromGlobal(diag_cookie);
        let _ = git.RevokeInterfaceFromGlobal(vts_cookie);
        return E_FAIL_HR;
    };

    if !DLL_PINNED.load(Ordering::Acquire) {
        let mut hmod = HMODULE::default();
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR((DllMain as *const () as usize) as *const u16),
            &mut hmod,
        )
        .is_ok()
        {
            DLL_PINNED.store(true, Ordering::Release);
        }
    }

    if let Ok(mut rt_guard) = site.runtime.lock() {
        *rt_guard = Some(Arc::clone(&runtime));
    }

    let rt_advisor = Arc::clone(&runtime);
    let rt_pipe = Arc::clone(&runtime);

    ACTIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
    if std::thread::Builder::new()
        .spawn(move || {
            run_advisor_thread(rt_advisor, vts_cookie);
            ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
        })
        .is_err()
    {
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
        if let Ok(mut rt_guard) = site.runtime.lock() {
            *rt_guard = None;
        }
        let _ = git.RevokeInterfaceFromGlobal(site_cookie);
        let _ = git.RevokeInterfaceFromGlobal(diag_cookie);
        let _ = git.RevokeInterfaceFromGlobal(vts_cookie);
        return E_FAIL_HR;
    }

    ACTIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
    if std::thread::Builder::new()
        .spawn(move || {
            if let Some(reverse_pipe) = rt_pipe.reverse_pipe_name.clone() {
                run_reverse_pipe_client_loop(rt_pipe, reverse_pipe);
            } else {
                run_pipe_server_loop(rt_pipe);
            }
            ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
        })
        .is_err()
    {
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
        runtime.stop_requested.store(true, Ordering::SeqCst);
        let _ = SetEvent(runtime.stop_event_handle());
        if let Ok(mut rt_guard) = site.runtime.lock() {
            *rt_guard = None;
        }
        let _ = git.RevokeInterfaceFromGlobal(site_cookie);
        return E_FAIL_HR;
    }

    let old_cookie = site
        .site_cookie
        .lock()
        .ok()
        .map(|mut sc| std::mem::replace(&mut *sc, site_cookie))
        .unwrap_or(0);
    if old_cookie != 0 {
        let _ = git.RevokeInterfaceFromGlobal(old_cookie);
    }

    S_OK
}

unsafe extern "system" fn site_get_site(
    this: *mut TapSite,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    let site = &*this;
    let cookie = site.site_cookie.lock().map(|g| *g).unwrap_or(0);
    if cookie == 0 {
        return E_FAIL_HR;
    }
    let Ok(git) = create_git() else {
        return E_FAIL_HR;
    };
    match git.GetInterfaceFromGlobal(cookie, riid, ppv) {
        Ok(()) => S_OK,
        Err(e) => e.code(),
    }
}

// ---------- COM ClassFactory & DLL Exports ----------

#[repr(C)]
struct RawClassFactoryVtbl {
    query_interface: unsafe extern "system" fn(
        *mut TapClassFactory,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut TapClassFactory) -> u32,
    release: unsafe extern "system" fn(*mut TapClassFactory) -> u32,
    create_instance: unsafe extern "system" fn(
        *mut TapClassFactory,
        *mut RawIUnknown,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    lock_server: unsafe extern "system" fn(*mut TapClassFactory, BOOL) -> HRESULT,
}

static FACTORY_VTBL: RawClassFactoryVtbl = RawClassFactoryVtbl {
    query_interface: factory_qi,
    add_ref: factory_add_ref,
    release: factory_release,
    create_instance: factory_create_instance,
    lock_server: factory_lock_server,
};

#[repr(C)]
struct TapClassFactory {
    lp_vtbl: *const RawClassFactoryVtbl,
    ref_count: AtomicU32,
}

impl TapClassFactory {
    fn new_com_ptr() -> *mut Self {
        ACTIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
        Box::into_raw(Box::new(Self {
            lp_vtbl: &FACTORY_VTBL,
            ref_count: AtomicU32::new(1),
        }))
    }
}

unsafe extern "system" fn factory_qi(
    this: *mut TapClassFactory,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    let iid = *riid;
    if iid == IID_IUNKNOWN || iid == IID_ICLASSFACTORY {
        *ppv = this.cast();
        factory_add_ref(this);
        S_OK
    } else {
        E_NOINTERFACE
    }
}

unsafe extern "system" fn factory_add_ref(this: *mut TapClassFactory) -> u32 {
    (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn factory_release(this: *mut TapClassFactory) -> u32 {
    let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this));
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
    prev - 1
}

unsafe extern "system" fn factory_create_instance(
    _this: *mut TapClassFactory,
    punk_outer: *mut RawIUnknown,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if !punk_outer.is_null() {
        return HRESULT(0x80040110u32 as i32); // CLASS_E_NOAGGREGATION
    }
    let site = TapSite::new_com_ptr();
    let hr = site_qi(site, riid, ppv);
    site_release(site);
    hr
}

unsafe extern "system" fn factory_lock_server(
    _this: *mut TapClassFactory,
    f_lock: BOOL,
) -> HRESULT {
    if f_lock.as_bool() {
        ACTIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
    } else {
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(
    _hinst: HINSTANCE,
    _reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    TRUE
}

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || rclsid.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if *rclsid != CLSID_PRETTY_WINDOWS_TAP {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory = TapClassFactory::new_com_ptr();
    let hr = factory_qi(factory, riid, ppv);
    factory_release(factory);
    hr
}

#[no_mangle]
pub unsafe extern "system" fn DllCanUnloadNow() -> HRESULT {
    if DLL_PINNED.load(Ordering::Acquire) || ACTIVE_OBJECTS.load(Ordering::SeqCst) != 0 {
        S_FALSE
    } else {
        S_OK
    }
}

// ---------- Advisor Thread (Owns VisualTreeWatcher registration & final cleanup) ----------

fn run_advisor_thread(rt: Arc<TapRuntime>, vts_cookie: u32) {
    unsafe {
        let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let git = create_git().ok();
        let mut vts_com = ComRaw::null();
        if let Some(ref g) = git {
            let _ = g.GetInterfaceFromGlobal(vts_cookie, &IID_IVISUALTREESERVICE, &mut vts_com.0);
        }

        if !vts_com.is_null() {
            let watcher = VisualTreeWatcher::new_com_ptr(Arc::clone(&rt));
            let vts = vts_com.as_ptr().cast::<RawIVisualTreeService>();
            let hr_adv =
                ((*(*vts).lp_vtbl).advise_visual_tree_change)(vts_com.as_ptr(), watcher.cast());
            rt.advise_hr.store(hr_adv.0, Ordering::Release);

            let advised = hr_adv.is_ok()
                || (rt.diag_flags.load(Ordering::Acquire) & DIAG_WATCHER_CALLED) != 0;
            if advised {
                let _ = WaitForSingleObject(rt.stop_event_handle(), INFINITE);
                let _ = ((*(*vts).lp_vtbl).unadvise_visual_tree_change)(
                    vts_com.as_ptr(),
                    watcher.cast(),
                );
            } else {
                // AdviseVisualTreeChange failed (e.g. another TAP instance is already
                // attached to this XAML host): stop immediately so this instance never
                // hijacks the reverse Named Pipe.
                rt.stop_requested.store(true, Ordering::SeqCst);
                let _ = SetEvent(rt.stop_event_handle());
            }
            watcher_release(watcher);
        } else {
            rt.advise_hr.store(E_FAIL_HR.0, Ordering::Release);
            rt.stop_requested.store(true, Ordering::SeqCst);
            let _ = SetEvent(rt.stop_event_handle());
        }

        drop(vts_com);

        let (leftover, leftover_tooltip_dispatchers, leftover_menu_dispatchers): (
            Vec<TrackedTarget>,
            Vec<OwnedDispatcher>,
            Vec<OwnedDispatcher>,
        ) = if let Ok(mut g) = rt.state.lock() {
            g.shutdown = true;
            (
                g.targets.drain().map(|(_, t)| t).collect(),
                g.tooltip_dispatchers
                    .drain(..)
                    .map(|(_, disp)| disp)
                    .collect(),
                g.menu_flyout_dispatchers
                    .drain(..)
                    .map(|(_, disp)| disp)
                    .collect(),
            )
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };
        for t in leftover {
            release_target_resources(t, git.as_ref());
        }
        for disp in leftover_tooltip_dispatchers {
            release_owned_dispatcher(disp, git.as_ref());
        }
        for disp in leftover_menu_dispatchers {
            release_owned_dispatcher(disp, git.as_ref());
        }

        let diag_cookie = rt.diag_cookie.swap(0, Ordering::AcqRel);
        if let Some(ref g) = git {
            if diag_cookie != 0 {
                let _ = g.RevokeInterfaceFromGlobal(diag_cookie);
            }
            if vts_cookie != 0 {
                let _ = g.RevokeInterfaceFromGlobal(vts_cookie);
            }
        }
        drop(git);

        if let Ok(mut done) = rt.advisor_finished.lock() {
            *done = true;
            rt.advisor_cv.notify_all();
        }

        if com_ok {
            CoUninitialize();
        }
    }
}

