mod appearance;
mod brush_store;
mod com_objects;
mod pipe;
mod queries;
mod raw_com;
mod ui_apply;

use super::tree_match::{
    appcontainer_pipe_name_for_pid, search_host_pipe_name_for_pid, shell_host_pipe_name_for_pid,
    start_menu_pipe_name_for_pid, TapHostScope, TargetKind, VisualTreeIndex,
};
use raw_com::*;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use ui_apply::{apply_for_ui_thread, on_target_added};
use windows::core::Interface;
use windows::System::{DispatcherQueue, DispatcherQueueHandler};
use windows::Win32::Foundation::{CloseHandle, HANDLE, S_FALSE, WAIT_OBJECT_0};
use windows::Win32::Security::{GetTokenInformation, TokenIsAppContainer, TOKEN_QUERY};
use windows::Win32::System::Com::IGlobalInterfaceTable;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, SetEvent,
    WaitForSingleObject, PROCESS_SYNCHRONIZE,
};

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
    pending_restores: Vec<(u64, TrackedTarget)>,
    ui_dispatchers: Vec<(u32, OwnedDispatcher)>,
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
                pending_restores: Vec::new(),
                ui_dispatchers: Vec::new(),
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

    fn prune_dead_ui_dispatcher(&self, dead_tid: u32, git: Option<&IGlobalInterfaceTable>) {
        let mut evicted = Vec::new();
        if let Ok(mut g) = self.state.lock() {
            let mut i = 0usize;
            while i < g.ui_dispatchers.len() {
                if g.ui_dispatchers[i].0 == dead_tid {
                    evicted.push(g.ui_dispatchers.swap_remove(i).1);
                } else {
                    i += 1;
                }
            }
        }
        for disp in evicted {
            release_owned_dispatcher(disp, git);
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
                        self.prune_dead_ui_dispatcher(thread_id, git);
                        if let Some(b) = barrier {
                            b.done_one();
                        }
                    }
                }
            }
            DispatchTarget::Core { thread_id, cookie } => {
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
                                    DispatchFilter::CoreTid(thread_id),
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
                    self.prune_dead_ui_dispatcher(thread_id, git);
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
                (Some(tid), OwnedDispatcher::Core(cookie)) if cookie != 0 => {
                    let key = DispatchKey::ThreadCore(tid);
                    if !seen_keys.contains(&key) {
                        seen_keys.push(key);
                        dispatch_list.push(DispatchTarget::Core {
                            thread_id: tid,
                            cookie,
                        });
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
                for (_, t) in &g.pending_restores {
                    push_unique(&mut seen_keys, &mut dispatch_list, t.thread_id, t.dispatcher);
                }
                for &(tid, disp) in &g.ui_dispatchers {
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

    fn dispatch_apply_for_thread(self: &Arc<Self>, target_tid: u32) {
        let mut seen_keys: Vec<DispatchKey> = Vec::new();
        let mut dispatch_list: Vec<DispatchTarget> = Vec::new();
        let push_for_tid = |seen_keys: &mut Vec<DispatchKey>,
                            dispatch_list: &mut Vec<DispatchTarget>,
                            thread_id: Option<u32>,
                            dispatcher: OwnedDispatcher| {
            if thread_id != Some(target_tid) {
                return;
            }
            match dispatcher {
                OwnedDispatcher::Queue(addr) if addr != 0 => {
                    let key = DispatchKey::ThreadQueue(target_tid);
                    if !seen_keys.contains(&key) {
                        unsafe {
                            let raw_ptr = addr as *mut c_void;
                            if let Some(borrowed) = DispatcherQueue::from_raw_borrowed(&raw_ptr) {
                                seen_keys.push(key);
                                dispatch_list.push(DispatchTarget::Queue {
                                    thread_id: target_tid,
                                    queue: borrowed.clone(),
                                });
                            }
                        }
                    }
                }
                OwnedDispatcher::Core(cookie) if cookie != 0 => {
                    let key = DispatchKey::ThreadCore(target_tid);
                    if !seen_keys.contains(&key) {
                        seen_keys.push(key);
                        dispatch_list.push(DispatchTarget::Core {
                            thread_id: target_tid,
                            cookie,
                        });
                    }
                }
                _ => {}
            }
        };

        if let Ok(g) = self.state.lock() {
            for t in g.targets.values() {
                push_for_tid(&mut seen_keys, &mut dispatch_list, t.thread_id, t.dispatcher);
            }
            for (_, t) in &g.pending_restores {
                push_for_tid(&mut seen_keys, &mut dispatch_list, t.thread_id, t.dispatcher);
            }
            for &(tid, disp) in &g.ui_dispatchers {
                push_for_tid(&mut seen_keys, &mut dispatch_list, Some(tid), disp);
            }
        }
        if dispatch_list.is_empty() {
            return;
        }
        let git = if dispatch_list
            .iter()
            .any(|d| matches!(d, DispatchTarget::Core { .. }))
        {
            unsafe { create_git().ok() }
        } else {
            None
        };
        for target in dispatch_list {
            self.enqueue_target_dispatch(target, git.as_ref(), None);
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
