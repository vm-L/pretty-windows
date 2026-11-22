use super::TapWorkerShared;
use super::pipe_io::{inject_and_wait_pipe, query_card_rect, query_menu_flyout_rects, send_alt_tab_item_style, send_command, send_input_switcher_fallback_style, send_input_switcher_suppress, send_menu_flyout_fallback_style, send_menu_flyout_suppress, send_osd_fallback_style, send_osd_suppress, send_tooltip_fallback_style, send_tooltip_suppress, send_virtual_desktop_bar_style};
use super::super::inject::{is_conflicting_taskbar_tool_running, is_process_alive, shadow_cache_dir, AltTabBoundsCache, ComMtaGuard, XamlDiagInjectStatus, CMD_ALTTAB_RECT, CMD_HIDE, CMD_HOVER_FLYOUT_RECT, CMD_INPUT_SWITCHER_RECT, CMD_OSD_RECT, CMD_PING, CMD_RESTORE, CMD_TOOLTIP_RECT, CONFLICT_RECHECK_COOLDOWN};
use super::super::{cleanup_stale_shadow_dlls, explorer_inject_retry_cooldown};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub(super) fn run_tap_worker(shared: Arc<(Mutex<TapWorkerShared>, Condvar, Condvar)>) {
    let _com = ComMtaGuard::enter();
    cleanup_stale_shadow_dlls(&shadow_cache_dir(), None);
    let (lock, wake_cv, done_cv) = &*shared;
    loop {
        let (
            desired_hidden,
            osd_suppressed,
            tooltip_suppressed,
            input_switcher_suppressed,
            menu_flyout_suppressed,
            target_pid,
            attached_pid,
            stop_worker,
            need_cmd_sync,
            want_alt_tab_query,
            want_hover_flyout_query,
            want_osd_query,
            want_tooltip_query,
            want_input_switcher_query,
            want_menu_flyout_query,
        ) = {
            let Ok(mut st) = lock.lock() else {
                break;
            };
            while !st.stop_worker
                && st.wake_seq == st.last_processed_seq
                && st.alt_tab_query_seq == st.last_alt_tab_query_seq
                && st.hover_flyout_query_seq == st.last_hover_flyout_query_seq
                && st.osd_query_seq == st.last_osd_query_seq
                && st.tooltip_query_seq == st.last_tooltip_query_seq
                && st.input_switcher_query_seq == st.last_input_switcher_query_seq
                && st.menu_flyout_query_seq == st.last_menu_flyout_query_seq
            {
                st = match wake_cv.wait(st) {
                    Ok(next) => next,
                    Err(_) => return,
                };
            }
            let cmd_dirty = st.wake_seq != st.last_processed_seq;
            let rect_dirty = st.alt_tab_query_seq != st.last_alt_tab_query_seq;
            let hover_rect_dirty =
                st.hover_flyout_query_seq != st.last_hover_flyout_query_seq;
            let osd_rect_dirty = st.osd_query_seq != st.last_osd_query_seq;
            let tooltip_rect_dirty = st.tooltip_query_seq != st.last_tooltip_query_seq;
            let input_switcher_rect_dirty =
                st.input_switcher_query_seq != st.last_input_switcher_query_seq;
            let menu_flyout_rect_dirty =
                st.menu_flyout_query_seq != st.last_menu_flyout_query_seq;
            st.last_processed_seq = st.wake_seq;
            st.last_alt_tab_query_seq = st.alt_tab_query_seq;
            st.last_hover_flyout_query_seq = st.hover_flyout_query_seq;
            st.last_osd_query_seq = st.osd_query_seq;
            st.last_tooltip_query_seq = st.tooltip_query_seq;
            st.last_input_switcher_query_seq = st.input_switcher_query_seq;
            st.last_menu_flyout_query_seq = st.menu_flyout_query_seq;
            (
                st.desired_hidden && !st.guard.is_tripped(),
                st.osd_suppressed,
                st.tooltip_suppressed,
                st.input_switcher_suppressed,
                st.menu_flyout_suppressed,
                st.target_pid,
                st.attached_pid,
                st.stop_worker,
                cmd_dirty,
                rect_dirty,
                hover_rect_dirty,
                osd_rect_dirty,
                tooltip_rect_dirty,
                input_switcher_rect_dirty,
                menu_flyout_rect_dirty,
            )
        };

        if stop_worker {
            let pid = attached_pid.unwrap_or(target_pid);
            if pid != 0 && (attached_pid.is_some() || send_command(pid, CMD_PING, 30)) {
                let _ = send_command(pid, CMD_RESTORE, 250);
            }
            if let Ok(mut st) = lock.lock() {
                st.detach_pid(None, true);
                st.osd_suppressed = false;
                st.tooltip_suppressed = false;
                st.input_switcher_suppressed = false;
                st.menu_flyout_suppressed = false;
                st.shutdown_done = true;
            }
            cleanup_stale_shadow_dlls(&shadow_cache_dir(), None);
            done_cv.notify_all();
            break;
        }

        if target_pid == 0 {
            continue;
        }

        let mut cur_attached = attached_pid.filter(|&p| p == target_pid);
        let mut did_cmd_sync = false;
        let mut just_attached = false;
        if let Some(pid) = cur_attached {
            if need_cmd_sync {
                let latest_hide = lock
                    .lock()
                    .map(|st| st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker)
                    .unwrap_or(false);
                let cmd = if latest_hide { CMD_HIDE } else { CMD_RESTORE };
                if send_command(pid, cmd, 250) {
                    did_cmd_sync = true;
                    if let Ok(mut st) = lock.lock() {
                        st.last_ping_ok = Some(Instant::now());
                        st.guard.record_success();
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if cur_attached.is_none() && send_command(target_pid, CMD_PING, 40) {
            cur_attached = Some(target_pid);
            just_attached = true;
            if let Ok(mut st) = lock.lock() {
                st.attached_pid = Some(target_pid);
                st.last_ping_ok = Some(Instant::now());
                st.clear_synced_flags();
                st.not_ready_retries = 0;
                st.conflict_detected = false;
                st.track_process_pid(target_pid);
                st.guard.record_success();
            }
        }

        if cur_attached.is_none() && desired_hidden {
            let can_try = lock
                .lock()
                .map(|st| {
                    let cooldown = explorer_inject_retry_cooldown(st.not_ready_retries);
                    st.guard.can_inject()
                        && st.last_inject_try.is_none_or(|t| t.elapsed() >= cooldown)
                        && (!st.conflict_detected
                            || st
                                .last_inject_try
                                .is_none_or(|t| t.elapsed() >= CONFLICT_RECHECK_COOLDOWN))
                })
                .unwrap_or(false);

            if can_try {
                if is_conflicting_taskbar_tool_running() {
                    if let Ok(mut st) = lock.lock() {
                        st.conflict_detected = true;
                        st.last_inject_try = Some(Instant::now());
                    }
                    continue;
                }

                if let Ok(mut st) = lock.lock() {
                    st.conflict_detected = false;
                    st.track_process_pid(target_pid);
                    let now = Instant::now();
                    st.last_inject_try = Some(now);
                    st.guard.record_inject_attempt(target_pid, now);
                }

                match inject_and_wait_pipe(target_pid) {
                    XamlDiagInjectStatus::Connected => {
                        cur_attached = Some(target_pid);
                        just_attached = true;
                        if let Ok(mut st) = lock.lock() {
                            st.attached_pid = Some(target_pid);
                            st.last_ping_ok = Some(Instant::now());
                            st.clear_synced_flags();
                            st.not_ready_retries = 0;
                            st.guard.record_success();
                        }
                    }
                    XamlDiagInjectStatus::EndpointNotReady => {
                        let alive = unsafe { is_process_alive(target_pid) };
                        if let Ok(mut st) = lock.lock() {
                            if alive {
                                st.guard.clear_failed_attempt_if_same_pid(target_pid);
                                st.not_ready_retries = st.not_ready_retries.saturating_add(1);
                            } else {
                                st.guard.record_failure();
                            }
                        }
                    }
                    XamlDiagInjectStatus::Failed => {
                        let alive = unsafe { is_process_alive(target_pid) };
                        if let Ok(mut st) = lock.lock() {
                            if alive {
                                st.guard.clear_failed_attempt_if_same_pid(target_pid);
                            }
                            st.guard.record_failure();
                        }
                    }
                }
            }
        }

        if !did_cmd_sync {
            if let Some(pid) = cur_attached {
                if just_attached
                    || need_cmd_sync
                    || (!want_alt_tab_query
                        && !want_hover_flyout_query
                        && !want_osd_query
                        && !want_tooltip_query
                        && !want_input_switcher_query
                        && !want_menu_flyout_query)
                {
                    let latest_hide = lock
                        .lock()
                        .map(|st| {
                            st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker
                        })
                        .unwrap_or(false);
                    let cmd = if latest_hide { CMD_HIDE } else { CMD_RESTORE };
                    if send_command(pid, cmd, 250) {
                        let notify = if let Ok(mut st) = lock.lock() {
                            st.last_ping_ok = Some(Instant::now());
                            if just_attached {
                                (st.notify_hwnd_raw, st.notify_msg)
                            } else {
                                (0, 0)
                            }
                        } else {
                            (0, 0)
                        };
                        if notify.0 != 0 && notify.1 != 0 {
                            unsafe {
                                let _ = PostMessageW(
                                    HWND(notify.0 as *mut std::ffi::c_void),
                                    notify.1,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        }
                    } else {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            st.detach_pid(Some(pid), true);
                        }
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_colors = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.alt_tab_item_colors != st.synced_alt_tab_item_colors)
                    .then_some(st.alt_tab_item_colors)
                    .flatten()
            });
            if let Some((bg_argb, border_argb)) = desired_colors {
                if send_alt_tab_item_style(pid, bg_argb, border_argb, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_alt_tab_item_colors = Some((bg_argb, border_argb));
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_osd_colors = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.osd_fallback_colors != st.synced_osd_fallback_colors)
                    .then_some(st.osd_fallback_colors)
                    .flatten()
            });
            if let Some((bg_argb, border_argb)) = desired_osd_colors {
                if send_osd_fallback_style(pid, bg_argb, border_argb, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_osd_fallback_colors = Some((bg_argb, border_argb));
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_osd_suppress = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.synced_osd_suppressed != Some(st.osd_suppressed))
                    .then_some(st.osd_suppressed)
            });
            if let Some(suppressed) = desired_osd_suppress {
                if send_osd_suppress(pid, suppressed, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_osd_suppressed = Some(suppressed);
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_tooltip_colors = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.tooltip_fallback_colors != st.synced_tooltip_fallback_colors)
                    .then_some(st.tooltip_fallback_colors)
                    .flatten()
            });
            if let Some((bg_argb, border_argb)) = desired_tooltip_colors {
                if send_tooltip_fallback_style(pid, bg_argb, border_argb, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_tooltip_fallback_colors = Some((bg_argb, border_argb));
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_tooltip_suppress = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.synced_tooltip_suppressed != Some(st.tooltip_suppressed))
                    .then_some(st.tooltip_suppressed)
            });
            if let Some(suppressed) = desired_tooltip_suppress {
                if send_tooltip_suppress(pid, suppressed, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_tooltip_suppressed = Some(suppressed);
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_menu_colors = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.menu_flyout_fallback_colors != st.synced_menu_flyout_fallback_colors)
                    .then_some(st.menu_flyout_fallback_colors)
                    .flatten()
            });
            if let Some((bg_argb, border_argb)) = desired_menu_colors {
                if send_menu_flyout_fallback_style(pid, bg_argb, border_argb, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_menu_flyout_fallback_colors = Some((bg_argb, border_argb));
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_vdb_colors = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.virtual_desktop_bar_colors != st.synced_virtual_desktop_bar_colors)
                    .then_some(st.virtual_desktop_bar_colors)
                    .flatten()
            });
            if let Some((bg_argb, border_argb)) = desired_vdb_colors {
                if send_virtual_desktop_bar_style(pid, bg_argb, border_argb, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_virtual_desktop_bar_colors = Some((bg_argb, border_argb));
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_input_switcher_colors = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.input_switcher_fallback_colors != st.synced_input_switcher_fallback_colors)
                    .then_some(st.input_switcher_fallback_colors)
                    .flatten()
            });
            if let Some((bg_argb, border_argb)) = desired_input_switcher_colors {
                if send_input_switcher_fallback_style(pid, bg_argb, border_argb, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_input_switcher_fallback_colors = Some((bg_argb, border_argb));
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_input_switcher_suppress = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.synced_input_switcher_suppressed != Some(st.input_switcher_suppressed))
                    .then_some(st.input_switcher_suppressed)
            });
            if let Some(suppressed) = desired_input_switcher_suppress {
                if send_input_switcher_suppress(pid, suppressed, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_input_switcher_suppressed = Some(suppressed);
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if let Some(pid) = cur_attached {
            let desired_menu_suppress = lock.lock().ok().and_then(|st| {
                (st.desired_hidden
                    && !st.guard.is_tripped()
                    && !st.stop_worker
                    && st.synced_menu_flyout_suppressed != Some(st.menu_flyout_suppressed))
                    .then_some(st.menu_flyout_suppressed)
            });
            if let Some(suppressed) = desired_menu_suppress {
                if send_menu_flyout_suppress(pid, suppressed, 250) {
                    if let Ok(mut st) = lock.lock() {
                        st.synced_menu_flyout_suppressed = Some(suppressed);
                        st.last_ping_ok = Some(Instant::now());
                    }
                } else {
                    cur_attached = None;
                    if let Ok(mut st) = lock.lock() {
                        st.detach_pid(Some(pid), true);
                    }
                }
            }
        }

        if want_alt_tab_query {
            let query_pid = cur_attached.filter(|_| {
                lock.lock()
                    .map(|st| st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker)
                    .unwrap_or(false)
            });
            if let Some(pid) = query_pid {
                match query_card_rect(pid, CMD_ALTTAB_RECT, 120) {
                    Ok(Some((thread_id, rect_dip))) => {
                        let notify = if let Ok(mut st) = lock.lock() {
                            st.alt_tab_bounds = Some(AltTabBoundsCache {
                                thread_id,
                                rect_dip,
                                updated_at: Instant::now(),
                            });
                            st.last_ping_ok = Some(Instant::now());
                            (st.notify_hwnd_raw, st.notify_msg)
                        } else {
                            (0, 0)
                        };
                        if notify.0 != 0 && notify.1 != 0 {
                            unsafe {
                                let _ = PostMessageW(
                                    HWND(notify.0 as *mut std::ffi::c_void),
                                    notify.1,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        if let Ok(mut st) = lock.lock() {
                            st.alt_tab_bounds = None;
                        }
                    }
                    Err(()) => {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            st.detach_pid(Some(pid), true);
                        }
                    }
                }
            } else if let Ok(mut st) = lock.lock() {
                st.alt_tab_bounds = None;
            }
        }

        if want_hover_flyout_query {
            let query_pid = cur_attached.filter(|_| {
                lock.lock()
                    .map(|st| st.desired_hidden && !st.guard.is_tripped() && !st.stop_worker)
                    .unwrap_or(false)
            });
            if let Some(pid) = query_pid {
                match query_card_rect(pid, CMD_HOVER_FLYOUT_RECT, 120) {
                    Ok(Some((thread_id, rect_dip))) => {
                        let notify = if let Ok(mut st) = lock.lock() {
                            let changed = match st.hover_flyout_bounds {
                                None => true,
                                Some(prev) => {
                                    prev.thread_id != thread_id
                                        || (prev.rect_dip[0] - rect_dip[0]).abs() > 0.25
                                        || (prev.rect_dip[1] - rect_dip[1]).abs() > 0.25
                                        || (prev.rect_dip[2] - rect_dip[2]).abs() > 0.25
                                        || (prev.rect_dip[3] - rect_dip[3]).abs() > 0.25
                                }
                            };
                            st.hover_flyout_bounds = Some(AltTabBoundsCache {
                                thread_id,
                                rect_dip,
                                updated_at: Instant::now(),
                            });
                            st.last_ping_ok = Some(Instant::now());
                            if changed {
                                (st.notify_hwnd_raw, st.notify_msg)
                            } else {
                                (0, 0)
                            }
                        } else {
                            (0, 0)
                        };
                        if notify.0 != 0 && notify.1 != 0 {
                            unsafe {
                                let _ = PostMessageW(
                                    HWND(notify.0 as *mut std::ffi::c_void),
                                    notify.1,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        let notify = if let Ok(mut st) = lock.lock() {
                            let had_bounds = st.hover_flyout_bounds.take().is_some();
                            if had_bounds {
                                (st.notify_hwnd_raw, st.notify_msg)
                            } else {
                                (0, 0)
                            }
                        } else {
                            (0, 0)
                        };
                        if notify.0 != 0 && notify.1 != 0 {
                            unsafe {
                                let _ = PostMessageW(
                                    HWND(notify.0 as *mut std::ffi::c_void),
                                    notify.1,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        }
                    }
                    Err(()) => {
                        cur_attached = None;
                        if let Ok(mut st) = lock.lock() {
                            st.detach_pid(Some(pid), true);
                        }
                    }
                }
            } else if let Ok(mut st) = lock.lock() {
                st.hover_flyout_bounds = None;
            }
        }

        if want_osd_query {
            sync_suppressed_card_rect_query(
                lock,
                cur_attached,
                CMD_OSD_RECT,
                osd_suppressed,
                |st| st.osd_suppressed,
                |st| &mut st.osd_bounds,
            );
        }

        if want_tooltip_query {
            sync_suppressed_card_rect_query(
                lock,
                cur_attached,
                CMD_TOOLTIP_RECT,
                tooltip_suppressed,
                |st| st.tooltip_suppressed,
                |st| &mut st.tooltip_bounds,
            );
        }

        if want_input_switcher_query {
            sync_suppressed_card_rect_query(
                lock,
                cur_attached,
                CMD_INPUT_SWITCHER_RECT,
                input_switcher_suppressed,
                |st| st.input_switcher_suppressed,
                |st| &mut st.input_switcher_bounds,
            );
        }

        if want_menu_flyout_query {
            sync_menu_flyout_rects_query(lock, cur_attached, menu_flyout_suppressed);
        }
    }
}

fn sync_menu_flyout_rects_query(
    lock: &Mutex<TapWorkerShared>,
    cur_attached: Option<u32>,
    cached_suppressed: bool,
) {
    let query_pid = if cached_suppressed {
        None
    } else {
        cur_attached.filter(|_| {
            lock.lock()
                .map(|st| {
                    st.desired_hidden
                        && !st.menu_flyout_suppressed
                        && !st.guard.is_tripped()
                        && !st.stop_worker
                })
                .unwrap_or(false)
        })
    };
    if let Some(pid) = query_pid {
        match query_menu_flyout_rects(pid, 120) {
            Ok(rects) => {
                let now = Instant::now();
                let next_bounds: Vec<AltTabBoundsCache> = rects
                    .into_iter()
                    .map(|(thread_id, rect_dip)| AltTabBoundsCache {
                        thread_id,
                        rect_dip,
                        updated_at: now,
                    })
                    .collect();
                let notify = if let Ok(mut st) = lock.lock() {
                    let changed = st.menu_flyout_bounds.len() != next_bounds.len()
                        || st.menu_flyout_bounds.iter().zip(next_bounds.iter()).any(
                            |(prev, next)| {
                                prev.thread_id != next.thread_id
                                    || (prev.rect_dip[0] - next.rect_dip[0]).abs() > 0.25
                                    || (prev.rect_dip[1] - next.rect_dip[1]).abs() > 0.25
                                    || (prev.rect_dip[2] - next.rect_dip[2]).abs() > 0.25
                                    || (prev.rect_dip[3] - next.rect_dip[3]).abs() > 0.25
                            },
                        );
                    st.menu_flyout_bounds = next_bounds;
                    st.last_ping_ok = Some(now);
                    if changed {
                        (st.notify_hwnd_raw, st.notify_msg)
                    } else {
                        (0, 0)
                    }
                } else {
                    (0, 0)
                };
                if notify.0 != 0 && notify.1 != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            HWND(notify.0 as *mut std::ffi::c_void),
                            notify.1,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            Err(()) => {
                if let Ok(mut st) = lock.lock() {
                    st.detach_pid(Some(pid), true);
                }
            }
        }
    } else if let Ok(mut st) = lock.lock() {
        st.menu_flyout_bounds.clear();
    }
}

fn sync_suppressed_card_rect_query(
    lock: &Mutex<TapWorkerShared>,
    cur_attached: Option<u32>,
    cmd: u8,
    cached_suppressed: bool,
    get_suppressed: fn(&TapWorkerShared) -> bool,
    get_bounds_mut: fn(&mut TapWorkerShared) -> &mut Option<AltTabBoundsCache>,
) {
    let query_pid = if cached_suppressed {
        None
    } else {
        cur_attached.filter(|_| {
            lock.lock()
                .map(|st| {
                    st.desired_hidden
                        && !get_suppressed(&st)
                        && !st.guard.is_tripped()
                        && !st.stop_worker
                })
                .unwrap_or(false)
        })
    };
    if let Some(pid) = query_pid {
        match query_card_rect(pid, cmd, 120) {
            Ok(Some((thread_id, rect_dip))) => {
                let notify = if let Ok(mut st) = lock.lock() {
                    let slot = get_bounds_mut(&mut st);
                    let changed = match *slot {
                        None => true,
                        Some(prev) => {
                            prev.thread_id != thread_id
                                || (prev.rect_dip[0] - rect_dip[0]).abs() > 0.25
                                || (prev.rect_dip[1] - rect_dip[1]).abs() > 0.25
                                || (prev.rect_dip[2] - rect_dip[2]).abs() > 0.25
                                || (prev.rect_dip[3] - rect_dip[3]).abs() > 0.25
                        }
                    };
                    *slot = Some(AltTabBoundsCache {
                        thread_id,
                        rect_dip,
                        updated_at: Instant::now(),
                    });
                    st.last_ping_ok = Some(Instant::now());
                    if changed {
                        (st.notify_hwnd_raw, st.notify_msg)
                    } else {
                        (0, 0)
                    }
                } else {
                    (0, 0)
                };
                if notify.0 != 0 && notify.1 != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            HWND(notify.0 as *mut std::ffi::c_void),
                            notify.1,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            Ok(None) => {
                let notify = if let Ok(mut st) = lock.lock() {
                    let had_bounds = get_bounds_mut(&mut st).take().is_some();
                    if had_bounds {
                        (st.notify_hwnd_raw, st.notify_msg)
                    } else {
                        (0, 0)
                    }
                } else {
                    (0, 0)
                };
                if notify.0 != 0 && notify.1 != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            HWND(notify.0 as *mut std::ffi::c_void),
                            notify.1,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            Err(()) => {
                if let Ok(mut st) = lock.lock() {
                    st.detach_pid(Some(pid), true);
                }
            }
        }
    } else if let Ok(mut st) = lock.lock() {
        *get_bounds_mut(&mut st) = None;
    }
}


