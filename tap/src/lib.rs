#![allow(non_snake_case)]

//! Lightweight XAML Visual Tree diagnostics TAP helper (`PrettyWindowsTap.dll`)
//! for safely toggling the Win11 XAML taskbar background opacity on the UI thread.

pub mod tree_match {
    use std::collections::{HashMap, HashSet, VecDeque};

    /// Shared COM CLSID (`8a4e6f21-9c3b-4d7a-b1e5-3f7c2a9d8410`) for `InitializeXamlDiagnosticsEx`.
    pub const CLSID_PRETTY_WINDOWS_TAP_U128: u128 = 0x8a4e6f21_9c3b_4d7a_b1e5_3f7c2a9d8410;

    /// IPC pipe command bytes exchanged between `PrettyWindows.exe` and `PrettyWindowsTap.dll`.
    pub const CMD_RESTORE: u8 = 0x00;
    pub const CMD_HIDE: u8 = 0x01;
    pub const CMD_PING: u8 = 0x02;
    pub const CMD_SHUTDOWN: u8 = 0x03;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct NodeMeta {
        pub parent: u64,
        pub type_name: String,
        pub name: String,
    }

    /// Tracks XAML visual tree parent-child links to isolate taskbar background rectangles safely.
    #[derive(Default, Debug)]
    pub struct VisualTreeIndex {
        nodes: HashMap<u64, NodeMeta>,
    }

    impl VisualTreeIndex {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn on_add(&mut self, parent: u64, child: u64, type_name: String, name: String) {
            if child == 0 {
                return;
            }
            self.nodes.insert(
                child,
                NodeMeta {
                    parent,
                    type_name,
                    name,
                },
            );
        }

        pub fn on_remove(&mut self, handle: u64) -> Vec<u64> {
            let mut removed = Vec::new();
            let mut queue = VecDeque::new();
            let mut seen = HashSet::new();
            if self.nodes.remove(&handle).is_some() {
                removed.push(handle);
                queue.push_back(handle);
                seen.insert(handle);
            }
            while let Some(parent_h) = queue.pop_front() {
                if removed.len() >= 4096 {
                    break;
                }
                let children: Vec<u64> = self
                    .nodes
                    .iter()
                    .filter_map(|(&h, meta)| (meta.parent == parent_h).then_some(h))
                    .collect();
                for ch in children {
                    if seen.insert(ch) && self.nodes.remove(&ch).is_some() {
                        removed.push(ch);
                        queue.push_back(ch);
                    }
                }
            }
            removed
        }

        pub fn has_valid_taskbar_ancestor(&self, handle: u64) -> bool {
            let Some(node) = self.nodes.get(&handle) else {
                return false;
            };
            let mut seen = [0u64; 32];
            let mut seen_len = 0usize;
            let mut cur = node.parent;
            let mut has_taskbar_frame = false;

            while cur != 0 && seen_len < seen.len() {
                if cur == handle || seen[..seen_len].contains(&cur) {
                    return false;
                }
                seen[seen_len] = cur;
                seen_len += 1;

                let Some(meta) = self.nodes.get(&cur) else {
                    break;
                };
                if meta.name.contains("Flyout")
                    || meta.name.contains("Tooltip")
                    || meta.type_name.contains("StartMenu")
                    || meta.type_name.contains("Flyout")
                    || meta.type_name.contains("Popup")
                {
                    return false;
                }
                if meta.type_name.contains("TaskbarFrame") {
                    has_taskbar_frame = true;
                }
                cur = meta.parent;
            }
            has_taskbar_frame
        }

        /// Returns `true` only for `BackgroundFill`/`BackgroundStroke` rectangles inside a non-flyout `TaskbarFrame`.
        pub fn is_taskbar_bg_target(&self, handle: u64) -> bool {
            let Some(node) = self.nodes.get(&handle) else {
                return false;
            };
            if !matches!(node.name.as_str(), "BackgroundFill" | "BackgroundStroke") {
                return false;
            }
            self.has_valid_taskbar_ancestor(handle)
        }

        pub fn collect_targets(&self) -> Vec<u64> {
            self.nodes
                .keys()
                .copied()
                .filter(|&h| self.is_taskbar_bg_target(h))
                .collect()
        }
    }

    pub fn pipe_name_for_pid(pid: u32) -> String {
        format!(r"\\.\pipe\PrettyWindows_TaskbarTap_{pid}")
    }
}

#[cfg(test)]
mod tests {
    use super::tree_match::*;
    use std::collections::HashSet;

