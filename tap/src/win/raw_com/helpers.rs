//! RAII COM pointer wrapper, FTM/GIT helpers, agile dispatcher handler, barriers, and WinRT DP/brush reflection.

use super::super::brush_store::ensure_cached_com;
use super::super::super::tree_match::TargetKind;
use super::*;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::core::{IInspectable, IUnknown, Interface, GUID, HRESULT, HSTRING};
use windows::System::DispatcherQueue;
use windows::Win32::Foundation::{E_NOINTERFACE, E_POINTER, S_OK};
use windows::Win32::System::Com::{
    CoCreateFreeThreadedMarshaler, CoCreateInstance, IGlobalInterfaceTable, CLSCTX_INPROC_SERVER,
};
use windows::Win32::System::WinRT::RoGetActivationFactory;


pub(in super::super) struct ComRaw(pub(in super::super) *mut c_void);

impl ComRaw {
    #[inline]
    pub(in super::super) fn null() -> Self {
        Self(null_mut())
    }

    #[inline]
    pub(in super::super) fn as_ptr(&self) -> *mut c_void {
        self.0
    }

    #[inline]
    pub(in super::super) fn is_null(&self) -> bool {
        self.0.is_null()
    }

    #[inline]
    pub(in super::super) fn clone_ref(&self) -> Self {
        add_ref_raw_com_addr(self.0 as usize);
        Self(self.0)
    }

    #[inline]
    pub(in super::super) fn into_raw(mut self) -> *mut c_void {
        let ptr = self.0;
        self.0 = null_mut();
        ptr
    }
}

impl Drop for ComRaw {
    fn drop(&mut self) {
        release_raw_com_addr(self.0 as usize);
        self.0 = null_mut();
    }
}

#[inline]
pub(in super::super) fn release_raw_com_addr(addr: usize) {
    if addr != 0 {
        unsafe {
            let unk = addr as *mut RawIUnknown;
            ((*(*unk).lp_vtbl).release)(unk.cast());
        }
    }
}

#[inline]
pub(in super::super) fn add_ref_raw_com_addr(addr: usize) {
    if addr != 0 {
        unsafe {
            let unk = addr as *mut RawIUnknown;
            ((*(*unk).lp_vtbl).add_ref)(unk.cast());
        }
    }
}

pub(in super::super) unsafe fn create_git() -> windows::core::Result<IGlobalInterfaceTable> {
    CoCreateInstance(&CLSID_STD_GLOBAL_INTERFACE_TABLE, None, CLSCTX_INPROC_SERVER)
}

/// Aggregates the Free-Threaded Marshaler onto `outer_unk` lazily so COM can marshal safely across apartments.
pub(in super::super) unsafe fn delegate_ftm_qi(
    outer_unk: *mut c_void,
    ftm_slot: &Mutex<Option<usize>>,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let mut ftm_addr = ftm_slot.lock().ok().and_then(|g| *g);
    if ftm_addr.is_none() {
        if let Some(outer) = IUnknown::from_raw_borrowed(&outer_unk) {
            if let Ok(new_ftm) = CoCreateFreeThreadedMarshaler(outer) {
                let new_addr = new_ftm.into_raw() as usize;
                if let Ok(mut g) = ftm_slot.lock() {
                    if g.is_none() {
                        *g = Some(new_addr);
                        ftm_addr = Some(new_addr);
                    } else {
                        ftm_addr = *g;
                        drop(g);
                        release_raw_com_addr(new_addr);
                    }
                } else {
                    release_raw_com_addr(new_addr);
                }
            }
        }
    }
    if let Some(addr) = ftm_addr {
        let unk = addr as *mut RawIUnknown;
        ((*(*unk).lp_vtbl).query_interface)(unk.cast(), riid, ppv)
    } else {
        E_NOINTERFACE
    }
}

// ---------- Agile IDispatchedHandler for ICoreDispatcher fallback ----------

