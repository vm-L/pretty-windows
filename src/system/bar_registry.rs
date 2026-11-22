use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible, PostMessageW};

#[derive(Clone, Copy, Default)]
struct TaskbarBarSlotEntry {
    hwnd_raw: isize,
    resync_msg: u32,
    span: Option<(i32, i32)>,
}

fn taskbar_bar_registry() -> &'static Mutex<[TaskbarBarSlotEntry; 2]> {
    static REGISTRY: OnceLock<Mutex<[TaskbarBarSlotEntry; 2]>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new([TaskbarBarSlotEntry::default(); 2]))
}

/// Registers a taskbar widget window handle and its async resync `WM_APP_*` message ID.
/// Preserves any bounds span published during window creation before handle registration.
pub fn register_taskbar_bar(slot: super::TaskbarBarSlot, hwnd: HWND, resync_msg: u32) {
    let mut guard = match taskbar_bar_registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let entry = &mut guard[slot.index()];
    entry.hwnd_raw = hwnd.0 as isize;
    entry.resync_msg = resync_msg;
}

/// Unregisters a taskbar widget handle (only if matching `hwnd`) and notifies its peer
/// to re-evaluate whether it can return toward its saved position.
pub fn unregister_taskbar_bar(slot: super::TaskbarBarSlot, hwnd: HWND) {
    let peer_to_notify = {
        let mut guard = match taskbar_bar_registry().lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let idx = slot.index();
        if guard[idx].hwnd_raw == hwnd.0 as isize {
            guard[idx] = TaskbarBarSlotEntry::default();
        }
        let peer = guard[slot.peer().index()];
        if peer.hwnd_raw != 0 && peer.resync_msg != 0 {
            Some((
                HWND(peer.hwnd_raw as *mut core::ffi::c_void),
                peer.resync_msg,
            ))
        } else {
            None
        }
    };
    if let Some((peer_hwnd, msg)) = peer_to_notify {
        unsafe {
            if IsWindow(peer_hwnd).as_bool() {
                let _ = PostMessageW(peer_hwnd, msg, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Updates the current horizontal screen span `(left, right)` of `slot` (or `None` when hidden).
pub fn update_taskbar_bar_bounds(slot: super::TaskbarBarSlot, span: Option<(i32, i32)>) {
    let mut guard = match taskbar_bar_registry().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard[slot.index()].span = span.filter(|&(l, r)| r > l);
}

/// Returns the peer bar's active horizontal screen span `(left, right)`, ignoring stale
/// or hidden windows once the peer handle has been registered.
pub fn peer_taskbar_bar_span(slot: super::TaskbarBarSlot) -> Option<(i32, i32)> {
    let peer = {
        let guard = match taskbar_bar_registry().lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard[slot.peer().index()]
    };
    let span = peer.span?;
    if peer.hwnd_raw != 0 {
        let hwnd = HWND(peer.hwnd_raw as *mut core::ffi::c_void);
        let alive_and_visible =
            unsafe { IsWindow(hwnd).as_bool() && IsWindowVisible(hwnd).as_bool() };
        if !alive_and_visible {
            return None;
        }
    }
    Some(span)
}

/// Asynchronously notifies the active peer bar to re-evaluate its position.
/// Must only be called after user drag commit, position reset, config width change, or close.
pub fn notify_peer_taskbar_bar_resync(slot: super::TaskbarBarSlot) {
    let peer_to_notify = {
        let guard = match taskbar_bar_registry().lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let peer = guard[slot.peer().index()];
        if peer.hwnd_raw != 0 && peer.resync_msg != 0 {
            Some((
                HWND(peer.hwnd_raw as *mut core::ffi::c_void),
                peer.resync_msg,
            ))
        } else {
            None
        }
    };
    if let Some((peer_hwnd, msg)) = peer_to_notify {
        unsafe {
            if IsWindow(peer_hwnd).as_bool() {
                let _ = PostMessageW(peer_hwnd, msg, WPARAM(0), LPARAM(0));
            }
        }
    }
}