    #[test]
    fn isolates_taskbar_fill_and_stroke_rectangles_and_excludes_flyouts_and_start_menu() {
        let mut idx = VisualTreeIndex::new();
        // Primary monitor taskbar tree
        idx.on_add(0, 1, "Taskbar.TaskbarFrame".into(), "TaskbarFrame".into());
        idx.on_add(
            1,
            2,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "RootGrid".into(),
        );
        idx.on_add(
            2,
            3,
            "Taskbar.TaskbarBackground".into(),
            "BackgroundControl".into(),
        );
        idx.on_add(3, 4, "Windows.UI.Xaml.Controls.Grid".into(), "".into());
        idx.on_add(
            4,
            5,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
        );
        idx.on_add(
            4,
            6,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundStroke".into(),
        );
        // Taskbar app icons sibling must never be matched
        idx.on_add(2, 7, "Taskbar.TaskListLabeledButtonPanel".into(), "".into());

        // Hover flyout nested under TaskbarFrame must NOT match
        idx.on_add(
            2,
            8,
            "Taskbar.TaskbarBackground".into(),
            "HoverFlyoutBackgroundControl".into(),
        );
        idx.on_add(8, 9, "Windows.UI.Xaml.Controls.Grid".into(), "".into());
        idx.on_add(
            9,
            10,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
        );

        // Win11 24H2 wrapped Secondary monitor taskbar tree (Border -> ScrollViewer -> ScrollContentPresenter -> TaskbarFrame)
        idx.on_add(
            0,
            20,
            "Windows.UI.Xaml.Controls.Border".into(),
            "OuterBorder".into(),
        );
        idx.on_add(
            20,
            21,
            "Windows.UI.Xaml.Controls.ScrollViewer".into(),
            "".into(),
        );
        idx.on_add(
            21,
            22,
            "Windows.UI.Xaml.Controls.ScrollContentPresenter".into(),
            "".into(),
        );
        idx.on_add(
            22,
            23,
            "Taskbar.TaskbarFrame".into(),
            "TaskbarFrame".into(),
        );
        idx.on_add(
            23,
            24,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "RootGrid".into(),
        );
        idx.on_add(
            24,
            25,
            "Taskbar.TaskbarBackground".into(),
            "BackgroundControl".into(),
        );
        idx.on_add(25, 26, "Windows.UI.Xaml.Controls.Grid".into(), "".into());
        idx.on_add(
            26,
            27,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
        );
        idx.on_add(
            26,
            28,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundStroke".into(),
        );

        // Start menu element with identical child names must NOT match
        idx.on_add(0, 100, "StartMenu.StartInnerFrame".into(), "".into());
        idx.on_add(
            100,
            101,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
        );
        idx.on_add(
            100,
            102,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundStroke".into(),
        );

        let targets: HashSet<u64> = idx.collect_targets().into_iter().collect();
        assert_eq!(targets, HashSet::from([5, 6, 27, 28]));
    }

    #[test]
    fn subtree_remove_reparent_and_cycle_guard_behave_safely() {
        let mut idx = VisualTreeIndex::new();
        idx.on_add(0, 1, "Taskbar.TaskbarFrame".into(), "".into());
        idx.on_add(1, 2, "Taskbar.TaskbarBackground".into(), "".into());
        idx.on_add(
            2,
            3,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
        );
        // Secondary TaskbarFrame
        idx.on_add(0, 20, "Taskbar.TaskbarFrame".into(), "".into());
        idx.on_add(
            2,
            4,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundStroke".into(),
        );
        // Reparent node 4 before removing subtree 1
        idx.on_add(
            20,
            4,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundStroke".into(),
        );

        let removed = idx.on_remove(1);
        assert!(removed.contains(&1) && removed.contains(&2) && removed.contains(&3));
        assert!(!removed.contains(&4));
        assert_eq!(idx.collect_targets(), vec![4]);

        // Cycle protection (even when cycle passes through TaskbarFrame)
        idx.on_add(52, 50, "Taskbar.TaskbarFrame".into(), "".into());
        idx.on_add(50, 52, "Windows.UI.Xaml.Controls.Grid".into(), "".into());
        idx.on_add(
            52,
            51,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
        );
        assert!(!idx.is_taskbar_bg_target(51));
    }
}

