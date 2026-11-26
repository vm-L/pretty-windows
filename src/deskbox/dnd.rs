//! Bi-directional OLE drag-and-drop (`IDropTarget` for drop-in and `DoDragDrop` for drag-out).

use std::cell::Cell;
use std::collections::VecDeque;
use std::mem::{size_of, ManuallyDrop};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr::{copy_nonoverlapping, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use windows::core::{implement, w, Result};
use windows::Win32::Foundation::{
    GlobalFree, BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
    DV_E_FORMATETC, E_NOTIMPL, HGLOBAL, HWND, LPARAM, OLE_E_ADVISENOTSUPPORTED, POINT, POINTL,
    S_FALSE, S_OK, WPARAM,
};
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumFORMATETC_Impl,
    IEnumSTATDATA, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0,
    TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
};
use windows::Win32::System::Ole::{
    DoDragDrop, ReleaseStgMedium, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE,
    DROPEFFECT_NONE, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl,
};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::{DragQueryFileW, DROPFILES, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, PostMessageW};

/// `WPARAM` event sub-codes posted via `msg_drop_notify` to the owning DeskBox window.
pub const DROP_EVENT_ENTER: usize = 1;
pub const DROP_EVENT_LEAVE: usize = 2;
pub const DROP_EVENT_FILES: usize = 3;

/// Heap-allocated payload delivered through `LPARAM` when `WPARAM == DROP_EVENT_FILES`.
pub struct DroppedFilesPayload {
    pub paths: Vec<PathBuf>,
    pub copy: bool,
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn preferred_drop_effect_fmt() -> u16 {
    static FMT: OnceLock<u16> = OnceLock::new();
    *FMT.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16 })
}

fn performed_drop_effect_fmt() -> u16 {
    static FMT: OnceLock<u16> = OnceLock::new();
    *FMT.get_or_init(|| unsafe { RegisterClipboardFormatW(w!("Performed DropEffect")) as u16 })
}

fn logical_performed_drop_effect_fmt() -> u16 {
    static FMT: OnceLock<u16> = OnceLock::new();
    *FMT.get_or_init(|| unsafe {
        RegisterClipboardFormatW(w!("Logical Performed DropEffect")) as u16
    })
}

fn preferred_drop_effect_format() -> FORMATETC {
    FORMATETC {
        cfFormat: preferred_drop_effect_fmt(),
        ptd: null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn build_drop_effect_hglobal(effect: DROPEFFECT) -> Option<HGLOBAL> {
    unsafe {
        let hglobal = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size_of::<u32>()).ok()?;
        let ptr = GlobalLock(hglobal);
        if ptr.is_null() {
            let _ = GlobalFree(hglobal);
            return None;
        }
        *(ptr as *mut u32) = effect.0;
        let _ = GlobalUnlock(hglobal);
        Some(hglobal)
    }
}

/// Signals an OLE drag source using `CFSTR_PERFORMEDDROPEFFECT` / `CFSTR_LOGICALPERFORMEDDROPEFFECT`.
/// For an optimized move (target moves files asynchronously via `SHFileOperationW`), setting
/// `CFSTR_PERFORMEDDROPEFFECT = DROPEFFECT_NONE` and `*pdweffect = DROPEFFECT_NONE` prevents
/// Explorer and third-party drag sources from deleting originals before our move runs (or if canceled).
fn notify_source_drop_effect(data_obj: &IDataObject, cf: u16, effect: DROPEFFECT) {
    if cf == 0 {
        return;
    }
    let Some(hglobal) = build_drop_effect_hglobal(effect) else {
        return;
    };
    let fmt = FORMATETC {
        cfFormat: cf,
        ptd: null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut med = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: hglobal },
        pUnkForRelease: ManuallyDrop::new(None),
    };
    // When `fRelease = TRUE`, the data object takes ownership only if `SetData` succeeds.
    if unsafe { data_obj.SetData(&fmt, &med, true) }.is_err() {
        unsafe {
            ReleaseStgMedium(&mut med);
        }
    }
}

