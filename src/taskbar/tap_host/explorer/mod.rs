mod pipe_io;
mod worker;

use worker::run_tap_worker;

use super::inject::{AltTabBoundsCache, WatchedProcessHandle, MAINTAIN_PING_INTERVAL};
use super::{explorer_inject_retry_cooldown, ProcessExitKind, TapSafetyGuard};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;

struct TapWorkerShared {
    guard: TapSafetyGuard,
    desired_hidden: bool,
    target_pid: u32,
    attached_pid: Option<u32>,
    conflict_detected: bool,
    wake_seq: u64,
    last_processed_seq: u64,
    alt_tab_query_seq: u64,
    last_alt_tab_query_seq: u64,
    alt_tab_bounds: Option<AltTabBoundsCache>,
    hover_flyout_query_seq: u64,
    last_hover_flyout_query_seq: u64,
    hover_flyout_bounds: Option<AltTabBoundsCache>,
    osd_query_seq: u64,
    last_osd_query_seq: u64,
    osd_bounds: Option<AltTabBoundsCache>,
    osd_suppressed: bool,
    synced_osd_suppressed: Option<bool>,
    osd_fallback_colors: Option<(u32, u32)>,
    synced_osd_fallback_colors: Option<(u32, u32)>,
    tooltip_query_seq: u64,
    last_tooltip_query_seq: u64,
    tooltip_bounds: Option<AltTabBoundsCache>,
    tooltip_suppressed: bool,
    synced_tooltip_suppressed: Option<bool>,
    tooltip_fallback_colors: Option<(u32, u32)>,
    synced_tooltip_fallback_colors: Option<(u32, u32)>,
    menu_flyout_query_seq: u64,
    last_menu_flyout_query_seq: u64,
    menu_flyout_bounds: Vec<AltTabBoundsCache>,
    menu_flyout_suppressed: bool,
    synced_menu_flyout_suppressed: Option<bool>,
    menu_flyout_fallback_colors: Option<(u32, u32)>,
    synced_menu_flyout_fallback_colors: Option<(u32, u32)>,
    input_switcher_query_seq: u64,
    last_input_switcher_query_seq: u64,
    input_switcher_bounds: Option<AltTabBoundsCache>,
    input_switcher_suppressed: bool,
    synced_input_switcher_suppressed: Option<bool>,
    input_switcher_fallback_colors: Option<(u32, u32)>,
    synced_input_switcher_fallback_colors: Option<(u32, u32)>,
    alt_tab_item_colors: Option<(u32, u32)>,
    synced_alt_tab_item_colors: Option<(u32, u32)>,
    virtual_desktop_bar_colors: Option<(u32, u32)>,
    synced_virtual_desktop_bar_colors: Option<(u32, u32)>,
    notify_hwnd_raw: usize,
    notify_msg: u32,
    last_inject_try: Option<Instant>,
    not_ready_retries: u32,
    watched_process: Option<WatchedProcessHandle>,
    last_ping_ok: Option<Instant>,
    stop_worker: bool,
    shutdown_done: bool,
}

impl TapWorkerShared {
    fn clear_synced_flags(&mut self) {
        self.synced_alt_tab_item_colors = None;
        self.synced_virtual_desktop_bar_colors = None;
        self.synced_osd_fallback_colors = None;
        self.synced_osd_suppressed = None;
        self.synced_tooltip_fallback_colors = None;
        self.synced_tooltip_suppressed = None;
        self.synced_menu_flyout_fallback_colors = None;
        self.synced_menu_flyout_suppressed = None;
        self.synced_input_switcher_fallback_colors = None;
        self.synced_input_switcher_suppressed = None;
    }

    fn clear_synced_and_bounds(&mut self, clear_all_bounds: bool) {
        self.clear_synced_flags();
        self.osd_bounds = None;
        self.tooltip_bounds = None;
        self.menu_flyout_bounds.clear();
        self.input_switcher_bounds = None;
        if clear_all_bounds {
            self.alt_tab_bounds = None;
            self.hover_flyout_bounds = None;
        }
    }