#[cfg(windows)]
mod win {
    use super::tree_match::{
        pipe_name_for_pid, VisualTreeIndex, CLSID_PRETTY_WINDOWS_TAP_U128, CMD_HIDE, CMD_PING,
        CMD_RESTORE, CMD_SHUTDOWN,
    };
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};
    use windows::core::{IUnknown, Interface, GUID, HRESULT, PCWSTR};
    use windows::System::{DispatcherQueue, DispatcherQueueHandler};
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, BOOL, CLASS_E_CLASSNOTAVAILABLE, E_NOINTERFACE, E_POINTER,
        ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, HINSTANCE, INVALID_HANDLE_VALUE, S_FALSE,
        S_OK, TRUE, WAIT_OBJECT_0,
    };
    use windows::Win32::Storage::FileSystem::{
        FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Com::{
        CoCreateFreeThreadedMarshaler, CoCreateInstance, CoInitializeEx, CoUninitialize,
        IGlobalInterfaceTable, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};
    use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, ResetEvent, SetEvent,
        WaitForMultipleObjects, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
    };

    const DLL_PROCESS_ATTACH_CODE: u32 = 1;
    const E_FAIL_HR: HRESULT = HRESULT(0x80004005u32 as i32);

    const CLSID_PRETTY_WINDOWS_TAP: GUID = GUID::from_u128(CLSID_PRETTY_WINDOWS_TAP_U128);
    const CLSID_STD_GLOBAL_INTERFACE_TABLE: GUID =
        GUID::from_u128(0x00000323_0000_0000_C000_000000000046);

    const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_C000_000000000046);
    const IID_IINSPECTABLE: GUID = GUID::from_u128(0xAF86E2E0_B12D_4C6A_9C5A_D7AA65101E90);
    const IID_ICLASSFACTORY: GUID = GUID::from_u128(0x00000001_0000_0000_C000_000000000046);
    const IID_IMARSHAL: GUID = GUID::from_u128(0x00000003_0000_0000_C000_000000000046);
    const IID_IWEAKREFERENCESOURCE: GUID = GUID::from_u128(0x00000038_0000_0000_C000_000000000046);
    const IID_IAGILEOBJECT: GUID = GUID::from_u128(0x94EA2B94_E9CC_49E0_C0FF_EE64CA8F5B90);
    const IID_IOBJECTWITHSITE: GUID = GUID::from_u128(0xFC4801A3_2BA9_11CF_A229_00AA003D7352);
    const IID_IVISUALTREESERVICECALLBACK: GUID =
        GUID::from_u128(0xAA7A8931_80E4_4FEC_8F3B_553F87B4966E);
    const IID_IVISUALTREESERVICECALLBACK2: GUID =
        GUID::from_u128(0xBAD9EB88_AE77_4397_B948_5FA2DB0A19EA);
    const IID_IVISUALTREESERVICE: GUID = GUID::from_u128(0xA593B11A_D17F_48BB_8F66_83910731C8A5);
    const IID_IXAMLDIAGNOSTICS: GUID = GUID::from_u128(0x18C9E2B6_3F43_4116_9F2B_FF935D7770D2);
    const IID_IUIELEMENT: GUID = GUID::from_u128(0x676D0BE9_B65C_41C6_BA40_58CF87F201C1);
    const IID_IDEPENDENCYOBJECT: GUID = GUID::from_u128(0x5C526665_F60E_4912_AF59_5FE0680F089D);
    const IID_ICOREDISPATCHER: GUID = GUID::from_u128(0x60DB2FA8_B705_4FDE_A7D6_EBBB1891D39E);
    const IID_IDISPATCHEDHANDLER: GUID = GUID::from_u128(0xD1F276C4_98D8_4636_BF49_EB79507548E9);

    static ACTIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);

    // Raw C ABI layouts strictly matching Windows SDK `xamlOM.h`, `windows.ui.xaml.h`,
    // `windows.ui.core.h`, and `weakreference.h`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawSourceInfo {
        file_name: *const u16,
        line_number: u32,
        column_number: u32,
        char_position: u32,
        hash: *const u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawParentChildRelation {
        parent: u64,
        child: u64,
        child_index: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawVisualElement {
        handle: u64,
        src_info: RawSourceInfo,
        type_name: *const u16,
        name: *const u16,
        num_children: u32,
    }

    const _: () = {
        assert!(size_of::<RawSourceInfo>() == 32);
        assert!(size_of::<RawParentChildRelation>() == 24);
        assert!(size_of::<RawVisualElement>() == 64);
    };

    #[repr(C)]
    struct RawIUnknownVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct RawIUnknown {
        lp_vtbl: *const RawIUnknownVtbl,
    }

    #[repr(C)]
    struct RawIVisualTreeServiceVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        advise_visual_tree_change: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
        unadvise_visual_tree_change: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIVisualTreeService {
        lp_vtbl: *const RawIVisualTreeServiceVtbl,
    }

    #[repr(C)]
    struct RawIXamlDiagnosticsVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_dispatcher: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        get_ui_layer: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        get_application: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        get_iinspectable_from_handle:
            unsafe extern "system" fn(*mut c_void, u64, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIXamlDiagnostics {
        lp_vtbl: *const RawIXamlDiagnosticsVtbl,
    }

    #[repr(C)]
    struct RawIUIElementVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_desired_size: unsafe extern "system" fn(*mut c_void),
        get_allow_drop: unsafe extern "system" fn(*mut c_void),
        put_allow_drop: unsafe extern "system" fn(*mut c_void),
        get_opacity: unsafe extern "system" fn(*mut c_void, *mut f64) -> HRESULT,
        put_opacity: unsafe extern "system" fn(*mut c_void, f64) -> HRESULT,
    }

    #[repr(C)]
    struct RawIUIElement {
        lp_vtbl: *const RawIUIElementVtbl,
    }

    #[repr(C)]
    struct RawIDependencyObjectVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_value: unsafe extern "system" fn(*mut c_void),
        set_value: unsafe extern "system" fn(*mut c_void),
        clear_value: unsafe extern "system" fn(*mut c_void),
        read_local_value: unsafe extern "system" fn(*mut c_void),
        get_animation_base_value: unsafe extern "system" fn(*mut c_void),
        get_dispatcher: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIDependencyObject {
        lp_vtbl: *const RawIDependencyObjectVtbl,
    }

    #[repr(C)]
    struct RawICoreDispatcherVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_has_thread_access: unsafe extern "system" fn(*mut c_void),
        process_events: unsafe extern "system" fn(*mut c_void),
        run_async: unsafe extern "system" fn(
            *mut c_void,
            i32,
            *mut c_void,
            *mut *mut c_void,
        ) -> HRESULT,
    }

    #[repr(C)]
    struct RawICoreDispatcher {
        lp_vtbl: *const RawICoreDispatcherVtbl,
    }

    #[repr(C)]
    struct RawIWeakReferenceSourceVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_weak_reference: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIWeakReferenceSource {
        lp_vtbl: *const RawIWeakReferenceSourceVtbl,
    }

    #[repr(C)]
    struct RawIWeakReferenceVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        resolve:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIWeakReference {
        lp_vtbl: *const RawIWeakReferenceVtbl,
    }

    struct ComRaw(*mut c_void);

    impl ComRaw {
        #[inline]
        fn null() -> Self {
            Self(null_mut())
        }

        #[inline]
        fn as_ptr(&self) -> *mut c_void {
            self.0
        }

        #[inline]
        fn is_null(&self) -> bool {
            self.0.is_null()
        }

        #[inline]
        fn into_raw(mut self) -> *mut c_void {
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
    fn release_raw_com_addr(addr: usize) {
        if addr != 0 {
            unsafe {
                let unk = addr as *mut RawIUnknown;
                ((*(*unk).lp_vtbl).release)(unk.cast());
            }
        }
    }

    #[inline]
    fn add_ref_raw_com_addr(addr: usize) {
        if addr != 0 {
            unsafe {
                let unk = addr as *mut RawIUnknown;
                ((*(*unk).lp_vtbl).add_ref)(unk.cast());
            }
        }
    }

    unsafe fn create_git() -> windows::core::Result<IGlobalInterfaceTable> {
        CoCreateInstance(&CLSID_STD_GLOBAL_INTERFACE_TABLE, None, CLSCTX_INPROC_SERVER)
    }

    /// Aggregates the Free-Threaded Marshaler onto `outer_unk` lazily so COM can marshal safely across apartments.
    unsafe fn delegate_ftm_qi(
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
    struct RawIDispatchedHandlerVtbl {
        query_interface: unsafe extern "system" fn(
            *mut CoreDispatchedHandler,
            *const GUID,
            *mut *mut c_void,
        ) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut CoreDispatchedHandler) -> u32,
        release: unsafe extern "system" fn(*mut CoreDispatchedHandler) -> u32,
        invoke: unsafe extern "system" fn(*mut CoreDispatchedHandler) -> HRESULT,
    }

    static DISPATCHED_HANDLER_VTBL: RawIDispatchedHandlerVtbl = RawIDispatchedHandlerVtbl {
        query_interface: dispatched_qi,
        add_ref: dispatched_add_ref,
        release: dispatched_release,
        invoke: dispatched_invoke,
    };

    #[repr(C)]
    struct CoreDispatchedHandler {
        lp_vtbl: *const RawIDispatchedHandlerVtbl,
        ref_count: AtomicU32,
        callback: Option<Box<dyn FnOnce() + Send>>,
    }

    impl CoreDispatchedHandler {
        fn new_com_ptr(cb: impl FnOnce() + Send + 'static) -> *mut Self {
            Box::into_raw(Box::new(Self {
                lp_vtbl: &DISPATCHED_HANDLER_VTBL,
                ref_count: AtomicU32::new(1),
                callback: Some(Box::new(cb)),
            }))
        }
    }

    unsafe extern "system" fn dispatched_qi(
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

    unsafe extern "system" fn dispatched_add_ref(this: *mut CoreDispatchedHandler) -> u32 {
        (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    unsafe extern "system" fn dispatched_release(this: *mut CoreDispatchedHandler) -> u32 {
        let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            std::sync::atomic::fence(Ordering::Acquire);
            drop(Box::from_raw(this));
        }
        prev - 1
    }

    unsafe extern "system" fn dispatched_invoke(this: *mut CoreDispatchedHandler) -> HRESULT {
        if let Some(cb) = (*this).callback.take() {
            cb();
        }
        S_OK
    }

    // ---------- Tracked Taskbar Targets & Runtime ----------

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum OwnedDispatcher {
        Queue(usize), // Owned raw pointer of agile `windows::System::DispatcherQueue`
        Core(u32),    // GIT cookie for `ICoreDispatcher`
    }

    struct TrackedTarget {
        thread_id: Option<u32>,
        weak_ref_raw: usize,
        orig_opacity: f64,
        dispatcher: OwnedDispatcher,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum DispatchKey {
        ThreadQueue(u32),
        CoreCookie(u32),
    }

    enum DispatchTarget {
        Queue {
            thread_id: u32,
            queue: DispatcherQueue,
        },
        Core {
            cookie: u32,
        },
    }

    struct PendingBarrier {
        remaining: Mutex<usize>,
        cv: Condvar,
    }

    impl PendingBarrier {
        fn new(count: usize) -> Arc<Self> {
            Arc::new(Self {
                remaining: Mutex::new(count),
                cv: Condvar::new(),
            })
        }

        fn done_one(&self) {
            if let Ok(mut rem) = self.remaining.lock() {
                if *rem > 0 {
                    *rem -= 1;
                }
                if *rem == 0 {
                    self.cv.notify_all();
                }
            }
        }

        fn wait_timeout(&self, max_wait: Duration) {
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

    struct BarrierGuard {
        barrier: Option<Arc<PendingBarrier>>,
    }

    impl Drop for BarrierGuard {
        fn drop(&mut self) {
            if let Some(b) = self.barrier.take() {
                b.done_one();
            }
        }
    }

    fn release_target_resources(target: TrackedTarget, git: Option<&IGlobalInterfaceTable>) {
        release_raw_com_addr(target.weak_ref_raw);
        match target.dispatcher {
            OwnedDispatcher::Queue(addr) => {
                if addr != 0 {
                    unsafe {
                        let _ = DispatcherQueue::from_raw(addr as *mut c_void);
                    }
                }
            }
            OwnedDispatcher::Core(cookie) => {
                if cookie != 0 {
                    if let Some(g) = git {
                        unsafe {
                            let _ = g.RevokeInterfaceFromGlobal(cookie);
                        }
                    }
                }
            }
        }
    }

    struct RuntimeState {
        desired_hide: bool,
        shutdown: bool,
        targets: HashMap<u64, TrackedTarget>,
    }

    struct TapRuntime {
        tree: Mutex<VisualTreeIndex>,
        state: Mutex<RuntimeState>,
        diag_cookie: AtomicU32,
        shutdown_started: AtomicBool,
        stop_requested: AtomicBool,
        stop_event_raw: usize,
        host_handle_raw: Mutex<Option<usize>>,
        advisor_finished: Mutex<bool>,
        advisor_cv: Condvar,
    }

    impl TapRuntime {
        fn new(diag_cookie: u32) -> windows::core::Result<Arc<Self>> {
            let stop_event = unsafe { CreateEventW(None, true, false, None)? };
            Ok(Arc::new(Self {
                tree: Mutex::new(VisualTreeIndex::new()),
                state: Mutex::new(RuntimeState {
                    desired_hide: false,
                    shutdown: false,
                    targets: HashMap::new(),
                }),
                diag_cookie: AtomicU32::new(diag_cookie),
                shutdown_started: AtomicBool::new(false),
                stop_requested: AtomicBool::new(false),
                stop_event_raw: stop_event.0 as usize,
                host_handle_raw: Mutex::new(None),
                advisor_finished: Mutex::new(false),
                advisor_cv: Condvar::new(),
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

        fn set_hide(self: &Arc<Self>, hide: bool, host_pid: u32) {
            if hide && host_pid != 0 {
                if let Ok(mut hh) = self.host_handle_raw.lock() {
                    let need_open = match *hh {
                        Some(raw) if raw != 0 => unsafe {
                            WaitForSingleObject(HANDLE(raw as *mut c_void), 0) == WAIT_OBJECT_0
                        },
                        _ => true,
                    };
                    if need_open {
                        if let Some(old_raw) = hh.take() {
                            if old_raw != 0 {
                                unsafe {
                                    let _ = CloseHandle(HANDLE(old_raw as *mut c_void));
                                }
                            }
                        }
                        if let Ok(new_h) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, host_pid) }
                        {
                            *hh = Some(new_h.0 as usize);
                        }
                    }
                }
            }

            if let Ok(mut g) = self.state.lock() {
                if !g.shutdown {
                    g.desired_hide = hide;
                }
            }
            self.dispatch_apply_to_ui_threads(0);
        }

        fn trigger_shutdown(self: &Arc<Self>, wait_ms: u64) {
            let first = !self.shutdown_started.swap(true, Ordering::AcqRel);
            if first {
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

        fn dispatch_apply_to_ui_threads(self: &Arc<Self>, wait_ms: u64) {
            let mut seen_keys: Vec<DispatchKey> = Vec::new();
            let mut dispatch_list: Vec<DispatchTarget> = Vec::new();

            if let Ok(g) = self.state.lock() {
                for t in g.targets.values() {
                    match (t.thread_id, t.dispatcher) {
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
                }
            }

            if dispatch_list.is_empty() {
                return;
            }

            let barrier = (wait_ms > 0).then(|| PendingBarrier::new(dispatch_list.len()));
            let git = if dispatch_list
                .iter()
                .any(|d| matches!(d, DispatchTarget::Core { .. }))
            {
                unsafe { create_git().ok() }
            } else {
                None
            };

            for target in dispatch_list {
                match target {
                    DispatchTarget::Queue { thread_id, queue } => {
                        if queue.HasThreadAccess().unwrap_or(false) {
                            apply_for_ui_thread(self, Some(thread_id));
                            if let Some(ref b) = barrier {
                                b.done_one();
                            }
                            continue;
                        }
                        let rt_cb = Arc::clone(self);
                        let barrier_cb = barrier.clone();
                        let handler = DispatcherQueueHandler::new(move || {
                            let _guard = BarrierGuard {
                                barrier: barrier_cb.clone(),
                            };
                            apply_for_ui_thread(&rt_cb, Some(thread_id));
                            Ok(())
                        });
                        match queue.TryEnqueue(&handler) {
                            Ok(true) => {}
                            _ => {
                                if let Some(ref b) = barrier {
                                    b.done_one();
                                }
                            }
                        }
                    }
                    DispatchTarget::Core { cookie } => {
                        let mut enqueued = false;
                        if let Some(ref g) = git {
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
                                    let barrier_cb = barrier.clone();
                                    let handler = CoreDispatchedHandler::new_com_ptr(move || {
                                        let _guard = BarrierGuard {
                                            barrier: barrier_cb,
                                        };
                                        apply_for_ui_thread(&rt_cb, None);
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
                            if let Some(ref b) = barrier {
                                b.done_one();
                            }
                        }
                    }
                }
            }

            if let Some(b) = barrier {
                b.wait_timeout(Duration::from_millis(wait_ms));
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
                if let Ok(mut hh) = self.host_handle_raw.lock() {
                    if let Some(raw) = hh.take() {
                        if raw != 0 {
                            let _ = CloseHandle(HANDLE(raw as *mut c_void));
                        }
                    }
                }
            }
        }
    }

    /// Runs strictly on an owning XAML UI thread via `DispatcherQueue` or `ICoreDispatcher`.
    fn apply_for_ui_thread(rt: &Arc<TapRuntime>, filter_queue_tid: Option<u32>) {
        struct SnapshotItem {
            handle: u64,
            weak_ref_raw: usize,
            orig_opacity: f64,
        }

        let (desired_opacity, items): (Option<f64>, Vec<SnapshotItem>) = {
            let Ok(g) = rt.state.lock() else {
                return;
            };
            let target_hide = g.desired_hide && !g.shutdown;
            let list = g
                .targets
                .iter()
                .filter(|(_, t)| match filter_queue_tid {
                    Some(tid) => t.thread_id == Some(tid),
                    None => matches!(t.dispatcher, OwnedDispatcher::Core(_)),
                })
                .map(|(&handle, t)| {
                    add_ref_raw_com_addr(t.weak_ref_raw);
                    SnapshotItem {
                        handle,
                        weak_ref_raw: t.weak_ref_raw,
                        orig_opacity: t.orig_opacity,
                    }
                })
                .collect();
            (if target_hide { Some(0.0) } else { None }, list)
        };

        if items.is_empty() {
            return;
        }

        let mut stale_handles = Vec::new();
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

                let mut ui_elem = ComRaw::null();
                let unk = insp.as_ptr().cast::<RawIUnknown>();
                let hr_qi =
                    ((*(*unk).lp_vtbl).query_interface)(insp.as_ptr(), &IID_IUIELEMENT, &mut ui_elem.0);
                if hr_qi.is_err() || ui_elem.is_null() {
                    continue;
                }

                let ui = ui_elem.as_ptr().cast::<RawIUIElement>();
                let target_val = desired_opacity.unwrap_or(item.orig_opacity);
                let mut current_val = -1.0f64;
                let hr_get = ((*(*ui).lp_vtbl).get_opacity)(ui_elem.as_ptr(), &mut current_val);
                if hr_get.is_err() || (current_val - target_val).abs() > 0.001 {
                    let _ = ((*(*ui).lp_vtbl).put_opacity)(ui_elem.as_ptr(), target_val);
                }
            }
        }

        if !stale_handles.is_empty() {
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
    }

    /// Runs on the XAML UI thread from `OnVisualTreeChange(Add)` for matching background shapes.
    unsafe fn on_target_added(rt: &Arc<TapRuntime>, handle: u64) {
        if handle == 0 || rt.is_shutdown() {
            return;
        }
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

        let insp_unk = insp.as_ptr().cast::<RawIUnknown>();
        let mut ui_elem = ComRaw::null();
        if ((*(*insp_unk).lp_vtbl).query_interface)(
            insp.as_ptr(),
            &IID_IUIELEMENT,
            &mut ui_elem.0,
        )
        .is_err()
            || ui_elem.is_null()
        {
            return;
        }

        let ui = ui_elem.as_ptr().cast::<RawIUIElement>();
        let mut orig = 1.0f64;
        if ((*(*ui).lp_vtbl).get_opacity)(ui_elem.as_ptr(), &mut orig).is_err()
            || !orig.is_finite()
            || orig <= 0.01
        {
            orig = 1.0;
        }

        let want_hide_initial = rt.desired_hide();
        if want_hide_initial {
            let _ = ((*(*ui).lp_vtbl).put_opacity)(ui_elem.as_ptr(), 0.0);
        }

        let mut weak_src = ComRaw::null();
        if ((*(*insp_unk).lp_vtbl).query_interface)(
            insp.as_ptr(),
            &IID_IWEAKREFERENCESOURCE,
            &mut weak_src.0,
        )
        .is_err()
            || weak_src.is_null()
        {
            return;
        }

        let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
        let mut weak_ref = ComRaw::null();
        if ((*(*src_iface).lp_vtbl).get_weak_reference)(weak_src.as_ptr(), &mut weak_ref.0).is_err()
            || weak_ref.is_null()
        {
            return;
        }

        let (thread_id, dispatcher) = if let Ok(dq) = DispatcherQueue::GetForCurrentThread() {
            (
                Some(GetCurrentThreadId()),
                OwnedDispatcher::Queue(dq.into_raw() as usize),
            )
        } else {
            let mut dep = ComRaw::null();
            if ((*(*insp_unk).lp_vtbl).query_interface)(
                insp.as_ptr(),
                &IID_IDEPENDENCYOBJECT,
                &mut dep.0,
            )
            .is_err()
                || dep.is_null()
            {
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
            (None, OwnedDispatcher::Core(cookie))
        };

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
                        orig_opacity: orig,
                        dispatcher,
                    },
                );
                want_hide_after = g.desired_hide;
            } else {
                evicted = Some(TrackedTarget {
                    thread_id,
                    weak_ref_raw: weak_ref.into_raw() as usize,
                    orig_opacity: orig,
                    dispatcher,
                });
            }
        } else {
            evicted = Some(TrackedTarget {
                thread_id,
                weak_ref_raw: weak_ref.into_raw() as usize,
                orig_opacity: orig,
                dispatcher,
            });
        }

        if let Some(old) = evicted {
            release_target_resources(old, Some(&git));
        }

        if shutdown_after {
            if want_hide_initial {
                let _ = ((*(*ui).lp_vtbl).put_opacity)(ui_elem.as_ptr(), orig);
            }
        } else if want_hide_after != want_hide_initial {
            let next = if want_hide_after { 0.0 } else { orig };
            let _ = ((*(*ui).lp_vtbl).put_opacity)(ui_elem.as_ptr(), next);
        }
    }

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
            let type_name = bstr_ptr_to_string(element.type_name);
            let name = bstr_ptr_to_string(element.name);
            let matched = if let Ok(mut idx) = rt.tree.lock() {
                idx.on_add(relation.parent, handle, type_name, name);
                idx.is_taskbar_bg_target(handle)
            } else {
                false
            };
            if matched {
                on_target_added(rt, handle);
            }
        } else {
            let removed = if let Ok(mut idx) = rt.tree.lock() {
                idx.on_remove(handle)
            } else {
                Vec::new()
            };
            if !removed.is_empty() {
                let mut evicted = Vec::new();
                if let Ok(mut g) = rt.state.lock() {
                    for rh in removed {
                        if let Some(old) = g.targets.remove(&rh) {
                            evicted.push(old);
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
        site_unk: Mutex<usize>,
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
                site_unk: Mutex::new(0),
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
            let raw_su = (*this)
                .site_unk
                .lock()
                .ok()
                .map(|mut su| {
                    let raw = *su;
                    *su = 0;
                    raw
                })
                .unwrap_or(0);
            release_raw_com_addr(raw_su);
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
            let old_su = site
                .site_unk
                .lock()
                .ok()
                .map(|mut su| {
                    let old = *su;
                    *su = 0;
                    old
                })
                .unwrap_or(0);
            release_raw_com_addr(old_su);
            return S_OK;
        }

        let prev_rt = site.runtime.lock().ok().and_then(|mut g| g.take());
        if let Some(old_rt) = prev_rt {
            old_rt.trigger_shutdown(300);
        }
        let prev_su = site
            .site_unk
            .lock()
            .ok()
            .map(|mut su| {
                let old = *su;
                ((*(*punk_site).lp_vtbl).add_ref)(punk_site.cast());
                *su = punk_site as usize;
                old
            })
            .unwrap_or(0);
        release_raw_com_addr(prev_su);

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

        let Ok(runtime) = TapRuntime::new(diag_cookie) else {
            let _ = git.RevokeInterfaceFromGlobal(diag_cookie);
            let _ = git.RevokeInterfaceFromGlobal(vts_cookie);
            return E_FAIL_HR;
        };

        if let Ok(mut rt_guard) = site.runtime.lock() {
            *rt_guard = Some(Arc::clone(&runtime));
        }

        let rt_advisor = Arc::clone(&runtime);
        let rt_pipe = Arc::clone(&runtime);

        ACTIVE_OBJECTS.fetch_add(2, Ordering::SeqCst);
        std::thread::spawn(move || {
            run_advisor_thread(rt_advisor, vts_cookie);
            ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
        });
        std::thread::spawn(move || {
            run_pipe_server_loop(rt_pipe);
            ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
        });

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
        let raw = site.site_unk.lock().map(|g| *g).unwrap_or(0);
        if raw == 0 {
            return E_FAIL_HR;
        }
        let unk = raw as *mut RawIUnknown;
        ((*(*unk).lp_vtbl).query_interface)(unk.cast(), riid, ppv)
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
        hinst: HINSTANCE,
        reason: u32,
        _reserved: *mut c_void,
    ) -> BOOL {
        if reason == DLL_PROCESS_ATTACH_CODE {
            let _ = DisableThreadLibraryCalls(hinst);
        }
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
        if ACTIVE_OBJECTS.load(Ordering::SeqCst) == 0 {
            S_OK
        } else {
            S_FALSE
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

                let _ = WaitForSingleObject(rt.stop_event_handle(), INFINITE);

                if hr_adv.is_ok() {
                    let _ = ((*(*vts).lp_vtbl).unadvise_visual_tree_change)(
                        vts_com.as_ptr(),
                        watcher.cast(),
                    );
                }
                watcher_release(watcher);
            } else {
                let _ = WaitForSingleObject(rt.stop_event_handle(), INFINITE);
            }

            drop(vts_com);

            let leftover: Vec<TrackedTarget> = if let Ok(mut g) = rt.state.lock() {
                g.shutdown = true;
                g.targets.drain().map(|(_, t)| t).collect()
            } else {
                Vec::new()
            };
            for t in leftover {
                release_target_resources(t, git.as_ref());
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

    // ---------- Named Pipe IPC + Host Crash Watchdog Thread ----------

    unsafe fn read_pipe_exact(pipe: HANDLE, event: HANDLE, buf: &mut [u8]) -> bool {
        let mut off = 0usize;
        while off < buf.len() {
            let _ = ResetEvent(event);
            let mut ov = OVERLAPPED {
                hEvent: event,
                ..Default::default()
            };
            let mut read = 0u32;
            let res = ReadFile(
                pipe,
                Some(&mut buf[off..]),
                Some(&mut read),
                Some(&mut ov),
            );
            let got = if res.is_ok() {
                read as usize
            } else if GetLastError() == ERROR_IO_PENDING {
                if WaitForSingleObject(event, 1000) != WAIT_OBJECT_0 {
                    let _ = CancelIo(pipe);
                    return false;
                }
                let mut tr = 0u32;
                if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
                    return false;
                }
                tr as usize
            } else {
                return false;
            };
            if got == 0 {
                return false;
            }
            off += got;
        }
        true
    }

    unsafe fn write_pipe_exact(pipe: HANDLE, event: HANDLE, buf: &[u8]) -> bool {
        let mut off = 0usize;
        while off < buf.len() {
            let _ = ResetEvent(event);
            let mut ov = OVERLAPPED {
                hEvent: event,
                ..Default::default()
            };
            let mut written = 0u32;
            let res = WriteFile(
                pipe,
                Some(&buf[off..]),
                Some(&mut written),
                Some(&mut ov),
            );
            let put = if res.is_ok() {
                written as usize
            } else if GetLastError() == ERROR_IO_PENDING {
                if WaitForSingleObject(event, 1000) != WAIT_OBJECT_0 {
                    let _ = CancelIo(pipe);
                    return false;
                }
                let mut tr = 0u32;
                if GetOverlappedResult(pipe, &ov, &mut tr, false).is_err() {
                    return false;
                }
                tr as usize
            } else {
                return false;
            };
            if put == 0 {
                return false;
            }
            off += put;
        }
        let _ = FlushFileBuffers(pipe);
        true
    }

    fn run_pipe_server_loop(rt: Arc<TapRuntime>) {
        unsafe {
            let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let pid = GetCurrentProcessId();
            let wide_name: Vec<u16> = pipe_name_for_pid(pid)
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            let pipe = CreateNamedPipeW(
                PCWSTR(wide_name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                64,
                64,
                0,
                None,
            );
            if pipe == INVALID_HANDLE_VALUE || pipe.is_invalid() {
                if com_ok {
                    CoUninitialize();
                }
                return;
            }

            let Ok(conn_event) = CreateEventW(None, true, false, None) else {
                let _ = CloseHandle(pipe);
                if com_ok {
                    CoUninitialize();
                }
                return;
            };
            let Ok(io_event) = CreateEventW(None, true, false, None) else {
                let _ = CloseHandle(conn_event);
                let _ = CloseHandle(pipe);
                if com_ok {
                    CoUninitialize();
                }
                return;
            };

            while !rt.stop_requested.load(Ordering::SeqCst) {
                let _ = ResetEvent(conn_event);
                let mut ov = OVERLAPPED {
                    hEvent: conn_event,
                    ..Default::default()
                };
                let conn_res = ConnectNamedPipe(pipe, Some(&mut ov));
                let connected = if conn_res.is_ok() {
                    true
                } else {
                    let err = GetLastError();
                    if err == ERROR_PIPE_CONNECTED {
                        true
                    } else if err == ERROR_IO_PENDING {
                        let mut wait_handles = [conn_event, rt.stop_event_handle(), HANDLE::default()];
                        let mut count = 2usize;
                        let mut host_h_copy = HANDLE::default();
                        if let Ok(hh) = rt.host_handle_raw.lock() {
                            if let Some(raw) = *hh {
                                if raw != 0 {
                                    let h = HANDLE(raw as *mut c_void);
                                    host_h_copy = h;
                                    wait_handles[2] = h;
                                    count = 3;
                                }
                            }
                        }

                        loop {
                            let w = WaitForMultipleObjects(&wait_handles[..count], false, 300);
                            if w == WAIT_OBJECT_0 {
                                let _ = &ov;
                                let mut tr = 0u32;
                                break GetOverlappedResult(pipe, &ov, &mut tr, false).is_ok();
                            } else if w == WAIT_EVENT_INDEX(1) || rt.stop_requested.load(Ordering::SeqCst) {
                                let _ = CancelIo(pipe);
                                break false;
                            } else if count == 3 && w == WAIT_EVENT_INDEX(2) {
                                let _ = CancelIo(pipe);
                                rt.trigger_shutdown(500);
                                break false;
                            } else if !host_h_copy.is_invalid()
                                && WaitForSingleObject(host_h_copy, 0) == WAIT_OBJECT_0
                            {
                                let _ = CancelIo(pipe);
                                rt.trigger_shutdown(500);
                                break false;
                            }
                        }
                    } else {
                        false
                    }
                };

                if !connected {
                    if rt.stop_requested.load(Ordering::SeqCst) {
                        break;
                    }
                    continue;
                }

                let mut req = [0u8; 5];
                if read_pipe_exact(pipe, io_event, &mut req) {
                    let cmd = req[0];
                    let host_pid = u32::from_le_bytes([req[1], req[2], req[3], req[4]]);
                    match cmd {
                        CMD_HIDE => {
                            rt.set_hide(true, host_pid);
                            let _ = write_pipe_exact(pipe, io_event, &[cmd]);
                        }
                        CMD_RESTORE => {
                            rt.set_hide(false, host_pid);
                            let _ = write_pipe_exact(pipe, io_event, &[cmd]);
                        }
                        CMD_PING => {
                            let _ = write_pipe_exact(pipe, io_event, &[cmd]);
                        }
                        CMD_SHUTDOWN => {
                            rt.trigger_shutdown(600);
                            let _ = write_pipe_exact(pipe, io_event, &[cmd]);
                            let _ = DisconnectNamedPipe(pipe);
                            break;
                        }
                        _ => {
                            let _ = write_pipe_exact(pipe, io_event, &[0xFF]);
                        }
                    }
                }

                let _ = DisconnectNamedPipe(pipe);
            }

            let _ = CloseHandle(io_event);
            let _ = CloseHandle(conn_event);
            let _ = CloseHandle(pipe);
            if com_ok {
                CoUninitialize();
            }
        }
    }

    #[inline]
    #[allow(non_snake_case)]
    const fn WAIT_EVENT_INDEX(idx: u32) -> windows::Win32::Foundation::WAIT_EVENT {
        windows::Win32::Foundation::WAIT_EVENT(WAIT_OBJECT_0.0 + idx)
    }
}
