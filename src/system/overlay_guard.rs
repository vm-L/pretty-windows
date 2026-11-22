use super::window_state::{ensure_window_above_taskbar, is_fullscreen_occluding_taskbar, is_taskbar_above_window, is_window_above, window_band};
use std::cell::RefCell;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering as WinOrdering};
use windows::core::w;
use windows::Win32::Foundation::{BOOL, HMODULE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowW, GetClassNameW, GetTopWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible, PostMessageW, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_FOCUS, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_CAPTUREEND, EVENT_SYSTEM_CAPTURESTART, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GWL_EXSTYLE, GW_HWNDNEXT, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WS_EX_TOPMOST};

#[derive(Default)]
struct TaskbarOverlayGuard {
    widgets: Vec<HWND>,
    backdrops: Vec<HWND>,
    in_callback: bool,
    global_fg_hook: HWINEVENTHOOK,
    global_min_hook: HWINEVENTHOOK,
    taskbar_capture_hook: HWINEVENTHOOK,
    taskbar_reorder_hook: HWINEVENTHOOK,
    bound_tb_pid: u32,
    bound_tb_tid: u32,
}

impl TaskbarOverlayGuard {
    fn unhook_taskbar_specific(&mut self) {
        unsafe {
            if !self.taskbar_capture_hook.is_invalid() {
                let _ = UnhookWinEvent(self.taskbar_capture_hook);
                self.taskbar_capture_hook = HWINEVENTHOOK::default();
            }
            if !self.taskbar_reorder_hook.is_invalid() {
                let _ = UnhookWinEvent(self.taskbar_reorder_hook);
                self.taskbar_reorder_hook = HWINEVENTHOOK::default();
            }
        }
        self.bound_tb_pid = 0;
        self.bound_tb_tid = 0;
    }

    fn unhook_all(&mut self) {
        unsafe {
            if !self.global_fg_hook.is_invalid() {
                let _ = UnhookWinEvent(self.global_fg_hook);
                self.global_fg_hook = HWINEVENTHOOK::default();
            }
            if !self.global_min_hook.is_invalid() {
                let _ = UnhookWinEvent(self.global_min_hook);
                self.global_min_hook = HWINEVENTHOOK::default();
            }
        }
        self.unhook_taskbar_specific();
    }

    fn rebind_taskbar_hooks(&mut self, pid: u32, tid: u32) {
        if self.bound_tb_pid == pid
            && self.bound_tb_tid == tid
            && !self.taskbar_capture_hook.is_invalid()
            && !self.taskbar_reorder_hook.is_invalid()
        {
            return;
        }
        self.unhook_taskbar_specific();
        if pid == 0 || tid == 0 {
            return;
        }
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        unsafe {
            self.taskbar_capture_hook = SetWinEventHook(
                EVENT_SYSTEM_CAPTURESTART,
                EVENT_SYSTEM_CAPTUREEND,
                HMODULE::default(),
                Some(taskbar_zorder_win_event_proc),
                pid,
                tid,
                flags,
            );
            self.taskbar_reorder_hook = SetWinEventHook(
                EVENT_OBJECT_REORDER,
                EVENT_OBJECT_FOCUS,
                HMODULE::default(),
                Some(taskbar_zorder_win_event_proc),
                pid,
                tid,
                flags,
            );
        }
        self.bound_tb_pid = pid;
        self.bound_tb_tid = tid;
    }

    fn ensure_hooks(&mut self) {
        if self.widgets.is_empty() {
            self.unhook_all();
            return;
        }
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        unsafe {
            if self.global_fg_hook.is_invalid() {
                self.global_fg_hook = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    HMODULE::default(),
                    Some(taskbar_zorder_win_event_proc),
                    0,
                    0,
                    flags,
                );
            }
            if self.global_min_hook.is_invalid() {
                self.global_min_hook = SetWinEventHook(
                    EVENT_SYSTEM_MINIMIZESTART,
                    EVENT_SYSTEM_MINIMIZEEND,
                    HMODULE::default(),
                    Some(taskbar_zorder_win_event_proc),
                    0,
                    0,
                    flags,
                );
            }
        }
        if let Ok(tb_hwnd) = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) } {
            if !tb_hwnd.is_invalid() {
                let mut pid = 0u32;
                let tid = unsafe { GetWindowThreadProcessId(tb_hwnd, Some(&mut pid)) };
                self.rebind_taskbar_hooks(pid, tid);
            }
        }
    }
}