fn extract_hdrop_paths(data_obj: &IDataObject, max_count: usize) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let fmt = hdrop_format();
    let Ok(mut med) = (unsafe { data_obj.GetData(&fmt) }) else {
        return paths;
    };
    if med.tymed == TYMED_HGLOBAL.0 as u32 {
        unsafe {
            let hglobal = med.u.hGlobal;
            if !hglobal.0.is_null() {
                let hdrop = HDROP(hglobal.0);
                let total = DragQueryFileW(hdrop, u32::MAX, None) as usize;
                let limit = total.min(max_count);
                for i in 0..limit {
                    let len = DragQueryFileW(hdrop, i as u32, None);
                    if len == 0 {
                        continue;
                    }
                    let mut buf = vec![0u16; (len as usize) + 1];
                    let copied = DragQueryFileW(hdrop, i as u32, Some(&mut buf));
                    if copied > 0 {
                        let s = String::from_utf16_lossy(&buf[..copied as usize]);
                        if !s.is_empty() {
                            paths.push(PathBuf::from(s));
                        }
                    }
                }
            }
        }
    }
    unsafe {
        ReleaseStgMedium(&mut med);
    }
    paths
}

/// Shared state updated cleanly by `BoxWnd` when its kind changes, plus a leak-free
/// queue for dropped file payloads consumed on `WM_APP_DESKBOX_DROP`.
pub struct DropTargetShared {
    pub is_collection: AtomicBool,
    pending_drops: Mutex<VecDeque<DroppedFilesPayload>>,
}

impl DropTargetShared {
    pub fn new(is_collection: bool) -> Self {
        Self {
            is_collection: AtomicBool::new(is_collection),
            pending_drops: Mutex::new(VecDeque::new()),
        }
    }

    pub fn push_drop(&self, payload: DroppedFilesPayload) {
        if let Ok(mut q) = self.pending_drops.lock() {
            q.push_back(payload);
        }
    }

    pub fn take_drops(&self) -> Vec<DroppedFilesPayload> {
        if let Ok(mut q) = self.pending_drops.lock() {
            q.drain(..).collect()
        } else {
            Vec::new()
        }
    }
}

/// `IDropTarget` COM object registered on each DeskBox window.
#[implement(IDropTarget)]
pub struct BoxDropTarget {
    box_hwnd: HWND,
    msg_drop_notify: u32,
    shared: Arc<DropTargetShared>,
    valid_drop: Cell<bool>,
}

impl BoxDropTarget {
    pub fn new(box_hwnd: HWND, msg_drop_notify: u32, shared: Arc<DropTargetShared>) -> Self {
        Self {
            box_hwnd,
            msg_drop_notify,
            shared,
            valid_drop: Cell::new(false),
        }
    }

    fn compute_effect(
        &self,
        key_state: MODIFIERKEYS_FLAGS,
        allowed: DROPEFFECT,
    ) -> DROPEFFECT {
        if !self.valid_drop.get() {
            return DROPEFFECT_NONE;
        }
        // Collection boxes ONLY reference paths and MUST always return COPY so Explorer never deletes originals!
        if self.shared.is_collection.load(Ordering::Relaxed) {
            return if (allowed.0 & DROPEFFECT_COPY.0) != 0 {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        // FolderPortal boxes move dropped files by default (even across volumes),
        // unless Ctrl is explicitly held or the drag source disallows MOVE.
        if (key_state.0 & MK_CONTROL.0) != 0 && (allowed.0 & DROPEFFECT_COPY.0) != 0 {
            return DROPEFFECT_COPY;
        }
        if (allowed.0 & DROPEFFECT_MOVE.0) != 0 {
            return DROPEFFECT_MOVE;
        }
        if (allowed.0 & DROPEFFECT_COPY.0) != 0 {
            return DROPEFFECT_COPY;
        }
        DROPEFFECT_NONE
    }

    fn post_hover_event(&self, event: usize) {
        unsafe {
            if !self.box_hwnd.is_invalid() && IsWindow(self.box_hwnd).as_bool() {
                let _ = PostMessageW(self.box_hwnd, self.msg_drop_notify, WPARAM(event), LPARAM(0));
            }
        }
    }
}

impl IDropTarget_Impl for BoxDropTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: Option<&IDataObject>,
        grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> Result<()> {
        let mut valid = false;
        if let Some(obj) = pdataobj {
            let fmt = hdrop_format();
            if unsafe { obj.QueryGetData(&fmt) } == S_OK {
                valid = true;
            }
        }
        self.valid_drop.set(valid);
        let allowed = if pdweffect.is_null() {
            DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0)
        } else {
            unsafe { *pdweffect }
        };
        let eff = self.compute_effect(grfkeystate, allowed);
        if !pdweffect.is_null() {
            unsafe {
                *pdweffect = eff;
            }
        }
        if valid {
            self.post_hover_event(DROP_EVENT_ENTER);
        }
        Ok(())
    }