    fn detach_pid(&mut self, expected_pid: Option<u32>, clear_all_bounds: bool) {
        if expected_pid.is_none_or(|pid| self.attached_pid == Some(pid)) {
            self.attached_pid = None;
            self.last_ping_ok = None;
        }
        self.clear_synced_and_bounds(clear_all_bounds);
    }

    fn track_process_pid(&mut self, pid: u32) {
        if pid == 0 {
            return;
        }
        if self.watched_process.as_ref().is_some_and(|w| w.pid() == pid) {
            return;
        }
        if let Some(handle) = WatchedProcessHandle::open(pid) {
            self.watched_process = Some(handle);
        }
    }

    fn inspect_old_pid_exit_kind(&mut self, current_pid: Option<u32>) -> ProcessExitKind {
        let Some(ref watched) = self.watched_process else {
            return ProcessExitKind::Unknown;
        };
        if current_pid == Some(watched.pid()) {
            return ProcessExitKind::Unknown;
        }
        let kind = watched.query_exit_kind();
        if current_pid.is_some() || kind != ProcessExitKind::Unknown {
            self.watched_process = None;
        }
        kind
    }

    fn reset_retry_state_for_new_pid(&mut self) {
        self.last_inject_try = None;
        self.not_ready_retries = 0;
        self.guard.reset_failures_on_new_pid();
    }
}

type StyleColorSelector =
    fn(&mut TapWorkerShared) -> (&mut Option<(u32, u32)>, &mut Option<(u32, u32)>);

/// Asynchronous Named Pipe + XAML Diagnostics client; never blocks the tray UI thread.
pub struct TapClient {
    shared: Arc<(Mutex<TapWorkerShared>, Condvar, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl TapClient {
    pub fn new() -> Self {
        Self {
            shared: Arc::new((
                Mutex::new(TapWorkerShared {
                    guard: TapSafetyGuard::new(),
                    desired_hidden: false,
                    target_pid: 0,
                    attached_pid: None,
                    conflict_detected: false,
                    wake_seq: 0,
                    last_processed_seq: 0,
                    alt_tab_query_seq: 0,
                    last_alt_tab_query_seq: 0,
                    alt_tab_bounds: None,
                    hover_flyout_query_seq: 0,
                    last_hover_flyout_query_seq: 0,
                    hover_flyout_bounds: None,
                    osd_query_seq: 0,
                    last_osd_query_seq: 0,
                    osd_bounds: None,
                    osd_suppressed: false,
                    synced_osd_suppressed: None,
                    osd_fallback_colors: None,
                    synced_osd_fallback_colors: None,
                    tooltip_query_seq: 0,
                    last_tooltip_query_seq: 0,
                    tooltip_bounds: None,
                    tooltip_suppressed: false,
                    synced_tooltip_suppressed: None,
                    tooltip_fallback_colors: None,
                    synced_tooltip_fallback_colors: None,
                    menu_flyout_query_seq: 0,
                    last_menu_flyout_query_seq: 0,
                    menu_flyout_bounds: Vec::new(),
                    menu_flyout_suppressed: false,
                    synced_menu_flyout_suppressed: None,
                    menu_flyout_fallback_colors: None,
                    synced_menu_flyout_fallback_colors: None,
                    input_switcher_query_seq: 0,
                    last_input_switcher_query_seq: 0,
                    input_switcher_bounds: None,
                    input_switcher_suppressed: false,
                    synced_input_switcher_suppressed: None,
                    input_switcher_fallback_colors: None,
                    synced_input_switcher_fallback_colors: None,
                    alt_tab_item_colors: None,
                    synced_alt_tab_item_colors: None,
                    virtual_desktop_bar_colors: None,
                    synced_virtual_desktop_bar_colors: None,
                    notify_hwnd_raw: 0,
                    notify_msg: 0,
                    last_inject_try: None,
                    not_ready_retries: 0,
                    watched_process: None,
                    last_ping_ok: None,
                    stop_worker: false,
                    shutdown_done: false,
                }),
                Condvar::new(),
                Condvar::new(),
            )),
            worker: None,
        }
    }

    pub fn set_notify_target(&mut self, hwnd: HWND, msg: u32) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.notify_hwnd_raw = hwnd.0 as usize;
            st.notify_msg = msg;
        }
    }

