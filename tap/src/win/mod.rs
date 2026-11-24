mod appearance;
mod brush_store;
mod com_objects;
mod pipe;
mod raw_com;

use super::tree_match::{appcontainer_pipe_name_for_pid, search_host_pipe_name_for_pid, shell_host_pipe_name_for_pid, start_menu_pipe_name_for_pid, TapHostScope, TargetKind, VisualTreeIndex, MAX_MENU_FLYOUT_RECTS, MAX_SHELL_CARD_RECTS};
use std::collections::{HashMap};
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::core::{IUnknown, Interface};
use windows::System::{DispatcherQueue, DispatcherQueueHandler};
use windows::Win32::Foundation::{CloseHandle, HANDLE, S_FALSE, WAIT_OBJECT_0};
use windows::Win32::Security::{GetTokenInformation, TokenIsAppContainer, TOKEN_QUERY};
use windows::Win32::System::Com::{IGlobalInterfaceTable};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, OpenProcessToken, SetEvent, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use appearance::{
    apply_shape_appearance, apply_target_appearance, measure_element_bounds_dip,
    measure_hover_flyout_bounds_dip, measure_input_switcher_bounds_dip,
    measure_menu_flyout_bounds_dip, measure_tooltip_bounds_dip,
    restore_all_alttab_subtargets_for_current_thread,
    restore_all_hover_flyout_shapes_for_current_thread,
    restore_all_menu_flyout_targets_for_current_thread,
    restore_all_shell_card_targets_for_current_thread,
    restore_all_tooltip_targets_for_current_thread, sync_alttab_subtree_items,
    sync_focus_secondary_thickness_prop, sync_hover_flyout_bg_shapes,
};
use brush_store::{
    clear_all_watches_for_current_thread, prewarm_overflow_brush_store_for_current_thread,
    purge_saved_brushes_for_current_thread, UI_BRUSH_STORE,
};
use raw_com::*;

unsafe fn is_current_process_appcontainer() -> bool {
    let mut token = HANDLE::default();
    if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err()
        || token.is_invalid()
    {
        return false;
    }
    let mut is_ac = 0u32;
    let mut ret_len = 0u32;
    let ok = GetTokenInformation(
        token,
        TokenIsAppContainer,
        Some((&mut is_ac as *mut u32).cast()),
        size_of::<u32>() as u32,
        &mut ret_len,
    )
    .is_ok();
    let _ = CloseHandle(token);
    ok && is_ac != 0
}


fn release_owned_dispatcher(
    dispatcher: OwnedDispatcher,
    git: Option<&IGlobalInterfaceTable>,
) {
    match dispatcher {
        OwnedDispatcher::Queue(addr) => {
            if addr != 0 {
                unsafe {
                    let _ = DispatcherQueue::from_raw(addr as *mut c_void);
                }
            }
        }
        OwnedDispatcher::Core(cookie) => {
            revoke_git_cookie(git, cookie);
        }
    }
}

fn release_target_resources(target: TrackedTarget, git: Option<&IGlobalInterfaceTable>) {
    release_raw_com_addr(target.weak_ref_raw);
    release_owned_dispatcher(target.dispatcher, git);
}

struct RuntimeState {
    desired_hide: bool,
    shutdown: bool,
    targets: HashMap<u64, TrackedTarget>,
    tooltip_dispatchers: Vec<(u32, OwnedDispatcher)>,
    menu_flyout_dispatchers: Vec<(u32, OwnedDispatcher)>,
}

unsafe fn detect_host_scope_and_pipe(is_appcontainer: bool) -> (TapHostScope, Option<String>) {
    let pid = GetCurrentProcessId();
    let mut buf = [0u16; 512];
    let len = GetModuleFileNameW(None, &mut buf) as usize;
    if len > 0 {
        let path = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
        let exe_name = path.rsplit(['\\', '/']).next().unwrap_or("");
        if exe_name.eq_ignore_ascii_case("StartMenuExperienceHost.exe") {
            return (
                TapHostScope::StartMenu,
                Some(start_menu_pipe_name_for_pid(pid)),
            );
        }
        if exe_name.eq_ignore_ascii_case("SearchHost.exe") {
            return (
                TapHostScope::SearchHost,
                Some(search_host_pipe_name_for_pid(pid)),
            );
        }
        if exe_name.eq_ignore_ascii_case("ShellHost.exe") {
            return (
                TapHostScope::ShellHost,
                Some(shell_host_pipe_name_for_pid(pid)),
            );
        }
    }
    if is_appcontainer {
        return (
            TapHostScope::ShellExperience,
            Some(appcontainer_pipe_name_for_pid(pid)),
        );
    }
    (TapHostScope::Explorer, None)
}