impl Drop for TaskbarOverlayGuard {
    fn drop(&mut self) {
        self.unhook_all();
    }
}

thread_local! {
    static OVERLAY_GUARD: RefCell<TaskbarOverlayGuard> = RefCell::new(TaskbarOverlayGuard::default());
}

struct ReentryResetGuard;

impl Drop for ReentryResetGuard {
    fn drop(&mut self) {
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard.in_callback = false;
            }
        });
    }
}

unsafe extern "system" fn taskbar_zorder_win_event_proc(
    _hwineventhook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _idobject: i32,
    _idchild: i32,
    _ideventthread: u32,
    _dwmseventtime: u32,
) {
    let Some(widgets) = OVERLAY_GUARD.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return None;
        };
        if guard.in_callback || guard.widgets.is_empty() {
            return None;
        }
        guard.in_callback = true;
        Some(guard.widgets.clone())
    }) else {
        return;
    };
    let _reset = ReentryResetGuard;

    let Ok(tb_hwnd) = FindWindowW(w!("Shell_TrayWnd"), None) else {
        return;
    };
    if tb_hwnd.is_invalid() {
        return;
    }

    let tb_band = window_band(tb_hwnd);
    let any_below = widgets
        .iter()
        .copied()
        .any(|w| IsWindowVisible(w).as_bool() && is_taskbar_above_window(w, tb_hwnd));
    if any_below && !is_fullscreen_occluding_taskbar(tb_hwnd) {
        for w in widgets {
            if !IsWindowVisible(w).as_bool() {
                continue;
            }
            if let (Some(wb), Some(tb)) = (window_band(w), tb_band) {
                if tb > wb {
                    let is_topmost =
                        (GetWindowLongW(w, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0;
                    if is_topmost && !is_any_backdrop_above(w) {
                        continue;
                    }
                }
            }
            ensure_window_above_taskbar(w, tb_hwnd);
        }
    }

    let mut cur_pid = 0u32;
    let cur_tid = GetWindowThreadProcessId(tb_hwnd, Some(&mut cur_pid));
    let need_rebind = OVERLAY_GUARD.with(|cell| {
        cell.try_borrow().is_ok_and(|g| {
            !g.widgets.is_empty()
                && (g.bound_tb_pid != cur_pid || g.bound_tb_tid != cur_tid)
        })
    });
    if need_rebind {
        OVERLAY_GUARD.with(|cell| {
            if let Ok(mut guard) = cell.try_borrow_mut() {
                guard.rebind_taskbar_hooks(cur_pid, cur_tid);
            }
        });
    }
}

pub(super) fn is_any_backdrop_above(bar_hwnd: HWND) -> bool {
    if bar_hwnd.is_invalid() {
        return false;
    }
    let backdrops = OVERLAY_GUARD.with(|cell| {
        cell.try_borrow()
            .map(|g| g.backdrops.clone())
            .unwrap_or_default()
    });
    backdrops.into_iter().any(|bg| unsafe {
        !bg.is_invalid()
            && IsWindow(bg).as_bool()
            && IsWindowVisible(bg).as_bool()
            && is_window_above(bg, bar_hwnd)
    })
}

/// Registers a behind-taskbar backdrop HWND so Z-order coordination keeps it below widgets.
pub fn register_taskbar_backdrop(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    OVERLAY_GUARD.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            guard
                .backdrops
                .retain(|&w| !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
            if !guard.backdrops.contains(&hwnd) {
                guard.backdrops.push(hwnd);
            }
        }
    });
}

/// Unregisters a behind-taskbar backdrop HWND before destruction.
pub fn unregister_taskbar_backdrop(hwnd: HWND) {
    OVERLAY_GUARD.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            guard
                .backdrops
                .retain(|&w| w != hwnd && !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
        }
    });
}