    pub fn request_alt_tab_rect(&mut self, explorer_pid: u32) {
        if explorer_pid == 0 {
            return;
        }
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            if st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker {
                st.target_pid = explorer_pid;
                st.alt_tab_query_seq = st.alt_tab_query_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.notify_all();
        }
    }

    pub fn clear_alt_tab_rect(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.alt_tab_bounds = None;
        }
    }

    pub fn cached_alt_tab_rect(&self) -> Option<AltTabBoundsCache> {
        self.shared.0.lock().ok().and_then(|st| st.alt_tab_bounds)
    }

    pub fn request_hover_flyout_rect(&mut self, explorer_pid: u32) {
        if explorer_pid == 0 {
            return;
        }
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            if st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker {
                st.target_pid = explorer_pid;
                st.hover_flyout_query_seq = st.hover_flyout_query_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.notify_all();
        }
    }

    pub fn clear_hover_flyout_rect(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.hover_flyout_bounds = None;
        }
    }

    pub fn cached_hover_flyout_rect(&self) -> Option<AltTabBoundsCache> {
        self.shared
            .0
            .lock()
            .ok()
            .and_then(|st| st.hover_flyout_bounds)
    }

    pub fn request_input_switcher_rect(&mut self, explorer_pid: u32) {
        if explorer_pid == 0 {
            return;
        }
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            if st.desired_hidden
                && !st.input_switcher_suppressed
                && !st.guard.is_tripped()
                && !st.stop_worker
            {
                st.target_pid = explorer_pid;
                st.input_switcher_query_seq = st.input_switcher_query_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.notify_all();
        }
    }

    pub fn clear_input_switcher_rect(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.input_switcher_bounds = None;
        }
    }

    pub fn cached_input_switcher_rect(&self) -> Option<AltTabBoundsCache> {
        self.shared
            .0
            .lock()
            .ok()
            .and_then(|st| st.input_switcher_bounds)
    }

    pub fn set_input_switcher_suppressed(&mut self, explorer_pid: u32, suppressed: bool) {
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            let changed = st.input_switcher_suppressed != suppressed;
            st.input_switcher_suppressed = suppressed;
            if suppressed {
                st.input_switcher_bounds = None;
            }
            if explorer_pid != 0 {
                st.target_pid = explorer_pid;
            }
            if changed && st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker {
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            if explorer_pid != 0 {
                self.ensure_worker_started();
            }
            if self.worker.is_some() {
                self.shared.1.notify_all();
            }
        }
    }

    pub fn request_osd_rect(&mut self, explorer_pid: u32) {
        if explorer_pid == 0 {
            return;
        }
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            if st.desired_hidden
                && !st.osd_suppressed
                && !st.guard.is_tripped()
                && !st.stop_worker
            {
                st.target_pid = explorer_pid;
                st.osd_query_seq = st.osd_query_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.notify_all();
        }
    }

    pub fn clear_osd_rect(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.osd_bounds = None;
        }
    }

    pub fn cached_osd_rect(&self) -> Option<AltTabBoundsCache> {
        self.shared.0.lock().ok().and_then(|st| st.osd_bounds)
    }

    pub fn set_osd_suppressed(&mut self, explorer_pid: u32, suppressed: bool) {
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            let changed = st.osd_suppressed != suppressed;
            st.osd_suppressed = suppressed;
            if suppressed {
                st.osd_bounds = None;
            }
            if explorer_pid != 0 {
                st.target_pid = explorer_pid;
            }
            if changed && st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker {
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            if explorer_pid != 0 {
                self.ensure_worker_started();
            }
            if self.worker.is_some() {
                self.shared.1.notify_all();
            }
        }
    }

    pub fn request_tooltip_rect(&mut self, explorer_pid: u32) {
        if explorer_pid == 0 {
            return;
        }
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            if st.desired_hidden
                && !st.tooltip_suppressed
                && !st.guard.is_tripped()
                && !st.stop_worker
            {
                st.target_pid = explorer_pid;
                st.tooltip_query_seq = st.tooltip_query_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.notify_all();
        }
    }

    pub fn clear_tooltip_rect(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.tooltip_bounds = None;
        }
    }

    pub fn cached_tooltip_rect(&self) -> Option<AltTabBoundsCache> {
        self.shared.0.lock().ok().and_then(|st| st.tooltip_bounds)
    }

    pub fn set_tooltip_suppressed(&mut self, explorer_pid: u32, suppressed: bool) {
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            let changed = st.tooltip_suppressed != suppressed;
            st.tooltip_suppressed = suppressed;
            if suppressed {
                st.tooltip_bounds = None;
            }
            if explorer_pid != 0 {
                st.target_pid = explorer_pid;
            }
            if changed && st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker {
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            if explorer_pid != 0 {
                self.ensure_worker_started();
            }
            if self.worker.is_some() {
                self.shared.1.notify_all();
            }
        }
    }

    fn update_style_color_pair(
        &mut self,
        explorer_pid: u32,
        colors: Option<(u32, u32)>,
        select: StyleColorSelector,
    ) {
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            let changed = {
                let (desired_slot, synced_slot) = select(&mut st);
                let changed = *desired_slot != colors;
                *desired_slot = colors;
                if colors.is_none() {
                    *synced_slot = None;
                }
                changed
            };
            if explorer_pid != 0 {
                st.target_pid = explorer_pid;
            }
            if changed
                && colors.is_some()
                && st.desired_hidden
                && !st.guard.is_tripped()
                && !st.stop_worker
            {
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            if explorer_pid != 0 {
                self.ensure_worker_started();
            }
            if self.worker.is_some() {
                self.shared.1.notify_all();
            }
        }
    }

    pub fn set_alt_tab_item_colors(
        &mut self,
        explorer_pid: u32,
        colors: Option<(u32, u32)>,
    ) {
        self.update_style_color_pair(explorer_pid, colors, |st| {
            (&mut st.alt_tab_item_colors, &mut st.synced_alt_tab_item_colors)
        });
    }

    pub fn set_osd_fallback_colors(
        &mut self,
        explorer_pid: u32,
        colors: Option<(u32, u32)>,
    ) {
        self.update_style_color_pair(explorer_pid, colors, |st| {
            (&mut st.osd_fallback_colors, &mut st.synced_osd_fallback_colors)
        });
    }

    pub fn set_tooltip_fallback_colors(
        &mut self,
        explorer_pid: u32,
        colors: Option<(u32, u32)>,
    ) {
        self.update_style_color_pair(explorer_pid, colors, |st| {
            (
                &mut st.tooltip_fallback_colors,
                &mut st.synced_tooltip_fallback_colors,
            )
        });
    }

    pub fn set_input_switcher_fallback_colors(
        &mut self,
        explorer_pid: u32,
        colors: Option<(u32, u32)>,
    ) {
        self.update_style_color_pair(explorer_pid, colors, |st| {
            (
                &mut st.input_switcher_fallback_colors,
                &mut st.synced_input_switcher_fallback_colors,
            )
        });
    }

    pub fn request_menu_flyout_rects(&mut self, explorer_pid: u32) {
        if explorer_pid == 0 {
            return;
        }
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            if st.desired_hidden
                && !st.menu_flyout_suppressed
                && !st.guard.is_tripped()
                && !st.stop_worker
            {
                st.target_pid = explorer_pid;
                st.menu_flyout_query_seq = st.menu_flyout_query_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.notify_all();
        }
    }

    pub fn clear_menu_flyout_rects(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.menu_flyout_bounds.clear();
        }
    }

    pub fn cached_menu_flyout_rects(&self) -> Vec<AltTabBoundsCache> {
        self.shared
            .0
            .lock()
            .map(|st| st.menu_flyout_bounds.clone())
            .unwrap_or_default()
    }

    pub fn set_menu_flyout_suppressed(&mut self, explorer_pid: u32, suppressed: bool) {
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            let changed = st.menu_flyout_suppressed != suppressed;
            st.menu_flyout_suppressed = suppressed;
            if suppressed {
                st.menu_flyout_bounds.clear();
            }
            if explorer_pid != 0 {
                st.target_pid = explorer_pid;
            }
            if changed && st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker {
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            if explorer_pid != 0 {
                self.ensure_worker_started();
            }
            if self.worker.is_some() {
                self.shared.1.notify_all();
            }
        }
    }

    pub fn set_menu_flyout_fallback_colors(
        &mut self,
        explorer_pid: u32,
        colors: Option<(u32, u32)>,
    ) {
        self.update_style_color_pair(explorer_pid, colors, |st| {
            (
                &mut st.menu_flyout_fallback_colors,
                &mut st.synced_menu_flyout_fallback_colors,
            )
        });
    }

    pub fn set_virtual_desktop_bar_colors(
        &mut self,
        explorer_pid: u32,
        colors: Option<(u32, u32)>,
    ) {
        self.update_style_color_pair(explorer_pid, colors, |st| {
            (
                &mut st.virtual_desktop_bar_colors,
                &mut st.synced_virtual_desktop_bar_colors,
            )
        });
    }

    pub fn is_circuit_tripped(&self) -> bool {
        self.shared
            .0
            .lock()
            .map(|g| g.guard.is_tripped())
            .unwrap_or(false)
    }

    pub fn on_explorer_restarted(&mut self, current_pid: Option<u32>) {
        let now = Instant::now();
        let need_worker = if let Ok(mut st) = self.shared.0.lock() {
            let exit_kind = st.inspect_old_pid_exit_kind(current_pid);
            if st.guard.on_explorer_restarted(current_pid, exit_kind, now) {
                st.desired_hidden = false;
            }
            let pid_changed = st
                .attached_pid
                .is_some_and(|attached| current_pid != Some(attached))
                || current_pid.is_some_and(|pid| st.target_pid != 0 && st.target_pid != pid);
            if pid_changed || st.attached_pid.is_none() || st.guard.is_tripped() {
                st.detach_pid(None, true);
                st.osd_suppressed = false;
                st.tooltip_suppressed = false;
                st.menu_flyout_suppressed = false;
                st.input_switcher_suppressed = false;
                if !st.guard.is_tripped() {
                    st.reset_retry_state_for_new_pid();
                }
                st.target_pid = current_pid.unwrap_or(0);
                if let Some(pid) = current_pid {
                    st.track_process_pid(pid);
                }
            } else {
                st.clear_synced_flags();
            }
            st.wake_seq = st.wake_seq.wrapping_add(1);
            st.desired_hidden && !st.guard.is_tripped() && st.target_pid != 0
        } else {
            false
        };
        if need_worker {
            self.ensure_worker_started();
        }
        self.shared.1.notify_all();
    }

    pub fn set_hide_background(&mut self, explorer_pid: u32, hide: bool, user_initiated: bool) {
        let need_worker = if let Ok(mut st) = self.shared.0.lock() {
            let now = Instant::now();
            if explorer_pid != 0 {
                let pid_changed = st.target_pid != 0 && st.target_pid != explorer_pid;
                let exit_kind = st.inspect_old_pid_exit_kind(Some(explorer_pid));
                if st.guard.on_explorer_pid_changed(explorer_pid, exit_kind, now) {
                    st.desired_hidden = false;
                    st.detach_pid(None, true);
                } else {
                    if user_initiated {
                        st.guard.reset_failures_on_user_action();
                        st.not_ready_retries = 0;
                        st.last_inject_try = None;
                    } else if pid_changed {
                        st.detach_pid(None, true);
                        st.reset_retry_state_for_new_pid();
                    }
                    st.desired_hidden = hide && !st.guard.is_tripped();
                }
                st.target_pid = explorer_pid;
                st.track_process_pid(explorer_pid);
            } else {
                if user_initiated && !st.guard.is_tripped() {
                    st.guard.reset_failures_on_user_action();
                    st.not_ready_retries = 0;
                    st.last_inject_try = None;
                }
                st.desired_hidden = hide && !st.guard.is_tripped();
                st.target_pid = 0;
            }
            if !hide {
                st.conflict_detected = false;
                st.osd_suppressed = false;
                st.synced_osd_suppressed = None;
                st.synced_osd_fallback_colors = None;
                st.osd_bounds = None;
                st.tooltip_suppressed = false;
                st.synced_tooltip_suppressed = None;
                st.synced_tooltip_fallback_colors = None;
                st.tooltip_bounds = None;
                st.menu_flyout_suppressed = false;
                st.synced_menu_flyout_suppressed = None;
                st.synced_menu_flyout_fallback_colors = None;
                st.menu_flyout_bounds.clear();
                st.input_switcher_suppressed = false;
                st.synced_input_switcher_suppressed = None;
                st.synced_input_switcher_fallback_colors = None;
                st.input_switcher_bounds = None;
            }
            st.wake_seq = st.wake_seq.wrapping_add(1);
            hide || st.attached_pid.is_some()
        } else {
            false
        };

        if need_worker {
            self.ensure_worker_started();
        }
        self.shared.1.notify_all();
    }

    pub fn maintain(&mut self, explorer_pid: u32) {
        if explorer_pid == 0 {
            return;
        }
        let now = Instant::now();
        let mut should_wake = false;
        if let Ok(mut st) = self.shared.0.lock() {
            let exit_kind = st.inspect_old_pid_exit_kind(Some(explorer_pid));
            if st.guard.on_explorer_pid_changed(explorer_pid, exit_kind, now) {
                st.desired_hidden = false;
                st.detach_pid(None, true);
                st.target_pid = explorer_pid;
                st.track_process_pid(explorer_pid);
                return;
            }

            if (st.target_pid != 0 && st.target_pid != explorer_pid)
                || st.attached_pid.is_some_and(|p| p != explorer_pid)
            {
                st.detach_pid(None, true);
                st.reset_retry_state_for_new_pid();
            }
            st.target_pid = explorer_pid;
            st.track_process_pid(explorer_pid);

            if st.desired_hidden && !st.guard.is_tripped() {
                let ping_due = st
                    .last_ping_ok
                    .is_none_or(|t| now.saturating_duration_since(t) >= MAINTAIN_PING_INTERVAL);
                let cooldown = explorer_inject_retry_cooldown(st.not_ready_retries);
                let need_attach = st.attached_pid != Some(explorer_pid)
                    && st.guard.can_inject()
                    && st
                        .last_inject_try
                        .is_none_or(|t| now.saturating_duration_since(t) >= cooldown);
                if need_attach || (st.attached_pid == Some(explorer_pid) && ping_due) {
                    st.wake_seq = st.wake_seq.wrapping_add(1);
                    should_wake = true;
                }
            }
        }
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.notify_all();
        }
    }

    /// Re-sends `CMD_HIDE` asynchronously if attached and hiding is desired (e.g. when the
    /// overflow flyout opens and may need fresh VisualTree background brushes updated).
    pub fn reassert_hide(&mut self, explorer_pid: u32) {
        if explorer_pid == 0 {
            return;
        }
        let should_wake = if let Ok(mut st) = self.shared.0.lock() {
            if st.desired_hidden
                && !st.guard.is_tripped()
                && st.attached_pid == Some(explorer_pid)
            {
                st.wake_seq = st.wake_seq.wrapping_add(1);
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_wake {
            self.ensure_worker_started();
            self.shared.1.notify_all();
        }
    }

    pub fn begin_shutdown(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.desired_hidden = false;
            st.stop_worker = true;
            st.wake_seq = st.wake_seq.wrapping_add(1);
        }
        self.shared.1.notify_all();
    }

    pub fn finish_shutdown(&mut self, deadline: Instant) {
        let Some(handle) = self.worker.take() else {
            return;
        };
        let timeout = deadline.saturating_duration_since(Instant::now());
        let (lock, _, done_cv) = &*self.shared;
        let exited_cleanly = if let Ok(st) = lock.lock() {
            if st.shutdown_done {
                true
            } else {
                done_cv
                    .wait_timeout_while(st, timeout, |s| !s.shutdown_done)
                    .is_ok_and(|(guard, _)| guard.shutdown_done)
            }
        } else {
            false
        };
        if exited_cleanly {
            let _ = handle.join();
        }
    }

    pub fn shutdown(&mut self) {
        self.begin_shutdown();
        self.finish_shutdown(Instant::now() + Duration::from_millis(800));
    }

    fn ensure_worker_started(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        self.worker = Some(std::thread::spawn(move || run_tap_worker(shared)));
    }
}

impl Drop for TapClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