struct TapRuntime {
    tree: Mutex<VisualTreeIndex>,
    state: Mutex<RuntimeState>,
    is_appcontainer: bool,
    host_scope: TapHostScope,
    reverse_pipe_name: Option<String>,
    alt_tab_bounds_dip: Mutex<Option<(u32, [f32; 4])>>,
    hover_flyout_bounds_dip: Mutex<Option<(u32, [f32; 4])>>,
    osd_bounds_dip: Mutex<Option<(u32, [f32; 4])>>,
    osd_suppressed: AtomicBool,
    osd_fallback_bg_argb: AtomicU32,
    osd_fallback_border_argb: AtomicU32,
    tooltip_bounds_dip: Mutex<Option<(u32, [f32; 4])>>,
    tooltip_suppressed: AtomicBool,
    tooltip_fallback_bg_argb: AtomicU32,
    tooltip_fallback_border_argb: AtomicU32,
    menu_flyout_bounds_by_tid: Mutex<HashMap<u32, Vec<[f32; 4]>>>,
    shell_card_bounds_by_handle: Mutex<HashMap<u64, (u32, [f32; 4])>>,
    menu_flyout_suppressed: AtomicBool,
    menu_flyout_fallback_bg_argb: AtomicU32,
    menu_flyout_fallback_border_argb: AtomicU32,
    start_modal_style: Mutex<(u8, bool, u32, u32)>,
    input_switcher_bounds_dip: Mutex<Option<(u32, [f32; 4])>>,
    input_switcher_suppressed: AtomicBool,
    input_switcher_fallback_bg_argb: AtomicU32,
    input_switcher_fallback_border_argb: AtomicU32,
    alt_tab_item_bg_argb: AtomicU32,
    alt_tab_item_border_argb: AtomicU32,
    virtual_desktop_bar_bg_argb: AtomicU32,
    virtual_desktop_bar_border_argb: AtomicU32,
    diag_cookie: AtomicU32,
    shutdown_started: AtomicBool,
    stop_requested: AtomicBool,
    stop_event_raw: usize,
    host_handle_raw: Mutex<Option<(u32, usize)>>,
    advisor_finished: Mutex<bool>,
    advisor_cv: Condvar,
    advise_hr: AtomicI32,
    last_put_opacity_hr: AtomicI32,
    last_put_visibility_hr: AtomicI32,
    diag_flags: AtomicU32,
}

impl TapRuntime {
    fn new(diag_cookie: u32) -> windows::core::Result<Arc<Self>> {
        let stop_event = unsafe { CreateEventW(None, true, false, None)? };
        let is_appcontainer = unsafe { is_current_process_appcontainer() };
        let (host_scope, reverse_pipe_name) =
            unsafe { detect_host_scope_and_pipe(is_appcontainer) };
        Ok(Arc::new(Self {
            tree: Mutex::new(VisualTreeIndex::new()),
            state: Mutex::new(RuntimeState {
                desired_hide: false,
                shutdown: false,
                targets: HashMap::new(),
                tooltip_dispatchers: Vec::new(),
                menu_flyout_dispatchers: Vec::new(),
            }),
            is_appcontainer,
            host_scope,
            reverse_pipe_name,
            alt_tab_bounds_dip: Mutex::new(None),
            hover_flyout_bounds_dip: Mutex::new(None),
            osd_bounds_dip: Mutex::new(None),
            osd_suppressed: AtomicBool::new(false),
            osd_fallback_bg_argb: AtomicU32::new(0x63_141822),
            osd_fallback_border_argb: AtomicU32::new(0x52_FFFFFF),
            tooltip_bounds_dip: Mutex::new(None),
            tooltip_suppressed: AtomicBool::new(false),
            tooltip_fallback_bg_argb: AtomicU32::new(0x6C_141822),
            tooltip_fallback_border_argb: AtomicU32::new(0x54_FFFFFF),
            menu_flyout_bounds_by_tid: Mutex::new(HashMap::new()),
            shell_card_bounds_by_handle: Mutex::new(HashMap::new()),
            menu_flyout_suppressed: AtomicBool::new(false),
            menu_flyout_fallback_bg_argb: AtomicU32::new(0x70_141822),
            menu_flyout_fallback_border_argb: AtomicU32::new(0x56_FFFFFF),
            start_modal_style: Mutex::new((4, false, 0xC2C2C7, 0xD1D1D6)),
            input_switcher_bounds_dip: Mutex::new(None),
            input_switcher_suppressed: AtomicBool::new(false),
            input_switcher_fallback_bg_argb: AtomicU32::new(0x70_141822),
            input_switcher_fallback_border_argb: AtomicU32::new(0x56_FFFFFF),
            alt_tab_item_bg_argb: AtomicU32::new(0x5B_1A1D24),
            alt_tab_item_border_argb: AtomicU32::new(0x42_FFFFFF),
            virtual_desktop_bar_bg_argb: AtomicU32::new(0x60_141A26),
            virtual_desktop_bar_border_argb: AtomicU32::new(0x58_E6F0FF),
            diag_cookie: AtomicU32::new(diag_cookie),
            shutdown_started: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            stop_event_raw: stop_event.0 as usize,
            host_handle_raw: Mutex::new(None),
            advisor_finished: Mutex::new(false),
            advisor_cv: Condvar::new(),
            advise_hr: AtomicI32::new(S_FALSE.0),
            last_put_opacity_hr: AtomicI32::new(0),
            last_put_visibility_hr: AtomicI32::new(0),
            diag_flags: AtomicU32::new(0),
        }))
    }