#[repr(C)]
pub(in super::super) struct RawIDispatchedHandlerVtbl {
    pub(in super::super) query_interface: unsafe extern "system" fn(
        *mut CoreDispatchedHandler,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(in super::super) add_ref: unsafe extern "system" fn(*mut CoreDispatchedHandler) -> u32,
    pub(in super::super) release: unsafe extern "system" fn(*mut CoreDispatchedHandler) -> u32,
    pub(in super::super) invoke: unsafe extern "system" fn(*mut CoreDispatchedHandler) -> HRESULT,
}

pub(in super::super) static DISPATCHED_HANDLER_VTBL: RawIDispatchedHandlerVtbl = RawIDispatchedHandlerVtbl {
    query_interface: dispatched_qi,
    add_ref: dispatched_add_ref,
    release: dispatched_release,
    invoke: dispatched_invoke,
};

#[repr(C)]
pub(in super::super) struct CoreDispatchedHandler {
    pub(in super::super) lp_vtbl: *const RawIDispatchedHandlerVtbl,
    pub(in super::super) ref_count: AtomicU32,
    pub(in super::super) callback: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl CoreDispatchedHandler {
    pub(in super::super) fn new_com_ptr(cb: impl FnOnce() + Send + 'static) -> *mut Self {
        Box::into_raw(Box::new(Self {
            lp_vtbl: &DISPATCHED_HANDLER_VTBL,
            ref_count: AtomicU32::new(1),
            callback: Mutex::new(Some(Box::new(cb))),
        }))
    }
}

pub(in super::super) unsafe extern "system" fn dispatched_qi(
    this: *mut CoreDispatchedHandler,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    let iid = *riid;
    if iid == IID_IUNKNOWN || iid == IID_IDISPATCHEDHANDLER || iid == IID_IAGILEOBJECT {
        *ppv = this.cast();
        dispatched_add_ref(this);
        S_OK
    } else {
        E_NOINTERFACE
    }
}

pub(in super::super) unsafe extern "system" fn dispatched_add_ref(this: *mut CoreDispatchedHandler) -> u32 {
    (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

pub(in super::super) unsafe extern "system" fn dispatched_release(this: *mut CoreDispatchedHandler) -> u32 {
    let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this));
    }
    prev - 1
}

pub(in super::super) unsafe extern "system" fn dispatched_invoke(this: *mut CoreDispatchedHandler) -> HRESULT {
    let cb = (*this).callback.lock().ok().and_then(|mut g| g.take());
    if let Some(cb) = cb {
        cb();
    }
    S_OK
}

// ---------- Tracked Taskbar Targets & Runtime ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum OwnedDispatcher {
    Queue(usize), // Owned raw pointer of agile `windows::System::DispatcherQueue`
    Core(u32),    // GIT cookie for `ICoreDispatcher`
}