    fn DragOver(
        &self,
        grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> Result<()> {
        let allowed = if pdweffect.is_null() {
            DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0)
        } else {
            unsafe { *pdweffect }
        };
        let eff = self.compute_effect(grfkeystate, allowed);
        if !pdweffect.is_null() {
            unsafe {
                *pdweffect = eff;
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        let was_valid = self.valid_drop.replace(false);
        if was_valid {
            self.post_hover_event(DROP_EVENT_LEAVE);
        }
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Option<&IDataObject>,
        grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> Result<()> {
        let allowed = if pdweffect.is_null() {
            DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0)
        } else {
            unsafe { *pdweffect }
        };
        let eff = self.compute_effect(grfkeystate, allowed);
        self.valid_drop.set(false);
        self.post_hover_event(DROP_EVENT_LEAVE);

        let mut delivered_eff = DROPEFFECT_NONE;
        if eff != DROPEFFECT_NONE {
            if let Some(obj) = pdataobj {
                let paths = extract_hdrop_paths(obj, usize::MAX);
                if !paths.is_empty() {
                    let is_move = eff == DROPEFFECT_MOVE;
                    self.shared.push_drop(DroppedFilesPayload {
                        paths,
                        copy: eff == DROPEFFECT_COPY,
                    });
                    let posted = unsafe {
                        !self.box_hwnd.is_invalid()
                            && IsWindow(self.box_hwnd).as_bool()
                            && PostMessageW(
                                self.box_hwnd,
                                self.msg_drop_notify,
                                WPARAM(DROP_EVENT_FILES),
                                LPARAM(0),
                            )
                            .is_ok()
                    };
                    if posted {
                        if is_move {
                            // Optimized move protocol: BoxWnd executes `SHFileOperationW(FO_MOVE)`
                            // asynchronously, so instruct the drag source NOT to delete originals.
                            notify_source_drop_effect(
                                obj,
                                logical_performed_drop_effect_fmt(),
                                DROPEFFECT_MOVE,
                            );
                            notify_source_drop_effect(
                                obj,
                                performed_drop_effect_fmt(),
                                DROPEFFECT_NONE,
                            );
                            delivered_eff = DROPEFFECT_NONE;
                        } else {
                            delivered_eff = eff;
                        }
                    } else {
                        let _ = self.shared.take_drops();
                    }
                }
            }
        }

        if !pdweffect.is_null() {
            unsafe {
                *pdweffect = delivered_eff;
            }
        }
        Ok(())
    }
}

fn build_hdrop_hglobal(paths: &[PathBuf]) -> Option<HGLOBAL> {
    let mut wide_chars: Vec<u16> = Vec::new();
    let mut any = false;
    for p in paths {
        let s = p.as_os_str();
        if s.is_empty() {
            continue;
        }
        wide_chars.extend(s.encode_wide());
        wide_chars.push(0);
        any = true;
    }
    if !any {
        return None;
    }
    wide_chars.push(0);

    let header_bytes = size_of::<DROPFILES>();
    let total_bytes = header_bytes + wide_chars.len() * size_of::<u16>();

    unsafe {
        let hglobal = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, total_bytes).ok()?;
        let ptr = GlobalLock(hglobal);
        if ptr.is_null() {
            let _ = GlobalFree(hglobal);
            return None;
        }
        let header = ptr as *mut DROPFILES;
        *header = DROPFILES {
            pFiles: header_bytes as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: BOOL(0),
            fWide: BOOL(1),
        };
        let dst_chars = ptr.cast::<u8>().add(header_bytes).cast::<u16>();
        copy_nonoverlapping(wide_chars.as_ptr(), dst_chars, wide_chars.len());
        let _ = GlobalUnlock(hglobal);
        Some(hglobal)
    }
}

#[implement(IEnumFORMATETC)]
struct FileFormatEnum {
    index: Cell<usize>,
    fmts: [FORMATETC; 2],
    fmt_count: usize,
}

impl FileFormatEnum {
    fn new() -> Self {
        let pref = preferred_drop_effect_format();
        let fmt_count = if pref.cfFormat != 0 { 2 } else { 1 };
        Self {
            index: Cell::new(0),
            fmts: [hdrop_format(), pref],
            fmt_count,
        }
    }
}

impl IEnumFORMATETC_Impl for FileFormatEnum_Impl {
    fn Next(
        &self,
        celt: u32,
        rgelt: *mut FORMATETC,
        pceltfetched: *mut u32,
    ) -> windows::core::HRESULT {
        if rgelt.is_null() || celt == 0 {
            if !pceltfetched.is_null() {
                unsafe {
                    *pceltfetched = 0;
                }
            }
            return S_FALSE;
        }
        let mut cur = self.index.get();
        let mut fetched = 0u32;
        while fetched < celt && cur < self.fmt_count {
            unsafe {
                *rgelt.add(fetched as usize) = self.fmts[cur];
            }
            cur += 1;
            fetched += 1;
        }
        self.index.set(cur);
        if !pceltfetched.is_null() {
            unsafe {
                *pceltfetched = fetched;
            }
        }
        if fetched == celt {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn Skip(&self, celt: u32) -> Result<()> {
        let next = self.index.get().saturating_add(celt as usize);
        self.index.set(next.min(self.fmt_count));
        if next <= self.fmt_count {
            Ok(())
        } else {
            Err(S_FALSE.into())
        }
    }

    fn Reset(&self) -> Result<()> {
        self.index.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumFORMATETC> {
        let cloned = FileFormatEnum {
            index: Cell::new(self.index.get()),
            fmts: self.fmts,
            fmt_count: self.fmt_count,
        };
        Ok(cloned.into())
    }
}

#[implement(IDataObject)]
struct FileListDataObject {
    paths: Vec<PathBuf>,
    prefer_move: bool,
}

impl FileListDataObject {
    fn matches_hdrop(fmt: *const FORMATETC) -> bool {
        if fmt.is_null() {
            return false;
        }
        let f = unsafe { &*fmt };
        f.cfFormat == CF_HDROP.0
            && f.dwAspect == DVASPECT_CONTENT.0
            && (f.tymed & (TYMED_HGLOBAL.0 as u32)) != 0
    }

    fn matches_preferred_effect(fmt: *const FORMATETC) -> bool {
        if fmt.is_null() {
            return false;
        }
        let pref = preferred_drop_effect_fmt();
        if pref == 0 {
            return false;
        }
        let f = unsafe { &*fmt };
        f.cfFormat == pref
            && f.dwAspect == DVASPECT_CONTENT.0
            && (f.tymed & (TYMED_HGLOBAL.0 as u32)) != 0
    }
}

impl IDataObject_Impl for FileListDataObject_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> Result<STGMEDIUM> {
        if FileListDataObject::matches_hdrop(pformatetcin) {
            let Some(hglobal) = build_hdrop_hglobal(&self.paths) else {
                return Err(DV_E_FORMATETC.into());
            };
            return Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: hglobal },
                pUnkForRelease: ManuallyDrop::new(None),
            });
        }
        if FileListDataObject::matches_preferred_effect(pformatetcin) {
            let effect = if self.prefer_move {
                DROPEFFECT_MOVE
            } else {
                DROPEFFECT_COPY
            };
            let Some(hglobal) = build_drop_effect_hglobal(effect) else {
                return Err(DV_E_FORMATETC.into());
            };
            return Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: hglobal },
                pUnkForRelease: ManuallyDrop::new(None),
            });
        }
        Err(DV_E_FORMATETC.into())
    }

    fn GetDataHere(&self, _pformatetc: *const FORMATETC, _pmedium: *mut STGMEDIUM) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> windows::core::HRESULT {
        if FileListDataObject::matches_hdrop(pformatetc)
            || FileListDataObject::matches_preferred_effect(pformatetc)
        {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }

    fn GetCanonicalFormatEtc(
        &self,
        _pformatectin: *const FORMATETC,
        pformatetcout: *mut FORMATETC,
    ) -> windows::core::HRESULT {
        if !pformatetcout.is_null() {
            unsafe {
                (*pformatetcout).ptd = null_mut();
            }
        }
        E_NOTIMPL
    }

    fn SetData(
        &self,
        pformatetc: *const FORMATETC,
        pmedium: *const STGMEDIUM,
        frelease: BOOL,
    ) -> Result<()> {
        if pformatetc.is_null() {
            return Err(DV_E_FORMATETC.into());
        }
        let cf = unsafe { (*pformatetc).cfFormat };
        let accepted = (cf != 0 && cf == performed_drop_effect_fmt())
            || (cf != 0 && cf == logical_performed_drop_effect_fmt());
        if !accepted {
            return Err(E_NOTIMPL.into());
        }
        if frelease.as_bool() && !pmedium.is_null() {
            unsafe {
                let mut owned = std::ptr::read(pmedium);
                ReleaseStgMedium(&mut owned);
            }
        }
        Ok(())
    }

    fn EnumFormatEtc(&self, dwdirection: u32) -> Result<IEnumFORMATETC> {
        if dwdirection == DATADIR_GET.0 as u32 {
            Ok(FileFormatEnum::new().into())
        } else {
            Err(E_NOTIMPL.into())
        }
    }

    fn DAdvise(
        &self,
        _pformatetc: *const FORMATETC,
        _advf: u32,
        _padvsink: Option<&IAdviseSink>,
    ) -> Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _dwconnection: u32) -> Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