    #[inline]
    fn stop_event_handle(&self) -> HANDLE {
        HANDLE(self.stop_event_raw as *mut c_void)
    }

    #[inline]
    fn is_shutdown(&self) -> bool {
        self.stop_requested.load(Ordering::Acquire)
            || self
                .state
                .lock()
                .map(|g| g.shutdown)
                .unwrap_or(true)
    }

    #[inline]
    fn desired_hide(&self) -> bool {
        self.state
            .lock()
            .map(|g| g.desired_hide && !g.shutdown)
            .unwrap_or(false)
    }

    #[inline]
    fn is_osd_suppressed(&self) -> bool {
        self.osd_suppressed.load(Ordering::Acquire)
    }

    #[inline]
    fn is_tooltip_suppressed(&self) -> bool {
        self.tooltip_suppressed.load(Ordering::Acquire)
    }

    #[inline]
    fn is_menu_flyout_suppressed(&self) -> bool {
        self.menu_flyout_suppressed.load(Ordering::Acquire)
    }

    #[inline]
    fn is_input_switcher_suppressed(&self) -> bool {
        self.input_switcher_suppressed.load(Ordering::Acquire)
    }

    fn clear_host_handle(&self) {
        if let Ok(mut hh) = self.host_handle_raw.lock() {
            if let Some((_, raw)) = hh.take() {
                if raw != 0 {
                    unsafe {
                        let _ = CloseHandle(HANDLE(raw as *mut c_void));
                    }
                }
            }
        }
    }