/// Resolves the Z-order anchor (`insert_after`) for a behind-taskbar backdrop window on the
/// same monitor as `tb_hwnd`, placing it below all visible same-band taskbar widgets and the
/// taskbar itself even if Windows elevates `tb_hwnd` to a higher band.
pub fn taskbar_backdrop_anchor(tb_hwnd: HWND, backdrop_hwnd: HWND) -> HWND {
    if tb_hwnd.is_invalid() {
        return tb_hwnd;
    }
    unsafe {
        let tb_mon = MonitorFromWindow(tb_hwnd, MONITOR_DEFAULTTONEAREST);
        let tb_band = window_band(tb_hwnd);
        let bg_band = window_band(backdrop_hwnd);

        let candidates: Vec<HWND> = OVERLAY_GUARD.with(|cell| {
            let Ok(guard) = cell.try_borrow() else {
                return Vec::new();
            };
            guard
                .widgets
                .iter()
                .copied()
                .filter(|&w| {
                    !w.is_invalid()
                        && w != backdrop_hwnd
                        && IsWindow(w).as_bool()
                        && IsWindowVisible(w).as_bool()
                        && MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST) == tb_mon
                        && match (bg_band, window_band(w)) {
                            (Some(bg), Some(wb)) => bg == wb,
                            _ => true,
                        }
                })
                .collect()
        });

        if candidates.is_empty() {
            return tb_hwnd;
        }

        let mut taskbar_z_rank: Option<usize> = None;
        let mut matched_overlays: Vec<(usize, usize)> = Vec::with_capacity(candidates.len());
        let mut rank = 0usize;
        let mut cur = GetTopWindow(None).unwrap_or_default();
        while !cur.is_invalid() && rank < 1024 {
            if cur == tb_hwnd && taskbar_z_rank.is_none() {
                taskbar_z_rank = Some(rank);
            }
            if candidates.contains(&cur)
                && !matched_overlays.iter().any(|&(id, _)| id == cur.0 as usize)
            {
                matched_overlays.push((cur.0 as usize, rank));
            }
            if taskbar_z_rank.is_some() && matched_overlays.len() == candidates.len() {
                break;
            }
            rank += 1;
            cur = GetWindow(cur, GW_HWNDNEXT).unwrap_or_default();
        }

        let anchor_id = super::pick_lowest_backdrop_anchor(
            tb_hwnd.0 as usize,
            tb_band,
            bg_band,
            taskbar_z_rank,
            &matched_overlays,
        );
        HWND(anchor_id as *mut core::ffi::c_void)
    }
}

/// Registers a taskbar widget HWND with the thread-local event-driven Z-order guard.
pub fn register_taskbar_overlay(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    OVERLAY_GUARD.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            guard
                .widgets
                .retain(|&w| !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
            if !guard.widgets.contains(&hwnd) {
                guard.widgets.push(hwnd);
            }
            guard.ensure_hooks();
        }
    });
}

/// Unregisters a taskbar widget HWND and unhooks global/taskbar listeners when no widgets remain.
pub fn unregister_taskbar_overlay(hwnd: HWND) {
    OVERLAY_GUARD.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            guard
                .widgets
                .retain(|&w| w != hwnd && !w.is_invalid() && unsafe { IsWindow(w).as_bool() });
            if guard.widgets.is_empty() {
                guard.unhook_all();
            }
        }
    });
}

/// Refreshes thread-scoped WinEvent hooks after Explorer recreates the taskbar (`TaskbarCreated`).
pub fn refresh_taskbar_overlay_hooks() {
    OVERLAY_GUARD.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            guard.ensure_hooks();
        }
    });
}