pub(in super::super) struct TrackedTarget {
    pub(in super::super) thread_id: Option<u32>,
    pub(in super::super) weak_ref_raw: usize,
    pub(in super::super) kind: TargetKind,
    pub(in super::super) orig_opacity: f64,
    pub(in super::super) orig_visibility: i32,
    pub(in super::super) dispatcher: OwnedDispatcher,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum DispatchFilter {
    QueueTid(u32),
    CoreTid(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum DispatchKey {
    ThreadQueue(u32),
    ThreadCore(u32),
}

pub(in super::super) enum DispatchTarget {
    Queue {
        thread_id: u32,
        queue: DispatcherQueue,
    },
    Core {
        thread_id: u32,
        cookie: u32,
    },
}

pub(in super::super) struct PendingBarrier {
    pub(in super::super) remaining: Mutex<usize>,
    pub(in super::super) cv: Condvar,
}

impl PendingBarrier {
    pub(in super::super) fn new(count: usize) -> Arc<Self> {
        Arc::new(Self {
            remaining: Mutex::new(count),
            cv: Condvar::new(),
        })
    }

    pub(in super::super) fn done_one(&self) {
        if let Ok(mut rem) = self.remaining.lock() {
            if *rem > 0 {
                *rem -= 1;
            }
            if *rem == 0 {
                self.cv.notify_all();
            }
        }
    }

    pub(in super::super) fn wait_timeout(&self, max_wait: Duration) {
        let Ok(mut rem) = self.remaining.lock() else {
            return;
        };
        let deadline = Instant::now() + max_wait;
        while *rem > 0 {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            if let Ok((next, _)) = self.cv.wait_timeout(rem, deadline - now) {
                rem = next;
            } else {
                break;
            }
        }
    }
}

pub(in super::super) struct BarrierGuard {
    pub(in super::super) barrier: Option<Arc<PendingBarrier>>,
}

impl Drop for BarrierGuard {
    fn drop(&mut self) {
        if let Some(b) = self.barrier.take() {
            b.done_one();
        }
    }
}

#[inline]
pub(in super::super) fn revoke_git_cookie(git: Option<&IGlobalInterfaceTable>, cookie: u32) {
    if cookie != 0 {
        if let Some(g) = git {
            unsafe {
                let _ = g.RevokeInterfaceFromGlobal(cookie);
            }
        }
    }
}

pub(in super::super) unsafe fn qi_interface(ptr: *mut c_void, iid: &GUID) -> ComRaw {
    if ptr.is_null() {
        return ComRaw::null();
    }
    let mut out = ComRaw::null();
    let unk = ptr.cast::<RawIUnknown>();
    if ((*(*unk).lp_vtbl).query_interface)(ptr, iid, &mut out.0).is_ok() {
        out
    } else {
        ComRaw::null()
    }
}

pub(in super::super) unsafe fn com_same_identity(a: *mut c_void, b: *mut c_void) -> bool {
    if a.is_null() || b.is_null() {
        return false;
    }
    let ua = qi_interface(a, &IID_IUNKNOWN);
    let ub = qi_interface(b, &IID_IUNKNOWN);
    !ua.is_null() && ua.as_ptr() == ub.as_ptr()
}

pub(in super::super) unsafe fn activate_factory_iface(class_name: &str, iid: &GUID) -> Option<ComRaw> {
    let class_id = HSTRING::from(class_name);
    let factory_insp: IInspectable = RoGetActivationFactory(&class_id).ok()?;
    let out = qi_interface(factory_insp.as_raw(), iid);
    (!out.is_null()).then_some(out)
}

pub(in super::super) unsafe fn fetch_solid_brush(color: RawUiColor) -> Option<ComRaw> {
    let factory_com = activate_factory_iface(
        "Windows.UI.Xaml.Media.SolidColorBrush",
        &IID_ISOLIDCOLORBRUSHFACTORY,
    )?;
    let fac = factory_com.as_ptr().cast::<RawISolidColorBrushFactory>();
    let mut solid_com = ComRaw::null();
    if ((*(*fac).lp_vtbl).create_instance_with_color)(
        factory_com.as_ptr(),
        color,
        &mut solid_com.0,
    )
    .is_err()
        || solid_com.is_null()
    {
        return None;
    }
    let brush_com = qi_interface(solid_com.as_ptr(), &IID_IBRUSH);
    (!brush_com.is_null()).then_some(brush_com)
}

pub(in super::super) unsafe fn fetch_transparent_brush() -> Option<ComRaw> {
    fetch_solid_brush(RawUiColor::TRANSPARENT)
}

pub(in super::super) unsafe fn fetch_grid_statics() -> Option<ComRaw> {
    activate_factory_iface("Windows.UI.Xaml.Controls.Grid", &IID_IGRIDSTATICS)
}

pub(in super::super) unsafe fn cached_grid_statics() -> Option<ComRaw> {
    ensure_cached_com(
        |s| s.grid_statics.as_ref(),
        |s, v| s.grid_statics = Some(v),
        fetch_grid_statics,
    )
}

pub(in super::super) unsafe fn fetch_unset_value() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.DependencyProperty",
        &IID_IDEPENDENCYPROPERTYSTATICS,
    )?;
    let statics = statics_com
        .as_ptr()
        .cast::<RawIDependencyPropertyStatics>();
    let mut unset = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_unset_value)(statics_com.as_ptr(), &mut unset.0).is_ok()
        && !unset.is_null()
    {
        Some(unset)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_border_bg_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Border", &IID_IBORDERSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIBorderStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_background_property)(statics_com.as_ptr(), &mut dp.0).is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_border_brush_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Border", &IID_IBORDERSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIBorderStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_border_brush_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_panel_bg_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Panel", &IID_IPANELSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIPanelStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_background_property)(statics_com.as_ptr(), &mut dp.0).is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_grid_border_brush_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Grid", &IID_IGRIDSTATICS2)?;
    let statics = statics_com.as_ptr().cast::<RawIGridStatics2>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_border_brush_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_visual_tree_helper() -> Option<ComRaw> {
    activate_factory_iface(
        "Windows.UI.Xaml.Media.VisualTreeHelper",
        &IID_IVISUALTREEHELPERSTATICS,
    )
}

pub(in super::super) unsafe fn cached_visual_tree_helper() -> Option<ComRaw> {
    ensure_cached_com(
        |s| s.visual_tree_helper.as_ref(),
        |s, v| s.visual_tree_helper = Some(v),
        fetch_visual_tree_helper,
    )
}

pub(in super::super) unsafe fn fetch_element_composition_preview() -> Option<ComRaw> {
    activate_factory_iface(
        "Windows.UI.Xaml.Hosting.ElementCompositionPreview",
        &IID_IELEMENTCOMPOSITIONPREVIEWSTATICS,
    )
}

pub(in super::super) unsafe fn cached_element_composition_preview() -> Option<ComRaw> {
    ensure_cached_com(
        |s| s.element_comp_preview.as_ref(),
        |s, v| s.element_comp_preview = Some(v),
        fetch_element_composition_preview,
    )
}

pub(in super::super) unsafe fn fetch_focus_secondary_brush_dp() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.FrameworkElement",
        &IID_IFRAMEWORKELEMENTSTATICS4,
    )?;
    let statics = statics_com.as_ptr().cast::<RawIFrameworkElementStatics4>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_focus_visual_secondary_brush_property)(
        statics_com.as_ptr(),
        &mut dp.0,
    )
    .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_focus_secondary_thickness_dp() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.FrameworkElement",
        &IID_IFRAMEWORKELEMENTSTATICS4,
    )?;
    let statics = statics_com.as_ptr().cast::<RawIFrameworkElementStatics4>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_focus_visual_secondary_thickness_property)(
        statics_com.as_ptr(),
        &mut dp.0,
    )
    .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_element_shadow_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.UIElement", &IID_IUIELEMENTSTATICS10)?;
    let statics = statics_com.as_ptr().cast::<RawIUIElementStatics10>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_shadow_property)(statics_com.as_ptr(), &mut dp.0).is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_content_presenter_bg_dp() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.Controls.ContentPresenter",
        &IID_ICONTENTPRESENTERSTATICS4,
    )?;
    let statics = statics_com
        .as_ptr()
        .cast::<RawIContentPresenterStatics4>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_background_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_content_presenter_border_brush_dp() -> Option<ComRaw> {
    let statics_com = activate_factory_iface(
        "Windows.UI.Xaml.Controls.ContentPresenter",
        &IID_ICONTENTPRESENTERSTATICS4,
    )?;
    let statics = statics_com
        .as_ptr()
        .cast::<RawIContentPresenterStatics4>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_border_brush_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_control_bg_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Control", &IID_ICONTROLSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIControlStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_background_property)(statics_com.as_ptr(), &mut dp.0).is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_control_border_brush_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Control", &IID_ICONTROLSTATICS)?;
    let statics = statics_com.as_ptr().cast::<RawIControlStatics>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_border_brush_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn fetch_control_corner_radius_dp() -> Option<ComRaw> {
    let statics_com =
        activate_factory_iface("Windows.UI.Xaml.Controls.Control", &IID_ICONTROLSTATICS7)?;
    let statics = statics_com.as_ptr().cast::<RawIControlStatics7>();
    let mut dp = ComRaw::null();
    if ((*(*statics).lp_vtbl).get_corner_radius_property)(statics_com.as_ptr(), &mut dp.0)
        .is_ok()
        && !dp.is_null()
    {
        Some(dp)
    } else {
        None
    }
}