    fn on_host_exited(self: &Arc<Self>) {
        self.clear_host_handle();
        self.osd_suppressed.store(false, Ordering::Release);
        if let Ok(mut b) = self.osd_bounds_dip.lock() {
            *b = None;
        }
        self.tooltip_suppressed.store(false, Ordering::Release);
        if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
            *b = None;
        }
        self.input_switcher_suppressed.store(false, Ordering::Release);
        if let Ok(mut b) = self.input_switcher_bounds_dip.lock() {
            *b = None;
        }
        self.menu_flyout_suppressed.store(false, Ordering::Release);
        if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
            b.clear();
        }
        if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
            b.clear();
        }
        if let Ok(mut g) = self.state.lock() {
            if !g.shutdown {
                g.desired_hide = false;
            }
        }
        self.dispatch_apply_to_ui_threads(250);
    }

    fn set_hide(self: &Arc<Self>, hide: bool, host_pid: u32) {
        if hide && !self.is_appcontainer && host_pid != 0 {
            if let Ok(mut hh) = self.host_handle_raw.lock() {
                let need_open = match *hh {
                    Some((prev_pid, raw)) if prev_pid == host_pid && raw != 0 => unsafe {
                        WaitForSingleObject(HANDLE(raw as *mut c_void), 0) == WAIT_OBJECT_0
                    },
                    _ => true,
                };
                if need_open {
                    if let Some((_, old_raw)) = hh.take() {
                        if old_raw != 0 {
                            unsafe {
                                let _ = CloseHandle(HANDLE(old_raw as *mut c_void));
                            }
                        }
                    }
                    if let Ok(new_h) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, host_pid) }
                    {
                        *hh = Some((host_pid, new_h.0 as usize));
                    }
                }
            }
        } else if !hide {
            self.clear_host_handle();
            self.osd_suppressed.store(false, Ordering::Release);
            if let Ok(mut b) = self.osd_bounds_dip.lock() {
                *b = None;
            }
            self.tooltip_suppressed.store(false, Ordering::Release);
            if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                *b = None;
            }
            self.input_switcher_suppressed.store(false, Ordering::Release);
            if let Ok(mut b) = self.input_switcher_bounds_dip.lock() {
                *b = None;
            }
            self.menu_flyout_suppressed.store(false, Ordering::Release);
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                b.clear();
            }
        }

        if let Ok(mut g) = self.state.lock() {
            if !g.shutdown {
                g.desired_hide = hide;
            }
        }
        let wait_ms = if hide { 0 } else { 150 };
        self.dispatch_apply_to_ui_threads(wait_ms);
    }

    fn trigger_shutdown(self: &Arc<Self>, wait_ms: u64) {
        let first = !self.shutdown_started.swap(true, Ordering::AcqRel);
        if first {
            self.osd_suppressed.store(false, Ordering::Release);
            if let Ok(mut b) = self.osd_bounds_dip.lock() {
                *b = None;
            }
            self.tooltip_suppressed.store(false, Ordering::Release);
            if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                *b = None;
            }
            self.input_switcher_suppressed.store(false, Ordering::Release);
            if let Ok(mut b) = self.input_switcher_bounds_dip.lock() {
                *b = None;
            }
            self.menu_flyout_suppressed.store(false, Ordering::Release);
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            if let Ok(mut g) = self.state.lock() {
                g.desired_hide = false;
                g.shutdown = true;
            }
            let restore_wait = if wait_ms > 0 { wait_ms.min(250) } else { 0 };
            self.dispatch_apply_to_ui_threads(restore_wait);
            self.stop_requested.store(true, Ordering::SeqCst);
            unsafe {
                let _ = SetEvent(self.stop_event_handle());
            }
        }

        if wait_ms > 0 {
            if let Ok(mut done) = self.advisor_finished.lock() {
                let deadline = Instant::now() + Duration::from_millis(wait_ms);
                while !*done {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    if let Ok((next, _)) =
                        self.advisor_cv.wait_timeout(done, deadline - now)
                    {
                        done = next;
                    } else {
                        break;
                    }
                }
            }
        }
    }

    fn enqueue_target_dispatch(
        self: &Arc<Self>,
        target: DispatchTarget,
        git: Option<&IGlobalInterfaceTable>,
        barrier: Option<&Arc<PendingBarrier>>,
    ) {
        match target {
            DispatchTarget::Queue { thread_id, queue } => {
                let rt_cb = Arc::clone(self);
                let barrier_cb = barrier.cloned();
                let handler = DispatcherQueueHandler::new(move || {
                    let _guard = BarrierGuard {
                        barrier: barrier_cb.clone(),
                    };
                    apply_for_ui_thread(&rt_cb, DispatchFilter::QueueTid(thread_id));
                    Ok(())
                });
                match queue.TryEnqueue(&handler) {
                    Ok(true) => {}
                    _ => {
                        if let Some(b) = barrier {
                            b.done_one();
                        }
                    }
                }
            }
            DispatchTarget::Core { cookie } => {
                let mut enqueued = false;
                if let Some(g) = git {
                    unsafe {
                        let mut cd_com = ComRaw::null();
                        if g.GetInterfaceFromGlobal(
                            cookie,
                            &IID_ICOREDISPATCHER,
                            &mut cd_com.0,
                        )
                        .is_ok()
                            && !cd_com.is_null()
                        {
                            let rt_cb = Arc::clone(self);
                            let barrier_cb = barrier.cloned();
                            let handler = CoreDispatchedHandler::new_com_ptr(move || {
                                let _guard = BarrierGuard {
                                    barrier: barrier_cb,
                                };
                                apply_for_ui_thread(
                                    &rt_cb,
                                    DispatchFilter::CoreCookie(cookie),
                                );
                            });
                            let cd = cd_com.as_ptr().cast::<RawICoreDispatcher>();
                            let mut async_act = ComRaw::null();
                            let hr = ((*(*cd).lp_vtbl).run_async)(
                                cd_com.as_ptr(),
                                0, // CoreDispatcherPriority_Normal
                                handler.cast(),
                                &mut async_act.0,
                            );
                            dispatched_release(handler);
                            enqueued = hr.is_ok();
                        }
                    }
                }
                if !enqueued {
                    if let Some(b) = barrier {
                        b.done_one();
                    }
                }
            }
        }
    }

    fn dispatch_apply_to_ui_threads(self: &Arc<Self>, wait_ms: u64) {
        self.dispatch_apply_filtered(wait_ms, None);
    }

    fn dispatch_apply_filtered(
        self: &Arc<Self>,
        wait_ms: u64,
        only_kind: Option<TargetKind>,
    ) -> usize {
        let mut seen_keys: Vec<DispatchKey> = Vec::new();
        let mut dispatch_list: Vec<DispatchTarget> = Vec::new();
        let push_unique = |seen_keys: &mut Vec<DispatchKey>,
                           dispatch_list: &mut Vec<DispatchTarget>,
                           thread_id: Option<u32>,
                           dispatcher: OwnedDispatcher| {
            match (thread_id, dispatcher) {
                (Some(tid), OwnedDispatcher::Queue(addr)) if addr != 0 => {
                    let key = DispatchKey::ThreadQueue(tid);
                    if !seen_keys.contains(&key) {
                        unsafe {
                            let raw_ptr = addr as *mut c_void;
                            if let Some(borrowed) =
                                DispatcherQueue::from_raw_borrowed(&raw_ptr)
                            {
                                seen_keys.push(key);
                                dispatch_list.push(DispatchTarget::Queue {
                                    thread_id: tid,
                                    queue: borrowed.clone(),
                                });
                            }
                        }
                    }
                }
                (_, OwnedDispatcher::Core(cookie)) if cookie != 0 => {
                    let key = DispatchKey::CoreCookie(cookie);
                    if !seen_keys.contains(&key) {
                        seen_keys.push(key);
                        dispatch_list.push(DispatchTarget::Core { cookie });
                    }
                }
                _ => {}
            }
        };

        if let Ok(g) = self.state.lock() {
            for t in g.targets.values() {
                if only_kind.is_some_and(|k| t.kind != k) {
                    continue;
                }
                push_unique(&mut seen_keys, &mut dispatch_list, t.thread_id, t.dispatcher);
            }
            if only_kind.is_none() {
                for &(tid, disp) in &g.tooltip_dispatchers {
                    push_unique(&mut seen_keys, &mut dispatch_list, Some(tid), disp);
                }
                for &(tid, disp) in &g.menu_flyout_dispatchers {
                    push_unique(&mut seen_keys, &mut dispatch_list, Some(tid), disp);
                }
            }
        }

        let count = dispatch_list.len();
        if count == 0 {
            return 0;
        }

        let barrier = (wait_ms > 0).then(|| PendingBarrier::new(count));
        let git = if dispatch_list
            .iter()
            .any(|d| matches!(d, DispatchTarget::Core { .. }))
        {
            unsafe { create_git().ok() }
        } else {
            None
        };

        for target in dispatch_list {
            self.enqueue_target_dispatch(target, git.as_ref(), barrier.as_ref());
        }

        if let Some(b) = barrier {
            b.wait_timeout(Duration::from_millis(wait_ms));
        }
        count
    }

    fn query_alt_tab_bounds_dip(self: &Arc<Self>, wait_ms: u64) -> Option<(u32, [f32; 4])> {
        let dispatched = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::AltTabBorder));
        if dispatched == 0 {
            if let Ok(mut b) = self.alt_tab_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.alt_tab_bounds_dip.lock().ok().and_then(|b| *b)
    }

    fn query_hover_flyout_bounds_dip(
        self: &Arc<Self>,
        wait_ms: u64,
    ) -> Option<(u32, [f32; 4])> {
        let dispatched =
            self.dispatch_apply_filtered(wait_ms, Some(TargetKind::HoverFlyoutBorder));
        if dispatched == 0 {
            if let Ok(mut b) = self.hover_flyout_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.hover_flyout_bounds_dip.lock().ok().and_then(|b| *b)
    }

    fn query_osd_bounds_dip(self: &Arc<Self>, wait_ms: u64) -> Option<(u32, [f32; 4])> {
        if self.is_osd_suppressed() {
            if let Ok(mut b) = self.osd_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        let dispatched = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::OsdPanel));
        if dispatched == 0 {
            if let Ok(mut b) = self.osd_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.osd_bounds_dip.lock().ok().and_then(|b| *b)
    }

    #[inline]
    fn osd_target_colors(&self) -> (RawUiColor, RawUiColor) {
        if self.is_osd_suppressed() {
            (
                RawUiColor::from_argb(self.osd_fallback_bg_argb.load(Ordering::Acquire)),
                RawUiColor::from_argb(self.osd_fallback_border_argb.load(Ordering::Acquire)),
            )
        } else {
            (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
        }
    }

    fn set_osd_suppressed(self: &Arc<Self>, suppressed: bool) {
        let prev = self.osd_suppressed.swap(suppressed, Ordering::AcqRel);
        if suppressed {
            if let Ok(mut b) = self.osd_bounds_dip.lock() {
                *b = None;
            }
        }
        if prev != suppressed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::OsdPanel));
        }
    }

    fn set_osd_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self.osd_fallback_bg_argb.swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .osd_fallback_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb)
            && self.desired_hide()
            && self.is_osd_suppressed()
        {
            self.dispatch_apply_filtered(0, Some(TargetKind::OsdPanel));
        }
    }

    fn query_input_switcher_bounds_dip(
        self: &Arc<Self>,
        wait_ms: u64,
    ) -> Option<(u32, [f32; 4])> {
        if self.is_input_switcher_suppressed() {
            if let Ok(mut b) = self.input_switcher_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        let dispatched =
            self.dispatch_apply_filtered(wait_ms, Some(TargetKind::InputSwitcherPanel));
        if dispatched == 0 {
            if let Ok(mut b) = self.input_switcher_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.input_switcher_bounds_dip.lock().ok().and_then(|b| *b)
    }

    #[inline]
    fn input_switcher_target_colors(&self) -> (RawUiColor, RawUiColor) {
        if self.is_input_switcher_suppressed() {
            (
                RawUiColor::from_argb(
                    self.input_switcher_fallback_bg_argb.load(Ordering::Acquire),
                ),
                RawUiColor::from_argb(
                    self.input_switcher_fallback_border_argb
                        .load(Ordering::Acquire),
                ),
            )
        } else {
            (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
        }
    }

    fn set_input_switcher_suppressed(self: &Arc<Self>, suppressed: bool) {
        let prev = self
            .input_switcher_suppressed
            .swap(suppressed, Ordering::AcqRel);
        if suppressed {
            if let Ok(mut b) = self.input_switcher_bounds_dip.lock() {
                *b = None;
            }
        }
        if prev != suppressed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::InputSwitcherPanel));
        }
    }

    fn set_input_switcher_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self
            .input_switcher_fallback_bg_argb
            .swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .input_switcher_fallback_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb)
            && self.desired_hide()
            && self.is_input_switcher_suppressed()
        {
            self.dispatch_apply_filtered(0, Some(TargetKind::InputSwitcherPanel));
        }
    }

    fn query_tooltip_bounds_dip(self: &Arc<Self>, wait_ms: u64) -> Option<(u32, [f32; 4])> {
        if self.is_tooltip_suppressed() {
            if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        let dispatched = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::ToolTipPresenter));
        if dispatched == 0 {
            if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                *b = None;
            }
            return None;
        }
        self.tooltip_bounds_dip.lock().ok().and_then(|b| *b)
    }

    #[inline]
    fn tooltip_target_colors(&self) -> (RawUiColor, RawUiColor) {
        if self.is_tooltip_suppressed() {
            (
                RawUiColor::from_argb(self.tooltip_fallback_bg_argb.load(Ordering::Acquire)),
                RawUiColor::from_argb(self.tooltip_fallback_border_argb.load(Ordering::Acquire)),
            )
        } else {
            (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
        }
    }

    fn set_tooltip_suppressed(self: &Arc<Self>, suppressed: bool) {
        let prev = self.tooltip_suppressed.swap(suppressed, Ordering::AcqRel);
        if suppressed {
            if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                *b = None;
            }
        }
        if prev != suppressed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::ToolTipPresenter));
        }
    }

    fn set_tooltip_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self.tooltip_fallback_bg_argb.swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .tooltip_fallback_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb)
            && self.desired_hide()
            && self.is_tooltip_suppressed()
        {
            self.dispatch_apply_filtered(0, Some(TargetKind::ToolTipPresenter));
        }
    }

    fn query_menu_flyout_bounds_dip(
        self: &Arc<Self>,
        wait_ms: u64,
    ) -> Vec<(u32, [f32; 4])> {
        if self.is_menu_flyout_suppressed() {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            return Vec::new();
        }
        let dispatched =
            self.dispatch_apply_filtered(wait_ms, Some(TargetKind::MenuFlyoutSurface));
        if dispatched == 0 {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            return Vec::new();
        }
        let (active_tids, has_core_menu) = if let Ok(g) = self.state.lock() {
            let mut tids = Vec::new();
            let mut has_core = false;
            for t in g.targets.values() {
                if t.kind == TargetKind::MenuFlyoutSurface {
                    if let Some(tid) = t.thread_id {
                        if !tids.contains(&tid) {
                            tids.push(tid);
                        }
                    } else {
                        has_core = true;
                    }
                }
            }
            (tids, has_core)
        } else {
            (Vec::new(), true)
        };
        let mut out = Vec::new();
        if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
            if !has_core_menu {
                b.retain(|tid, _| active_tids.contains(tid));
            }
            for (&tid, rects) in b.iter() {
                for &r in rects {
                    out.push((tid, r));
                }
            }
        }
        out.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1[1].total_cmp(&b.1[1]))
                .then_with(|| a.1[0].total_cmp(&b.1[0]))
        });
        out.truncate(MAX_MENU_FLYOUT_RECTS);
        out
    }

    fn query_shell_card_bounds_dip(
        self: &Arc<Self>,
        wait_ms: u64,
    ) -> Vec<(u32, [f32; 4])> {
        if self.is_menu_flyout_suppressed() {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                b.clear();
            }
            return Vec::new();
        }

        let (menu_tids, shell_handles, has_core_menu) =
            if let Ok(g) = self.state.lock() {
                let mut m_tids = Vec::new();
                let mut s_handles = Vec::new();
                let mut core_menu = false;
                for (&h, t) in &g.targets {
                    match t.kind {
                        TargetKind::MenuFlyoutSurface => {
                            if let Some(tid) = t.thread_id {
                                if !m_tids.contains(&tid) {
                                    m_tids.push(tid);
                                }
                            } else {
                                core_menu = true;
                            }
                        }
                        TargetKind::ShellCardSurface => {
                            s_handles.push(h);
                        }
                        _ => {}
                    }
                }
                (m_tids, s_handles, core_menu)
            } else {
                (Vec::new(), Vec::new(), false)
            };

        let need_menu_dispatch = has_core_menu || !menu_tids.is_empty();
        let need_shell_dispatch = !shell_handles.is_empty();
        if !need_menu_dispatch && !need_shell_dispatch {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                b.clear();
            }
            return Vec::new();
        }

        if need_menu_dispatch {
            let _ = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::MenuFlyoutSurface));
        }
        if need_shell_dispatch {
            let _ = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::ShellCardSurface));
        }

        let mut out: Vec<(u32, [f32; 4])> = Vec::new();
        if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
            if !need_menu_dispatch {
                b.clear();
            } else {
                if !has_core_menu {
                    b.retain(|tid, _| menu_tids.contains(tid));
                }
                for (&tid, rects) in b.iter() {
                    for &r in rects {
                        out.push((tid, r));
                    }
                }
            }
        }
        if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
            if !need_shell_dispatch {
                b.clear();
            } else {
                b.retain(|h, _| shell_handles.contains(h));
                for &(tid, r) in b.values() {
                    let is_dup = out.iter().any(|&(pt, prev)| {
                        pt == tid
                            && (prev[0] - r[0]).abs() <= 4.0
                            && (prev[1] - r[1]).abs() <= 4.0
                            && (prev[2] - r[2]).abs() <= 4.0
                            && (prev[3] - r[3]).abs() <= 4.0
                    });
                    if !is_dup {
                        out.push((tid, r));
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1[1].total_cmp(&b.1[1]))
                .then_with(|| a.1[0].total_cmp(&b.1[0]))
                .then_with(|| a.1[3].total_cmp(&b.1[3]))
                .then_with(|| a.1[2].total_cmp(&b.1[2]))
        });
        out.truncate(MAX_SHELL_CARD_RECTS);
        out
    }

    #[inline]
    fn start_modal_style_spec(&self) -> (u8, bool, u32, u32) {
        self.start_modal_style
            .lock()
            .map(|g| *g)
            .unwrap_or((4, false, 0xC2C2C7, 0xD1D1D6))
    }

    fn set_start_modal_style(
        self: &Arc<Self>,
        style_id: u8,
        light_glass: bool,
        halo0_rgb: u32,
        halo1_rgb: u32,
    ) {
        let next = (style_id, light_glass, halo0_rgb, halo1_rgb);
        let changed = self
            .start_modal_style
            .lock()
            .map(|mut g| {
                let prev = *g;
                *g = next;
                prev != next
            })
            .unwrap_or(false);
        if changed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::StartMenuModalSurface));
        }
    }

    #[inline]
    fn menu_flyout_target_colors(&self) -> (RawUiColor, RawUiColor) {
        if self.is_menu_flyout_suppressed() {
            (
                RawUiColor::from_argb(
                    self.menu_flyout_fallback_bg_argb.load(Ordering::Acquire),
                ),
                RawUiColor::from_argb(
                    self.menu_flyout_fallback_border_argb
                        .load(Ordering::Acquire),
                ),
            )
        } else {
            (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
        }
    }

    fn set_menu_flyout_suppressed(self: &Arc<Self>, suppressed: bool) {
        let prev = self.menu_flyout_suppressed.swap(suppressed, Ordering::AcqRel);
        if suppressed {
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                b.clear();
            }
            if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                b.clear();
            }
        }
        if prev != suppressed && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::MenuFlyoutSurface));
            self.dispatch_apply_filtered(0, Some(TargetKind::ShellCardSurface));
        }
    }

    fn set_menu_flyout_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self
            .menu_flyout_fallback_bg_argb
            .swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .menu_flyout_fallback_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb)
            && self.desired_hide()
            && self.is_menu_flyout_suppressed()
        {
            self.dispatch_apply_filtered(0, Some(TargetKind::MenuFlyoutSurface));
            self.dispatch_apply_filtered(0, Some(TargetKind::ShellCardSurface));
        }
    }

    fn set_alt_tab_item_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self.alt_tab_item_bg_argb.swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .alt_tab_item_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb) && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::AltTabItemBorder));
            self.dispatch_apply_filtered(0, Some(TargetKind::AltTabBorder));
        }
    }

    fn virtual_desktop_bar_target_colors(&self) -> (RawUiColor, RawUiColor) {
        (
            RawUiColor::from_argb(self.virtual_desktop_bar_bg_argb.load(Ordering::Acquire)),
            RawUiColor::from_argb(
                self.virtual_desktop_bar_border_argb.load(Ordering::Acquire),
            ),
        )
    }

    fn set_virtual_desktop_bar_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
        let prev_bg = self
            .virtual_desktop_bar_bg_argb
            .swap(bg_argb, Ordering::AcqRel);
        let prev_bd = self
            .virtual_desktop_bar_border_argb
            .swap(border_argb, Ordering::AcqRel);
        if (prev_bg != bg_argb || prev_bd != border_argb) && self.desired_hide() {
            self.dispatch_apply_filtered(0, Some(TargetKind::VirtualDesktopBarSurface));
        }
    }
}