/// Collects screen bounding rectangles (with a 16px DWM shadow margin) of visible transient
/// shell flyouts or tooltips intersecting `capture_rect`, so popup backdrop captures can
/// cleanly inpaint them out before they fade/dismiss on activation.
pub fn visible_transient_overlay_rects(capture_rect: RECT) -> Vec<RECT> {
    struct CollectCtx {
        capture: RECT,
        rects: Vec<RECT>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut CollectCtx);
        if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut cls_buf = [0u16; 96];
        let len = GetClassNameW(hwnd, &mut cls_buf);
        if len <= 0 {
            return BOOL(1);
        }
        let class_name = String::from_utf16_lossy(&cls_buf[..len as usize]);
        if !crate::system::is_transient_backdrop_occluder_class(&class_name) {
            return BOOL(1);
        }

        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return BOOL(1);
        }

        let mut win_rect = RECT::default();
        if GetWindowRect(hwnd, &mut win_rect).is_err() {
            return BOOL(1);
        }
        let mut ext_rect = RECT::default();
        let bounds = if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut ext_rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
        .is_ok()
        {
            RECT {
                left: win_rect.left.min(ext_rect.left),
                top: win_rect.top.min(ext_rect.top),
                right: win_rect.right.max(ext_rect.right),
                bottom: win_rect.bottom.max(ext_rect.bottom),
            }
        } else {
            win_rect
        };

        let w = bounds.right - bounds.left;
        let h = bounds.bottom - bounds.top;
        if w <= 0 || h <= 0 || w > 960 || h > 720 {
            return BOOL(1);
        }

        const SHADOW_PAD_PX: i32 = 16;
        let inter = RECT {
            left: (bounds.left - SHADOW_PAD_PX).max(ctx.capture.left),
            top: (bounds.top - SHADOW_PAD_PX).max(ctx.capture.top),
            right: (bounds.right + SHADOW_PAD_PX).min(ctx.capture.right),
            bottom: (bounds.bottom + SHADOW_PAD_PX).min(ctx.capture.bottom),
        };
        if inter.right > inter.left && inter.bottom > inter.top {
            ctx.rects.push(inter);
        }
        BOOL(1)
    }

    if capture_rect.right <= capture_rect.left || capture_rect.bottom <= capture_rect.top {
        return Vec::new();
    }
    let mut ctx = CollectCtx {
        capture: capture_rect,
        rects: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM((&mut ctx as *mut CollectCtx) as isize));
    }
        ctx.rects
}

static OVERFLOW_NOTIFY_HWND: AtomicUsize = AtomicUsize::new(0);
static OVERFLOW_NOTIFY_MSG: AtomicU32 = AtomicU32::new(0);
static OVERFLOW_NOTIFY_PENDING: AtomicBool = AtomicBool::new(false);

/// Clears the coalescing flag so subsequent overflow WinEvents can schedule another host sync.
pub fn clear_overflow_notify_pending() {
    OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
}

unsafe extern "system" fn overflow_win_event_proc(
    _hwineventhook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    idobject: i32,
    _idchild: i32,
    _ideventthread: u32,
    _dwmseventtime: u32,
) {
    if hwnd.is_invalid() || idobject != 0 {
        return;
    }
    let target_raw = OVERFLOW_NOTIFY_HWND.load(WinOrdering::Acquire);
    let notify_msg = OVERFLOW_NOTIFY_MSG.load(WinOrdering::Acquire);
    if target_raw == 0 || notify_msg == 0 {
        return;
    }
    let mut cls_buf = [0u16; 64];
    let len = GetClassNameW(hwnd, &mut cls_buf);
    if len <= 0 {
        return;
    }
    let cls = String::from_utf16_lossy(&cls_buf[..len as usize]);
    if matches!(
        cls.as_str(),
        "TopLevelWindowForOverflowXamlIsland"
            | "NotifyIconOverflowWindow"
            | "XamlExplorerHostIslandWindow"
            | "XamlExplorerHostIslandWindow_WASDK"
            | "Shell_InputSwitchTopLevelWindow"
            | "Xaml_WindowedPopupClass"
            | "Windows.UI.Core.CoreWindow"
            | "ControlCenterWindow"
    ) && !OVERFLOW_NOTIFY_PENDING.swap(true, WinOrdering::AcqRel)
        && PostMessageW(
            HWND(target_raw as *mut core::ffi::c_void),
            notify_msg,
            WPARAM(0),
            LPARAM(0),
        )
        .is_err()
    {
        OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
    }
}

unsafe fn install_window_event_hooks(pid: u32) -> (HWINEVENTHOOK, HWINEVENTHOOK, HWINEVENTHOOK) {
    if pid == 0 {
        return (
            HWINEVENTHOOK::default(),
            HWINEVENTHOOK::default(),
            HWINEVENTHOOK::default(),
        );
    }
    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    (
        SetWinEventHook(
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_HIDE,
            HMODULE::default(),
            Some(overflow_win_event_proc),
            pid,
            0,
            flags,
        ),
        SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            HMODULE::default(),
            Some(overflow_win_event_proc),
            pid,
            0,
            flags,
        ),
        SetWinEventHook(
            EVENT_OBJECT_CLOAKED,
            EVENT_OBJECT_UNCLOAKED,
            HMODULE::default(),
            Some(overflow_win_event_proc),
            pid,
            0,
            flags,
        ),
    )
}

