//! Windows Shell `IContextMenu3` native file context menu with pre-warming and fast binding.

use std::cell::RefCell;
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::rc::Rc;
use std::sync::Once;
use std::time::Duration;
use windows::core::{Interface, HSTRING, PCSTR, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IContextMenu, IContextMenu2, IContextMenu3, IShellFolder, ILCreateFromPathW, ILFindLastID,
    ILFree, SHBindToParent, CMF_NORMAL, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFOEX,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, SetForegroundWindow, TrackPopupMenuEx, HMENU, SW_SHOWNORMAL,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};

const SHELL_CMD_FIRST: u32 = 100;
const SHELL_CMD_LAST: u32 = 0x7FFF;
const CMIC_MASK_UNICODE: u32 = 0x0000_4000;
const CMF_ITEMMENU_FLAG: u32 = 0x0000_0080;
const CMF_ASYNCVERBSTATE_FLAG: u32 = 0x0000_0400;

static PREWARM_ONCE: Once = Once::new();

/// Result of tracking a native file right-click context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMenuOutcome {
    Cancelled,
    ShellInvoked,
}

/// Active `IContextMenu2`/`3` interfaces during `TrackPopupMenuEx` submenu rendering.
/// Stored in its own `RefCell` outside `DeskBoxInner` so re-entrant menu messages never double-borrow.
#[derive(Default)]
pub struct ActiveShellMenu {
    cm2: Option<IContextMenu2>,
    cm3: Option<IContextMenu3>,
}

impl ActiveShellMenu {
    pub fn try_handle_wnd_msg(
        slot: &Rc<RefCell<Option<Self>>>,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        if !matches!(
            msg,
            WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
        ) {
            return None;
        }
        let guard = slot.try_borrow().ok()?;
        let active = guard.as_ref()?;
        if let Some(ref cm3) = active.cm3 {
            let mut lres = LRESULT(0);
            if unsafe { cm3.HandleMenuMsg2(msg, wparam, lparam, Some(&mut lres)) }.is_ok() {
                return Some(lres);
            }
        }
        if let Some(ref cm2) = active.cm2 {
            if msg != WM_MENUCHAR && unsafe { cm2.HandleMenuMsg(msg, wparam, lparam) }.is_ok() {
                return Some(LRESULT(0));
            }
        }
        None
    }
}

struct OwnedPidl(*mut ITEMIDLIST);

impl Drop for OwnedPidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                ILFree(Some(self.0.cast()));
            }
            self.0 = null_mut();
        }
    }
}

struct OwnedHMenu(HMENU);

impl Drop for OwnedHMenu {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DestroyMenu(self.0);
            }
        }
    }
}

/// Pre-built STA-owned native Windows Shell context menu ready for instant tracking on button release.
pub struct PreparedShellMenu {
    paths: Vec<PathBuf>,
    hmenu: OwnedHMenu,
    cm: IContextMenu,
    cm2: Option<IContextMenu2>,
    cm3: Option<IContextMenu3>,
}

impl PreparedShellMenu {
    pub fn matches_paths(&self, paths: &[PathBuf]) -> bool {
        self.paths == paths
    }
}

/// Pre-warms Windows Shell extension DLLs and COM verb handlers once on a background STA thread
/// so the first user right-click inside DeskBox does not stall UI responsiveness.
pub fn spawn_shell_menu_prewarm_once() {
    PREWARM_ONCE.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("deskbox-menu-prewarm".to_string())
            .spawn(|| {
                std::thread::sleep(Duration::from_millis(600));
                prewarm_shell_extensions();
            });
    });
}