pub(in super::super) unsafe fn com_identity_key(ptr: *mut c_void) -> u64 {
    let unk = qi_interface(ptr, &IID_IUNKNOWN);
    let raw = if !unk.is_null() {
        unk.as_ptr() as u64
    } else {
        ptr as u64
    };
    (1u64 << 63) | (raw & !(1u64 << 63))
}

pub(in super::super) unsafe fn element_framework_name(ptr: *mut c_void) -> Option<String> {
    let fe_com = qi_interface(ptr, &IID_IFRAMEWORKELEMENT);
    if fe_com.is_null() {
        return None;
    }
    let fe = fe_com.as_ptr().cast::<RawIFrameworkElement>();
    let mut hs = HSTRING::new();
    if ((*(*fe).lp_vtbl).get_name)(fe_com.as_ptr(), &mut hs).is_ok() {
        Some(hs.to_string_lossy())
    } else {
        None
    }
}

pub(in super::super) unsafe fn element_runtime_class_name(ptr: *mut c_void) -> Option<String> {
    let insp_com = qi_interface(ptr, &IID_IINSPECTABLE);
    if insp_com.is_null() {
        return None;
    }
    let insp = insp_com.as_ptr().cast::<RawIInspectable>();
    let mut hs = HSTRING::new();
    if ((*(*insp).lp_vtbl).get_runtime_class_name)(insp_com.as_ptr(), &mut hs).is_ok() {
        Some(hs.to_string_lossy())
    } else {
        None
    }
}