#[implement(IDropSource)]
struct BoxDropSource;

impl IDropSource_Impl for BoxDropSource_Impl {
    fn QueryContinueDrag(
        &self,
        fescapepressed: BOOL,
        grfkeystate: MODIFIERKEYS_FLAGS,
    ) -> windows::core::HRESULT {
        if fescapepressed.as_bool() {
            return DRAGDROP_S_CANCEL;
        }
        if (grfkeystate.0 & MK_LBUTTON.0) == 0 {
            return DRAGDROP_S_DROP;
        }
        S_OK
    }

    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> windows::core::HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Runs a synchronous OLE drag-out session for `paths` so users can drag items from any DeskBox
/// into Explorer, browsers, WeChat, desktop, or another DeskBox.
///
/// MUST be invoked outside any `RefCell` borrow because `DoDragDrop` spins a nested modal message loop.
pub fn start_file_drag_out(paths: &[PathBuf], allow_move: bool) -> (DROPEFFECT, bool) {
    if paths.is_empty() {
        return (DROPEFFECT_NONE, false);
    }
    let data_obj: IDataObject = FileListDataObject {
        paths: paths.to_vec(),
        prefer_move: allow_move,
    }
    .into();
    let drop_src: IDropSource = BoxDropSource.into();
    let allowed = if allow_move {
        DROPEFFECT_COPY | DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    };
    let mut performed = DROPEFFECT_NONE;
    let hr = unsafe { DoDragDrop(&data_obj, &drop_src, allowed, &mut performed) };
    (performed, hr == DRAGDROP_S_DROP)
}