fn prewarm_shell_extensions() {
    unsafe {
        let com_ok = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let file_probe = std::env::current_exe()
            .ok()
            .filter(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\notepad.exe"));
        let dir_probe = PathBuf::from(r"C:\Windows");

        for probe in [&file_probe, &dir_probe] {
            if !probe.exists() {
                continue;
            }
            for _ in 0..2 {
                if let Some(cm) =
                    bind_context_menu_for_paths(HWND::default(), std::slice::from_ref(probe))
                {
                    if let Ok(raw_menu) = CreatePopupMenu() {
                        let hmenu = OwnedHMenu(raw_menu);
                        let _ = cm.QueryContextMenu(
                            hmenu.0,
                            0,
                            SHELL_CMD_FIRST,
                            SHELL_CMD_LAST,
                            CMF_NORMAL | CMF_ITEMMENU_FLAG | CMF_ASYNCVERBSTATE_FLAG,
                        );
                    }
                }
            }
        }
        if com_ok {
            CoUninitialize();
        }
    }
}

/// Binds and queries the native Windows Shell `IContextMenu` for `paths` on the UI STA thread.
///
/// MUST be invoked outside any `DeskBoxInner` borrow!
pub fn prepare_native_file_context_menu(
    hwnd: HWND,
    paths: &[PathBuf],
) -> Option<PreparedShellMenu> {
    if paths.is_empty() {
        return None;
    }

    let cm = bind_context_menu_for_paths(hwnd, paths)?;
    let raw_menu = unsafe { CreatePopupMenu().ok()? };
    let hmenu = OwnedHMenu(raw_menu);

    let qcm_ok = unsafe {
        cm.QueryContextMenu(
            hmenu.0,
            0,
            SHELL_CMD_FIRST,
            SHELL_CMD_LAST,
            CMF_NORMAL | CMF_ITEMMENU_FLAG | CMF_ASYNCVERBSTATE_FLAG,
        )
        .is_ok()
    };
    if !qcm_ok {
        return None;
    }

    let cm2 = cm.cast::<IContextMenu2>().ok();
    let cm3 = cm.cast::<IContextMenu3>().ok();
    Some(PreparedShellMenu {
        paths: paths.to_vec(),
        hmenu,
        cm,
        cm2,
        cm3,
    })
}

/// Displays a previously prepared native Windows Shell context menu at `screen_pt`.
///
/// MUST be invoked outside any `DeskBoxInner` borrow!
pub fn track_prepared_shell_menu(
    hwnd: HWND,
    screen_pt: POINT,
    prepared: PreparedShellMenu,
    active_slot: &Rc<RefCell<Option<ActiveShellMenu>>>,
) -> ShellMenuOutcome {
    if let Ok(mut slot) = active_slot.try_borrow_mut() {
        *slot = Some(ActiveShellMenu {
            cm2: prepared.cm2,
            cm3: prepared.cm3,
        });
    }

    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
    let chosen = unsafe {
        TrackPopupMenuEx(
            prepared.hmenu.0,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            screen_pt.x,
            screen_pt.y,
            hwnd,
            None,
        )
    };

    if let Ok(mut slot) = active_slot.try_borrow_mut() {
        *slot = None;
    }

    let cmd = chosen.0 as u32;
    if !(SHELL_CMD_FIRST..=SHELL_CMD_LAST).contains(&cmd) {
        return ShellMenuOutcome::Cancelled;
    }

    let offset = (cmd - SHELL_CMD_FIRST) as usize;
    let ici = CMINVOKECOMMANDINFOEX {
        cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
        hwnd,
        lpVerb: PCSTR(offset as *const u8),
        lpVerbW: PCWSTR(offset as *const u16),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: screen_pt,
        ..Default::default()
    };
    let _ = unsafe {
        prepared
            .cm
            .InvokeCommand((&ici as *const CMINVOKECOMMANDINFOEX).cast())
    };
    ShellMenuOutcome::ShellInvoked
}

fn bind_context_menu_for_paths(hwnd: HWND, paths: &[PathBuf]) -> Option<IContextMenu> {
    let first_existing = paths.iter().find(|p| p.exists())?;
    let first_parent_key = first_existing
        .parent()
        .map(|p| super::config::normalize_path_key(&p.to_string_lossy()));
    let targets: Vec<&PathBuf> = if let Some(ref parent_key) = first_parent_key {
        paths
            .iter()
            .filter(|p| {
                p.exists()
                    && p.parent().is_some_and(|par| {
                        super::config::normalize_path_key(&par.to_string_lossy()) == *parent_key
                    })
            })
            .collect()
    } else {
        // Drive roots (e.g. `C:\`) have no Path::parent(); bind the single item under This PC.
        vec![first_existing]
    };
    if targets.is_empty() {
        return None;
    }

    let mut owned_full_pidls: Vec<OwnedPidl> = Vec::with_capacity(targets.len());
    let mut parent_folder: Option<IShellFolder> = None;
    let mut child_pidls: Vec<*const ITEMIDLIST> = Vec::with_capacity(targets.len());

    for path in targets {
        let wide = HSTRING::from(path.as_os_str());
        let full_raw = unsafe { ILCreateFromPathW(&wide) };
        if full_raw.is_null() {
            continue;
        }
        let owned = OwnedPidl(full_raw);
        let child_ptr = if parent_folder.is_none() {
            let mut ptr: *mut ITEMIDLIST = null_mut();
            let Ok(folder) =
                (unsafe { SHBindToParent::<IShellFolder>(owned.0, Some(&mut ptr)) })
            else {
                continue;
            };
            parent_folder = Some(folder);
            ptr
        } else {
            unsafe { ILFindLastID(owned.0) }
        };
        if child_ptr.is_null() {
            continue;
        }
        child_pidls.push(child_ptr.cast_const());
        owned_full_pidls.push(owned);
    }

    let folder = parent_folder?;
    if child_pidls.is_empty() {
        return None;
    }
    unsafe { folder.GetUIObjectOf::<_, IContextMenu>(hwnd, &child_pidls, None).ok() }
}