impl Drop for TapRuntime {
    fn drop(&mut self) {
        unsafe {
            let se = self.stop_event_handle();
            if !se.is_invalid() {
                let _ = CloseHandle(se);
            }
        }
        self.clear_host_handle();
    }
}

/// Runs strictly on an owning XAML UI thread via `DispatcherQueue` or `ICoreDispatcher`.
fn apply_for_ui_thread(rt: &Arc<TapRuntime>, filter: DispatchFilter) {
    struct SnapshotItem {
        handle: u64,
        weak_ref_raw: usize,
        kind: TargetKind,
        orig_opacity: f64,
        orig_visibility: i32,
    }

    let (desired_hide, items): (bool, Vec<SnapshotItem>) = {
        let Ok(g) = rt.state.lock() else {
            return;
        };
        let target_hide = g.desired_hide && !g.shutdown;
        let list = g
            .targets
            .iter()
            .filter(|(_, t)| match (filter, t.thread_id, t.dispatcher) {
                (DispatchFilter::QueueTid(tid), Some(t_tid), OwnedDispatcher::Queue(_)) => {
                    t_tid == tid
                }
                (DispatchFilter::CoreCookie(c), _, OwnedDispatcher::Core(t_c)) => t_c == c,
                _ => false,
            })
            .map(|(&handle, t)| {
                add_ref_raw_com_addr(t.weak_ref_raw);
                SnapshotItem {
                    handle,
                    weak_ref_raw: t.weak_ref_raw,
                    kind: t.kind,
                    orig_opacity: t.orig_opacity,
                    orig_visibility: t.orig_visibility,
                }
            })
            .collect();
        (target_hide, list)
    };

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
        let weak_com = ComRaw(item.weak_ref_raw as *mut c_void);
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
        purge_saved_brushes_for_current_thread(&stale_handles);
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
unsafe fn on_target_added(rt: &Arc<TapRuntime>, handle: u64, kind: TargetKind) {
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

    let cur_tid = GetCurrentThreadId();
    let mut tooltip_extra_disp: Option<OwnedDispatcher> = None;
    let mut menu_flyout_extra_disp: Option<OwnedDispatcher> = None;
    let (thread_id, dispatcher, async_dispatch) =
        if let Ok(dq) = DispatcherQueue::GetForCurrentThread() {
            let dq_for_enqueue = dq.clone();
            if kind == TargetKind::ToolTipPresenter {
                tooltip_extra_disp =
                    Some(OwnedDispatcher::Queue(dq.clone().into_raw() as usize));
            } else if matches!(
                kind,
                TargetKind::MenuFlyoutSurface | TargetKind::StartMenuModalSurface
            ) {
                menu_flyout_extra_disp =
                    Some(OwnedDispatcher::Queue(dq.clone().into_raw() as usize));
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
            if kind == TargetKind::ToolTipPresenter {
                if let Ok(extra_cookie) =
                    git.RegisterInterfaceInGlobal(cd_unk, &IID_ICOREDISPATCHER)
                {
                    tooltip_extra_disp = Some(OwnedDispatcher::Core(extra_cookie));
                }
            } else if matches!(
                kind,
                TargetKind::MenuFlyoutSurface
                    | TargetKind::ShellCardSurface
                    | TargetKind::ShellCardSubSurface
                    | TargetKind::StartMenuModalSurface
            ) {
                if let Ok(extra_cookie) =
                    git.RegisterInterfaceInGlobal(cd_unk, &IID_ICOREDISPATCHER)
                {
                    menu_flyout_extra_disp = Some(OwnedDispatcher::Core(extra_cookie));
                }
            }
            (
                None,
                OwnedDispatcher::Core(cookie),
                DispatchTarget::Core { cookie },
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
            if let Some(extra) = tooltip_extra_disp.take() {
                if !g.tooltip_dispatchers.iter().any(|(tid, _)| *tid == cur_tid) {
                    g.tooltip_dispatchers.push((cur_tid, extra));
                } else {
                    tooltip_extra_disp = Some(extra);
                }
            }
            if let Some(extra) = menu_flyout_extra_disp.take() {
                if !g.menu_flyout_dispatchers.iter().any(|(tid, _)| *tid == cur_tid) {
                    g.menu_flyout_dispatchers.push((cur_tid, extra));
                } else {
                    menu_flyout_extra_disp = Some(extra);
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

    if let Some(unused_extra) = tooltip_extra_disp {
        release_owned_dispatcher(unused_extra, Some(&git));
    }
    if let Some(unused_extra) = menu_flyout_extra_disp {
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