unsafe fn unhook_triplet(hooks: [&mut HWINEVENTHOOK; 3]) {
    for hook in hooks {
        if !hook.is_invalid() {
            let _ = UnhookWinEvent(*hook);
            *hook = HWINEVENTHOOK::default();
        }
    }
}

fn refresh_win_event_triplet(
    pid_slot: &mut u32,
    hooks: [&mut HWINEVENTHOOK; 3],
    new_pid: u32,
) {
    if *pid_slot == new_pid && (!hooks[0].is_invalid() || new_pid == 0) {
        return;
    }
    let [h0, h1, h2] = hooks;
    unsafe {
        unhook_triplet([h0, h1, h2]);
    }
    *pid_slot = new_pid;
    if new_pid == 0 {
        return;
    }
    let (nh0, nh1, nh2) = unsafe { install_window_event_hooks(new_pid) };
    *h0 = nh0;
    *h1 = nh1;
    *h2 = nh2;
}

/// RAII process-scoped WinEvent hook notifying the tray host immediately when Explorer,
/// `ShellExperienceHost`, `ShellHost`, `StartMenuExperienceHost`, or `SearchHost` flyouts show, move, cloak, or hide.
pub struct OverflowEventHook {
    host_hwnd: HWND,
    notify_msg: u32,
    explorer_pid: u32,
    show_hide_hook: HWINEVENTHOOK,
    location_hook: HWINEVENTHOOK,
    cloak_hook: HWINEVENTHOOK,
    shell_exp_pid: u32,
    shell_show_hide_hook: HWINEVENTHOOK,
    shell_location_hook: HWINEVENTHOOK,
    shell_cloak_hook: HWINEVENTHOOK,
    shell_host_pid: u32,
    shell_host_show_hide_hook: HWINEVENTHOOK,
    shell_host_location_hook: HWINEVENTHOOK,
    shell_host_cloak_hook: HWINEVENTHOOK,
    start_menu_pid: u32,
    start_menu_show_hide_hook: HWINEVENTHOOK,
    start_menu_location_hook: HWINEVENTHOOK,
    start_menu_cloak_hook: HWINEVENTHOOK,
    search_host_pid: u32,
    search_host_show_hide_hook: HWINEVENTHOOK,
    search_host_location_hook: HWINEVENTHOOK,
    search_host_cloak_hook: HWINEVENTHOOK,
}

impl OverflowEventHook {
    pub fn new(host_hwnd: HWND, notify_msg: u32) -> Self {
        OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
        OVERFLOW_NOTIFY_HWND.store(host_hwnd.0 as usize, WinOrdering::Release);
        OVERFLOW_NOTIFY_MSG.store(notify_msg, WinOrdering::Release);
        Self {
            host_hwnd,
            notify_msg,
            explorer_pid: 0,
            show_hide_hook: HWINEVENTHOOK::default(),
            location_hook: HWINEVENTHOOK::default(),
            cloak_hook: HWINEVENTHOOK::default(),
            shell_exp_pid: 0,
            shell_show_hide_hook: HWINEVENTHOOK::default(),
            shell_location_hook: HWINEVENTHOOK::default(),
            shell_cloak_hook: HWINEVENTHOOK::default(),
            shell_host_pid: 0,
            shell_host_show_hide_hook: HWINEVENTHOOK::default(),
            shell_host_location_hook: HWINEVENTHOOK::default(),
            shell_host_cloak_hook: HWINEVENTHOOK::default(),
            start_menu_pid: 0,
            start_menu_show_hide_hook: HWINEVENTHOOK::default(),
            start_menu_location_hook: HWINEVENTHOOK::default(),
            start_menu_cloak_hook: HWINEVENTHOOK::default(),
            search_host_pid: 0,
            search_host_show_hide_hook: HWINEVENTHOOK::default(),
            search_host_location_hook: HWINEVENTHOOK::default(),
            search_host_cloak_hook: HWINEVENTHOOK::default(),
        }
    }