pub(in super::super) unsafe fn solid_brush_color(brush_ptr: *mut c_void) -> Option<RawUiColor> {
    if brush_ptr.is_null() {
        return None;
    }
    let solid_com = qi_interface(brush_ptr, &IID_ISOLIDCOLORBRUSH);
    if solid_com.is_null() {
        return None;
    }
    let solid = solid_com.as_ptr().cast::<RawISolidColorBrush>();
    let mut col = RawUiColor {
        a: 0,
        r: 0,
        g: 0,
        b: 0,
    };
    ((*(*solid).lp_vtbl).get_color)(solid_com.as_ptr(), &mut col)
        .is_ok()
        .then_some(col)
}

pub(in super::super) unsafe fn is_transparent_solid_brush(brush_ptr: *mut c_void) -> bool {
    solid_brush_color(brush_ptr).is_some_and(|col| col.a == 0)
}

pub(in super::super) unsafe fn is_solid_brush_with_color(brush_ptr: *mut c_void, target: RawUiColor) -> bool {
    solid_brush_color(brush_ptr).is_some_and(|col| col == target)
}

/// Returns `true` when two XAML `IBrush` objects represent the same brush instance or identical
/// solid color. Used after `ClearValue` to detect when XAML dropped a `VisualState.Setter`
/// (e.g., Win11 accent color on Overflow or Calendar) and fell back to a different base brush.
pub(in super::super) unsafe fn brushes_equivalent(a: *mut c_void, b: *mut c_void) -> bool {
    if a.is_null() || b.is_null() {
        return a.is_null() && b.is_null();
    }
    if com_same_identity(a, b) {
        return true;
    }
    match (solid_brush_color(a), solid_brush_color(b)) {
        (Some(ca), Some(cb)) => ca == cb,
        _ => false,
    }
}

