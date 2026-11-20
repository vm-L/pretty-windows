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
    pub const CMD_STATUS: u8 = 0x04;
    pub const CMD_VERSION: u8 = 0x05;
    pub const TAP_PROTOCOL_VERSION: u8 = 2;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum TargetKind {
        ShapeElement,
        OverflowBorder,
        OverflowPanel,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct NodeMeta {
        pub parent: u64,
        pub type_name: String,
        pub name: String,
        pub thread_id: u32,
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

        pub fn node_count(&self) -> usize {
            self.nodes.len()
        }

        pub fn on_add(&mut self, parent: u64, child: u64, type_name: String, name: String) {
            self.on_add_on_thread(parent, child, type_name, name, 0);
        }

        pub fn on_add_on_thread(
            &mut self,
            parent: u64,
            child: u64,
            type_name: String,
            name: String,
            thread_id: u32,
        ) {
            if child == 0 {
                return;
            }
            self.nodes.insert(
                child,
                NodeMeta {
                    parent,
                    type_name,
                    name,
                    thread_id,
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
                if meta.type_name.contains("TaskbarFrame")
                    || meta.type_name.contains("TaskbarBackground")
                {
                    has_taskbar_frame = true;
                }
                cur = meta.parent;
            }
            has_taskbar_frame
        }

        fn is_forbidden_overflow_node(meta: &NodeMeta) -> bool {
            let contains_notify_icon_non_overflow = (meta.type_name.contains("NotifyIcon")
                && !meta.type_name.contains("Overflow"))
                || (meta.name.contains("NotifyIcon") && !meta.name.contains("Overflow"));
            contains_notify_icon_non_overflow
                || meta.type_name.contains("ItemsControl")
                || meta.name.contains("ItemsControl")
                || meta.type_name.contains("ItemsPresenter")
                || meta.name.contains("ItemsPresenter")
                || meta.type_name.contains("WrapGrid")
                || meta.name.contains("WrapGrid")
                || meta.type_name.contains("Button")
                || meta.name.contains("Button")
                || meta.type_name.contains("Tooltip")
                || meta.name.contains("Tooltip")
                || meta.type_name.contains("MenuFlyout")
                || meta.name.contains("MenuFlyout")
                || meta.type_name.contains("StartMenu")
                || meta.name.contains("StartMenu")
        }

        fn is_overflow_container_node(meta: &NodeMeta) -> bool {
            meta.name == "OverflowRootGrid"
                || meta.type_name.contains("NotificationAreaOverflow")
                || meta.type_name.contains("NotifyIconOverflow")
                || meta.type_name.contains("OverflowView")
        }

        pub fn has_forbidden_overflow_ancestor(&self, handle: u64) -> bool {
            let Some(node) = self.nodes.get(&handle) else {
                return true;
            };
            if Self::is_forbidden_overflow_node(node) {
                return true;
            }
            let mut seen = [0u64; 32];
            let mut seen_len = 0usize;
            let mut cur = node.parent;
            while cur != 0 && seen_len < seen.len() {
                if cur == handle || seen[..seen_len].contains(&cur) {
                    return true;
                }
                seen[seen_len] = cur;
                seen_len += 1;
                let Some(meta) = self.nodes.get(&cur) else {
                    break;
                };
                if Self::is_forbidden_overflow_node(meta) {
                    return true;
                }
                cur = meta.parent;
            }
            false
        }

        pub fn has_valid_overflow_ancestor(&self, handle: u64) -> bool {
            let Some(node) = self.nodes.get(&handle) else {
                return false;
            };
            if Self::is_forbidden_overflow_node(node) {
                return false;
            }
            let mut seen = [0u64; 32];
            let mut seen_len = 0usize;
            let mut cur = node.parent;
            let mut has_overflow = false;
            while cur != 0 && seen_len < seen.len() {
                if cur == handle || seen[..seen_len].contains(&cur) {
                    return false;
                }
                seen[seen_len] = cur;
                seen_len += 1;
                let Some(meta) = self.nodes.get(&cur) else {
                    break;
                };
                if Self::is_forbidden_overflow_node(meta) {
                    return false;
                }
                if Self::is_overflow_container_node(meta) {
                    has_overflow = true;
                }
                cur = meta.parent;
            }
            has_overflow
        }

        pub fn classify_target(&self, handle: u64) -> Option<TargetKind> {
            let node = self.nodes.get(&handle)?;
            if matches!(
                node.name.as_str(),
                "BackgroundFill" | "BackgroundStroke" | "ScreenEdgeStroke"
            ) {
                if self.has_valid_taskbar_ancestor(handle) || self.has_valid_overflow_ancestor(handle)
                {
                    return Some(TargetKind::ShapeElement);
                }
                return None;
            }
            if node.name == "OverflowRootGrid" && !self.has_forbidden_overflow_ancestor(handle) {
                return Some(TargetKind::OverflowPanel);
            }
            if node.name == "OverflowFlyoutBackgroundBorder"
                && !self.has_forbidden_overflow_ancestor(handle)
            {
                return Some(TargetKind::OverflowBorder);
            }
            if (node.type_name.ends_with(".Border") || node.type_name == "Border")
                && !self.has_forbidden_overflow_ancestor(handle)
            {
                let parent_is_root = self
                    .nodes
                    .get(&node.parent)
                    .is_some_and(|p| p.name == "OverflowRootGrid");
                if parent_is_root
                    || (node.name.contains("Background") && self.has_valid_overflow_ancestor(handle))
                {
                    return Some(TargetKind::OverflowBorder);
                }
            }
            None
        }

        /// Returns `true` only for `BackgroundFill`/`BackgroundStroke`/`ScreenEdgeStroke` rectangles inside a non-flyout taskbar container.
        pub fn is_taskbar_bg_target(&self, handle: u64) -> bool {
            self.classify_target(handle) == Some(TargetKind::ShapeElement)
        }

        pub fn collect_classified_targets(&self) -> Vec<(u64, TargetKind)> {
            self.nodes
                .keys()
                .copied()
                .filter_map(|h| self.classify_target(h).map(|k| (h, k)))
                .collect()
        }

        pub fn collect_classified_targets_for_thread(
            &self,
            thread_id: u32,
        ) -> Vec<(u64, TargetKind)> {
            self.nodes
                .iter()
                .filter(|(_, meta)| meta.thread_id == thread_id)
                .filter_map(|(&h, _)| self.classify_target(h).map(|k| (h, k)))
                .collect()
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

        // ScreenEdgeStroke directly under TaskbarBackground (even before TaskbarFrame attaches above)
        idx.on_add(0, 30, "Taskbar.TaskbarBackground".into(), "TaskbarBG".into());
        idx.on_add(
            30,
            31,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "ScreenEdgeStroke".into(),
        );

        // Out-of-order add: child BackgroundFill arrives before parent TaskbarBackground arrives
        idx.on_add(
            41,
            42,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
        );
        assert!(!idx.is_taskbar_bg_target(42));
        idx.on_add(0, 41, "Taskbar.TaskbarBackground".into(), "LateBG".into());
        assert!(idx.is_taskbar_bg_target(42));

        let targets: HashSet<u64> = idx.collect_targets().into_iter().collect();
        assert_eq!(targets, HashSet::from([5, 6, 27, 28, 31, 42]));
        assert!(idx.node_count() >= 15);
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

    #[test]
    fn classifies_overflow_border_and_panel_while_excluding_icon_tiles() {
        let mut idx = VisualTreeIndex::new();
        // 1. Standard Win11 overflow flyout tree: NotificationAreaOverflow > Grid#OverflowRootGrid > Border
        idx.on_add(
            0,
            200,
            "SystemTray.NotificationAreaOverflow".into(),
            "OverflowView".into(),
        );
        idx.on_add(
            200,
            201,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "OverflowRootGrid".into(),
        );
        idx.on_add(201, 202, "Windows.UI.Xaml.Controls.Border".into(), "".into());
        idx.on_add(
            200,
            203,
            "Windows.UI.Xaml.Controls.Border".into(),
            "OverflowFlyoutBackgroundBorder".into(),
        );

        // 2. Child icon buttons & tiles inside OverflowRootGrid > Border must NEVER be touched
        idx.on_add(
            202,
            210,
            "Windows.UI.Xaml.Controls.ItemsControl".into(),
            "OverflowItemsControl".into(),
        );
        idx.on_add(
            210,
            211,
            "Windows.UI.Xaml.Controls.ItemsPresenter".into(),
            "".into(),
        );
        idx.on_add(
            211,
            212,
            "Windows.UI.Xaml.Controls.WrapGrid".into(),
            "".into(),
        );
        idx.on_add(
            212,
            213,
            "SystemTray.NotifyIconView".into(),
            "".into(),
        );
        idx.on_add(213, 214, "Windows.UI.Xaml.Controls.Button".into(), "".into());
        idx.on_add(
            214,
            215,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
        );

        // 3. Out-of-order add: background border arrives before OverflowRootGrid parent arrives
        idx.on_add(230, 231, "Windows.UI.Xaml.Controls.Border".into(), "".into());
        assert_eq!(idx.classify_target(231), None);
        idx.on_add(
            0,
            230,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "OverflowRootGrid".into(),
        );

        assert_eq!(idx.classify_target(201), Some(TargetKind::OverflowPanel));
        assert_eq!(idx.classify_target(202), Some(TargetKind::OverflowBorder));
        assert_eq!(idx.classify_target(203), Some(TargetKind::OverflowBorder));
        assert_eq!(idx.classify_target(230), Some(TargetKind::OverflowPanel));
        assert_eq!(idx.classify_target(231), Some(TargetKind::OverflowBorder));
        assert_eq!(idx.classify_target(215), None);

        let classified: HashSet<(u64, TargetKind)> =
            idx.collect_classified_targets().into_iter().collect();
        assert_eq!(
            classified,
            HashSet::from([
                (201, TargetKind::OverflowPanel),
                (202, TargetKind::OverflowBorder),
                (203, TargetKind::OverflowBorder),
                (230, TargetKind::OverflowPanel),
                (231, TargetKind::OverflowBorder),
            ])
        );
    }

    #[test]
    fn collect_classified_targets_for_thread_isolates_ui_threads() {
        let mut idx = VisualTreeIndex::new();
        idx.on_add_on_thread(0, 1, "Taskbar.TaskbarFrame".into(), "TaskbarFrame".into(), 101);
        idx.on_add_on_thread(
            1,
            2,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
            101,
        );

        idx.on_add_on_thread(
            0,
            10,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "OverflowRootGrid".into(),
            202,
        );
        idx.on_add_on_thread(
            10,
            11,
            "Windows.UI.Xaml.Controls.Border".into(),
            "".into(),
            202,
        );

        let t1: HashSet<(u64, TargetKind)> =
            idx.collect_classified_targets_for_thread(101).into_iter().collect();
        let t2: HashSet<(u64, TargetKind)> =
            idx.collect_classified_targets_for_thread(202).into_iter().collect();

        assert_eq!(t1, HashSet::from([(2, TargetKind::ShapeElement)]));
        assert_eq!(
            t2,
            HashSet::from([
                (10, TargetKind::OverflowPanel),
                (11, TargetKind::OverflowBorder),
            ])
        );
    }
}

#[cfg(windows)]
mod win {
    use super::tree_match::{
        pipe_name_for_pid, TargetKind, VisualTreeIndex, CLSID_PRETTY_WINDOWS_TAP_U128, CMD_HIDE,
        CMD_PING, CMD_RESTORE, CMD_SHUTDOWN, CMD_STATUS, CMD_VERSION, TAP_PROTOCOL_VERSION,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};
    use windows::core::{IInspectable, IUnknown, Interface, GUID, HRESULT, HSTRING, PCWSTR};
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
    use windows::Win32::System::WinRT::RoGetActivationFactory;

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
    const IID_IDEPENDENCYOBJECT2: GUID = GUID::from_u128(0x29FED85D_3D22_43A1_ADD0_17027C08B212);
    const IID_IDEPENDENCYPROPERTYCHANGEDCALLBACK: GUID =
        GUID::from_u128(0x45883D16_27BF_4BC1_AC26_94C1601F3A49);
    const IID_ICOREDISPATCHER: GUID = GUID::from_u128(0x60DB2FA8_B705_4FDE_A7D6_EBBB1891D39E);
    const IID_IDISPATCHEDHANDLER: GUID = GUID::from_u128(0xD1F276C4_98D8_4636_BF49_EB79507548E9);
    const IID_IBORDER: GUID = GUID::from_u128(0x797C4539_45BD_4633_A044_BFB02EF5170F);
    const IID_IBORDERSTATICS: GUID = GUID::from_u128(0xB81D1639_E911_4FD7_A4C4_B9C7F008B7FC);
    const IID_IPANEL: GUID = GUID::from_u128(0xA50A4BBD_8361_469C_90DA_E9A40C7474DF);
    const IID_IPANELSTATICS: GUID = GUID::from_u128(0xF23D6B5D_8330_47A6_A046_25F509B25232);
    const IID_IBRUSH: GUID = GUID::from_u128(0x8806A321_1E06_422C_A1CC_01696559E021);
    const IID_ISOLIDCOLORBRUSH: GUID = GUID::from_u128(0x9D850850_66F3_48DF_9A8F_824BD5E070AF);
    const IID_ISOLIDCOLORBRUSHFACTORY: GUID =
        GUID::from_u128(0xD935CE0C_86F5_4DA6_8A27_B1619EF7F92B);
    const IID_IDEPENDENCYPROPERTYSTATICS: GUID =
        GUID::from_u128(0x49E5F28F_8259_4D5C_AAE0_83D56DBB68D9);

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
        get_clip: unsafe extern "system" fn(*mut c_void),
        put_clip: unsafe extern "system" fn(*mut c_void),
        get_render_transform: unsafe extern "system" fn(*mut c_void),
        put_render_transform: unsafe extern "system" fn(*mut c_void),
        get_projection: unsafe extern "system" fn(*mut c_void),
        put_projection: unsafe extern "system" fn(*mut c_void),
        get_render_transform_origin: unsafe extern "system" fn(*mut c_void),
        put_render_transform_origin: unsafe extern "system" fn(*mut c_void),
        get_is_hit_test_visible: unsafe extern "system" fn(*mut c_void),
        put_is_hit_test_visible: unsafe extern "system" fn(*mut c_void),
        get_visibility: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
        put_visibility: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    }

    const VISIBILITY_VISIBLE: i32 = 0;
    const VISIBILITY_COLLAPSED: i32 = 1;

    const DIAG_WATCHER_CALLED: u32 = 1 << 0;
    const DIAG_SAW_FRAME_OR_BG: u32 = 1 << 1;
    const DIAG_SAW_TARGET_NAME: u32 = 1 << 2;
    const DIAG_TARGET_MATCHED: u32 = 1 << 3;
    const DIAG_ENTER_ON_TARGET: u32 = 1 << 4;
    const DIAG_DIAG_FROM_GIT_OK: u32 = 1 << 5;
    const DIAG_GET_INSP_OK: u32 = 1 << 6;
    const DIAG_QI_UI_OK: u32 = 1 << 7;
    const DIAG_QI_WEAK_OK: u32 = 1 << 8;
    const DIAG_DISPATCHER_OK: u32 = 1 << 9;
    const DIAG_INSERTED_TARGET: u32 = 1 << 10;
    const DIAG_APPLY_RAN: u32 = 1 << 11;
    const DIAG_OVERFLOW_APPLIED: u32 = 1 << 12;

    #[repr(C)]
    struct RawIUIElement {
        lp_vtbl: *const RawIUIElementVtbl,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawUiColor {
        a: u8,
        r: u8,
        g: u8,
        b: u8,
    }

    const _: () = {
        assert!(size_of::<RawUiColor>() == 4);
        assert!(core::mem::align_of::<RawUiColor>() == 1);
    };

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIBorderVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
        get_border_thickness: unsafe extern "system" fn(*mut c_void),
        put_border_thickness: unsafe extern "system" fn(*mut c_void),
        get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIBorder {
        lp_vtbl: *const RawIBorderVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIPanelVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_children: unsafe extern "system" fn(*mut c_void),
        get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIPanel {
        lp_vtbl: *const RawIPanelVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawISolidColorBrushVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_color: unsafe extern "system" fn(*mut c_void, *mut RawUiColor) -> HRESULT,
        put_color: unsafe extern "system" fn(*mut c_void, RawUiColor) -> HRESULT,
    }

    #[repr(C)]
    struct RawISolidColorBrush {
        lp_vtbl: *const RawISolidColorBrushVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawISolidColorBrushFactoryVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        create_instance_with_color:
            unsafe extern "system" fn(*mut c_void, RawUiColor, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawISolidColorBrushFactory {
        lp_vtbl: *const RawISolidColorBrushFactoryVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
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
        clear_value: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
        read_local_value:
            unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
        get_animation_base_value: unsafe extern "system" fn(*mut c_void),
        get_dispatcher: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIDependencyObject {
        lp_vtbl: *const RawIDependencyObjectVtbl,
    }

    #[repr(C)]
    struct RawIDependencyObject2Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        register_property_changed_callback: unsafe extern "system" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
            *mut i64,
        ) -> HRESULT,
        unregister_property_changed_callback:
            unsafe extern "system" fn(*mut c_void, *mut c_void, i64) -> HRESULT,
    }

    #[repr(C)]
    struct RawIDependencyObject2 {
        lp_vtbl: *const RawIDependencyObject2Vtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIBorderStaticsVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_border_brush_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        get_border_thickness_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        get_background_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIBorderStatics {
        lp_vtbl: *const RawIBorderStaticsVtbl,
    }

    #[repr(C)]
    struct RawIPanelStaticsVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_background_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIPanelStatics {
        lp_vtbl: *const RawIPanelStaticsVtbl,
    }

    #[repr(C)]
    struct RawIDependencyPropertyStaticsVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_unset_value: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIDependencyPropertyStatics {
        lp_vtbl: *const RawIDependencyPropertyStaticsVtbl,
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
        fn clone_ref(&self) -> Self {
            add_ref_raw_com_addr(self.0 as usize);
            Self(self.0)
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
        kind: TargetKind,
        orig_opacity: f64,
        orig_visibility: i32,
        dispatcher: OwnedDispatcher,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum DispatchFilter {
        QueueTid(u32),
        CoreCookie(u32),
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

    #[inline]
    fn revoke_git_cookie(git: Option<&IGlobalInterfaceTable>, cookie: u32) {
        if cookie != 0 {
            if let Some(g) = git {
                unsafe {
                    let _ = g.RevokeInterfaceFromGlobal(cookie);
                }
            }
        }
    }

    unsafe fn qi_interface(ptr: *mut c_void, iid: &GUID) -> ComRaw {
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

    unsafe fn com_same_identity(a: *mut c_void, b: *mut c_void) -> bool {
        if a.is_null() || b.is_null() {
            return false;
        }
        let ua = qi_interface(a, &IID_IUNKNOWN);
        let ub = qi_interface(b, &IID_IUNKNOWN);
        !ua.is_null() && ua.as_ptr() == ub.as_ptr()
    }

    unsafe fn activate_factory_iface(class_name: &str, iid: &GUID) -> Option<ComRaw> {
        let class_id = HSTRING::from(class_name);
        let factory_insp: IInspectable = RoGetActivationFactory(&class_id).ok()?;
        let out = qi_interface(factory_insp.as_raw(), iid);
        (!out.is_null()).then_some(out)
    }

    unsafe fn fetch_transparent_brush() -> Option<ComRaw> {
        let factory_com = activate_factory_iface(
            "Windows.UI.Xaml.Media.SolidColorBrush",
            &IID_ISOLIDCOLORBRUSHFACTORY,
        )?;
        let fac = factory_com.as_ptr().cast::<RawISolidColorBrushFactory>();
        let mut solid_com = ComRaw::null();
        if ((*(*fac).lp_vtbl).create_instance_with_color)(
            factory_com.as_ptr(),
            RawUiColor {
                a: 0,
                r: 0,
                g: 0,
                b: 0,
            },
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

    unsafe fn fetch_unset_value() -> Option<ComRaw> {
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

    unsafe fn fetch_border_bg_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_border_brush_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_panel_bg_dp() -> Option<ComRaw> {
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

    unsafe fn is_transparent_solid_brush(brush_ptr: *mut c_void) -> bool {
        if brush_ptr.is_null() {
            return false;
        }
        let solid_com = qi_interface(brush_ptr, &IID_ISOLIDCOLORBRUSH);
        if solid_com.is_null() {
            return false;
        }
        let solid = solid_com.as_ptr().cast::<RawISolidColorBrush>();
        let mut col = RawUiColor {
            a: 255,
            r: 0,
            g: 0,
            b: 0,
        };
        ((*(*solid).lp_vtbl).get_color)(solid_com.as_ptr(), &mut col).is_ok() && col.a == 0
    }

    enum SavedBrush {
        Unset,
        Local(ComRaw),
    }

    #[derive(Default)]
    struct SavedTargetBrushes {
        background: Option<SavedBrush>,
        border_brush: Option<SavedBrush>,
    }

    struct PropWatchEntry {
        weak_obj: ComRaw,
        dp: ComRaw,
        token: i64,
    }

    #[derive(Default)]
    struct UiThreadBrushStore {
        trans_brush: Option<ComRaw>,
        unset_value: Option<ComRaw>,
        border_bg_dp: Option<ComRaw>,
        border_brush_dp: Option<ComRaw>,
        panel_bg_dp: Option<ComRaw>,
        prewarmed: bool,
        saved: HashMap<u64, SavedTargetBrushes>,
        watches: HashMap<(u64, u8), PropWatchEntry>,
    }

    thread_local! {
        static UI_BRUSH_STORE: RefCell<UiThreadBrushStore> =
            RefCell::new(UiThreadBrushStore::default());
        static IN_PROP_SYNC: Cell<bool> = const { Cell::new(false) };
    }

    struct PropSyncGuard {
        prev: bool,
    }

    impl PropSyncGuard {
        fn enter() -> Self {
            let prev = IN_PROP_SYNC.with(|c| {
                let old = c.get();
                c.set(true);
                old
            });
            Self { prev }
        }
    }

    impl Drop for PropSyncGuard {
        fn drop(&mut self) {
            let prev = self.prev;
            IN_PROP_SYNC.with(|c| c.set(prev));
        }
    }

    #[repr(C)]
    struct RawPropChangedCallbackVtbl {
        query_interface: unsafe extern "system" fn(
            *mut PropChangedCallback,
            *const GUID,
            *mut *mut c_void,
        ) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut PropChangedCallback) -> u32,
        release: unsafe extern "system" fn(*mut PropChangedCallback) -> u32,
        invoke: unsafe extern "system" fn(
            *mut PropChangedCallback,
            *mut c_void,
            *mut c_void,
        ) -> HRESULT,
    }

    static PROP_CHANGED_VTBL: RawPropChangedCallbackVtbl = RawPropChangedCallbackVtbl {
        query_interface: prop_changed_qi,
        add_ref: prop_changed_add_ref,
        release: prop_changed_release,
        invoke: prop_changed_invoke,
    };

    #[repr(C)]
    struct PropChangedCallback {
        lp_vtbl: *const RawPropChangedCallbackVtbl,
        ref_count: AtomicU32,
        runtime: Arc<TapRuntime>,
        handle: u64,
        kind: TargetKind,
        prop: BrushPropKind,
    }

    impl PropChangedCallback {
        fn new_com_ptr(
            runtime: Arc<TapRuntime>,
            handle: u64,
            kind: TargetKind,
            prop: BrushPropKind,
        ) -> *mut Self {
            ACTIVE_OBJECTS.fetch_add(1, Ordering::SeqCst);
            Box::into_raw(Box::new(Self {
                lp_vtbl: &PROP_CHANGED_VTBL,
                ref_count: AtomicU32::new(1),
                runtime,
                handle,
                kind,
                prop,
            }))
        }
    }

    unsafe extern "system" fn prop_changed_qi(
        this: *mut PropChangedCallback,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT {
        if ppv.is_null() || riid.is_null() {
            return E_POINTER;
        }
        *ppv = null_mut();
        let iid = *riid;
        if iid == IID_IUNKNOWN
            || iid == IID_IDEPENDENCYPROPERTYCHANGEDCALLBACK
            || iid == IID_IAGILEOBJECT
        {
            *ppv = this.cast();
            prop_changed_add_ref(this);
            S_OK
        } else {
            E_NOINTERFACE
        }
    }

    unsafe extern "system" fn prop_changed_add_ref(this: *mut PropChangedCallback) -> u32 {
        (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    unsafe extern "system" fn prop_changed_release(this: *mut PropChangedCallback) -> u32 {
        let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            std::sync::atomic::fence(Ordering::Acquire);
            drop(Box::from_raw(this));
            ACTIVE_OBJECTS.fetch_sub(1, Ordering::SeqCst);
        }
        prev - 1
    }

    unsafe extern "system" fn prop_changed_invoke(
        this: *mut PropChangedCallback,
        sender: *mut c_void,
        _dp: *mut c_void,
    ) -> HRESULT {
        if sender.is_null() {
            return S_OK;
        }
        let cb = &*this;
        if cb.runtime.is_shutdown() || !cb.runtime.desired_hide() {
            return S_OK;
        }
        if IN_PROP_SYNC.with(|c| c.get()) {
            return S_OK;
        }
        let _sync_guard = PropSyncGuard::enter();
        match cb.kind {
            TargetKind::OverflowBorder => {
                let border_com = qi_interface(sender, &IID_IBORDER);
                if !border_com.is_null() {
                    sync_overflow_brush_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        border_com.as_ptr(),
                        cb.prop,
                        true,
                        None,
                    );
                }
            }
            TargetKind::OverflowPanel => {
                let panel_com = qi_interface(sender, &IID_IPANEL);
                if !panel_com.is_null() {
                    sync_overflow_brush_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        panel_com.as_ptr(),
                        cb.prop,
                        true,
                        None,
                    );
                }
            }
            TargetKind::ShapeElement => {}
        }
        S_OK
    }

    unsafe fn prewarm_overflow_brush_store_for_current_thread() {
        let already = UI_BRUSH_STORE.with(|cell| {
            let Ok(mut s) = cell.try_borrow_mut() else {
                return true;
            };
            if s.prewarmed {
                true
            } else {
                s.prewarmed = true;
                false
            }
        });
        if already {
            return;
        }
        let _ = ensure_cached_com(
            |s| s.trans_brush.as_ref(),
            |s, v| s.trans_brush = Some(v),
            fetch_transparent_brush,
        );
        let _ = ensure_cached_com(
            |s| s.unset_value.as_ref(),
            |s, v| s.unset_value = Some(v),
            fetch_unset_value,
        );
        let _ = ensure_cached_com(
            |s| s.border_bg_dp.as_ref(),
            |s, v| s.border_bg_dp = Some(v),
            fetch_border_bg_dp,
        );
        let _ = ensure_cached_com(
            |s| s.border_brush_dp.as_ref(),
            |s, v| s.border_brush_dp = Some(v),
            fetch_border_brush_dp,
        );
        let _ = ensure_cached_com(
            |s| s.panel_bg_dp.as_ref(),
            |s, v| s.panel_bg_dp = Some(v),
            fetch_panel_bg_dp,
        );
    }

    unsafe fn unregister_watch_entry(entry: PropWatchEntry) {
        if entry.token != 0 && !entry.weak_obj.is_null() && !entry.dp.is_null() {
            let wr = entry.weak_obj.as_ptr().cast::<RawIWeakReference>();
            let mut insp = ComRaw::null();
            if ((*(*wr).lp_vtbl).resolve)(
                entry.weak_obj.as_ptr(),
                &IID_IINSPECTABLE,
                &mut insp.0,
            )
            .is_ok()
                && !insp.is_null()
            {
                let dep2 = qi_interface(insp.as_ptr(), &IID_IDEPENDENCYOBJECT2);
                if !dep2.is_null() {
                    let d2 = dep2.as_ptr().cast::<RawIDependencyObject2>();
                    let _ = ((*(*d2).lp_vtbl).unregister_property_changed_callback)(
                        dep2.as_ptr(),
                        entry.dp.as_ptr(),
                        entry.token,
                    );
                }
            }
        }
        drop(entry);
    }

    unsafe fn ensure_prop_watch_registered(
        rt: &Arc<TapRuntime>,
        handle: u64,
        kind: TargetKind,
        prop: BrushPropKind,
        insp_ptr: *mut c_void,
    ) {
        if handle == 0 || insp_ptr.is_null() {
            return;
        }
        let key = (handle, prop.key());
        let exists = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .is_some_and(|s| s.watches.contains_key(&key))
        });
        if exists {
            return;
        }

        let Some(dp) = prop.dependency_property() else {
            return;
        };
        let dep2 = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT2);
        if dep2.is_null() {
            return;
        }
        let weak_src = qi_interface(insp_ptr, &IID_IWEAKREFERENCESOURCE);
        if weak_src.is_null() {
            return;
        }
        let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
        let mut weak_obj = ComRaw::null();
        if ((*(*src_iface).lp_vtbl).get_weak_reference)(weak_src.as_ptr(), &mut weak_obj.0)
            .is_err()
            || weak_obj.is_null()
        {
            return;
        }

        let cb = PropChangedCallback::new_com_ptr(Arc::clone(rt), handle, kind, prop);
        let d2 = dep2.as_ptr().cast::<RawIDependencyObject2>();
        let mut token = 0i64;
        let hr = ((*(*d2).lp_vtbl).register_property_changed_callback)(
            dep2.as_ptr(),
            dp.as_ptr(),
            cb.cast(),
            &mut token,
        );
        prop_changed_release(cb);
        if hr.is_err() || token == 0 {
            return;
        }

        let new_entry = PropWatchEntry {
            weak_obj,
            dp,
            token,
        };
        let dup = UI_BRUSH_STORE.with(|cell| {
            if let Ok(mut s) = cell.try_borrow_mut() {
                if !s.watches.contains_key(&key) {
                    s.watches.insert(key, new_entry);
                    return None;
                }
            }
            Some(new_entry)
        });
        if let Some(entry) = dup {
            unregister_watch_entry(entry);
        }
    }

    fn clear_all_watches_for_current_thread() {
        let removed_watches: Vec<PropWatchEntry> = UI_BRUSH_STORE.with(|cell| {
            let Ok(mut s) = cell.try_borrow_mut() else {
                return Vec::new();
            };
            s.watches.drain().map(|(_, e)| e).collect()
        });
        for entry in removed_watches {
            unsafe {
                unregister_watch_entry(entry);
            }
        }
    }

    unsafe fn ensure_cached_com(
        pick: fn(&UiThreadBrushStore) -> Option<&ComRaw>,
        put: fn(&mut UiThreadBrushStore, ComRaw),
        fetch: unsafe fn() -> Option<ComRaw>,
    ) -> Option<ComRaw> {
        if let Some(hit) = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .and_then(|s| pick(&s).map(ComRaw::clone_ref))
        }) {
            return Some(hit);
        }
        let created = fetch()?;
        let to_store = created.clone_ref();
        let rejected = UI_BRUSH_STORE.with(|cell| {
            if let Ok(mut s) = cell.try_borrow_mut() {
                if pick(&s).is_none() {
                    put(&mut s, to_store);
                    return None;
                }
            }
            Some(to_store)
        });
        drop(rejected);
        Some(created)
    }

    fn has_saved_brush_slot(handle: u64, is_border_brush: bool) -> bool {
        UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow().ok().is_some_and(|s| {
                s.saved.get(&handle).is_some_and(|entry| {
                    if is_border_brush {
                        entry.border_brush.is_some()
                    } else {
                        entry.background.is_some()
                    }
                })
            })
        })
    }

    fn save_brush_slot_if_absent(handle: u64, is_border_brush: bool, val: SavedBrush) {
        let displaced = UI_BRUSH_STORE.with(|cell| {
            if let Ok(mut s) = cell.try_borrow_mut() {
                let entry = s.saved.entry(handle).or_default();
                let slot = if is_border_brush {
                    &mut entry.border_brush
                } else {
                    &mut entry.background
                };
                if slot.is_none() {
                    *slot = Some(val);
                    return None;
                }
            }
            Some(val)
        });
        drop(displaced);
    }

    fn take_saved_target_brushes(handle: u64) -> SavedTargetBrushes {
        UI_BRUSH_STORE
            .with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| s.saved.remove(&handle))
            })
            .unwrap_or_default()
    }

    fn purge_saved_brushes_for_current_thread(handles: &[u64]) {
        if handles.is_empty() {
            return;
        }
        let (removed_brushes, removed_watches): (Vec<SavedTargetBrushes>, Vec<PropWatchEntry>) =
            UI_BRUSH_STORE.with(|cell| {
                let Ok(mut s) = cell.try_borrow_mut() else {
                    return (Vec::new(), Vec::new());
                };
                let brushes = if s.saved.is_empty() {
                    Vec::new()
                } else {
                    handles.iter().filter_map(|h| s.saved.remove(h)).collect()
                };
                let mut watches = Vec::new();
                if !s.watches.is_empty() {
                    for &h in handles {
                        for prop_idx in 0u8..3u8 {
                            if let Some(entry) = s.watches.remove(&(h, prop_idx)) {
                                watches.push(entry);
                            }
                        }
                    }
                }
                (brushes, watches)
            });
        drop(removed_brushes);
        for entry in removed_watches {
            unsafe {
                unregister_watch_entry(entry);
            }
        }
    }

    unsafe fn read_orig_brush(
        insp_ptr: *mut c_void,
        dp: Option<&ComRaw>,
        unset_val: Option<&ComRaw>,
        current_brush: &ComRaw,
    ) -> Option<SavedBrush> {
        if let (Some(dp_com), Some(unset_com)) = (dp, unset_val) {
            let dep_com = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
            if !dep_com.is_null() {
                let dep = dep_com.as_ptr().cast::<RawIDependencyObject>();
                let mut local_val = ComRaw::null();
                if ((*(*dep).lp_vtbl).read_local_value)(
                    dep_com.as_ptr(),
                    dp_com.as_ptr(),
                    &mut local_val.0,
                )
                .is_ok()
                    && !local_val.is_null()
                {
                    if com_same_identity(local_val.as_ptr(), unset_com.as_ptr()) {
                        return Some(SavedBrush::Unset);
                    }
                    let local_brush = qi_interface(local_val.as_ptr(), &IID_IBRUSH);
                    if !local_brush.is_null() && !is_transparent_solid_brush(local_brush.as_ptr())
                    {
                        return Some(SavedBrush::Local(local_brush));
                    }
                }
            }
        }
        if !current_brush.is_null() && !is_transparent_solid_brush(current_brush.as_ptr()) {
            Some(SavedBrush::Local(current_brush.clone_ref()))
        } else if current_brush.is_null() {
            Some(SavedBrush::Unset)
        } else {
            None
        }
    }

    unsafe fn clear_dp_value(insp_ptr: *mut c_void, dp: Option<&ComRaw>) -> bool {
        let Some(dp_com) = dp else {
            return false;
        };
        let dep_com = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        if dep_com.is_null() {
            return false;
        }
        let dep = dep_com.as_ptr().cast::<RawIDependencyObject>();
        ((*(*dep).lp_vtbl).clear_value)(dep_com.as_ptr(), dp_com.as_ptr()).is_ok()
    }

    #[derive(Clone, Copy)]
    enum BrushPropKind {
        BorderBackground,
        BorderBrush,
        PanelBackground,
    }

    impl BrushPropKind {
        #[inline]
        fn key(self) -> u8 {
            match self {
                Self::BorderBackground => 0,
                Self::BorderBrush => 1,
                Self::PanelBackground => 2,
            }
        }

        #[inline]
        fn is_border_brush_slot(self) -> bool {
            matches!(self, Self::BorderBrush)
        }

        #[inline]
        fn allow_null_on_hide(self) -> bool {
            matches!(self, Self::BorderBackground)
        }

        unsafe fn get(self, iface: *mut c_void) -> (HRESULT, ComRaw) {
            let mut cur = ComRaw::null();
            let hr = match self {
                Self::BorderBackground => {
                    let b = iface.cast::<RawIBorder>();
                    ((*(*b).lp_vtbl).get_background)(iface, &mut cur.0)
                }
                Self::BorderBrush => {
                    let b = iface.cast::<RawIBorder>();
                    ((*(*b).lp_vtbl).get_border_brush)(iface, &mut cur.0)
                }
                Self::PanelBackground => {
                    let p = iface.cast::<RawIPanel>();
                    ((*(*p).lp_vtbl).get_background)(iface, &mut cur.0)
                }
            };
            (hr, cur)
        }

        unsafe fn put(self, iface: *mut c_void, brush: *mut c_void) -> HRESULT {
            match self {
                Self::BorderBackground => {
                    let b = iface.cast::<RawIBorder>();
                    ((*(*b).lp_vtbl).put_background)(iface, brush)
                }
                Self::BorderBrush => {
                    let b = iface.cast::<RawIBorder>();
                    ((*(*b).lp_vtbl).put_border_brush)(iface, brush)
                }
                Self::PanelBackground => {
                    let p = iface.cast::<RawIPanel>();
                    ((*(*p).lp_vtbl).put_background)(iface, brush)
                }
            }
        }

        unsafe fn dependency_property(self) -> Option<ComRaw> {
            match self {
                Self::BorderBackground => ensure_cached_com(
                    |s| s.border_bg_dp.as_ref(),
                    |s, v| s.border_bg_dp = Some(v),
                    fetch_border_bg_dp,
                ),
                Self::BorderBrush => ensure_cached_com(
                    |s| s.border_brush_dp.as_ref(),
                    |s, v| s.border_brush_dp = Some(v),
                    fetch_border_brush_dp,
                ),
                Self::PanelBackground => ensure_cached_com(
                    |s| s.panel_bg_dp.as_ref(),
                    |s, v| s.panel_bg_dp = Some(v),
                    fetch_panel_bg_dp,
                ),
            }
        }
    }

    unsafe fn sync_overflow_brush_prop(
        rt: &Arc<TapRuntime>,
        handle: u64,
        insp_ptr: *mut c_void,
        iface: *mut c_void,
        prop: BrushPropKind,
        desired_hide: bool,
        restored_saved: Option<SavedBrush>,
    ) {
        if desired_hide {
            let (hr_get, cur) = prop.get(iface);
            if hr_get.is_err() || is_transparent_solid_brush(cur.as_ptr()) {
                return;
            }
            if !prop.allow_null_on_hide() && cur.is_null() {
                return;
            }
            let Some(tb) = ensure_cached_com(
                |s| s.trans_brush.as_ref(),
                |s, v| s.trans_brush = Some(v),
                fetch_transparent_brush,
            ) else {
                return;
            };
            let captured = if !has_saved_brush_slot(handle, prop.is_border_brush_slot()) {
                let dp = prop.dependency_property();
                let unset = ensure_cached_com(
                    |s| s.unset_value.as_ref(),
                    |s, v| s.unset_value = Some(v),
                    fetch_unset_value,
                );
                read_orig_brush(insp_ptr, dp.as_ref(), unset.as_ref(), &cur)
            } else {
                None
            };
            if {
                let _sync_guard = PropSyncGuard::enter();
                prop.put(iface, tb.as_ptr()).is_ok()
            } {
                if let Some(orig) = captured {
                    save_brush_slot_if_absent(handle, prop.is_border_brush_slot(), orig);
                }
                rt.diag_flags
                    .fetch_or(DIAG_OVERFLOW_APPLIED, Ordering::Relaxed);
            }
        } else {
            let _sync_guard = PropSyncGuard::enter();
            match restored_saved {
                Some(SavedBrush::Local(orig)) => {
                    let _ = prop.put(iface, orig.as_ptr());
                }
                Some(SavedBrush::Unset) => {
                    let dp = prop.dependency_property();
                    if !clear_dp_value(insp_ptr, dp.as_ref()) {
                        let _ = prop.put(iface, null_mut());
                    }
                }
                None => {
                    let (hr_get, cur) = prop.get(iface);
                    if hr_get.is_ok() && is_transparent_solid_brush(cur.as_ptr()) {
                        let dp = prop.dependency_property();
                        if !clear_dp_value(insp_ptr, dp.as_ref()) {
                            let _ = prop.put(iface, null_mut());
                        }
                    }
                }
            }
        }
    }

    unsafe fn apply_shape_appearance(
        rt: &Arc<TapRuntime>,
        insp_ptr: *mut c_void,
        desired_hide: bool,
        orig_opacity: f64,
        orig_visibility: i32,
    ) {
        if insp_ptr.is_null() {
            return;
        }
        let ui_elem = qi_interface(insp_ptr, &IID_IUIELEMENT);
        if ui_elem.is_null() {
            return;
        }
        let ui = ui_elem.as_ptr().cast::<RawIUIElement>();
        let target_op = if desired_hide { 0.0 } else { orig_opacity };
        let mut current_val = -1.0f64;
        let hr_get = ((*(*ui).lp_vtbl).get_opacity)(ui_elem.as_ptr(), &mut current_val);
        if hr_get.is_err() || (current_val - target_op).abs() > 0.001 {
            let hr_op = ((*(*ui).lp_vtbl).put_opacity)(ui_elem.as_ptr(), target_op);
            rt.last_put_opacity_hr.store(hr_op.0, Ordering::Release);
        }

        let target_vis = if desired_hide {
            VISIBILITY_COLLAPSED
        } else {
            orig_visibility
        };
        let mut current_vis = -1i32;
        let hr_gv = ((*(*ui).lp_vtbl).get_visibility)(ui_elem.as_ptr(), &mut current_vis);
        if hr_gv.is_err() || current_vis != target_vis {
            let hr_vis = ((*(*ui).lp_vtbl).put_visibility)(ui_elem.as_ptr(), target_vis);
            rt.last_put_visibility_hr.store(hr_vis.0, Ordering::Release);
        }
    }

    unsafe fn apply_target_appearance(
        rt: &Arc<TapRuntime>,
        handle: u64,
        insp_ptr: *mut c_void,
        kind: TargetKind,
        desired_hide: bool,
        orig_opacity: f64,
        orig_visibility: i32,
    ) {
        if insp_ptr.is_null() {
            return;
        }
        match kind {
            TargetKind::ShapeElement => {
                apply_shape_appearance(rt, insp_ptr, desired_hide, orig_opacity, orig_visibility);
            }
            TargetKind::OverflowBorder => {
                let border_com = qi_interface(insp_ptr, &IID_IBORDER);
                if border_com.is_null() {
                    return;
                }
                let saved = if desired_hide {
                    SavedTargetBrushes::default()
                } else {
                    take_saved_target_brushes(handle)
                };
                sync_overflow_brush_prop(
                    rt,
                    handle,
                    insp_ptr,
                    border_com.as_ptr(),
                    BrushPropKind::BorderBackground,
                    desired_hide,
                    saved.background,
                );
                sync_overflow_brush_prop(
                    rt,
                    handle,
                    insp_ptr,
                    border_com.as_ptr(),
                    BrushPropKind::BorderBrush,
                    desired_hide,
                    saved.border_brush,
                );
                if desired_hide {
                    ensure_prop_watch_registered(
                        rt,
                        handle,
                        TargetKind::OverflowBorder,
                        BrushPropKind::BorderBackground,
                        insp_ptr,
                    );
                    ensure_prop_watch_registered(
                        rt,
                        handle,
                        TargetKind::OverflowBorder,
                        BrushPropKind::BorderBrush,
                        insp_ptr,
                    );
                }
            }
            TargetKind::OverflowPanel => {
                let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
                if panel_com.is_null() {
                    return;
                }
                let saved = if desired_hide {
                    SavedTargetBrushes::default()
                } else {
                    take_saved_target_brushes(handle)
                };
                sync_overflow_brush_prop(
                    rt,
                    handle,
                    insp_ptr,
                    panel_com.as_ptr(),
                    BrushPropKind::PanelBackground,
                    desired_hide,
                    saved.background,
                );
                if desired_hide {
                    ensure_prop_watch_registered(
                        rt,
                        handle,
                        TargetKind::OverflowPanel,
                        BrushPropKind::PanelBackground,
                        insp_ptr,
                    );
                }
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
                revoke_git_cookie(git, cookie);
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
                advise_hr: AtomicI32::new(-1),
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
            if let Ok(mut g) = self.state.lock() {
                if !g.shutdown {
                    g.desired_hide = false;
                }
            }
            self.dispatch_apply_to_ui_threads(250);
        }

        fn set_hide(self: &Arc<Self>, hide: bool, host_pid: u32) {
            if hide && host_pid != 0 {
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
                self.enqueue_target_dispatch(target, git.as_ref(), barrier.as_ref());
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

        if items.is_empty() {
            return;
        }
        rt.diag_flags.fetch_or(DIAG_APPLY_RAN, Ordering::Relaxed);
        if desired_hide {
            unsafe {
                prewarm_overflow_brush_store_for_current_thread();
            }
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

                apply_target_appearance(
                    rt,
                    item.handle,
                    insp.as_ptr(),
                    item.kind,
                    desired_hide,
                    item.orig_opacity,
                    item.orig_visibility,
                );
            }
        }

        if !stale_handles.is_empty() {
            purge_saved_brushes_for_current_thread(&stale_handles);
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

        if rt.is_shutdown() {
            clear_all_watches_for_current_thread();
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
        if kind == TargetKind::ShapeElement {
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

        let want_hide_initial = kind == TargetKind::ShapeElement && rt.desired_hide();
        if want_hide_initial {
            apply_shape_appearance(rt, insp.as_ptr(), true, orig, orig_vis);
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

        let (thread_id, dispatcher, async_dispatch) =
            if let Ok(dq) = DispatcherQueue::GetForCurrentThread() {
                let tid = GetCurrentThreadId();
                let dq_for_enqueue = dq.clone();
                (
                    Some(tid),
                    OwnedDispatcher::Queue(dq.into_raw() as usize),
                    DispatchTarget::Queue {
                        thread_id: tid,
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

        if let Some(old) = evicted {
            release_target_resources(old, Some(&git));
        }

        if shutdown_after {
            if want_hide_initial {
                apply_shape_appearance(rt, insp.as_ptr(), false, orig, orig_vis);
            }
        } else if kind == TargetKind::ShapeElement {
            if want_hide_after != want_hide_initial {
                apply_shape_appearance(rt, insp.as_ptr(), want_hide_after, orig, orig_vis);
            }
        } else if want_hide_after {
            let core_git = matches!(async_dispatch, DispatchTarget::Core { .. }).then_some(&git);
            rt.enqueue_target_dispatch(async_dispatch, core_git, None);
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
            rt.diag_flags.fetch_or(DIAG_WATCHER_CALLED, Ordering::Relaxed);
            let cur_tid = GetCurrentThreadId();
            let type_name = bstr_ptr_to_string(element.type_name);
            let name = bstr_ptr_to_string(element.name);
            let is_container_root = type_name.contains("TaskbarFrame")
                || type_name.contains("TaskbarBackground")
                || name == "OverflowRootGrid"
                || type_name.contains("NotificationAreaOverflow")
                || type_name.contains("NotifyIconOverflow")
                || type_name.contains("OverflowView");
            if is_container_root {
                rt.diag_flags.fetch_or(DIAG_SAW_FRAME_OR_BG, Ordering::Relaxed);
            }
            if matches!(
                name.as_str(),
                "BackgroundFill" | "BackgroundStroke" | "ScreenEdgeStroke"
            ) {
                rt.diag_flags.fetch_or(DIAG_SAW_TARGET_NAME, Ordering::Relaxed);
            }
            let (matched_kind, extra_candidates) = if let Ok(mut idx) = rt.tree.lock() {
                idx.on_add_on_thread(relation.parent, handle, type_name, name, cur_tid);
                let matched_kind = idx.classify_target(handle);
                let extra = if is_container_root {
                    idx.collect_classified_targets_for_thread(cur_tid)
                } else {
                    Vec::new()
                };
                (matched_kind, extra)
            } else {
                (None, Vec::new())
            };
            if let Some(kind) = matched_kind {
                rt.diag_flags.fetch_or(DIAG_TARGET_MATCHED, Ordering::Relaxed);
                on_target_added(rt, handle, kind);
            }
            if !extra_candidates.is_empty() {
                let pending: Vec<(u64, TargetKind)> = if let Ok(g) = rt.state.lock() {
                    extra_candidates
                        .into_iter()
                        .filter(|&(h, _)| h != handle && !g.targets.contains_key(&h))
                        .collect()
                } else {
                    Vec::new()
                };
                for (extra_h, extra_kind) in pending {
                    rt.diag_flags.fetch_or(DIAG_TARGET_MATCHED, Ordering::Relaxed);
                    on_target_added(rt, extra_h, extra_kind);
                }
            }
        } else {
            let removed = if let Ok(mut idx) = rt.tree.lock() {
                idx.on_remove(handle)
            } else {
                Vec::new()
            };
            if !removed.is_empty() {
                purge_saved_brushes_for_current_thread(&removed);
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
                rt.advise_hr.store(hr_adv.0, Ordering::Release);

                let _ = WaitForSingleObject(rt.stop_event_handle(), INFINITE);

                if hr_adv.is_ok()
                    || (rt.diag_flags.load(Ordering::Acquire) & DIAG_WATCHER_CALLED) != 0
                {
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
                            if let Some((_, raw)) = *hh {
                                if raw != 0 {
                                    let h = HANDLE(raw as *mut c_void);
                                    host_h_copy = h;
                                    wait_handles[2] = h;
                                    count = 3;
                                }
                            }
                        }

                        loop {
                            if rt.stop_requested.load(Ordering::SeqCst) {
                                let _ = CancelIo(pipe);
                                break false;
                            }
                            let w = WaitForMultipleObjects(&wait_handles[..count], false, INFINITE);
                            if w == WAIT_OBJECT_0 {
                                let _ = &ov;
                                let mut tr = 0u32;
                                break GetOverlappedResult(pipe, &ov, &mut tr, false).is_ok();
                            } else if w == WAIT_EVENT_INDEX(1) || rt.stop_requested.load(Ordering::SeqCst) {
                                let _ = CancelIo(pipe);
                                break false;
                            } else if (count == 3 && w == WAIT_EVENT_INDEX(2))
                                || (!host_h_copy.is_invalid()
                                    && WaitForSingleObject(host_h_copy, 0) == WAIT_OBJECT_0)
                            {
                                rt.on_host_exited();
                                count = 2;
                                host_h_copy = HANDLE::default();
                                wait_handles[2] = HANDLE::default();
                            } else {
                                let _ = CancelIo(pipe);
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
                        CMD_VERSION => {
                            let _ = write_pipe_exact(
                                pipe,
                                io_event,
                                &[CMD_VERSION, TAP_PROTOCOL_VERSION],
                            );
                        }
                        CMD_SHUTDOWN => {
                            rt.trigger_shutdown(600);
                            let _ = write_pipe_exact(pipe, io_event, &[cmd]);
                            let _ = DisconnectNamedPipe(pipe);
                            break;
                        }
                        CMD_STATUS => {
                            let desired = u8::from(rt.desired_hide());
                            let advise_hr = rt.advise_hr.load(Ordering::Acquire).to_le_bytes();
                            let node_cnt = rt
                                .tree
                                .lock()
                                .map(|t| t.node_count().min(u32::MAX as usize) as u32)
                                .unwrap_or(0)
                                .to_le_bytes();
                            let target_cnt = rt
                                .state
                                .lock()
                                .map(|g| g.targets.len().min(u32::MAX as usize) as u32)
                                .unwrap_or(0)
                                .to_le_bytes();
                            let op_hr =
                                rt.last_put_opacity_hr.load(Ordering::Acquire).to_le_bytes();
                            let vis_hr =
                                rt.last_put_visibility_hr.load(Ordering::Acquire).to_le_bytes();
                            let flags = rt.diag_flags.load(Ordering::Acquire).to_le_bytes();
                            let mut resp = [0u8; 26];
                            resp[0] = CMD_STATUS;
                            resp[1] = desired;
                            resp[2..6].copy_from_slice(&advise_hr);
                            resp[6..10].copy_from_slice(&node_cnt);
                            resp[10..14].copy_from_slice(&target_cnt);
                            resp[14..18].copy_from_slice(&op_hr);
                            resp[18..22].copy_from_slice(&vis_hr);
                            resp[22..26].copy_from_slice(&flags);
                            let _ = write_pipe_exact(pipe, io_event, &resp);
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