    pub fn refresh_for_explorer_pid(&mut self, pid: u32) {
        OVERFLOW_NOTIFY_HWND.store(self.host_hwnd.0 as usize, WinOrdering::Release);
        OVERFLOW_NOTIFY_MSG.store(self.notify_msg, WinOrdering::Release);
        refresh_win_event_triplet(
            &mut self.explorer_pid,
            [
                &mut self.show_hide_hook,
                &mut self.location_hook,
                &mut self.cloak_hook,
            ],
            pid,
        );
    }

    pub fn refresh_for_shell_experience_pid(&mut self, pid: u32) {
        OVERFLOW_NOTIFY_HWND.store(self.host_hwnd.0 as usize, WinOrdering::Release);
        OVERFLOW_NOTIFY_MSG.store(self.notify_msg, WinOrdering::Release);
        refresh_win_event_triplet(
            &mut self.shell_exp_pid,
            [
                &mut self.shell_show_hide_hook,
                &mut self.shell_location_hook,
                &mut self.shell_cloak_hook,
            ],
            pid,
        );
    }

    pub fn refresh_for_shell_host_pid(&mut self, pid: u32) {
        OVERFLOW_NOTIFY_HWND.store(self.host_hwnd.0 as usize, WinOrdering::Release);
        OVERFLOW_NOTIFY_MSG.store(self.notify_msg, WinOrdering::Release);
        refresh_win_event_triplet(
            &mut self.shell_host_pid,
            [
                &mut self.shell_host_show_hide_hook,
                &mut self.shell_host_location_hook,
                &mut self.shell_host_cloak_hook,
            ],
            pid,
        );
    }

    pub fn refresh_for_start_menu_pid(&mut self, pid: u32) {
        OVERFLOW_NOTIFY_HWND.store(self.host_hwnd.0 as usize, WinOrdering::Release);
        OVERFLOW_NOTIFY_MSG.store(self.notify_msg, WinOrdering::Release);
        refresh_win_event_triplet(
            &mut self.start_menu_pid,
            [
                &mut self.start_menu_show_hide_hook,
                &mut self.start_menu_location_hook,
                &mut self.start_menu_cloak_hook,
            ],
            pid,
        );
    }

    pub fn refresh_for_search_host_pid(&mut self, pid: u32) {
        OVERFLOW_NOTIFY_HWND.store(self.host_hwnd.0 as usize, WinOrdering::Release);
        OVERFLOW_NOTIFY_MSG.store(self.notify_msg, WinOrdering::Release);
        refresh_win_event_triplet(
            &mut self.search_host_pid,
            [
                &mut self.search_host_show_hide_hook,
                &mut self.search_host_location_hook,
                &mut self.search_host_cloak_hook,
            ],
            pid,
        );
    }

    fn unhook_all(&mut self) {
        unsafe {
            unhook_triplet([
                &mut self.show_hide_hook,
                &mut self.location_hook,
                &mut self.cloak_hook,
            ]);
            unhook_triplet([
                &mut self.shell_show_hide_hook,
                &mut self.shell_location_hook,
                &mut self.shell_cloak_hook,
            ]);
            unhook_triplet([
                &mut self.shell_host_show_hide_hook,
                &mut self.shell_host_location_hook,
                &mut self.shell_host_cloak_hook,
            ]);
            unhook_triplet([
                &mut self.start_menu_show_hide_hook,
                &mut self.start_menu_location_hook,
                &mut self.start_menu_cloak_hook,
            ]);
            unhook_triplet([
                &mut self.search_host_show_hide_hook,
                &mut self.search_host_location_hook,
                &mut self.search_host_cloak_hook,
            ]);
        }
        self.explorer_pid = 0;
        self.shell_exp_pid = 0;
        self.shell_host_pid = 0;
        self.start_menu_pid = 0;
        self.search_host_pid = 0;
    }
}

impl Drop for OverflowEventHook {
    fn drop(&mut self) {
        self.unhook_all();
        OVERFLOW_NOTIFY_PENDING.store(false, WinOrdering::Release);
        OVERFLOW_NOTIFY_HWND.store(0, WinOrdering::Release);
        OVERFLOW_NOTIFY_MSG.store(0, WinOrdering::Release);
    }
}
