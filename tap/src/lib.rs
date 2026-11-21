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
    pub const CMD_ALTTAB_RECT: u8 = 0x06;
    pub const CMD_SET_ALTTAB_ITEM_STYLE: u8 = 0x07;
    pub const CMD_HOVER_FLYOUT_RECT: u8 = 0x08;
    pub const CMD_OSD_RECT: u8 = 0x09;
    pub const CMD_SET_OSD_SUPPRESS: u8 = 0x0A;
    pub const CMD_SET_OSD_FALLBACK_STYLE: u8 = 0x0B;
    pub const CMD_TOOLTIP_RECT: u8 = 0x0C;
    pub const CMD_SET_TOOLTIP_SUPPRESS: u8 = 0x0D;
    pub const CMD_SET_TOOLTIP_FALLBACK_STYLE: u8 = 0x0E;
    pub const CMD_MENU_FLYOUT_RECTS: u8 = 0x0F;
    pub const CMD_SET_MENU_FLYOUT_SUPPRESS: u8 = 0x10;
    pub const CMD_SET_MENU_FLYOUT_FALLBACK_STYLE: u8 = 0x11;
    pub const CMD_SHELL_CARD_RECTS: u8 = 0x12;
    pub const MAX_MENU_FLYOUT_RECTS: usize = 3;
    pub const MAX_SHELL_CARD_RECTS: usize = 8;
    pub const TAP_PROTOCOL_VERSION: u8 = 11;
    pub const ALTTAB_ITEM_OVERLAY_NAME: &str = "PrettyWindowsAltTabItemBg";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TapHostScope {
        Explorer,
        ShellExperience,
        ShellHost,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum TargetKind {
        ShapeElement,
        OverflowBorder,
        OverflowPanel,
        AltTabBorder,
        AltTabItemBorder,
        AltTabItemFocus,
        HoverFlyoutBorder,
        HoverFlyoutShape,
        OsdPanel,
        ToolTipPresenter,
        MenuFlyoutSurface,
        ShellCardSurface,
        ShellCardSubSurface,
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
            let mut seen = [0u64; 48];
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

        fn has_ancestor_matching(
            &self,
            handle: u64,
            forbid: fn(&NodeMeta) -> bool,
            container: fn(&NodeMeta) -> bool,
            require_container: bool,
        ) -> bool {
            let Some(node) = self.nodes.get(&handle) else {
                return false;
            };
            if forbid(node) {
                return false;
            }
            let mut seen = [0u64; 48];
            let mut seen_len = 0usize;
            let mut cur = node.parent;
            let mut has_container = !require_container;
            while cur != 0 && seen_len < seen.len() {
                if cur == handle || seen[..seen_len].contains(&cur) {
                    return false;
                }
                seen[seen_len] = cur;
                seen_len += 1;
                let Some(meta) = self.nodes.get(&cur) else {
                    break;
                };
                if forbid(meta) {
                    return false;
                }
                if require_container && container(meta) {
                    has_container = true;
                }
                cur = meta.parent;
            }
            has_container
        }

        pub fn has_forbidden_overflow_ancestor(&self, handle: u64) -> bool {
            !self.has_ancestor_matching(
                handle,
                Self::is_forbidden_overflow_node,
                Self::is_overflow_container_node,
                false,
            )
        }

        pub fn has_valid_overflow_ancestor(&self, handle: u64) -> bool {
            self.has_ancestor_matching(
                handle,
                Self::is_forbidden_overflow_node,
                Self::is_overflow_container_node,
                true,
            )
        }

        fn is_forbidden_alttab_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            t.contains("TaskView")
                || n.contains("TaskView")
                || t.contains("SnapAssist")
                || n.contains("SnapAssist")
                || t.contains("GamingTaskView")
                || n.contains("GamingTaskView")
                || t.contains("VirtualDesktop")
                || n.contains("VirtualDesktop")
                || t.contains("SwitchItemList")
                || n.contains("SwitchItemList")
                || t.contains("SwitchItemControl")
                || n.contains("SwitchItemControl")
                || t.contains("SwitchItemElement")
                || n.contains("SwitchItemElement")
                || t.contains("SwitchItemListViewItem")
                || n.contains("SwitchItemListViewItem")
                || t.contains("SwitchItemThumbnail")
                || n.contains("SwitchItemThumbnail")
                || t.contains("ScrollViewer")
                || n.contains("ScrollViewer")
                || t.contains("Button")
                || n.contains("Button")
                || t.contains("Tooltip")
                || n.contains("Tooltip")
                || t.contains("MenuFlyout")
                || n.contains("MenuFlyout")
                || t.contains("StartMenu")
                || n.contains("StartMenu")
        }

        fn is_alttab_container_node(meta: &NodeMeta) -> bool {
            meta.name == "ModalRootGrid"
                || meta.name == "AltTabRoot"
                || meta.type_name.contains("Switcher.AltTab")
        }

        pub fn has_forbidden_alttab_ancestor(&self, handle: u64) -> bool {
            !self.has_ancestor_matching(
                handle,
                Self::is_forbidden_alttab_node,
                Self::is_alttab_container_node,
                false,
            )
        }

        pub fn has_valid_alttab_ancestor(&self, handle: u64) -> bool {
            self.has_ancestor_matching(
                handle,
                Self::is_forbidden_alttab_node,
                Self::is_alttab_container_node,
                true,
            )
        }

        fn is_forbidden_alttab_item_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            t.contains("TaskView")
                || n.contains("TaskView")
                || t.contains("SnapAssist")
                || n.contains("SnapAssist")
                || t.contains("GamingTaskView")
                || n.contains("GamingTaskView")
                || t.contains("VirtualDesktop")
                || n.contains("VirtualDesktop")
                || t.contains("TaskbarSwitcher")
                || n.contains("TaskbarSwitcher")
                || t.contains("SwitchItemElement")
                || n.contains("SwitchItemElement")
                || t.contains("SwitchItemControl")
                || n.contains("SwitchItemControl")
                || t.contains("SwitchItemThumbnail")
                || n.contains("SwitchItemThumbnail")
                || t.contains("Button")
                || n.contains("Button")
                || t.contains("Tooltip")
                || n.contains("Tooltip")
                || t.contains("MenuFlyout")
                || n.contains("MenuFlyout")
                || t.contains("StartMenu")
                || n.contains("StartMenu")
        }

        pub fn has_valid_alttab_item_ancestor(&self, handle: u64) -> bool {
            self.has_ancestor_matching(
                handle,
                Self::is_forbidden_alttab_item_node,
                Self::is_alttab_container_node,
                true,
            )
        }

        fn is_forbidden_hover_flyout_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            t.contains("HoverFlyoutContent")
                || n.contains("HoverFlyoutContent")
                || t.contains("TaskItemThumbnail")
                || n.contains("TaskItemThumbnail")
                || t.contains("OverflowFlyoutList")
                || n.contains("OverflowFlyoutList")
                || t.contains("AppTaskHoverCard")
                || n.contains("AppTaskHoverCard")
                || t.contains("RecoHoverCard")
                || n.contains("RecoHoverCard")
                || t.contains("ItemsRepeater")
                || n.contains("ItemsRepeater")
                || t.contains("Button")
                || n.contains("Button")
                || t.contains("Tooltip")
                || n.contains("Tooltip")
                || t.contains("ToolTip")
                || n.contains("ToolTip")
                || t.contains("MenuFlyout")
                || n.contains("MenuFlyout")
                || t.contains("StartMenu")
                || n.contains("StartMenu")
                || t.contains("ScrollViewer")
                || n.contains("ScrollViewer")
                || t.contains("ScrollBar")
                || n.contains("ScrollBar")
        }

        fn is_hover_flyout_container_node(meta: &NodeMeta) -> bool {
            meta.type_name.contains("FlyoutFrame")
                || matches!(
                    meta.name.as_str(),
                    "HoverFlyoutCanvas"
                        | "HoverFlyoutGrid"
                        | "HoverFlyoutBackground"
                        | "HoverFlyoutBackgroundControl"
                )
        }

        pub fn has_forbidden_hover_flyout_ancestor(&self, handle: u64) -> bool {
            !self.has_ancestor_matching(
                handle,
                Self::is_forbidden_hover_flyout_node,
                Self::is_hover_flyout_container_node,
                false,
            )
        }

        pub fn has_valid_hover_flyout_ancestor(&self, handle: u64) -> bool {
            self.has_ancestor_matching(
                handle,
                Self::is_forbidden_hover_flyout_node,
                Self::is_hover_flyout_container_node,
                true,
            )
        }

        fn is_forbidden_osd_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            t.contains("Button")
                || n.contains("Button")
                || t.contains("Slider")
                || n.contains("Slider")
                || t.contains("Thumb")
                || n.contains("Thumb")
                || t.contains("TextBlock")
                || n.contains("TextBlock")
                || t.contains("AnimatedIcon")
                || n.contains("AnimatedIcon")
                || t.contains("Tooltip")
                || n.contains("Tooltip")
                || t.contains("ToolTip")
                || n.contains("ToolTip")
                || t.contains("MenuFlyout")
                || n.contains("MenuFlyout")
                || t.contains("StartMenu")
                || n.contains("StartMenu")
                || t.contains("TaskView")
                || n.contains("TaskView")
                || t.contains("ScrollViewer")
                || n.contains("ScrollViewer")
                || t.contains("ScrollBar")
                || n.contains("ScrollBar")
        }

        fn is_osd_container_node(meta: &NodeMeta) -> bool {
            meta.type_name.contains("HWConfirmatorUI")
                || meta.type_name.contains("ConfirmatorHostControl")
                || meta.name == "ConfirmatorFrameCanvas"
        }

        pub fn has_forbidden_osd_ancestor(&self, handle: u64) -> bool {
            !self.has_ancestor_matching(
                handle,
                Self::is_forbidden_osd_node,
                Self::is_osd_container_node,
                false,
            )
        }

        fn is_forbidden_tooltip_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            t.contains("StartMenu")
                || n.contains("StartMenu")
                || t.contains("MenuFlyout")
                || n.contains("MenuFlyout")
                || t.contains("ComboBox")
                || n.contains("ComboBox")
                || t.contains("TeachingTip")
                || n.contains("TeachingTip")
                || t.contains("FlyoutPresenter")
                || n.contains("FlyoutPresenter")
        }

        pub fn has_forbidden_tooltip_ancestor(&self, handle: u64) -> bool {
            !self.has_ancestor_matching(
                handle,
                Self::is_forbidden_tooltip_node,
                |_| false,
                false,
            )
        }

        fn is_forbidden_menu_flyout_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            t.contains("StartMenu")
                || n.contains("StartMenu")
                || t.contains("ToolTip")
                || n.contains("ToolTip")
                || t.contains("Tooltip")
                || n.contains("Tooltip")
                || t.contains("ComboBox")
                || n.contains("ComboBox")
                || t.contains("TeachingTip")
                || n.contains("TeachingTip")
                || n == "OverflowRootGrid"
                || t.contains("HoverFlyout")
                || n.contains("HoverFlyout")
                || t.contains("AltTab")
                || n.contains("AltTab")
                || t.contains("Confirmator")
                || n.contains("Confirmator")
                || t.contains("MenuFlyoutItem")
                || t.contains("MenuFlyoutSubItem")
                || t.contains("ListViewItem")
                || t.contains("JumpListItem")
                || t.contains("Button")
        }

        pub fn has_forbidden_menu_flyout_ancestor(&self, handle: u64) -> bool {
            !self.has_ancestor_matching(
                handle,
                Self::is_forbidden_menu_flyout_node,
                |_| false,
                false,
            )
        }

        fn is_forbidden_jumplist_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            t.contains("JumpListItem")
                || t.contains("SystemItemControl")
                || t.contains("SystemItemListViewItem")
                || t.contains("JumpListListViewItem")
                || t.contains("JumpListCategoryHeader")
                || t.contains("ListViewItem")
                || t.contains("MenuFlyoutItem")
                || t.contains("MenuFlyoutSubItem")
                || t.contains("Button")
                || t.contains("ToolTip")
                || n.contains("ToolTip")
                || t.contains("Tooltip")
                || n.contains("Tooltip")
                || t.contains("Toast")
                || n.contains("Toast")
                || t.contains("Notification")
                || n.contains("Notification")
                || t.contains("ActionCenter")
                || n.contains("ActionCenter")
                || t.contains("ControlCenter")
                || n.contains("ControlCenter")
                || t.contains("MeetNow")
                || n.contains("MeetNow")
        }

        fn is_valid_jumplist_container_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            t.contains("TaskbarJumpListFrame")
                || t.contains("JumpListControl")
                || t.contains("ControlHostMenuFlyoutPresenter")
        }

        pub fn has_valid_jumplist_ancestor(&self, handle: u64) -> bool {
            self.has_ancestor_matching(
                handle,
                Self::is_forbidden_jumplist_node,
                Self::is_valid_jumplist_container_node,
                true,
            )
        }

        fn has_restyled_acrylic_on_thread(&self, thread_id: u32) -> bool {
            self.nodes.values().any(|meta| {
                meta.thread_id == thread_id && meta.name == "JumpListRestyledAcrylic"
            })
        }

        pub fn thread_has_alttab_container(&self, tid: u32) -> bool {
            self.nodes.iter().any(|(&h, meta)| {
                meta.thread_id == tid
                    && (meta.name == "AltTabRoot"
                        || meta.type_name.contains("Switcher.AltTab")
                        || (meta.name == "ModalRootGrid"
                            && self.has_ancestor_matching(
                                h,
                                Self::is_forbidden_alttab_item_node,
                                Self::is_alttab_container_node,
                                false,
                            )))
            })
        }

        pub fn classify_target(&self, handle: u64) -> Option<TargetKind> {
            self.classify_target_in_scope(handle, TapHostScope::Explorer)
        }

        pub fn classify_target_in_scope(
            &self,
            handle: u64,
            scope: TapHostScope,
        ) -> Option<TargetKind> {
            match scope {
                TapHostScope::Explorer => self.classify_explorer_target(handle),
                TapHostScope::ShellExperience => self.classify_shell_experience_target(handle),
                TapHostScope::ShellHost => self.classify_shell_host_target(handle),
            }
        }

        fn classify_explorer_target(&self, handle: u64) -> Option<TargetKind> {
            let node = self.nodes.get(&handle)?;
            if node.name == ALTTAB_ITEM_OVERLAY_NAME {
                return None;
            }
            if node.name == "ConfirmatorMainGrid" && !self.has_forbidden_osd_ancestor(handle) {
                return Some(TargetKind::OsdPanel);
            }
            if node.name == "LayoutRoot"
                && (node.type_name.ends_with(".ContentPresenter")
                    || node.type_name == "ContentPresenter")
                && self
                    .nodes
                    .get(&node.parent)
                    .is_some_and(|p| p.type_name.ends_with(".ToolTip") || p.type_name == "ToolTip")
                && !self.has_forbidden_tooltip_ancestor(handle)
            {
                return Some(TargetKind::ToolTipPresenter);
            }
            if matches!(
                node.name.as_str(),
                "BackgroundFill" | "BackgroundStroke" | "ScreenEdgeStroke"
            ) {
                if self.has_valid_taskbar_ancestor(handle) || self.has_valid_overflow_ancestor(handle)
                {
                    return Some(TargetKind::ShapeElement);
                }
                if self.has_valid_hover_flyout_ancestor(handle) {
                    return Some(TargetKind::HoverFlyoutShape);
                }
                return None;
            }
            if node.name == "HoverFlyoutBackground"
                && (node.type_name.ends_with(".Border") || node.type_name == "Border")
                && !self.has_forbidden_hover_flyout_ancestor(handle)
            {
                return Some(TargetKind::HoverFlyoutBorder);
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
            if !self.has_forbidden_alttab_ancestor(handle) && self.has_valid_alttab_ancestor(handle)
            {
                if node.name == "BackgroundElement" {
                    return Some(TargetKind::AltTabBorder);
                }
                if (node.type_name.ends_with(".Border") || node.type_name == "Border")
                    && self.nodes.get(&node.parent).is_some_and(|p| {
                        matches!(p.name.as_str(), "ModalRootGrid" | "AltTabRootGrid")
                    })
                {
                    return Some(TargetKind::AltTabBorder);
                }
            }
            if (node.type_name.ends_with(".Border") || node.type_name == "Border")
                && node.name == "BackgroundBorder"
                && self.has_valid_alttab_item_ancestor(handle)
            {
                return Some(TargetKind::AltTabItemBorder);
            }
            if node.type_name.contains("ListViewItem") && self.has_valid_alttab_item_ancestor(handle)
            {
                return Some(TargetKind::AltTabItemFocus);
            }
            if !self.has_forbidden_menu_flyout_ancestor(handle) {
                let is_border =
                    node.type_name.ends_with(".Border") || node.type_name == "Border";
                let is_grid = node.type_name.ends_with(".Grid") || node.type_name == "Grid";
                let parent_opt = self.nodes.get(&node.parent);
                if is_border
                    && parent_opt.is_some_and(|p| p.type_name.contains("MenuFlyoutPresenter"))
                {
                    return Some(TargetKind::MenuFlyoutSurface);
                }
                if (is_border || is_grid)
                    && matches!(
                        node.name.as_str(),
                        "MenuFlyoutPresenterBorder"
                            | "JumpListBorder"
                            | "JumpListBackground"
                            | "BackgroundBorder"
                            | "RootGrid"
                            | "LayoutRoot"
                    )
                    && parent_opt.is_some_and(|p| {
                        (p.type_name.contains("JumpList") || p.type_name.contains("JumpView"))
                            && !p.type_name.contains("Item")
                    })
                {
                    return Some(TargetKind::MenuFlyoutSurface);
                }
            }
            None
        }

        fn classify_jumplist_target(&self, handle: u64) -> Option<TargetKind> {
            let node = self.nodes.get(&handle)?;
            if !self.has_valid_jumplist_ancestor(handle) {
                return None;
            }
            let is_border = node.type_name.ends_with(".Border") || node.type_name == "Border";
            let is_grid = node.type_name.ends_with(".Grid") || node.type_name == "Grid";
            let parent_opt = self.nodes.get(&node.parent);
            if (is_border || is_grid) && node.name == "JumpListRestyledAcrylic" {
                return Some(TargetKind::MenuFlyoutSurface);
            }
            if is_border
                && parent_opt.is_some_and(|p| p.type_name.contains("MenuFlyoutPresenter"))
            {
                return Some(TargetKind::MenuFlyoutSurface);
            }
            if !self.has_restyled_acrylic_on_thread(node.thread_id)
                && (is_border || is_grid)
                && matches!(
                    node.name.as_str(),
                    "JumpListBorder"
                        | "JumpListBackground"
                        | "JumpListGrid"
                        | "BackgroundBorder"
                        | "RootBorder"
                        | "RootGrid"
                        | "LayoutRoot"
                )
                && parent_opt.is_some_and(|p| {
                    p.type_name.contains("TaskbarJumpListFrame")
                        || p.type_name.contains("JumpListControl")
                })
            {
                return Some(TargetKind::MenuFlyoutSurface);
            }
            None
        }

        fn is_forbidden_shell_card_item_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            t.contains("ListViewItem")
                || t.contains("GridViewItem")
                || t.contains("Button")
                || t.contains("ToggleButton")
                || t.contains("RepeatButton")
                || t.contains("Slider")
                || t.contains("CheckBox")
                || t.contains("ComboBox")
                || t.contains("MenuFlyoutItem")
                || t.contains("MenuFlyoutSubItem")
                || t.contains("ScrollBar")
                || t.contains("CalendarViewDayItem")
                || t.contains("ToolTip")
                || n.contains("ToolTip")
                || t.contains("Tooltip")
                || n.contains("Tooltip")
                || t.contains("MeetNow")
                || n.contains("MeetNow")
        }

        fn has_forbidden_shell_card_item_ancestor(&self, handle: u64) -> bool {
            !self.has_ancestor_matching(
                handle,
                Self::is_forbidden_shell_card_item_node,
                |_| false,
                false,
            )
        }

        fn is_notification_center_container_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            matches!(
                n,
                "NotificationCenterGrid"
                    | "NotificationCenterView"
                    | "NotificationCenterPage"
                    | "NotificationCenterRoot"
            ) || t.contains("NotificationCenter")
        }

        fn has_notification_center_ancestor(&self, handle: u64) -> bool {
            let mut cur = self.nodes.get(&handle).map(|n| n.parent).unwrap_or(0);
            let mut steps = 0usize;
            while cur != 0 && steps < 64 {
                let Some(meta) = self.nodes.get(&cur) else {
                    break;
                };
                if Self::is_notification_center_container_node(meta) {
                    return true;
                }
                cur = meta.parent;
                steps += 1;
            }
            false
        }

        fn has_primary_shell_card_ancestor(&self, handle: u64) -> bool {
            let mut cur = self.nodes.get(&handle).map(|n| n.parent).unwrap_or(0);
            let mut steps = 0usize;
            while cur != 0 && steps < 64 {
                let Some(meta) = self.nodes.get(&cur) else {
                    break;
                };
                if matches!(
                    meta.name.as_str(),
                    "NotificationCenterGrid"
                        | "CalendarCenterGrid"
                        | "ControlCenterRegion"
                        | "RootGridBorder"
                        | "L1Grid"
                        | "L2Frame"
                ) {
                    return true;
                }
                cur = meta.parent;
                steps += 1;
            }
            false
        }

        fn is_valid_floating_toast_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            t.contains("FlexibleNormalToastView")
                || t.contains("FlexiblePriorityToastView")
                || t.contains("FlexibleToastView")
                || t.contains("ToastNotificationView")
        }

        fn is_valid_control_center_node(meta: &NodeMeta) -> bool {
            let t = meta.type_name.as_str();
            let n = meta.name.as_str();
            t.contains("ControlCenter")
                || t.contains("QuickActions")
                || matches!(
                    n,
                    "ControlCenterRegion" | "RootGridBorder" | "L1Grid" | "L2Frame"
                )
        }

        fn has_control_center_root_surface_on_thread(&self, thread_id: u32) -> bool {
            self.nodes.iter().any(|(&h, meta)| {
                meta.thread_id == thread_id
                    && matches!(meta.name.as_str(), "RootGridBorder" | "ControlCenterRegion")
                    && (meta.type_name.ends_with(".Border")
                        || meta.type_name == "Border"
                        || meta.type_name.ends_with(".Grid")
                        || meta.type_name == "Grid")
                    && !self.has_forbidden_shell_card_item_ancestor(h)
            })
        }

        fn has_l1_or_l2_surface_on_thread(&self, thread_id: u32) -> bool {
            self.nodes.iter().any(|(&h, meta)| {
                meta.thread_id == thread_id
                    && matches!(meta.name.as_str(), "L1Grid" | "L2Frame")
                    && (meta.type_name.ends_with(".Border")
                        || meta.type_name == "Border"
                        || meta.type_name.ends_with(".Grid")
                        || meta.type_name == "Grid")
                    && !self.has_forbidden_shell_card_item_ancestor(h)
            })
        }

        fn has_toast_background_border2_on_thread(&self, thread_id: u32) -> bool {
            self.nodes.iter().any(|(&h, meta)| {
                meta.thread_id == thread_id
                    && meta.name == "ToastBackgroundBorder2"
                    && (meta.type_name.ends_with(".Border") || meta.type_name == "Border")
                    && !self.has_forbidden_shell_card_item_ancestor(h)
                    && !self.has_notification_center_ancestor(h)
            })
        }

        fn classify_control_center_card_node(&self, handle: u64, node: &NodeMeta) -> Option<TargetKind> {
            let is_border = node.type_name.ends_with(".Border") || node.type_name == "Border";
            let is_grid = node.type_name.ends_with(".Grid") || node.type_name == "Grid";
            if !is_border && !is_grid {
                return None;
            }
            if matches!(
                node.name.as_str(),
                "BackgroundBlur" | "FooterGrid" | "Footer" | "PaneFooter"
            ) {
                return Some(TargetKind::ShellCardSubSurface);
            }
            if matches!(node.name.as_str(), "RootGridBorder" | "ControlCenterRegion") {
                return Some(TargetKind::ShellCardSurface);
            }
            if matches!(
                node.name.as_str(),
                "MediaTransportControlsRegion" | "MediaTransportControlsGrid"
            ) {
                if self.has_primary_shell_card_ancestor(handle) {
                    return Some(TargetKind::ShellCardSubSurface);
                }
                return Some(TargetKind::ShellCardSurface);
            }
            if matches!(node.name.as_str(), "L1Grid" | "L2Frame") {
                if self.has_control_center_root_surface_on_thread(node.thread_id) {
                    return Some(TargetKind::ShellCardSubSurface);
                }
                return Some(TargetKind::ShellCardSurface);
            }
            if node.name == "MainGrid"
                && self.has_ancestor_matching(
                    handle,
                    Self::is_forbidden_shell_card_item_node,
                    Self::is_valid_control_center_node,
                    true,
                )
            {
                if self.has_control_center_root_surface_on_thread(node.thread_id)
                    || self.has_l1_or_l2_surface_on_thread(node.thread_id)
                {
                    return Some(TargetKind::ShellCardSubSurface);
                }
                return Some(TargetKind::ShellCardSurface);
            }
            None
        }

        fn classify_shell_experience_target(&self, handle: u64) -> Option<TargetKind> {
            if self.has_valid_jumplist_ancestor(handle) {
                return self.classify_jumplist_target(handle);
            }
            let node = self.nodes.get(&handle)?;
            if self.has_forbidden_shell_card_item_ancestor(handle) {
                return None;
            }
            let is_border = node.type_name.ends_with(".Border") || node.type_name == "Border";
            let is_grid = node.type_name.ends_with(".Grid") || node.type_name == "Grid";
            if !is_border && !is_grid {
                return None;
            }

            // Notification Center & Calendar (Win+N)
            if is_grid && matches!(node.name.as_str(), "NotificationCenterGrid" | "CalendarCenterGrid") {
                return Some(TargetKind::ShellCardSurface);
            }
            if is_border && node.name == "CalendarHeaderMinimizedOverlay" {
                return Some(TargetKind::ShellCardSubSurface);
            }

            // Floating system Toasts (strictly isolated from Notification Center history)
            if is_border && matches!(node.name.as_str(), "ToastBackgroundBorder2" | "ToastBackgroundBorder") {
                if self.has_notification_center_ancestor(handle) {
                    return None;
                }
                if !self.has_ancestor_matching(
                    handle,
                    Self::is_forbidden_shell_card_item_node,
                    Self::is_valid_floating_toast_node,
                    true,
                ) {
                    return None;
                }
                if node.name == "ToastBackgroundBorder2" {
                    return Some(TargetKind::ShellCardSurface);
                }
                if self.has_toast_background_border2_on_thread(node.thread_id) {
                    return Some(TargetKind::ShellCardSubSurface);
                }
                return Some(TargetKind::ShellCardSurface);
            }

            // Legacy Win11 22H2/23H2 ControlCenter & ActionCenter Media/Footer surfaces in ShellExperienceHost
            self.classify_control_center_card_node(handle, node)
        }

        fn classify_shell_host_target(&self, handle: u64) -> Option<TargetKind> {
            let node = self.nodes.get(&handle)?;
            if self.has_forbidden_shell_card_item_ancestor(handle) {
                return None;
            }
            let is_border = node.type_name.ends_with(".Border") || node.type_name == "Border";
            if is_border
                && !self.has_forbidden_menu_flyout_ancestor(handle)
                && self
                    .nodes
                    .get(&node.parent)
                    .is_some_and(|p| p.type_name.contains("MenuFlyoutPresenter"))
            {
                return Some(TargetKind::MenuFlyoutSurface);
            }
            self.classify_control_center_card_node(handle, node)
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
            self.collect_classified_targets_for_thread_in_scope(thread_id, TapHostScope::Explorer)
        }

        pub fn collect_classified_targets_for_thread_in_scope(
            &self,
            thread_id: u32,
            scope: TapHostScope,
        ) -> Vec<(u64, TargetKind)> {
            self.nodes
                .iter()
                .filter(|(_, meta)| meta.thread_id == thread_id)
                .filter_map(|(&h, _)| self.classify_target_in_scope(h, scope).map(|k| (h, k)))
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

    pub fn appcontainer_pipe_name_for_pid(pid: u32) -> String {
        format!(r"\\.\pipe\PrettyWindows_ShellExpTap_{pid}")
    }

    pub fn shell_host_pipe_name_for_pid(pid: u32) -> String {
        format!(r"\\.\pipe\PrettyWindows_ShellHostTap_{pid}")
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

    #[test]
    fn classifies_alttab_background_element_and_excludes_items_and_taskview() {
        let mut idx = VisualTreeIndex::new();
        // 1. Alt+Tab visual tree: AltTab > Grid#ModalRootGrid > Border#BackgroundElement
        idx.on_add(
            0,
            400,
            "WindowsInternal.ComposableShell.Experiences.Switcher.AltTab".into(),
            "AltTabRoot".into(),
        );
        idx.on_add(
            400,
            401,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ModalRootGrid".into(),
        );
        idx.on_add(
            401,
            402,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundElement".into(),
        );
        // Item cards inside SwitchItemList / SwitchItemControl must NEVER be touched
        idx.on_add(
            402,
            410,
            "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemList".into(),
            "SwitchItemListControl".into(),
        );
        idx.on_add(
            410,
            411,
            "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemControl".into(),
            "".into(),
        );
        idx.on_add(
            411,
            412,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
        );

        // 2. Out-of-order add: BackgroundElement arrives before AltTabRoot attaches above
        idx.on_add(
            420,
            421,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ModalRootGrid".into(),
        );
        idx.on_add(
            421,
            422,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundElement".into(),
        );
        assert_eq!(idx.classify_target(422), Some(TargetKind::AltTabBorder));

        // 3. TaskView and SnapAssist with identically named borders must NOT match
        idx.on_add(
            0,
            450,
            "WindowsInternal.ComposableShell.Experiences.Switcher.TaskView".into(),
            "TaskViewRoot".into(),
        );
        idx.on_add(
            450,
            451,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ModalRootGrid".into(),
        );
        idx.on_add(
            451,
            452,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundElement".into(),
        );
        idx.on_add(
            0,
            460,
            "WindowsInternal.ComposableShell.Experiences.Switcher.SnapAssist".into(),
            "SnapAssistRoot".into(),
        );
        idx.on_add(
            460,
            461,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ModalRootGrid".into(),
        );
        idx.on_add(
            461,
            462,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundElement".into(),
        );

        assert_eq!(idx.classify_target(402), Some(TargetKind::AltTabBorder));
        assert_eq!(idx.classify_target(412), None);
        assert_eq!(idx.classify_target(452), None);
        assert_eq!(idx.classify_target(462), None);
    }

    #[test]
    fn classifies_alttab_item_background_border_while_excluding_focus_and_taskview() {
        let mut idx = VisualTreeIndex::new();
        // Alt+Tab item container visual tree:
        // AltTabRoot -> ModalRootGrid -> BackgroundElement -> SwitchItemList -> ScrollViewer
        // -> SwitchItemListViewItem -> Grid#Root -> Border#BackgroundBorder / Border#FocusRect
        idx.on_add_on_thread(
            0,
            500,
            "WindowsInternal.ComposableShell.Experiences.Switcher.AltTab".into(),
            "AltTabRoot".into(),
            77,
        );
        idx.on_add_on_thread(
            500,
            501,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ModalRootGrid".into(),
            77,
        );
        idx.on_add_on_thread(
            501,
            502,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundElement".into(),
            77,
        );
        idx.on_add_on_thread(
            502,
            503,
            "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemList".into(),
            "SwitchItemListControl".into(),
            77,
        );
        idx.on_add_on_thread(
            503,
            504,
            "Windows.UI.Xaml.Controls.ScrollViewer".into(),
            "".into(),
            77,
        );
        idx.on_add_on_thread(
            504,
            505,
            "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemListViewItem".into(),
            "".into(),
            77,
        );
        idx.on_add_on_thread(505, 506, "Windows.UI.Xaml.Controls.Grid".into(), "Root".into(), 77);
        idx.on_add_on_thread(
            506,
            507,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
            77,
        );
        idx.on_add_on_thread(
            506,
            508,
            "Windows.UI.Xaml.Controls.Border".into(),
            "FocusRect".into(),
            77,
        );

        // TaskView / SnapAssist / TaskbarSwitcher SwitchItemElement must NOT match AltTabItemBorder.
        idx.on_add_on_thread(
            0,
            600,
            "WindowsInternal.ComposableShell.Experiences.Switcher.TaskView".into(),
            "TaskViewRoot".into(),
            88,
        );
        idx.on_add_on_thread(
            600,
            601,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ModalRootGrid".into(),
            88,
        );
        idx.on_add_on_thread(
            601,
            602,
            "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemElement".into(),
            "".into(),
            88,
        );
        idx.on_add_on_thread(
            602,
            603,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
            88,
        );
        idx.on_add_on_thread(
            505,
            604,
            "WindowsInternal.ComposableShell.Experiences.Switcher.TaskbarSwitcher".into(),
            "TaskbarSwitcherRoot".into(),
            77,
        );
        idx.on_add_on_thread(
            604,
            605,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
            77,
        );
        idx.on_add_on_thread(
            601,
            606,
            "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemListViewItem".into(),
            "".into(),
            88,
        );

        // Out-of-order subtree addition: BackgroundBorder arrives before SwitchItemListViewItem attaches to thread container.
        idx.on_add_on_thread(
            700,
            701,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "Root".into(),
            77,
        );
        idx.on_add_on_thread(
            701,
            702,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
            77,
        );
        assert_eq!(idx.classify_target(702), None);
        idx.on_add_on_thread(
            504,
            700,
            "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemListViewItem".into(),
            "".into(),
            77,
        );

        assert!(idx.thread_has_alttab_container(77));
        assert!(!idx.thread_has_alttab_container(88));
        assert_eq!(idx.classify_target(502), Some(TargetKind::AltTabBorder));
        assert_eq!(idx.classify_target(505), Some(TargetKind::AltTabItemFocus));
        assert_eq!(idx.classify_target(507), Some(TargetKind::AltTabItemBorder));
        assert_eq!(idx.classify_target(508), None);
        assert_eq!(idx.classify_target(603), None);
        assert_eq!(idx.classify_target(605), None);
        assert_eq!(idx.classify_target(606), None);
        assert_eq!(idx.classify_target(700), Some(TargetKind::AltTabItemFocus));
        assert_eq!(idx.classify_target(702), Some(TargetKind::AltTabItemBorder));
    }

    #[test]
    fn classifies_hover_flyout_background_and_shapes_while_excluding_thumbnail_items() {
        let mut idx = VisualTreeIndex::new();
        // 1. Standard Win11 FlyoutFrame visual tree
        idx.on_add_on_thread(0, 800, "Taskbar.FlyoutFrame".into(), "FlyoutFrame".into(), 4060);
        idx.on_add_on_thread(
            800,
            801,
            "Windows.UI.Xaml.Controls.Canvas".into(),
            "HoverFlyoutCanvas".into(),
            4060,
        );
        idx.on_add_on_thread(
            801,
            802,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "HoverFlyoutGrid".into(),
            4060,
        );
        idx.on_add_on_thread(
            802,
            803,
            "Windows.UI.Xaml.Controls.Border".into(),
            "HoverFlyoutBackground".into(),
            4060,
        );
        idx.on_add_on_thread(
            803,
            804,
            "Taskbar.TaskbarBackground".into(),
            "HoverFlyoutBackgroundControl".into(),
            4060,
        );
        idx.on_add_on_thread(804, 805, "Windows.UI.Xaml.Controls.Grid".into(), "".into(), 4060);
        idx.on_add_on_thread(
            805,
            806,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
            4060,
        );
        idx.on_add_on_thread(
            805,
            807,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundStroke".into(),
            4060,
        );
        idx.on_add_on_thread(
            805,
            808,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "ScreenEdgeStroke".into(),
            4060,
        );

        // 2. Content inside HoverFlyoutContent (thumbnail cards & overflow list) must NEVER be touched
        idx.on_add_on_thread(
            803,
            810,
            "Windows.UI.Xaml.Controls.ContentPresenter".into(),
            "HoverFlyoutContent".into(),
            4060,
        );
        idx.on_add_on_thread(
            810,
            811,
            "Taskbar.TaskItemThumbnailList".into(),
            "TaskItemThumbnailList".into(),
            4060,
        );
        idx.on_add_on_thread(
            811,
            812,
            "Taskbar.TaskItemThumbnailView".into(),
            "".into(),
            4060,
        );
        idx.on_add_on_thread(
            812,
            813,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
            4060,
        );
        idx.on_add_on_thread(
            812,
            814,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
            4060,
        );
        idx.on_add_on_thread(
            810,
            815,
            "Taskbar.OverflowFlyoutList".into(),
            "".into(),
            4060,
        );
        idx.on_add_on_thread(
            815,
            816,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
            4060,
        );

        // 3. Out-of-order add: BackgroundFill under HoverFlyoutBackgroundControl arrives before FlyoutFrame
        idx.on_add_on_thread(
            820,
            821,
            "Taskbar.TaskbarBackground".into(),
            "HoverFlyoutBackgroundControl".into(),
            4060,
        );
        idx.on_add_on_thread(
            821,
            822,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
            4060,
        );
        assert_eq!(idx.classify_target(822), Some(TargetKind::HoverFlyoutShape));
        assert!(!idx.is_taskbar_bg_target(822));

        // 4. Generic MenuFlyout / ToolTip with identically named rectangles must NOT match
        idx.on_add_on_thread(
            0,
            830,
            "Windows.UI.Xaml.Controls.MenuFlyout".into(),
            "".into(),
            4060,
        );
        idx.on_add_on_thread(
            830,
            831,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
            4060,
        );
        idx.on_add_on_thread(
            0,
            832,
            "Windows.UI.Xaml.Controls.ToolTip".into(),
            "".into(),
            4060,
        );
        idx.on_add_on_thread(
            832,
            833,
            "Windows.UI.Xaml.Shapes.Rectangle".into(),
            "BackgroundFill".into(),
            4060,
        );

        assert_eq!(idx.classify_target(803), Some(TargetKind::HoverFlyoutBorder));
        assert_eq!(idx.classify_target(806), Some(TargetKind::HoverFlyoutShape));
        assert_eq!(idx.classify_target(807), Some(TargetKind::HoverFlyoutShape));
        assert_eq!(idx.classify_target(808), Some(TargetKind::HoverFlyoutShape));
        assert!(!idx.is_taskbar_bg_target(806));
        assert_eq!(idx.classify_target(813), None);
        assert_eq!(idx.classify_target(814), None);
        assert_eq!(idx.classify_target(816), None);
        assert_eq!(idx.classify_target(831), None);
        assert_eq!(idx.classify_target(833), None);
    }

    #[test]
    fn ignores_alttab_item_overlay_border_while_classifying_native_background_border() {
        let mut idx = VisualTreeIndex::new();
        idx.on_add_on_thread(
            0,
            900,
            "WindowsInternal.ComposableShell.Experiences.Switcher.AltTab".into(),
            "AltTabRoot".into(),
            1234,
        );
        idx.on_add_on_thread(
            900,
            901,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ModalRootGrid".into(),
            1234,
        );
        idx.on_add_on_thread(
            901,
            902,
            "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemList".into(),
            "SwitchItemList".into(),
            1234,
        );
        idx.on_add_on_thread(
            902,
            903,
            "Windows.UI.Xaml.Controls.ListViewItem".into(),
            "".into(),
            1234,
        );
        idx.on_add_on_thread(
            903,
            904,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "Root".into(),
            1234,
        );
        idx.on_add_on_thread(
            904,
            905,
            "Windows.UI.Xaml.Controls.Border".into(),
            ALTTAB_ITEM_OVERLAY_NAME.into(),
            1234,
        );
        idx.on_add_on_thread(
            904,
            906,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
            1234,
        );

        assert_eq!(idx.classify_target(905), None);
        assert_eq!(idx.classify_target(906), Some(TargetKind::AltTabItemBorder));
    }

    #[test]
    fn classifies_osd_main_grid_and_excludes_inner_controls() {
        let mut idx = VisualTreeIndex::new();
        // 1. Standard Win11 HWConfirmatorUI tree:
        // ConfirmatorHostControl > Canvas#ConfirmatorFrameCanvas > Grid > Grid#ConfirmatorMainGrid
        idx.on_add_on_thread(
            0,
            1000,
            "HWConfirmatorUI.ConfirmatorHostControl".into(),
            "".into(),
            5724,
        );
        idx.on_add_on_thread(
            1000,
            1001,
            "Windows.UI.Xaml.Controls.Canvas".into(),
            "ConfirmatorFrameCanvas".into(),
            5724,
        );
        idx.on_add_on_thread(
            1001,
            1002,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "".into(),
            5724,
        );
        idx.on_add_on_thread(
            1002,
            1003,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ConfirmatorMainGrid".into(),
            5724,
        );
        // Inner OSD controls must never be matched
        idx.on_add_on_thread(
            1003,
            1004,
            "HWConfirmatorUI.CustomThumbSlider".into(),
            "volumeSlider".into(),
            5724,
        );
        idx.on_add_on_thread(
            1003,
            1005,
            "Windows.UI.Xaml.Controls.Button".into(),
            "MuteButton".into(),
            5724,
        );
        idx.on_add_on_thread(
            1003,
            1006,
            "Windows.UI.Xaml.Controls.TextBlock".into(),
            "volumeLevelText".into(),
            5724,
        );
        idx.on_add_on_thread(
            1005,
            1007,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
            5724,
        );

        // 2. Out-of-order add: ConfirmatorMainGrid arrives before ConfirmatorFrameCanvas attaches
        idx.on_add_on_thread(
            1020,
            1021,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ConfirmatorMainGrid".into(),
            5724,
        );
        assert_eq!(idx.classify_target(1021), Some(TargetKind::OsdPanel));
        idx.on_add_on_thread(
            0,
            1020,
            "Windows.UI.Xaml.Controls.Canvas".into(),
            "ConfirmatorFrameCanvas".into(),
            5724,
        );

        // 3. Nested under forbidden node (e.g. StartMenu) or unrelated Grid must NOT match
        idx.on_add_on_thread(
            0,
            1030,
            "StartMenu.StartInnerFrame".into(),
            "StartMenu".into(),
            5724,
        );
        idx.on_add_on_thread(
            1030,
            1031,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ConfirmatorMainGrid".into(),
            5724,
        );

        assert_eq!(idx.classify_target(1002), None);
        assert_eq!(idx.classify_target(1003), Some(TargetKind::OsdPanel));
        assert_eq!(idx.classify_target(1004), None);
        assert_eq!(idx.classify_target(1005), None);
        assert_eq!(idx.classify_target(1006), None);
        assert_eq!(idx.classify_target(1007), None);
        assert_eq!(idx.classify_target(1021), Some(TargetKind::OsdPanel));
        assert_eq!(idx.classify_target(1031), None);
    }

    #[test]
    fn classifies_tooltip_layout_root_and_rejects_menuflyout_or_non_tooltip_presenters() {
        let mut idx = VisualTreeIndex::new();

        // 1. Standard Win11 XAML ToolTip > ContentPresenter#LayoutRoot > TextBlock
        idx.on_add_on_thread(
            0,
            2000,
            "Windows.UI.Xaml.Controls.Primitives.PopupRoot".into(),
            "PopupRoot".into(),
            25024,
        );
        idx.on_add_on_thread(
            2000,
            2001,
            "Windows.UI.Xaml.Controls.ToolTip".into(),
            "".into(),
            25024,
        );
        idx.on_add_on_thread(
            2001,
            2002,
            "Windows.UI.Xaml.Controls.ContentPresenter".into(),
            "LayoutRoot".into(),
            25024,
        );
        idx.on_add_on_thread(
            2002,
            2003,
            "Windows.UI.Xaml.Controls.TextBlock".into(),
            "".into(),
            25024,
        );

        // 2. Out-of-order add: LayoutRoot arrives before parent ToolTip
        idx.on_add_on_thread(
            2020,
            2021,
            "Windows.UI.Xaml.Controls.ContentPresenter".into(),
            "LayoutRoot".into(),
            25024,
        );
        assert_eq!(idx.classify_target(2021), None);
        idx.on_add_on_thread(
            0,
            2020,
            "Windows.UI.Xaml.Controls.ToolTip".into(),
            "TrayToolTip".into(),
            25024,
        );
        assert_eq!(idx.classify_target(2021), Some(TargetKind::ToolTipPresenter));

        // 3. ContentPresenter#LayoutRoot under MenuFlyoutPresenter, Button, or StartMenu must NEVER match
        idx.on_add_on_thread(
            0,
            2030,
            "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
            "".into(),
            25024,
        );
        idx.on_add_on_thread(
            2030,
            2031,
            "Windows.UI.Xaml.Controls.ContentPresenter".into(),
            "LayoutRoot".into(),
            25024,
        );
        idx.on_add_on_thread(
            0,
            2040,
            "Windows.UI.Xaml.Controls.Button".into(),
            "".into(),
            25024,
        );
        idx.on_add_on_thread(
            2040,
            2041,
            "Windows.UI.Xaml.Controls.ContentPresenter".into(),
            "LayoutRoot".into(),
            25024,
        );
        idx.on_add_on_thread(
            0,
            2050,
            "StartMenu.StartInnerFrame".into(),
            "StartMenu".into(),
            25024,
        );
        idx.on_add_on_thread(
            2050,
            2051,
            "Windows.UI.Xaml.Controls.ToolTip".into(),
            "".into(),
            25024,
        );
        idx.on_add_on_thread(
            2051,
            2052,
            "Windows.UI.Xaml.Controls.ContentPresenter".into(),
            "LayoutRoot".into(),
            25024,
        );

        assert_eq!(idx.classify_target(2001), None);
        assert_eq!(idx.classify_target(2002), Some(TargetKind::ToolTipPresenter));
        assert_eq!(idx.classify_target(2003), None);
        assert_eq!(idx.classify_target(2031), None);
        assert_eq!(idx.classify_target(2041), None);
        assert_eq!(idx.classify_target(2052), None);
    }

    #[test]
    fn classifies_menu_flyout_and_jumplist_surfaces_with_strict_isolation() {
        let mut idx = VisualTreeIndex::new();

        // 1. Standard cascading MenuFlyouts: main menu (3001) + submenu (3011)
        idx.on_add_on_thread(
            0,
            3000,
            "Windows.UI.Xaml.Controls.Primitives.PopupRoot".into(),
            "".into(),
            100,
        );
        idx.on_add_on_thread(
            3000,
            3001,
            "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
            "".into(),
            100,
        );
        idx.on_add_on_thread(
            3001,
            3002,
            "Windows.UI.Xaml.Controls.Border".into(),
            "MenuFlyoutPresenterBorder".into(),
            100,
        );
        idx.on_add_on_thread(
            3002,
            3003,
            "Windows.UI.Xaml.Controls.MenuFlyoutItem".into(),
            "".into(),
            100,
        );
        idx.on_add_on_thread(
            3003,
            3004,
            "Windows.UI.Xaml.Controls.Border".into(),
            "LayoutRoot".into(),
            100,
        );

        // Out-of-order submenu border arrives before presenter
        idx.on_add_on_thread(
            3011,
            3012,
            "Windows.UI.Xaml.Controls.Border".into(),
            "LayoutRoot".into(),
            100,
        );
        idx.on_add_on_thread(
            3000,
            3011,
            "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
            "".into(),
            100,
        );

        // 2. Standalone JumpList surface
        idx.on_add_on_thread(
            3000,
            3020,
            "JumpViewUI.JumpListControl".into(),
            "JumpList".into(),
            100,
        );
        idx.on_add_on_thread(
            3020,
            3021,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "RootGrid".into(),
            100,
        );
        idx.on_add_on_thread(
            3021,
            3022,
            "JumpViewUI.JumpListItemControl".into(),
            "".into(),
            100,
        );
        idx.on_add_on_thread(
            3022,
            3023,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
            100,
        );

        // 3. Forbidden containers (StartMenu, ComboBox, ToolTip) must never match
        idx.on_add_on_thread(0, 3030, "StartMenu.StartInnerView".into(), "StartMenu".into(), 100);
        idx.on_add_on_thread(
            3030,
            3031,
            "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
            "".into(),
            100,
        );
        idx.on_add_on_thread(
            3031,
            3032,
            "Windows.UI.Xaml.Controls.Border".into(),
            "MenuFlyoutPresenterBorder".into(),
            100,
        );
        idx.on_add_on_thread(0, 3040, "Windows.UI.Xaml.Controls.ComboBox".into(), "".into(), 100);
        idx.on_add_on_thread(
            3040,
            3041,
            "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
            "".into(),
            100,
        );
        idx.on_add_on_thread(
            3041,
            3042,
            "Windows.UI.Xaml.Controls.Border".into(),
            "MenuFlyoutPresenterBorder".into(),
            100,
        );

        assert_eq!(idx.classify_target(3002), Some(TargetKind::MenuFlyoutSurface));
        assert_eq!(idx.classify_target(3012), Some(TargetKind::MenuFlyoutSurface));
        assert_eq!(idx.classify_target(3021), Some(TargetKind::MenuFlyoutSurface));
        assert_eq!(idx.classify_target(3004), None);
        assert_eq!(idx.classify_target(3023), None);
        assert_eq!(idx.classify_target(3032), None);
        assert_eq!(idx.classify_target(3042), None);
    }

    #[test]
    fn jumplist_scope_targets_taskbar_jumplist_and_excludes_notifications_and_items() {
        let mut idx = VisualTreeIndex::new();

        // 1. Win11 TaskbarJumpListFrame > Grid#JumpListGrid > Border#JumpListRestyledAcrylic + item chrome
        idx.on_add_on_thread(
            0,
            4001,
            "JumpViewUI.TaskbarJumpListFrame".into(),
            "TaskbarJumpListFrame".into(),
            200,
        );
        idx.on_add_on_thread(
            4001,
            4002,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "JumpListGrid".into(),
            200,
        );
        idx.on_add_on_thread(
            4002,
            4003,
            "Windows.UI.Xaml.Controls.Border".into(),
            "JumpListRestyledAcrylic".into(),
            200,
        );
        idx.on_add_on_thread(
            4002,
            4004,
            "JumpViewUI.JumpListItemControl".into(),
            "".into(),
            200,
        );
        idx.on_add_on_thread(
            4004,
            4005,
            "Windows.UI.Xaml.Controls.Border".into(),
            "BackgroundBorder".into(),
            200,
        );
        idx.on_add_on_thread(
            4002,
            4006,
            "JumpViewUI.SystemItemControl".into(),
            "".into(),
            200,
        );
        idx.on_add_on_thread(
            4006,
            4007,
            "Windows.UI.Xaml.Controls.Border".into(),
            "LayoutRoot".into(),
            200,
        );

        // 2. Nested context menu ControlHostMenuFlyoutPresenter inside JumpViewUI
        idx.on_add_on_thread(
            0,
            4010,
            "JumpViewUI.ControlHostMenuFlyoutPresenter".into(),
            "".into(),
            200,
        );
        idx.on_add_on_thread(
            4010,
            4011,
            "Windows.UI.Xaml.Controls.Border".into(),
            "MenuFlyoutPresenterBorder".into(),
            200,
        );

        // 3. Notifications / Action Center Toast and nested MenuFlyoutPresenter must be excluded
        idx.on_add_on_thread(
            0,
            4020,
            "ActionCenter.ToastNotificationView".into(),
            "ToastRoot".into(),
            200,
        );
        idx.on_add_on_thread(
            4020,
            4021,
            "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
            "".into(),
            200,
        );
        idx.on_add_on_thread(
            4021,
            4022,
            "Windows.UI.Xaml.Controls.Border".into(),
            "MenuFlyoutPresenterBorder".into(),
            200,
        );
        // Plain MenuFlyoutPresenter in ShellExperienceHost without JumpList ancestry must not match
        idx.on_add_on_thread(
            0,
            4025,
            "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
            "".into(),
            200,
        );
        idx.on_add_on_thread(
            4025,
            4026,
            "Windows.UI.Xaml.Controls.Border".into(),
            "MenuFlyoutPresenterBorder".into(),
            200,
        );

        assert_eq!(
            idx.classify_target_in_scope(4003, TapHostScope::ShellExperience),
            Some(TargetKind::MenuFlyoutSurface)
        );
        // When JumpListRestyledAcrylic is present on the thread, Grid#JumpListGrid must not duplicate it
        assert_eq!(
            idx.classify_target_in_scope(4002, TapHostScope::ShellExperience),
            None
        );
        assert_eq!(
            idx.classify_target_in_scope(4011, TapHostScope::ShellExperience),
            Some(TargetKind::MenuFlyoutSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(4005, TapHostScope::ShellExperience),
            None
        );
        assert_eq!(
            idx.classify_target_in_scope(4007, TapHostScope::ShellExperience),
            None
        );
        assert_eq!(
            idx.classify_target_in_scope(4022, TapHostScope::ShellExperience),
            None
        );
        assert_eq!(
            idx.classify_target_in_scope(4026, TapHostScope::ShellExperience),
            None
        );

        // 4. Legacy/variant thread without JumpListRestyledAcrylic falls back to TaskbarJumpListFrame > Border#LayoutRoot
        idx.on_add_on_thread(
            0,
            4030,
            "JumpViewUI.TaskbarJumpListFrame".into(),
            "".into(),
            201,
        );
        idx.on_add_on_thread(
            4030,
            4031,
            "Windows.UI.Xaml.Controls.Border".into(),
            "LayoutRoot".into(),
            201,
        );
        assert_eq!(
            idx.classify_target_in_scope(4031, TapHostScope::ShellExperience),
            Some(TargetKind::MenuFlyoutSurface)
        );
    }

    #[test]
    fn shell_experience_targets_notifications_calendar_toasts_and_isolates_history() {
        let mut idx = VisualTreeIndex::new();

        // 1. Notification Center + Calendar + minimized header overlay + history toast item
        idx.on_add_on_thread(
            0,
            5001,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "NotificationCenterGrid".into(),
            301,
        );
        idx.on_add_on_thread(
            5001,
            5002,
            "ActionCenter.FlexibleNormalToastView".into(),
            "".into(),
            301,
        );
        idx.on_add_on_thread(
            5002,
            5003,
            "Windows.UI.Xaml.Controls.Border".into(),
            "ToastBackgroundBorder2".into(),
            301,
        );
        idx.on_add_on_thread(
            5001,
            5004,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "FooterGrid".into(),
            301,
        );
        idx.on_add_on_thread(
            0,
            5010,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "CalendarCenterGrid".into(),
            301,
        );
        idx.on_add_on_thread(
            5010,
            5011,
            "Windows.UI.Xaml.Controls.Border".into(),
            "CalendarHeaderMinimizedOverlay".into(),
            301,
        );
        idx.on_add_on_thread(
            5010,
            5012,
            "Windows.UI.Xaml.Controls.Primitives.CalendarViewDayItem".into(),
            "".into(),
            301,
        );
        idx.on_add_on_thread(
            5012,
            5013,
            "Windows.UI.Xaml.Controls.Border".into(),
            "ToastBackgroundBorder2".into(),
            301,
        );

        assert_eq!(
            idx.classify_target_in_scope(5001, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(5003, TapHostScope::ShellExperience),
            None
        );
        assert_eq!(
            idx.classify_target_in_scope(5004, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSubSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(5010, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(5011, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSubSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(5013, TapHostScope::ShellExperience),
            None
        );

        // 2. Floating Toast with both ToastBackgroundBorder2 and inner ToastBackgroundBorder + interactive Button
        idx.on_add_on_thread(
            0,
            5100,
            "ActionCenter.FlexibleNormalToastView".into(),
            "".into(),
            302,
        );
        idx.on_add_on_thread(
            5100,
            5101,
            "Windows.UI.Xaml.Controls.Border".into(),
            "ToastBackgroundBorder2".into(),
            302,
        );
        idx.on_add_on_thread(
            5101,
            5102,
            "Windows.UI.Xaml.Controls.Border".into(),
            "ToastBackgroundBorder".into(),
            302,
        );
        idx.on_add_on_thread(
            5101,
            5103,
            "Windows.UI.Xaml.Controls.Button".into(),
            "DismissButton".into(),
            302,
        );
        idx.on_add_on_thread(
            5103,
            5104,
            "Windows.UI.Xaml.Controls.Border".into(),
            "ToastBackgroundBorder".into(),
            302,
        );

        assert_eq!(
            idx.classify_target_in_scope(5101, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(5102, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSubSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(5104, TapHostScope::ShellExperience),
            None
        );

        // 3. Single-border floating toast without ToastBackgroundBorder2 promotes ToastBackgroundBorder
        idx.on_add_on_thread(
            0,
            5200,
            "ActionCenter.FlexiblePriorityToastView".into(),
            "".into(),
            303,
        );
        idx.on_add_on_thread(
            5200,
            5201,
            "Windows.UI.Xaml.Controls.Border".into(),
            "ToastBackgroundBorder".into(),
            303,
        );
        assert_eq!(
            idx.classify_target_in_scope(5201, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSurface)
        );

        // 4. Legacy Win11 ActionCenter Quick Settings + Media Card inside ShellExperienceHost
        idx.on_add_on_thread(
            0,
            5300,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "ControlCenterRegion".into(),
            304,
        );
        idx.on_add_on_thread(
            0,
            5301,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "MediaTransportControlsRegion".into(),
            304,
        );
        idx.on_add_on_thread(
            5300,
            5302,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "FooterGrid".into(),
            304,
        );
        assert_eq!(
            idx.classify_target_in_scope(5300, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(5301, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(5302, TapHostScope::ShellExperience),
            Some(TargetKind::ShellCardSubSurface)
        );
    }

    #[test]
    fn shell_host_targets_quick_settings_media_card_subsurfaces_and_fallback_order() {
        let mut idx = VisualTreeIndex::new();

        // 1. Thread without RootGridBorder: MainGrid + L1Grid delivered before RootGridBorder
        idx.on_add_on_thread(0, 6000, "ControlCenter.ControlCenterView".into(), "".into(), 401);
        idx.on_add_on_thread(6000, 6001, "Windows.UI.Xaml.Controls.Grid".into(), "RootGrid".into(), 401);
        idx.on_add_on_thread(6001, 6002, "Windows.UI.Xaml.Controls.Grid".into(), "MainGrid".into(), 401);
        idx.on_add_on_thread(6002, 6003, "Windows.UI.Xaml.Controls.Grid".into(), "L1Grid".into(), 401);
        idx.on_add_on_thread(6003, 6004, "Windows.UI.Xaml.Controls.Border".into(), "BackgroundBlur".into(), 401);
        idx.on_add_on_thread(6003, 6005, "Windows.UI.Xaml.Controls.Grid".into(), "PaneFooter".into(), 401);
        idx.on_add_on_thread(
            6001,
            6006,
            "Windows.UI.Xaml.Controls.Grid".into(),
            "MediaTransportControlsRegion".into(),
            401,
        );
        // Interactive quick toggle tile border must stay untouched
        idx.on_add_on_thread(
            6003,
            6007,
            "Windows.UI.Xaml.Controls.Primitives.ToggleButton".into(),
            "".into(),
            401,
        );
        idx.on_add_on_thread(
            6007,
            6008,
            "Windows.UI.Xaml.Controls.Border".into(),
            "RootGridBorder".into(),
            401,
        );

        assert_eq!(
            idx.classify_target_in_scope(6002, TapHostScope::ShellHost),
            Some(TargetKind::ShellCardSubSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(6003, TapHostScope::ShellHost),
            Some(TargetKind::ShellCardSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(6004, TapHostScope::ShellHost),
            Some(TargetKind::ShellCardSubSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(6005, TapHostScope::ShellHost),
            Some(TargetKind::ShellCardSubSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(6006, TapHostScope::ShellHost),
            Some(TargetKind::ShellCardSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(6008, TapHostScope::ShellHost),
            None
        );

        // Once non-button RootGridBorder arrives on the same thread, L1Grid downgrades to SubSurface
        idx.on_add_on_thread(
            6002,
            6009,
            "Windows.UI.Xaml.Controls.Border".into(),
            "RootGridBorder".into(),
            401,
        );
        assert_eq!(
            idx.classify_target_in_scope(6009, TapHostScope::ShellHost),
            Some(TargetKind::ShellCardSurface)
        );
        assert_eq!(
            idx.classify_target_in_scope(6003, TapHostScope::ShellHost),
            Some(TargetKind::ShellCardSubSurface)
        );

        // 2. Context menu inside ShellHost matches MenuFlyoutSurface
        idx.on_add_on_thread(
            0,
            6020,
            "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
            "".into(),
            401,
        );
        idx.on_add_on_thread(
            6020,
            6021,
            "Windows.UI.Xaml.Controls.Border".into(),
            "MenuFlyoutPresenterBorder".into(),
            401,
        );
        assert_eq!(
            idx.classify_target_in_scope(6021, TapHostScope::ShellHost),
            Some(TargetKind::MenuFlyoutSurface)
        );
    }
}

#[cfg(windows)]
mod win {
    use super::tree_match::{
        appcontainer_pipe_name_for_pid, pipe_name_for_pid, shell_host_pipe_name_for_pid,
        TapHostScope, TargetKind, VisualTreeIndex, ALTTAB_ITEM_OVERLAY_NAME,
        CLSID_PRETTY_WINDOWS_TAP_U128, CMD_ALTTAB_RECT, CMD_HIDE, CMD_HOVER_FLYOUT_RECT,
        CMD_MENU_FLYOUT_RECTS, CMD_OSD_RECT, CMD_PING, CMD_RESTORE, CMD_SET_ALTTAB_ITEM_STYLE,
        CMD_SET_MENU_FLYOUT_FALLBACK_STYLE, CMD_SET_MENU_FLYOUT_SUPPRESS,
        CMD_SET_OSD_FALLBACK_STYLE, CMD_SET_OSD_SUPPRESS, CMD_SET_TOOLTIP_FALLBACK_STYLE,
        CMD_SET_TOOLTIP_SUPPRESS, CMD_SHELL_CARD_RECTS, CMD_SHUTDOWN, CMD_STATUS, CMD_TOOLTIP_RECT,
        CMD_VERSION, MAX_MENU_FLYOUT_RECTS, MAX_SHELL_CARD_RECTS, TAP_PROTOCOL_VERSION,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, VecDeque};
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
        ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE,
        HANDLE, HINSTANCE, INVALID_HANDLE_VALUE, S_FALSE, S_OK, TRUE, WAIT_OBJECT_0,
    };
    use windows::Win32::Security::{GetTokenInformation, TokenIsAppContainer, TOKEN_QUERY};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE,
        OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
    use windows::Win32::System::Com::{
        CoCreateFreeThreadedMarshaler, CoCreateInstance, CoInitializeEx, CoUninitialize,
        IGlobalInterfaceTable, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};
    use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW,
        PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, OpenProcess,
        OpenProcessToken, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
        INFINITE, PROCESS_SYNCHRONIZE,
    };
    use windows::Win32::System::WinRT::{RoActivateInstance, RoGetActivationFactory};

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
    const IID_IUIELEMENT10: GUID = GUID::from_u128(0xD531C629_AD2C_5F6B_ADCF_FB87287D18D7);
    const IID_IUIELEMENTSTATICS10: GUID = GUID::from_u128(0x60D25362_4B3E_53DA_8B78_38DB94AE8F26);
    const IID_IFRAMEWORKELEMENT: GUID = GUID::from_u128(0xA391D09B_4A99_4B7C_9D8D_6FA5D01F6FBF);
    const IID_IFRAMEWORKELEMENT4: GUID = GUID::from_u128(0x6B765BB3_FBA3_4404_BDEE_1A45D1CA5F21);
    const IID_IFRAMEWORKELEMENTSTATICS4: GUID =
        GUID::from_u128(0x9C41B155_C5D8_4663_BFF2_D8D54FB5DBB3);
    const IID_IDEPENDENCYOBJECT: GUID = GUID::from_u128(0x5C526665_F60E_4912_AF59_5FE0680F089D);
    const IID_IDEPENDENCYOBJECT2: GUID = GUID::from_u128(0x29FED85D_3D22_43A1_ADD0_17027C08B212);
    const IID_IDEPENDENCYPROPERTYCHANGEDCALLBACK: GUID =
        GUID::from_u128(0x45883D16_27BF_4BC1_AC26_94C1601F3A49);
    const IID_ICOREDISPATCHER: GUID = GUID::from_u128(0x60DB2FA8_B705_4FDE_A7D6_EBBB1891D39E);
    const IID_IDISPATCHEDHANDLER: GUID = GUID::from_u128(0xD1F276C4_98D8_4636_BF49_EB79507548E9);
    const IID_IBORDER: GUID = GUID::from_u128(0x797C4539_45BD_4633_A044_BFB02EF5170F);
    const IID_IBORDERSTATICS: GUID = GUID::from_u128(0xB81D1639_E911_4FD7_A4C4_B9C7F008B7FC);
    const IID_ICONTENTPRESENTER4: GUID = GUID::from_u128(0x997B1B17_3BF8_432B_8FD9_68C7BAB21F9D);
    const IID_ICONTENTPRESENTERSTATICS4: GUID =
        GUID::from_u128(0xFF8BE292_AF28_47EB_A5C3_DC44CED40FD2);
    const IID_ICONTROL: GUID = GUID::from_u128(0xA8912263_2951_4F58_A9C5_5A134EAA7F07);
    const IID_ICONTROL7: GUID = GUID::from_u128(0x3550DBE1_E561_5934_A3EF_7DB8BF6E3B45);
    const IID_ICONTROLSTATICS: GUID = GUID::from_u128(0x473B305B_877D_4C35_8FCA_46A05F96607A);
    const IID_ICONTROLSTATICS7: GUID = GUID::from_u128(0xAEE1CA45_C910_5DAB_A266_FDF50C0BEDD2);
    const IID_ITOOLTIP: GUID = GUID::from_u128(0x7DE5D75F_4415_429F_B523_699BDB8C30DB);
    const IID_IPANEL: GUID = GUID::from_u128(0xA50A4BBD_8361_469C_90DA_E9A40C7474DF);
    const IID_IPANELSTATICS: GUID = GUID::from_u128(0xF23D6B5D_8330_47A6_A046_25F509B25232);
    const IID_IGRID2: GUID = GUID::from_u128(0xF76EFA41_380E_45DB_BE87_9E1326BA4B57);
    const IID_IGRIDSTATICS2: GUID = GUID::from_u128(0x2D666658_58B6_4A1B_841B_875BA93B5D6A);
    const IID_IBRUSH: GUID = GUID::from_u128(0x8806A321_1E06_422C_A1CC_01696559E021);
    const IID_ISOLIDCOLORBRUSH: GUID = GUID::from_u128(0x9D850850_66F3_48DF_9A8F_824BD5E070AF);
    const IID_ISOLIDCOLORBRUSHFACTORY: GUID =
        GUID::from_u128(0xD935CE0C_86F5_4DA6_8A27_B1619EF7F92B);
    const IID_IDEPENDENCYPROPERTYSTATICS: GUID =
        GUID::from_u128(0x49E5F28F_8259_4D5C_AAE0_83D56DBB68D9);
    const IID_IVISUALTREEHELPERSTATICS: GUID =
        GUID::from_u128(0xE75758C4_D25D_4B1D_971F_596F17F12BAA);
    const IID_IELEMENTCOMPOSITIONPREVIEWSTATICS: GUID =
        GUID::from_u128(0x08C92B38_EC99_4C55_BC85_A1C180B27646);

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
    #[derive(Clone, Copy)]
    struct RawSize {
        width: f32,
        height: f32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawPoint {
        x: f32,
        y: f32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default, PartialEq)]
    struct RawThickness {
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default, PartialEq)]
    struct RawCornerRadius {
        top_left: f64,
        top_right: f64,
        bottom_right: f64,
        bottom_left: f64,
    }

    #[repr(C)]
    #[allow(dead_code)]
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
        get_is_hit_test_visible: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
        put_is_hit_test_visible: unsafe extern "system" fn(*mut c_void, u8) -> HRESULT,
        get_visibility: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
        put_visibility: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
        get_render_size: unsafe extern "system" fn(*mut c_void, *mut RawSize) -> HRESULT,
        _pad24_97: [*const c_void; 74],
        transform_to_visual:
            unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIGeneralTransformVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_inverse: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        transform_point:
            unsafe extern "system" fn(*mut c_void, RawPoint, *mut RawPoint) -> HRESULT,
    }

    #[repr(C)]
    struct RawIGeneralTransform {
        lp_vtbl: *const RawIGeneralTransformVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIVisualTreeHelperStaticsVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_9: [*const c_void; 4],
        get_child:
            unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut *mut c_void) -> HRESULT,
        get_children_count: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i32) -> HRESULT,
        get_parent:
            unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIVisualTreeHelperStatics {
        lp_vtbl: *const RawIVisualTreeHelperStaticsVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIInspectableVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void, *mut HSTRING) -> HRESULT,
        get_trust_level: unsafe extern "system" fn(*mut c_void),
    }

    #[repr(C)]
    struct RawIInspectable {
        lp_vtbl: *const RawIInspectableVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIFrameworkElementVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_30: [*const c_void; 25],
        get_margin: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
        put_margin: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
        get_name: unsafe extern "system" fn(*mut c_void, *mut HSTRING) -> HRESULT,
        put_name: unsafe extern "system" fn(*mut c_void, core::mem::MaybeUninit<HSTRING>) -> HRESULT,
    }

    #[repr(C)]
    struct RawIFrameworkElement {
        lp_vtbl: *const RawIFrameworkElementVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIFrameworkElement4Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_9: [*const c_void; 4],
        get_focus_visual_secondary_thickness:
            unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
        put_focus_visual_secondary_thickness:
            unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
        _pad12_13: [*const c_void; 2],
        get_focus_visual_secondary_brush:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_focus_visual_secondary_brush:
            unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIFrameworkElement4 {
        lp_vtbl: *const RawIFrameworkElement4Vtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIFrameworkElementStatics4Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_7: [*const c_void; 2],
        get_focus_visual_secondary_thickness_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        _pad9: *const c_void,
        get_focus_visual_secondary_brush_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIFrameworkElementStatics4 {
        lp_vtbl: *const RawIFrameworkElementStatics4Vtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIUIElement10Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_10: [*const c_void; 5],
        get_shadow: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_shadow: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIUIElement10 {
        lp_vtbl: *const RawIUIElement10Vtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIUIElementStatics10Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_shadow_property: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIUIElementStatics10 {
        lp_vtbl: *const RawIUIElementStatics10Vtbl,
    }

    const _: () = {
        assert!(size_of::<RawSize>() == 8);
        assert!(size_of::<RawPoint>() == 8);
        assert!(size_of::<RawThickness>() == 32);
        assert!(size_of::<RawCornerRadius>() == 32);
        assert!(size_of::<HSTRING>() == size_of::<usize>());
        assert!(
            core::mem::offset_of!(RawIUIElementVtbl, put_is_hit_test_visible)
                == 20 * size_of::<usize>()
        );
        assert!(core::mem::offset_of!(RawIUIElementVtbl, get_render_size) == 23 * size_of::<usize>());
        assert!(
            core::mem::offset_of!(RawIUIElementVtbl, transform_to_visual) == 98 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIUIElement10Vtbl, get_shadow) == 11 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIUIElement10Vtbl, put_shadow) == 12 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIUIElementStatics10Vtbl, get_shadow_property)
                == 6 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIVisualTreeHelperStaticsVtbl, get_child)
                == 10 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIVisualTreeHelperStaticsVtbl, get_children_count)
                == 11 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIVisualTreeHelperStaticsVtbl, get_parent)
                == 12 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIFrameworkElementVtbl, get_margin) == 31 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIFrameworkElementVtbl, put_margin) == 32 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIFrameworkElementVtbl, get_name) == 33 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIFrameworkElementVtbl, put_name) == 34 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(
                RawIFrameworkElement4Vtbl,
                get_focus_visual_secondary_thickness
            ) == 10 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(
                RawIFrameworkElement4Vtbl,
                put_focus_visual_secondary_thickness
            ) == 11 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(
                RawIFrameworkElement4Vtbl,
                get_focus_visual_secondary_brush
            ) == 14 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(
                RawIFrameworkElement4Vtbl,
                put_focus_visual_secondary_brush
            ) == 15 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(
                RawIFrameworkElementStatics4Vtbl,
                get_focus_visual_secondary_thickness_property
            ) == 8 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(
                RawIFrameworkElementStatics4Vtbl,
                get_focus_visual_secondary_brush_property
            ) == 10 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIBorderVtbl, get_border_thickness) == 8 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIBorderVtbl, put_border_thickness) == 9 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIBorderVtbl, get_corner_radius) == 12 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIBorderVtbl, put_corner_radius) == 13 * size_of::<usize>()
        );
        assert!(core::mem::offset_of!(RawIPanelVtbl, get_children) == 6 * size_of::<usize>());
        assert!(
            core::mem::offset_of!(RawIUIElementVectorVtbl, get_at) == 6 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIUIElementVectorVtbl, get_size) == 7 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIUIElementVectorVtbl, insert_at) == 11 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIUIElementVectorVtbl, remove_at) == 12 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIElementCompositionPreviewStaticsVtbl, get_element_visual)
                == 6 * size_of::<usize>()
        );
        assert!(core::mem::offset_of!(RawIVisualVtbl, get_is_visible) == 18 * size_of::<usize>());
        assert!(core::mem::offset_of!(RawIVisualVtbl, put_is_visible) == 19 * size_of::<usize>());
        assert!(core::mem::offset_of!(RawIGrid2Vtbl, get_border_brush) == 6 * size_of::<usize>());
        assert!(core::mem::offset_of!(RawIGrid2Vtbl, put_border_brush) == 7 * size_of::<usize>());
        assert!(
            core::mem::offset_of!(RawIGridStatics2Vtbl, get_border_brush_property)
                == 6 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIContentPresenter4Vtbl, get_border_brush)
                == 14 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIContentPresenter4Vtbl, put_border_brush)
                == 15 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIContentPresenter4Vtbl, get_corner_radius)
                == 18 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIContentPresenter4Vtbl, put_corner_radius)
                == 19 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIContentPresenter4Vtbl, get_background)
                == 22 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIContentPresenter4Vtbl, put_background)
                == 23 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIContentPresenterStatics4Vtbl, get_border_brush_property)
                == 10 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIContentPresenterStatics4Vtbl, get_background_property)
                == 14 * size_of::<usize>()
        );
        assert!(core::mem::offset_of!(RawIControlVtbl, get_background) == 36 * size_of::<usize>());
        assert!(core::mem::offset_of!(RawIControlVtbl, put_background) == 37 * size_of::<usize>());
        assert!(core::mem::offset_of!(RawIControlVtbl, get_border_brush) == 40 * size_of::<usize>());
        assert!(core::mem::offset_of!(RawIControlVtbl, put_border_brush) == 41 * size_of::<usize>());
        assert!(core::mem::offset_of!(RawIControl7Vtbl, get_corner_radius) == 8 * size_of::<usize>());
        assert!(core::mem::offset_of!(RawIControl7Vtbl, put_corner_radius) == 9 * size_of::<usize>());
        assert!(
            core::mem::offset_of!(RawIControlStaticsVtbl, get_background_property)
                == 21 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIControlStaticsVtbl, get_border_brush_property)
                == 23 * size_of::<usize>()
        );
        assert!(
            core::mem::offset_of!(RawIControlStatics7Vtbl, get_corner_radius_property)
                == 7 * size_of::<usize>()
        );
        assert!(core::mem::offset_of!(RawIToolTipVtbl, get_is_open) == 8 * size_of::<usize>());
    };

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
    const DIAG_ALTTAB_APPLIED: u32 = 1 << 13;
    const DIAG_ALTTAB_ITEM_APPLIED: u32 = 1 << 14;
    const DIAG_ALTTAB_FOCUS_APPLIED: u32 = 1 << 15;
    const DIAG_HOVER_FLYOUT_APPLIED: u32 = 1 << 16;
    const DIAG_HOVER_FLYOUT_SHADOW_APPLIED: u32 = 1 << 17;
    const DIAG_OSD_APPLIED: u32 = 1 << 18;
    const DIAG_OSD_SHADOW_APPLIED: u32 = 1 << 19;
    const DIAG_TOOLTIP_APPLIED: u32 = 1 << 20;
    const DIAG_TOOLTIP_SHADOW_APPLIED: u32 = 1 << 21;
    const DIAG_MENU_FLYOUT_APPLIED: u32 = 1 << 22;
    const DIAG_MENU_FLYOUT_SHADOW_APPLIED: u32 = 1 << 23;
    const DIAG_SHELL_CARD_APPLIED: u32 = 1 << 24;
    const DIAG_SHELL_CARD_SHADOW_APPLIED: u32 = 1 << 25;

    #[repr(C)]
    struct RawIUIElement {
        lp_vtbl: *const RawIUIElementVtbl,
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct RawUiColor {
        a: u8,
        r: u8,
        g: u8,
        b: u8,
    }

    impl RawUiColor {
        const TRANSPARENT: Self = Self {
            a: 0,
            r: 0,
            g: 0,
            b: 0,
        };

        const fn from_argb(argb: u32) -> Self {
            Self {
                a: ((argb >> 24) & 0xFF) as u8,
                r: ((argb >> 16) & 0xFF) as u8,
                g: ((argb >> 8) & 0xFF) as u8,
                b: (argb & 0xFF) as u8,
            }
        }

        const fn to_argb(self) -> u32 {
            ((self.a as u32) << 24)
                | ((self.r as u32) << 16)
                | ((self.g as u32) << 8)
                | (self.b as u32)
        }
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
        get_border_thickness: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
        put_border_thickness: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
        get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
        get_corner_radius: unsafe extern "system" fn(*mut c_void, *mut RawCornerRadius) -> HRESULT,
        put_corner_radius: unsafe extern "system" fn(*mut c_void, RawCornerRadius) -> HRESULT,
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
        get_children: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIPanel {
        lp_vtbl: *const RawIPanelVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIUIElementVectorVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_at: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
        get_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
        _pad8_10: [*const c_void; 3],
        insert_at: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT,
        remove_at: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    }

    #[repr(C)]
    struct RawIUIElementVector {
        lp_vtbl: *const RawIUIElementVectorVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIElementCompositionPreviewStaticsVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_element_visual:
            unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIElementCompositionPreviewStatics {
        lp_vtbl: *const RawIElementCompositionPreviewStaticsVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIVisualVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_17: [*const c_void; 12],
        get_is_visible: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
        put_is_visible: unsafe extern "system" fn(*mut c_void, u8) -> HRESULT,
    }

    #[repr(C)]
    struct RawIVisual {
        lp_vtbl: *const RawIVisualVtbl,
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
    #[allow(dead_code)]
    struct RawIGrid2Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIGrid2 {
        lp_vtbl: *const RawIGrid2Vtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIGridStatics2Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_border_brush_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIGridStatics2 {
        lp_vtbl: *const RawIGridStatics2Vtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIContentPresenter4Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_13: [*const c_void; 8],
        get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
        get_border_thickness: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
        put_border_thickness: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
        get_corner_radius: unsafe extern "system" fn(*mut c_void, *mut RawCornerRadius) -> HRESULT,
        put_corner_radius: unsafe extern "system" fn(*mut c_void, RawCornerRadius) -> HRESULT,
        get_padding: unsafe extern "system" fn(*mut c_void, *mut RawThickness) -> HRESULT,
        put_padding: unsafe extern "system" fn(*mut c_void, RawThickness) -> HRESULT,
        get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIContentPresenter4 {
        lp_vtbl: *const RawIContentPresenter4Vtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIContentPresenterStatics4Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_9: [*const c_void; 4],
        get_border_brush_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        _pad11_13: [*const c_void; 3],
        get_background_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIContentPresenterStatics4 {
        lp_vtbl: *const RawIContentPresenterStatics4Vtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIToolTipVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        get_horizontal_offset: unsafe extern "system" fn(*mut c_void, *mut f64) -> HRESULT,
        put_horizontal_offset: unsafe extern "system" fn(*mut c_void, f64) -> HRESULT,
        get_is_open: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    }

    #[repr(C)]
    struct RawIToolTip {
        lp_vtbl: *const RawIToolTipVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIControlVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_35: [*const c_void; 30],
        get_background: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_background: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
        _pad38_39: [*const c_void; 2],
        get_border_brush: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        put_border_brush: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIControl {
        lp_vtbl: *const RawIControlVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIControl7Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_7: [*const c_void; 2],
        get_corner_radius: unsafe extern "system" fn(*mut c_void, *mut RawCornerRadius) -> HRESULT,
        put_corner_radius: unsafe extern "system" fn(*mut c_void, RawCornerRadius) -> HRESULT,
    }

    #[repr(C)]
    struct RawIControl7 {
        lp_vtbl: *const RawIControl7Vtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIControlStaticsVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6_20: [*const c_void; 15],
        get_background_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        _pad22: *const c_void,
        get_border_brush_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIControlStatics {
        lp_vtbl: *const RawIControlStaticsVtbl,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct RawIControlStatics7Vtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_iids: unsafe extern "system" fn(*mut c_void),
        get_runtime_class_name: unsafe extern "system" fn(*mut c_void),
        get_trust_level: unsafe extern "system" fn(*mut c_void),
        _pad6: *const c_void,
        get_corner_radius_property:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    }

    #[repr(C)]
    struct RawIControlStatics7 {
        lp_vtbl: *const RawIControlStatics7Vtbl,
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

    unsafe fn fetch_solid_brush(color: RawUiColor) -> Option<ComRaw> {
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

    unsafe fn fetch_transparent_brush() -> Option<ComRaw> {
        fetch_solid_brush(RawUiColor::TRANSPARENT)
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

    unsafe fn fetch_grid_border_brush_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_visual_tree_helper() -> Option<ComRaw> {
        activate_factory_iface(
            "Windows.UI.Xaml.Media.VisualTreeHelper",
            &IID_IVISUALTREEHELPERSTATICS,
        )
    }

    unsafe fn cached_visual_tree_helper() -> Option<ComRaw> {
        ensure_cached_com(
            |s| s.visual_tree_helper.as_ref(),
            |s, v| s.visual_tree_helper = Some(v),
            fetch_visual_tree_helper,
        )
    }

    unsafe fn fetch_element_composition_preview() -> Option<ComRaw> {
        activate_factory_iface(
            "Windows.UI.Xaml.Hosting.ElementCompositionPreview",
            &IID_IELEMENTCOMPOSITIONPREVIEWSTATICS,
        )
    }

    unsafe fn cached_element_composition_preview() -> Option<ComRaw> {
        ensure_cached_com(
            |s| s.element_comp_preview.as_ref(),
            |s, v| s.element_comp_preview = Some(v),
            fetch_element_composition_preview,
        )
    }

    unsafe fn fetch_focus_secondary_brush_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_focus_secondary_thickness_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_element_shadow_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_content_presenter_bg_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_content_presenter_border_brush_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_control_bg_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_control_border_brush_dp() -> Option<ComRaw> {
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

    unsafe fn fetch_control_corner_radius_dp() -> Option<ComRaw> {
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

    unsafe fn com_identity_key(ptr: *mut c_void) -> u64 {
        let unk = qi_interface(ptr, &IID_IUNKNOWN);
        let raw = if !unk.is_null() {
            unk.as_ptr() as u64
        } else {
            ptr as u64
        };
        (1u64 << 63) | (raw & !(1u64 << 63))
    }

    unsafe fn element_framework_name(ptr: *mut c_void) -> Option<String> {
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

    unsafe fn element_runtime_class_name(ptr: *mut c_void) -> Option<String> {
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

    unsafe fn is_solid_brush_with_color(brush_ptr: *mut c_void, target: RawUiColor) -> bool {
        if brush_ptr.is_null() {
            return false;
        }
        let solid_com = qi_interface(brush_ptr, &IID_ISOLIDCOLORBRUSH);
        if solid_com.is_null() {
            return false;
        }
        let solid = solid_com.as_ptr().cast::<RawISolidColorBrush>();
        let mut col = RawUiColor {
            a: target.a.wrapping_add(1),
            r: 0,
            g: 0,
            b: 0,
        };
        ((*(*solid).lp_vtbl).get_color)(solid_com.as_ptr(), &mut col).is_ok() && col == target
    }

    enum SavedBrush {
        Unset,
        Local(ComRaw),
    }

    enum SavedCornerRadius {
        Unset,
        Local(RawCornerRadius),
    }

    struct TooltipTrackedEntry {
        presenter_weak: ComRaw,
        tooltip_key: Option<u64>,
        tooltip_weak: Option<ComRaw>,
    }

    #[derive(Default)]
    struct SavedTargetBrushes {
        background: Option<SavedBrush>,
        border_brush: Option<SavedBrush>,
        focus_secondary: Option<SavedBrush>,
        shadow: Option<SavedBrush>,
    }

    #[derive(Clone, Copy)]
    enum BrushSlot {
        Background,
        BorderBrush,
        FocusSecondary,
        Shadow,
    }

    struct PropWatchEntry {
        weak_obj: ComRaw,
        dp: ComRaw,
        token: i64,
    }

    #[derive(Default)]
    struct UiThreadBrushStore {
        trans_brush: Option<ComRaw>,
        tinted_brushes: HashMap<u32, ComRaw>,
        unset_value: Option<ComRaw>,
        border_bg_dp: Option<ComRaw>,
        border_brush_dp: Option<ComRaw>,
        panel_bg_dp: Option<ComRaw>,
        grid_border_brush_dp: Option<ComRaw>,
        content_presenter_bg_dp: Option<ComRaw>,
        content_presenter_border_brush_dp: Option<ComRaw>,
        control_bg_dp: Option<ComRaw>,
        control_border_brush_dp: Option<ComRaw>,
        control_corner_radius_dp: Option<ComRaw>,
        focus_secondary_brush_dp: Option<ComRaw>,
        focus_secondary_thickness_dp: Option<ComRaw>,
        element_shadow_dp: Option<ComRaw>,
        visual_tree_helper: Option<ComRaw>,
        element_comp_preview: Option<ComRaw>,
        prewarmed: bool,
        saved: HashMap<u64, SavedTargetBrushes>,
        watches: HashMap<(u64, u8), PropWatchEntry>,
        alttab_subtargets: HashMap<u64, (TargetKind, ComRaw)>,
        alttab_item_orig_comp_visible: HashMap<u64, u8>,
        alttab_item_orig_focus_thickness: HashMap<u64, RawThickness>,
        hover_flyout_shapes: HashMap<u64, (ComRaw, f64, i32)>,
        tooltip_orig_corner_radius: HashMap<u64, SavedCornerRadius>,
        tooltip_modified_targets: HashMap<u64, TooltipTrackedEntry>,
        menu_flyout_orig_corner_radius: HashMap<u64, SavedCornerRadius>,
        menu_flyout_modified_targets: HashMap<u64, TooltipTrackedEntry>,
        shell_card_orig_corner_radius: HashMap<u64, SavedCornerRadius>,
        shell_card_modified_targets: HashMap<u64, TooltipTrackedEntry>,
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
                        RawUiColor::TRANSPARENT,
                        true,
                        None,
                        DIAG_OVERFLOW_APPLIED,
                    );
                }
            }
            TargetKind::AltTabBorder => {
                let border_com = qi_interface(sender, &IID_IBORDER);
                if !border_com.is_null() {
                    sync_overflow_brush_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        border_com.as_ptr(),
                        cb.prop,
                        RawUiColor::TRANSPARENT,
                        true,
                        None,
                        DIAG_ALTTAB_APPLIED,
                    );
                }
            }
            TargetKind::AltTabItemBorder => {
                let border_com = qi_interface(sender, &IID_IBORDER);
                if !border_com.is_null() {
                    let target_color = match cb.prop {
                        BrushPropKind::BorderBrush => RawUiColor::from_argb(
                            cb.runtime.alt_tab_item_border_argb.load(Ordering::Acquire),
                        ),
                        _ => RawUiColor::from_argb(
                            cb.runtime.alt_tab_item_bg_argb.load(Ordering::Acquire),
                        ),
                    };
                    sync_overflow_brush_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        border_com.as_ptr(),
                        cb.prop,
                        target_color,
                        true,
                        None,
                        DIAG_ALTTAB_ITEM_APPLIED,
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
                        RawUiColor::TRANSPARENT,
                        true,
                        None,
                        DIAG_OVERFLOW_APPLIED,
                    );
                }
            }
            TargetKind::AltTabItemFocus => {
                if matches!(cb.prop, BrushPropKind::FocusVisualSecondaryThickness) {
                    sync_focus_secondary_thickness_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        true,
                        true,
                    );
                } else {
                    let fe4_com = qi_interface(sender, &IID_IFRAMEWORKELEMENT4);
                    if !fe4_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            fe4_com.as_ptr(),
                            BrushPropKind::FocusVisualSecondaryBrush,
                            RawUiColor::TRANSPARENT,
                            true,
                            None,
                            DIAG_ALTTAB_FOCUS_APPLIED,
                        );
                    }
                }
            }
            TargetKind::HoverFlyoutBorder => {
                if matches!(cb.prop, BrushPropKind::ElementShadow) {
                    sync_element_shadow_prop(
                        &cb.runtime,
                        cb.handle,
                        sender,
                        true,
                        None,
                        DIAG_HOVER_FLYOUT_SHADOW_APPLIED,
                        false,
                    );
                } else {
                    let border_com = qi_interface(sender, &IID_IBORDER);
                    if !border_com.is_null() {
                        sync_overflow_brush_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            border_com.as_ptr(),
                            cb.prop,
                            RawUiColor::TRANSPARENT,
                            true,
                            None,
                            DIAG_HOVER_FLYOUT_APPLIED,
                        );
                    }
                }
            }
            TargetKind::OsdPanel => {
                let (target_bg, target_border) = cb.runtime.osd_target_colors();
                match cb.prop {
                    BrushPropKind::PanelBackground => {
                        let panel_com = qi_interface(sender, &IID_IPANEL);
                        if !panel_com.is_null() {
                            sync_overflow_brush_prop(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                panel_com.as_ptr(),
                                BrushPropKind::PanelBackground,
                                target_bg,
                                true,
                                None,
                                DIAG_OSD_APPLIED,
                            );
                        }
                    }
                    BrushPropKind::GridBorderBrush => {
                        let grid2_com = qi_interface(sender, &IID_IGRID2);
                        if !grid2_com.is_null() {
                            sync_overflow_brush_prop(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                grid2_com.as_ptr(),
                                BrushPropKind::GridBorderBrush,
                                target_border,
                                true,
                                None,
                                DIAG_OSD_APPLIED,
                            );
                        }
                    }
                    BrushPropKind::ElementShadow => {
                        sync_element_shadow_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            true,
                            None,
                            DIAG_OSD_SHADOW_APPLIED,
                            false,
                        );
                    }
                    _ => {}
                }
            }
            TargetKind::ToolTipPresenter => {
                let (target_bg, target_border) = cb.runtime.tooltip_target_colors();
                match cb.prop {
                    BrushPropKind::ControlBackground => {
                        let ctrl_com = qi_interface(sender, &IID_ICONTROL);
                        if !ctrl_com.is_null() {
                            sync_overflow_brush_prop(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                ctrl_com.as_ptr(),
                                BrushPropKind::ControlBackground,
                                target_bg,
                                true,
                                None,
                                DIAG_TOOLTIP_APPLIED,
                            );
                        }
                    }
                    BrushPropKind::ControlBorderBrush => {
                        let ctrl_com = qi_interface(sender, &IID_ICONTROL);
                        if !ctrl_com.is_null() {
                            sync_overflow_brush_prop(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                ctrl_com.as_ptr(),
                                BrushPropKind::ControlBorderBrush,
                                target_border,
                                true,
                                None,
                                DIAG_TOOLTIP_APPLIED,
                            );
                        }
                    }
                    BrushPropKind::ContentPresenterBackground => {
                        let cp_com = qi_interface(sender, &IID_ICONTENTPRESENTER4);
                        if !cp_com.is_null() {
                            let _ = sync_overflow_brush_prop_with_mode(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                cp_com.as_ptr(),
                                BrushPropKind::ContentPresenterBackground,
                                target_bg,
                                true,
                                None,
                                DIAG_TOOLTIP_APPLIED,
                                true,
                            );
                        }
                    }
                    BrushPropKind::ContentPresenterBorderBrush => {
                        let cp_com = qi_interface(sender, &IID_ICONTENTPRESENTER4);
                        if !cp_com.is_null() {
                            let _ = sync_overflow_brush_prop_with_mode(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                cp_com.as_ptr(),
                                BrushPropKind::ContentPresenterBorderBrush,
                                target_border,
                                true,
                                None,
                                DIAG_TOOLTIP_APPLIED,
                                true,
                            );
                        }
                    }
                    BrushPropKind::ElementShadow => {
                        let is_cp = !qi_interface(sender, &IID_ICONTENTPRESENTER4).is_null();
                        sync_element_shadow_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            true,
                            None,
                            DIAG_TOOLTIP_SHADOW_APPLIED,
                            is_cp,
                        );
                    }
                    _ => {}
                }
            }
            TargetKind::MenuFlyoutSurface => {
                let (target_bg, target_border) = cb.runtime.menu_flyout_target_colors();
                match cb.prop {
                    BrushPropKind::ControlBackground | BrushPropKind::ControlBorderBrush => {
                        let ctrl_com = qi_interface(sender, &IID_ICONTROL);
                        if !ctrl_com.is_null() {
                            let col = if matches!(cb.prop, BrushPropKind::ControlBackground) {
                                target_bg
                            } else {
                                target_border
                            };
                            sync_overflow_brush_prop(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                ctrl_com.as_ptr(),
                                cb.prop,
                                col,
                                true,
                                None,
                                DIAG_MENU_FLYOUT_APPLIED,
                            );
                        }
                    }
                    BrushPropKind::BorderBackground | BrushPropKind::BorderBrush => {
                        let border_com = qi_interface(sender, &IID_IBORDER);
                        if !border_com.is_null() {
                            let col = if matches!(cb.prop, BrushPropKind::BorderBackground) {
                                target_bg
                            } else {
                                target_border
                            };
                            let _ = sync_overflow_brush_prop_with_mode(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                border_com.as_ptr(),
                                cb.prop,
                                col,
                                true,
                                None,
                                DIAG_MENU_FLYOUT_APPLIED,
                                true,
                            );
                        }
                    }
                    BrushPropKind::PanelBackground => {
                        let panel_com = qi_interface(sender, &IID_IPANEL);
                        if !panel_com.is_null() {
                            sync_overflow_brush_prop(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                panel_com.as_ptr(),
                                BrushPropKind::PanelBackground,
                                target_bg,
                                true,
                                None,
                                DIAG_MENU_FLYOUT_APPLIED,
                            );
                        }
                    }
                    BrushPropKind::GridBorderBrush => {
                        let grid2_com = qi_interface(sender, &IID_IGRID2);
                        if !grid2_com.is_null() {
                            sync_overflow_brush_prop(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                grid2_com.as_ptr(),
                                BrushPropKind::GridBorderBrush,
                                target_border,
                                true,
                                None,
                                DIAG_MENU_FLYOUT_APPLIED,
                            );
                        }
                    }
                    BrushPropKind::ElementShadow => {
                        sync_element_shadow_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            true,
                            None,
                            DIAG_MENU_FLYOUT_SHADOW_APPLIED,
                            false,
                        );
                    }
                    _ => {}
                }
            }
            TargetKind::ShellCardSurface | TargetKind::ShellCardSubSurface => {
                let (target_bg, target_border) = if cb.kind == TargetKind::ShellCardSubSurface {
                    (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
                } else {
                    cb.runtime.menu_flyout_target_colors()
                };
                match cb.prop {
                    BrushPropKind::BorderBackground => {
                        let border_com = qi_interface(sender, &IID_IBORDER);
                        if !border_com.is_null() {
                            let _ = sync_overflow_brush_prop_with_mode(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                border_com.as_ptr(),
                                BrushPropKind::BorderBackground,
                                target_bg,
                                true,
                                None,
                                DIAG_SHELL_CARD_APPLIED,
                                true,
                            );
                        }
                    }
                    BrushPropKind::BorderBrush => {
                        let border_com = qi_interface(sender, &IID_IBORDER);
                        if !border_com.is_null() {
                            let _ = sync_overflow_brush_prop_with_mode(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                border_com.as_ptr(),
                                BrushPropKind::BorderBrush,
                                target_border,
                                true,
                                None,
                                DIAG_SHELL_CARD_APPLIED,
                                true,
                            );
                        }
                    }
                    BrushPropKind::PanelBackground => {
                        let panel_com = qi_interface(sender, &IID_IPANEL);
                        if !panel_com.is_null() {
                            sync_overflow_brush_prop(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                panel_com.as_ptr(),
                                BrushPropKind::PanelBackground,
                                target_bg,
                                true,
                                None,
                                DIAG_SHELL_CARD_APPLIED,
                            );
                        }
                    }
                    BrushPropKind::GridBorderBrush => {
                        let grid2_com = qi_interface(sender, &IID_IGRID2);
                        if !grid2_com.is_null() {
                            sync_overflow_brush_prop(
                                &cb.runtime,
                                cb.handle,
                                sender,
                                grid2_com.as_ptr(),
                                BrushPropKind::GridBorderBrush,
                                target_border,
                                true,
                                None,
                                DIAG_SHELL_CARD_APPLIED,
                            );
                        }
                    }
                    BrushPropKind::ElementShadow => {
                        sync_element_shadow_prop(
                            &cb.runtime,
                            cb.handle,
                            sender,
                            true,
                            None,
                            DIAG_SHELL_CARD_SHADOW_APPLIED,
                            false,
                        );
                    }
                    _ => {}
                }
            }
            TargetKind::ShapeElement | TargetKind::HoverFlyoutShape => {}
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
        let _ = ensure_cached_com(
            |s| s.grid_border_brush_dp.as_ref(),
            |s, v| s.grid_border_brush_dp = Some(v),
            fetch_grid_border_brush_dp,
        );
        let _ = ensure_cached_com(
            |s| s.content_presenter_bg_dp.as_ref(),
            |s, v| s.content_presenter_bg_dp = Some(v),
            fetch_content_presenter_bg_dp,
        );
        let _ = ensure_cached_com(
            |s| s.content_presenter_border_brush_dp.as_ref(),
            |s, v| s.content_presenter_border_brush_dp = Some(v),
            fetch_content_presenter_border_brush_dp,
        );
        let _ = ensure_cached_com(
            |s| s.control_bg_dp.as_ref(),
            |s, v| s.control_bg_dp = Some(v),
            fetch_control_bg_dp,
        );
        let _ = ensure_cached_com(
            |s| s.control_border_brush_dp.as_ref(),
            |s, v| s.control_border_brush_dp = Some(v),
            fetch_control_border_brush_dp,
        );
        let _ = ensure_cached_com(
            |s| s.control_corner_radius_dp.as_ref(),
            |s, v| s.control_corner_radius_dp = Some(v),
            fetch_control_corner_radius_dp,
        );
        let _ = ensure_cached_com(
            |s| s.focus_secondary_brush_dp.as_ref(),
            |s, v| s.focus_secondary_brush_dp = Some(v),
            fetch_focus_secondary_brush_dp,
        );
        let _ = ensure_cached_com(
            |s| s.focus_secondary_thickness_dp.as_ref(),
            |s, v| s.focus_secondary_thickness_dp = Some(v),
            fetch_focus_secondary_thickness_dp,
        );
        let _ = ensure_cached_com(
            |s| s.element_shadow_dp.as_ref(),
            |s, v| s.element_shadow_dp = Some(v),
            fetch_element_shadow_dp,
        );
        let _ = cached_visual_tree_helper();
        let _ = cached_element_composition_preview();
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

    unsafe fn cached_solid_brush(color: RawUiColor) -> Option<ComRaw> {
        if color.a == 0 {
            return ensure_cached_com(
                |s| s.trans_brush.as_ref(),
                |s, v| s.trans_brush = Some(v),
                fetch_transparent_brush,
            );
        }
        let key = color.to_argb();
        if let Some(hit) = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .and_then(|s| s.tinted_brushes.get(&key).map(ComRaw::clone_ref))
        }) {
            return Some(hit);
        }
        let created = fetch_solid_brush(color)?;
        let to_store = created.clone_ref();
        let (rejected, evicted) = UI_BRUSH_STORE.with(|cell| {
            if let Ok(mut s) = cell.try_borrow_mut() {
                if s.tinted_brushes.contains_key(&key) {
                    return (Some(to_store), None);
                }
                let drained = if s.tinted_brushes.len() >= 32 {
                    Some(s.tinted_brushes.drain().map(|(_, v)| v).collect::<Vec<_>>())
                } else {
                    None
                };
                s.tinted_brushes.insert(key, to_store);
                return (None, drained);
            }
            (Some(to_store), None)
        });
        drop(rejected);
        drop(evicted);
        Some(created)
    }

    fn has_saved_brush_slot(handle: u64, slot: BrushSlot) -> bool {
        UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow().ok().is_some_and(|s| {
                s.saved.get(&handle).is_some_and(|entry| match slot {
                    BrushSlot::Background => entry.background.is_some(),
                    BrushSlot::BorderBrush => entry.border_brush.is_some(),
                    BrushSlot::FocusSecondary => entry.focus_secondary.is_some(),
                    BrushSlot::Shadow => entry.shadow.is_some(),
                })
            })
        })
    }

    fn save_brush_slot_if_absent(handle: u64, slot: BrushSlot, val: SavedBrush) {
        let displaced = UI_BRUSH_STORE.with(|cell| {
            if let Ok(mut s) = cell.try_borrow_mut() {
                let entry = s.saved.entry(handle).or_default();
                let target = match slot {
                    BrushSlot::Background => &mut entry.background,
                    BrushSlot::BorderBrush => &mut entry.border_brush,
                    BrushSlot::FocusSecondary => &mut entry.focus_secondary,
                    BrushSlot::Shadow => &mut entry.shadow,
                };
                if target.is_none() {
                    *target = Some(val);
                    return None;
                }
            }
            Some(val)
        });
        drop(displaced);
    }

    fn is_alttab_subtarget_tracked(key: u64) -> bool {
        UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .is_some_and(|s| s.alttab_subtargets.contains_key(&key))
        })
    }

    unsafe fn track_alttab_subtarget_weak(key: u64, kind: TargetKind, insp_ptr: *mut c_void) {
        if key == 0 || insp_ptr.is_null() || is_alttab_subtarget_tracked(key) {
            return;
        }
        let weak_src = qi_interface(insp_ptr, &IID_IWEAKREFERENCESOURCE);
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

        let need_prune = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .is_some_and(|s| s.alttab_subtargets.len() >= 256)
        });
        if need_prune {
            let candidates: Vec<(u64, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .map(|s| {
                        s.alttab_subtargets
                            .iter()
                            .map(|(&k, (_, w))| (k, w.clone_ref()))
                            .collect()
                    })
                    .unwrap_or_default()
            });
            let mut dead = Vec::new();
            for (ck, wr_com) in candidates {
                let wr = wr_com.as_ptr().cast::<RawIWeakReference>();
                let mut resolved = ComRaw::null();
                if ((*(*wr).lp_vtbl).resolve)(wr_com.as_ptr(), &IID_IINSPECTABLE, &mut resolved.0)
                    .is_err()
                    || resolved.is_null()
                {
                    dead.push(ck);
                }
            }
            if !dead.is_empty() {
                let removed = UI_BRUSH_STORE.with(|cell| {
                    let Ok(mut s) = cell.try_borrow_mut() else {
                        return Vec::new();
                    };
                    dead.iter()
                        .filter_map(|k| s.alttab_subtargets.remove(k))
                        .collect::<Vec<_>>()
                });
                drop(removed);
                purge_saved_brushes_for_current_thread(&dead);
            }
        }

        let displaced = UI_BRUSH_STORE.with(|cell| {
            if let Ok(mut s) = cell.try_borrow_mut() {
                if !s.alttab_subtargets.contains_key(&key) {
                    s.alttab_subtargets.insert(key, (kind, weak_ref));
                    return None;
                }
            }
            Some(weak_ref)
        });
        drop(displaced);
    }

    fn unregister_watches_for_keys(keys: &[u64]) {
        if keys.is_empty() {
            return;
        }
        let removed_watches: Vec<PropWatchEntry> = UI_BRUSH_STORE.with(|cell| {
            let Ok(mut s) = cell.try_borrow_mut() else {
                return Vec::new();
            };
            let mut watches = Vec::new();
            if !s.watches.is_empty() {
                for &k in keys {
                    for prop_idx in 0u8..BrushPropKind::COUNT {
                        if let Some(entry) = s.watches.remove(&(k, prop_idx)) {
                            watches.push(entry);
                        }
                    }
                }
            }
            watches
        });
        for entry in removed_watches {
            unsafe {
                unregister_watch_entry(entry);
            }
        }
    }

    unsafe fn make_weak_ref_com(insp_ptr: *mut c_void) -> Option<ComRaw> {
        if insp_ptr.is_null() {
            return None;
        }
        let weak_src = qi_interface(insp_ptr, &IID_IWEAKREFERENCESOURCE);
        if weak_src.is_null() {
            return None;
        }
        let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
        let mut weak_ref = ComRaw::null();
        if ((*(*src_iface).lp_vtbl).get_weak_reference)(weak_src.as_ptr(), &mut weak_ref.0).is_ok()
            && !weak_ref.is_null()
        {
            Some(weak_ref)
        } else {
            None
        }
    }

    unsafe fn resolve_weak_inspectable(weak_com: &ComRaw) -> Option<ComRaw> {
        if weak_com.is_null() {
            return None;
        }
        let wr = weak_com.as_ptr().cast::<RawIWeakReference>();
        let mut insp = ComRaw::null();
        if ((*(*wr).lp_vtbl).resolve)(weak_com.as_ptr(), &IID_IINSPECTABLE, &mut insp.0).is_ok()
            && !insp.is_null()
        {
            Some(insp)
        } else {
            None
        }
    }

    unsafe fn resolve_weak_raw_inspectable(weak_raw: usize) -> Option<ComRaw> {
        if weak_raw == 0 {
            return None;
        }
        add_ref_raw_com_addr(weak_raw);
        let weak_com = ComRaw(weak_raw as *mut c_void);
        resolve_weak_inspectable(&weak_com)
    }

    unsafe fn track_paired_modified_target(
        primary_key: u64,
        primary_insp: *mut c_void,
        parent_pair: Option<(u64, *mut c_void)>,
        get_map: fn(&mut UiThreadBrushStore) -> &mut HashMap<u64, TooltipTrackedEntry>,
        read_map: fn(&UiThreadBrushStore) -> &HashMap<u64, TooltipTrackedEntry>,
    ) {
        if primary_key == 0 || primary_insp.is_null() {
            return;
        }
        let (already_exists, need_parent_fill, need_prune) = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow()
                .ok()
                .map(|s| {
                    let map = read_map(&s);
                    let exists = map.get(&primary_key);
                    let fill_parent = exists.is_some_and(|e| e.tooltip_weak.is_none())
                        && parent_pair.is_some();
                    (
                        exists.is_some(),
                        fill_parent,
                        exists.is_none() && map.len() >= 128,
                    )
                })
                .unwrap_or((false, false, false))
        });

        if already_exists {
            if need_parent_fill {
                if let Some((pk, p_ptr)) = parent_pair {
                    if let Some(p_weak) = make_weak_ref_com(p_ptr) {
                        let rejected = UI_BRUSH_STORE.with(|cell| {
                            if let Ok(mut s) = cell.try_borrow_mut() {
                                if let Some(entry) = get_map(&mut s).get_mut(&primary_key) {
                                    if entry.tooltip_weak.is_none() {
                                        entry.tooltip_key = Some(pk);
                                        entry.tooltip_weak = Some(p_weak);
                                        return None;
                                    }
                                }
                            }
                            Some(p_weak)
                        });
                        drop(rejected);
                    }
                }
            }
            return;
        }

        if need_prune {
            let candidates: Vec<(u64, ComRaw, Option<u64>, Option<ComRaw>)> = UI_BRUSH_STORE
                .with(|cell| {
                    cell.try_borrow()
                        .ok()
                        .map(|s| {
                            read_map(&s)
                                .iter()
                                .map(|(&k, e)| {
                                    (
                                        k,
                                        e.presenter_weak.clone_ref(),
                                        e.tooltip_key,
                                        e.tooltip_weak.as_ref().map(ComRaw::clone_ref),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                });
            let mut dead_entry_keys = Vec::new();
            let mut dead_purge_keys = Vec::new();
            for (pk, p_weak, tk_opt, t_weak_opt) in candidates {
                let p_alive = resolve_weak_inspectable(&p_weak).is_some();
                let t_alive = t_weak_opt
                    .as_ref()
                    .and_then(|tw| resolve_weak_inspectable(tw))
                    .is_some();
                if !p_alive && !t_alive {
                    dead_entry_keys.push(pk);
                    dead_purge_keys.push(pk);
                    if let Some(tk) = tk_opt {
                        dead_purge_keys.push(tk);
                    }
                }
            }
            if !dead_entry_keys.is_empty() {
                let removed: Vec<TooltipTrackedEntry> = UI_BRUSH_STORE.with(|cell| {
                    let Ok(mut s) = cell.try_borrow_mut() else {
                        return Vec::new();
                    };
                    let map = get_map(&mut s);
                    dead_entry_keys.iter().filter_map(|k| map.remove(k)).collect()
                });
                drop(removed);
                purge_saved_brushes_for_current_thread(&dead_purge_keys);
            }
        }

        let Some(presenter_weak) = make_weak_ref_com(primary_insp) else {
            return;
        };
        let (tooltip_key, tooltip_weak) = match parent_pair {
            Some((tk, t_ptr)) => match make_weak_ref_com(t_ptr) {
                Some(tw) => (Some(tk), Some(tw)),
                None => (Some(tk), None),
            },
            None => (None, None),
        };
        let displaced = UI_BRUSH_STORE.with(|cell| {
            if let Ok(mut s) = cell.try_borrow_mut() {
                let map = get_map(&mut s);
                if !map.contains_key(&primary_key) {
                    map.insert(
                        primary_key,
                        TooltipTrackedEntry {
                            presenter_weak,
                            tooltip_key,
                            tooltip_weak,
                        },
                    );
                    return None;
                }
            }
            Some((presenter_weak, tooltip_weak))
        });
        drop(displaced);
    }

    unsafe fn track_tooltip_modified_target(
        presenter_key: u64,
        presenter_insp: *mut c_void,
        parent_tooltip: Option<(u64, *mut c_void)>,
    ) {
        track_paired_modified_target(
            presenter_key,
            presenter_insp,
            parent_tooltip,
            |s| &mut s.tooltip_modified_targets,
            |s| &s.tooltip_modified_targets,
        );
    }

    unsafe fn track_menu_flyout_modified_target(
        surface_key: u64,
        surface_insp: *mut c_void,
        parent_ctrl: Option<(u64, *mut c_void)>,
    ) {
        track_paired_modified_target(
            surface_key,
            surface_insp,
            parent_ctrl,
            |s| &mut s.menu_flyout_modified_targets,
            |s| &s.menu_flyout_modified_targets,
        );
    }

    unsafe fn track_shell_card_modified_target(surface_key: u64, surface_insp: *mut c_void) {
        track_paired_modified_target(
            surface_key,
            surface_insp,
            None,
            |s| &mut s.shell_card_modified_targets,
            |s| &s.shell_card_modified_targets,
        );
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
                if !s.tooltip_orig_corner_radius.is_empty() {
                    for &h in handles {
                        s.tooltip_orig_corner_radius.remove(&h);
                    }
                }
                if !s.menu_flyout_orig_corner_radius.is_empty() {
                    for &h in handles {
                        s.menu_flyout_orig_corner_radius.remove(&h);
                    }
                }
                if !s.shell_card_orig_corner_radius.is_empty() {
                    for &h in handles {
                        s.shell_card_orig_corner_radius.remove(&h);
                    }
                }
                let brushes: Vec<SavedTargetBrushes> = if s.saved.is_empty() {
                    Vec::new()
                } else {
                    handles.iter().filter_map(|h| s.saved.remove(h)).collect()
                };
                if !s.alttab_item_orig_comp_visible.is_empty() {
                    for &h in handles {
                        s.alttab_item_orig_comp_visible.remove(&h);
                    }
                }
                if !s.alttab_item_orig_focus_thickness.is_empty() {
                    for &h in handles {
                        s.alttab_item_orig_focus_thickness.remove(&h);
                    }
                }
                let mut watches = Vec::new();
                if !s.watches.is_empty() {
                    for &h in handles {
                        for prop_idx in 0u8..BrushPropKind::COUNT {
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
        FocusVisualSecondaryBrush,
        ElementShadow,
        FocusVisualSecondaryThickness,
        GridBorderBrush,
        ContentPresenterBackground,
        ContentPresenterBorderBrush,
        ControlBackground,
        ControlBorderBrush,
    }

    impl BrushPropKind {
        const COUNT: u8 = 11;

        #[inline]
        fn key(self) -> u8 {
            match self {
                Self::BorderBackground => 0,
                Self::BorderBrush => 1,
                Self::PanelBackground => 2,
                Self::FocusVisualSecondaryBrush => 3,
                Self::ElementShadow => 4,
                Self::FocusVisualSecondaryThickness => 5,
                Self::GridBorderBrush => 6,
                Self::ContentPresenterBackground => 7,
                Self::ContentPresenterBorderBrush => 8,
                Self::ControlBackground => 9,
                Self::ControlBorderBrush => 10,
            }
        }

        #[inline]
        fn brush_slot(self) -> BrushSlot {
            match self {
                Self::BorderBrush
                | Self::GridBorderBrush
                | Self::ContentPresenterBorderBrush
                | Self::ControlBorderBrush => BrushSlot::BorderBrush,
                Self::FocusVisualSecondaryBrush | Self::FocusVisualSecondaryThickness => {
                    BrushSlot::FocusSecondary
                }
                Self::ElementShadow => BrushSlot::Shadow,
                Self::BorderBackground
                | Self::PanelBackground
                | Self::ContentPresenterBackground
                | Self::ControlBackground => BrushSlot::Background,
            }
        }

        #[inline]
        fn allow_null_on_hide(self) -> bool {
            matches!(
                self,
                Self::BorderBackground | Self::FocusVisualSecondaryBrush
            )
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
                Self::GridBorderBrush => {
                    let g = iface.cast::<RawIGrid2>();
                    ((*(*g).lp_vtbl).get_border_brush)(iface, &mut cur.0)
                }
                Self::ContentPresenterBackground => {
                    let cp = iface.cast::<RawIContentPresenter4>();
                    ((*(*cp).lp_vtbl).get_background)(iface, &mut cur.0)
                }
                Self::ContentPresenterBorderBrush => {
                    let cp = iface.cast::<RawIContentPresenter4>();
                    ((*(*cp).lp_vtbl).get_border_brush)(iface, &mut cur.0)
                }
                Self::ControlBackground => {
                    let c = iface.cast::<RawIControl>();
                    ((*(*c).lp_vtbl).get_background)(iface, &mut cur.0)
                }
                Self::ControlBorderBrush => {
                    let c = iface.cast::<RawIControl>();
                    ((*(*c).lp_vtbl).get_border_brush)(iface, &mut cur.0)
                }
                Self::FocusVisualSecondaryBrush => {
                    let fe4 = iface.cast::<RawIFrameworkElement4>();
                    ((*(*fe4).lp_vtbl).get_focus_visual_secondary_brush)(iface, &mut cur.0)
                }
                Self::ElementShadow => {
                    let ui10 = iface.cast::<RawIUIElement10>();
                    ((*(*ui10).lp_vtbl).get_shadow)(iface, &mut cur.0)
                }
                Self::FocusVisualSecondaryThickness => E_FAIL_HR,
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
                Self::GridBorderBrush => {
                    let g = iface.cast::<RawIGrid2>();
                    ((*(*g).lp_vtbl).put_border_brush)(iface, brush)
                }
                Self::ContentPresenterBackground => {
                    let cp = iface.cast::<RawIContentPresenter4>();
                    ((*(*cp).lp_vtbl).put_background)(iface, brush)
                }
                Self::ContentPresenterBorderBrush => {
                    let cp = iface.cast::<RawIContentPresenter4>();
                    ((*(*cp).lp_vtbl).put_border_brush)(iface, brush)
                }
                Self::ControlBackground => {
                    let c = iface.cast::<RawIControl>();
                    ((*(*c).lp_vtbl).put_background)(iface, brush)
                }
                Self::ControlBorderBrush => {
                    let c = iface.cast::<RawIControl>();
                    ((*(*c).lp_vtbl).put_border_brush)(iface, brush)
                }
                Self::FocusVisualSecondaryBrush => {
                    let fe4 = iface.cast::<RawIFrameworkElement4>();
                    ((*(*fe4).lp_vtbl).put_focus_visual_secondary_brush)(iface, brush)
                }
                Self::ElementShadow => {
                    let ui10 = iface.cast::<RawIUIElement10>();
                    ((*(*ui10).lp_vtbl).put_shadow)(iface, brush)
                }
                Self::FocusVisualSecondaryThickness => E_FAIL_HR,
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
                Self::GridBorderBrush => ensure_cached_com(
                    |s| s.grid_border_brush_dp.as_ref(),
                    |s, v| s.grid_border_brush_dp = Some(v),
                    fetch_grid_border_brush_dp,
                ),
                Self::ContentPresenterBackground => ensure_cached_com(
                    |s| s.content_presenter_bg_dp.as_ref(),
                    |s, v| s.content_presenter_bg_dp = Some(v),
                    fetch_content_presenter_bg_dp,
                ),
                Self::ContentPresenterBorderBrush => ensure_cached_com(
                    |s| s.content_presenter_border_brush_dp.as_ref(),
                    |s, v| s.content_presenter_border_brush_dp = Some(v),
                    fetch_content_presenter_border_brush_dp,
                ),
                Self::ControlBackground => ensure_cached_com(
                    |s| s.control_bg_dp.as_ref(),
                    |s, v| s.control_bg_dp = Some(v),
                    fetch_control_bg_dp,
                ),
                Self::ControlBorderBrush => ensure_cached_com(
                    |s| s.control_border_brush_dp.as_ref(),
                    |s, v| s.control_border_brush_dp = Some(v),
                    fetch_control_border_brush_dp,
                ),
                Self::FocusVisualSecondaryBrush => ensure_cached_com(
                    |s| s.focus_secondary_brush_dp.as_ref(),
                    |s, v| s.focus_secondary_brush_dp = Some(v),
                    fetch_focus_secondary_brush_dp,
                ),
                Self::ElementShadow => ensure_cached_com(
                    |s| s.element_shadow_dp.as_ref(),
                    |s, v| s.element_shadow_dp = Some(v),
                    fetch_element_shadow_dp,
                ),
                Self::FocusVisualSecondaryThickness => ensure_cached_com(
                    |s| s.focus_secondary_thickness_dp.as_ref(),
                    |s, v| s.focus_secondary_thickness_dp = Some(v),
                    fetch_focus_secondary_thickness_dp,
                ),
            }
        }
    }

    unsafe fn read_orig_shadow(
        insp_ptr: *mut c_void,
        dp: Option<&ComRaw>,
        unset_val: Option<&ComRaw>,
        current_shadow: &ComRaw,
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
                    return Some(SavedBrush::Local(local_val));
                }
            }
        }
        if !current_shadow.is_null() {
            Some(SavedBrush::Local(current_shadow.clone_ref()))
        } else {
            Some(SavedBrush::Unset)
        }
    }

    unsafe fn sync_element_shadow_prop(
        rt: &Arc<TapRuntime>,
        handle: u64,
        insp_ptr: *mut c_void,
        desired_hide: bool,
        restored_saved: Option<SavedBrush>,
        applied_flag: u32,
        prefer_local_on_non_null: bool,
    ) {
        let ui10_com = qi_interface(insp_ptr, &IID_IUIELEMENT10);
        if ui10_com.is_null() {
            return;
        }
        let ui10 = ui10_com.as_ptr().cast::<RawIUIElement10>();
        if desired_hide {
            let mut cur_shadow = ComRaw::null();
            let hr_get = ((*(*ui10).lp_vtbl).get_shadow)(ui10_com.as_ptr(), &mut cur_shadow.0);
            if hr_get.is_err() || cur_shadow.is_null() {
                return;
            }
            let captured = if !has_saved_brush_slot(handle, BrushSlot::Shadow) {
                if prefer_local_on_non_null {
                    Some(SavedBrush::Local(cur_shadow.clone_ref()))
                } else {
                    let dp = BrushPropKind::ElementShadow.dependency_property();
                    let unset = ensure_cached_com(
                        |s| s.unset_value.as_ref(),
                        |s, v| s.unset_value = Some(v),
                        fetch_unset_value,
                    );
                    read_orig_shadow(insp_ptr, dp.as_ref(), unset.as_ref(), &cur_shadow)
                }
            } else {
                None
            };
            if {
                let _sync_guard = PropSyncGuard::enter();
                ((*(*ui10).lp_vtbl).put_shadow)(ui10_com.as_ptr(), null_mut()).is_ok()
            } {
                if let Some(orig) = captured {
                    save_brush_slot_if_absent(handle, BrushSlot::Shadow, orig);
                }
                rt.diag_flags.fetch_or(applied_flag, Ordering::Relaxed);
            }
        } else {
            let _sync_guard = PropSyncGuard::enter();
            match restored_saved {
                Some(SavedBrush::Local(orig)) => {
                    let _ = ((*(*ui10).lp_vtbl).put_shadow)(ui10_com.as_ptr(), orig.as_ptr());
                }
                Some(SavedBrush::Unset) => {
                    let dp = BrushPropKind::ElementShadow.dependency_property();
                    if !clear_dp_value(insp_ptr, dp.as_ref()) {
                        let _ = ((*(*ui10).lp_vtbl).put_shadow)(ui10_com.as_ptr(), null_mut());
                    }
                }
                None => {
                    if !prefer_local_on_non_null {
                        let dp = BrushPropKind::ElementShadow.dependency_property();
                        if !clear_dp_value(insp_ptr, dp.as_ref()) {
                            let _ = ((*(*ui10).lp_vtbl).put_shadow)(ui10_com.as_ptr(), null_mut());
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn sync_overflow_brush_prop_with_mode(
        rt: &Arc<TapRuntime>,
        handle: u64,
        insp_ptr: *mut c_void,
        iface: *mut c_void,
        prop: BrushPropKind,
        target_color: RawUiColor,
        desired_hide: bool,
        restored_saved: Option<SavedBrush>,
        applied_flag: u32,
        prefer_local_on_non_null: bool,
    ) -> bool {
        if desired_hide {
            let (hr_get, cur) = prop.get(iface);
            if hr_get.is_err() || is_solid_brush_with_color(cur.as_ptr(), target_color) {
                return false;
            }
            if !prop.allow_null_on_hide() && cur.is_null() && target_color.a == 0 {
                return false;
            }
            let Some(tb) = cached_solid_brush(target_color) else {
                return false;
            };
            let captured = if !has_saved_brush_slot(handle, prop.brush_slot()) {
                if prefer_local_on_non_null && !cur.is_null() {
                    Some(SavedBrush::Local(cur.clone_ref()))
                } else {
                    let dp = prop.dependency_property();
                    let unset = ensure_cached_com(
                        |s| s.unset_value.as_ref(),
                        |s, v| s.unset_value = Some(v),
                        fetch_unset_value,
                    );
                    read_orig_brush(insp_ptr, dp.as_ref(), unset.as_ref(), &cur)
                }
            } else {
                None
            };
            if {
                let _sync_guard = PropSyncGuard::enter();
                prop.put(iface, tb.as_ptr()).is_ok()
            } {
                if let Some(orig) = captured {
                    save_brush_slot_if_absent(handle, prop.brush_slot(), orig);
                }
                rt.diag_flags.fetch_or(applied_flag, Ordering::Relaxed);
                return true;
            }
            false
        } else {
            let _sync_guard = PropSyncGuard::enter();
            match restored_saved {
                Some(SavedBrush::Local(orig)) => {
                    let _ = prop.put(iface, orig.as_ptr());
                    true
                }
                Some(SavedBrush::Unset) => {
                    let dp = prop.dependency_property();
                    if !clear_dp_value(insp_ptr, dp.as_ref()) {
                        let _ = prop.put(iface, null_mut());
                    }
                    true
                }
                None => {
                    if prefer_local_on_non_null {
                        return false;
                    }
                    let (hr_get, cur) = prop.get(iface);
                    if hr_get.is_ok()
                        && (is_transparent_solid_brush(cur.as_ptr())
                            || is_solid_brush_with_color(cur.as_ptr(), target_color))
                    {
                        let dp = prop.dependency_property();
                        if !clear_dp_value(insp_ptr, dp.as_ref()) {
                            let _ = prop.put(iface, null_mut());
                        }
                        true
                    } else {
                        false
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn sync_overflow_brush_prop(
        rt: &Arc<TapRuntime>,
        handle: u64,
        insp_ptr: *mut c_void,
        iface: *mut c_void,
        prop: BrushPropKind,
        target_color: RawUiColor,
        desired_hide: bool,
        restored_saved: Option<SavedBrush>,
        applied_flag: u32,
    ) {
        let _ = sync_overflow_brush_prop_with_mode(
            rt,
            handle,
            insp_ptr,
            iface,
            prop,
            target_color,
            desired_hide,
            restored_saved,
            applied_flag,
            false,
        );
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

    unsafe fn measure_element_bounds_dip(
        insp_ptr: *mut c_void,
        min_w: f32,
        min_h: f32,
        min_opacity: f64,
    ) -> Option<[f32; 4]> {
        if insp_ptr.is_null() {
            return None;
        }
        let ui_elem = qi_interface(insp_ptr, &IID_IUIELEMENT);
        if ui_elem.is_null() {
            return None;
        }
        let ui = ui_elem.as_ptr().cast::<RawIUIElement>();
        let mut vis = VISIBILITY_COLLAPSED;
        if ((*(*ui).lp_vtbl).get_visibility)(ui_elem.as_ptr(), &mut vis).is_err()
            || vis != VISIBILITY_VISIBLE
        {
            return None;
        }
        let mut op = 0.0f64;
        if ((*(*ui).lp_vtbl).get_opacity)(ui_elem.as_ptr(), &mut op).is_err()
            || !op.is_finite()
            || op <= min_opacity
        {
            return None;
        }
        if let Some(vth) = cached_visual_tree_helper() {
            let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
            let mut cur_dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
            for hop in 0..32 {
                if cur_dep.is_null() {
                    return None;
                }
                let mut parent_dep = ComRaw::null();
                if ((*(*vth_iface).lp_vtbl).get_parent)(
                    vth.as_ptr(),
                    cur_dep.as_ptr(),
                    &mut parent_dep.0,
                )
                .is_err()
                {
                    return None;
                }
                if parent_dep.is_null() {
                    if hop == 0 {
                        return None;
                    }
                    break;
                }
                let p_ui = qi_interface(parent_dep.as_ptr(), &IID_IUIELEMENT);
                if !p_ui.is_null() {
                    let p_ui_iface = p_ui.as_ptr().cast::<RawIUIElement>();
                    let mut p_vis = VISIBILITY_COLLAPSED;
                    if ((*(*p_ui_iface).lp_vtbl).get_visibility)(p_ui.as_ptr(), &mut p_vis)
                        .is_err()
                        || p_vis != VISIBILITY_VISIBLE
                    {
                        return None;
                    }
                    let mut p_op = 0.0f64;
                    if ((*(*p_ui_iface).lp_vtbl).get_opacity)(p_ui.as_ptr(), &mut p_op).is_err()
                        || !p_op.is_finite()
                        || p_op <= min_opacity
                    {
                        return None;
                    }
                }
                cur_dep = parent_dep;
            }
        }
        let mut sz = RawSize {
            width: 0.0,
            height: 0.0,
        };
        if ((*(*ui).lp_vtbl).get_render_size)(ui_elem.as_ptr(), &mut sz).is_err()
            || !sz.width.is_finite()
            || !sz.height.is_finite()
            || sz.width < min_w
            || sz.height < min_h
        {
            return None;
        }
        let mut gt_com = ComRaw::null();
        if ((*(*ui).lp_vtbl).transform_to_visual)(ui_elem.as_ptr(), null_mut(), &mut gt_com.0)
            .is_err()
            || gt_com.is_null()
        {
            return None;
        }
        let gt = gt_com.as_ptr().cast::<RawIGeneralTransform>();
        let mut p0 = RawPoint { x: 0.0, y: 0.0 };
        let mut p1 = RawPoint { x: 0.0, y: 0.0 };
        if ((*(*gt).lp_vtbl).transform_point)(
            gt_com.as_ptr(),
            RawPoint { x: 0.0, y: 0.0 },
            &mut p0,
        )
        .is_err()
            || ((*(*gt).lp_vtbl).transform_point)(
                gt_com.as_ptr(),
                RawPoint {
                    x: sz.width,
                    y: sz.height,
                },
                &mut p1,
            )
            .is_err()
        {
            return None;
        }
        if !p0.x.is_finite()
            || !p0.y.is_finite()
            || !p1.x.is_finite()
            || !p1.y.is_finite()
            || (p1.x - p0.x) < min_w
            || (p1.y - p0.y) < min_h
        {
            return None;
        }
        Some([p0.x, p0.y, p1.x, p1.y])
    }

    unsafe fn measure_hover_flyout_bounds_dip(insp_ptr: *mut c_void) -> Option<[f32; 4]> {
        if let Some(bounds) = measure_element_bounds_dip(insp_ptr, 48.0, 36.0, 0.05) {
            return Some(bounds);
        }
        let vth = cached_visual_tree_helper()?;
        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
        let dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        if dep.is_null() {
            return None;
        }
        let mut parent_dep = ComRaw::null();
        if ((*(*vth_iface).lp_vtbl).get_parent)(vth.as_ptr(), dep.as_ptr(), &mut parent_dep.0)
            .is_err()
            || parent_dep.is_null()
        {
            return None;
        }
        if element_framework_name(parent_dep.as_ptr()).as_deref() == Some("HoverFlyoutGrid") {
            return measure_element_bounds_dip(parent_dep.as_ptr(), 48.0, 36.0, 0.05);
        }
        None
    }

    unsafe fn measure_tooltip_bounds_dip(insp_ptr: *mut c_void) -> Option<[f32; 4]> {
        if let Some(vth) = cached_visual_tree_helper() {
            let dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
            if !dep.is_null() {
                let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
                let mut parent_dep = ComRaw::null();
                if ((*(*vth_iface).lp_vtbl).get_parent)(
                    vth.as_ptr(),
                    dep.as_ptr(),
                    &mut parent_dep.0,
                )
                .is_ok()
                    && !parent_dep.is_null()
                {
                    let tt_com = qi_interface(parent_dep.as_ptr(), &IID_ITOOLTIP);
                    if !tt_com.is_null() {
                        let tt = tt_com.as_ptr().cast::<RawIToolTip>();
                        let mut is_open = 1u8;
                        if ((*(*tt).lp_vtbl).get_is_open)(tt_com.as_ptr(), &mut is_open).is_ok()
                            && is_open == 0
                        {
                            return None;
                        }
                    }
                }
            }
        }
        measure_element_bounds_dip(insp_ptr, 20.0, 16.0, 0.05)
    }

    unsafe fn measure_menu_flyout_bounds_dip(insp_ptr: *mut c_void) -> Option<[f32; 4]> {
        let Some(vth) = cached_visual_tree_helper() else {
            return None;
        };
        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
        let mut cur_dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        let mut ancestor_hops = 0usize;
        for _ in 0..8 {
            if cur_dep.is_null() {
                break;
            }
            let mut parent_dep = ComRaw::null();
            if ((*(*vth_iface).lp_vtbl).get_parent)(
                vth.as_ptr(),
                cur_dep.as_ptr(),
                &mut parent_dep.0,
            )
            .is_err()
                || parent_dep.is_null()
            {
                break;
            }
            let p_ui = qi_interface(parent_dep.as_ptr(), &IID_IUIELEMENT);
            if !p_ui.is_null() {
                let p_ui_iface = p_ui.as_ptr().cast::<RawIUIElement>();
                let mut p_vis = VISIBILITY_COLLAPSED;
                if ((*(*p_ui_iface).lp_vtbl).get_visibility)(p_ui.as_ptr(), &mut p_vis).is_err()
                    || p_vis != VISIBILITY_VISIBLE
                {
                    return None;
                }
                let mut p_op = 0.0f64;
                if ((*(*p_ui_iface).lp_vtbl).get_opacity)(p_ui.as_ptr(), &mut p_op).is_err()
                    || !p_op.is_finite()
                    || p_op < 0.05
                {
                    return None;
                }
            }
            ancestor_hops += 1;
            cur_dep = parent_dep;
        }
        if ancestor_hops < 2 {
            return None;
        }
        measure_element_bounds_dip(insp_ptr, 64.0, 28.0, 0.05)
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn apply_border_target_appearance(
        rt: &Arc<TapRuntime>,
        handle: u64,
        insp_ptr: *mut c_void,
        kind: TargetKind,
        bg_color: RawUiColor,
        border_color: RawUiColor,
        desired_hide: bool,
        applied_flag: u32,
    ) {
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
            bg_color,
            desired_hide,
            saved.background,
            applied_flag,
        );
        sync_overflow_brush_prop(
            rt,
            handle,
            insp_ptr,
            border_com.as_ptr(),
            BrushPropKind::BorderBrush,
            border_color,
            desired_hide,
            saved.border_brush,
            applied_flag,
        );
        if desired_hide {
            rt.diag_flags.fetch_or(applied_flag, Ordering::Relaxed);
            ensure_prop_watch_registered(
                rt,
                handle,
                kind,
                BrushPropKind::BorderBackground,
                insp_ptr,
            );
            ensure_prop_watch_registered(
                rt,
                handle,
                kind,
                BrushPropKind::BorderBrush,
                insp_ptr,
            );
        }
    }

    unsafe fn sync_focus_secondary_thickness_prop(
        rt: &Arc<TapRuntime>,
        key: u64,
        insp_ptr: *mut c_void,
        desired_hide: bool,
        allow_clear_dp: bool,
    ) {
        let fe4_com = qi_interface(insp_ptr, &IID_IFRAMEWORKELEMENT4);
        if fe4_com.is_null() {
            return;
        }
        let fe4 = fe4_com.as_ptr().cast::<RawIFrameworkElement4>();
        if desired_hide {
            let mut cur_thick = RawThickness::default();
            let hr_get = ((*(*fe4).lp_vtbl).get_focus_visual_secondary_thickness)(
                fe4_com.as_ptr(),
                &mut cur_thick,
            );
            let zero = RawThickness::default();
            if hr_get.is_ok() && !approx_eq_thickness(cur_thick, zero) {
                UI_BRUSH_STORE.with(|cell| {
                    if let Ok(mut s) = cell.try_borrow_mut() {
                        s.alttab_item_orig_focus_thickness
                            .entry(key)
                            .or_insert(cur_thick);
                    }
                });
            }
            if hr_get.is_err() || !approx_eq_thickness(cur_thick, zero) {
                let _sync_guard = PropSyncGuard::enter();
                if ((*(*fe4).lp_vtbl).put_focus_visual_secondary_thickness)(
                    fe4_com.as_ptr(),
                    zero,
                )
                .is_ok()
                {
                    rt.diag_flags
                        .fetch_or(DIAG_ALTTAB_FOCUS_APPLIED, Ordering::Relaxed);
                }
            }
        } else {
            let saved_thick = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| s.alttab_item_orig_focus_thickness.remove(&key))
            });
            let _sync_guard = PropSyncGuard::enter();
            let cleared = allow_clear_dp
                && clear_dp_value(
                    insp_ptr,
                    BrushPropKind::FocusVisualSecondaryThickness
                        .dependency_property()
                        .as_ref(),
                );
            if !cleared {
                if let Some(orig) = saved_thick {
                    let _ = ((*(*fe4).lp_vtbl).put_focus_visual_secondary_thickness)(
                        fe4_com.as_ptr(),
                        orig,
                    );
                }
            }
        }
    }

    unsafe fn apply_focus_secondary_appearance(
        rt: &Arc<TapRuntime>,
        insp_ptr: *mut c_void,
        desired_hide: bool,
    ) {
        if insp_ptr.is_null() {
            return;
        }
        let key = com_identity_key(insp_ptr);
        let fe4_com = qi_interface(insp_ptr, &IID_IFRAMEWORKELEMENT4);
        if fe4_com.is_null() {
            return;
        }
        if desired_hide {
            track_alttab_subtarget_weak(key, TargetKind::AltTabItemFocus, insp_ptr);
            sync_focus_secondary_thickness_prop(rt, key, insp_ptr, true, true);
            sync_overflow_brush_prop(
                rt,
                key,
                insp_ptr,
                fe4_com.as_ptr(),
                BrushPropKind::FocusVisualSecondaryBrush,
                RawUiColor::TRANSPARENT,
                true,
                None,
                DIAG_ALTTAB_FOCUS_APPLIED,
            );
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::AltTabItemFocus,
                BrushPropKind::FocusVisualSecondaryBrush,
                insp_ptr,
            );
            ensure_prop_watch_registered(
                rt,
                key,
                TargetKind::AltTabItemFocus,
                BrushPropKind::FocusVisualSecondaryThickness,
                insp_ptr,
            );
        } else {
            let saved = take_saved_target_brushes(key);
            sync_focus_secondary_thickness_prop(rt, key, insp_ptr, false, true);
            sync_overflow_brush_prop(
                rt,
                key,
                insp_ptr,
                fe4_com.as_ptr(),
                BrushPropKind::FocusVisualSecondaryBrush,
                RawUiColor::TRANSPARENT,
                false,
                saved.focus_secondary,
                DIAG_ALTTAB_FOCUS_APPLIED,
            );
        }
    }

    #[inline]
    fn approx_eq_thickness(a: RawThickness, b: RawThickness) -> bool {
        (a.left - b.left).abs() <= 1e-3
            && (a.top - b.top).abs() <= 1e-3
            && (a.right - b.right).abs() <= 1e-3
            && (a.bottom - b.bottom).abs() <= 1e-3
    }

    #[inline]
    fn approx_eq_corner_radius(a: RawCornerRadius, b: RawCornerRadius) -> bool {
        (a.top_left - b.top_left).abs() <= 1e-3
            && (a.top_right - b.top_right).abs() <= 1e-3
            && (a.bottom_right - b.bottom_right).abs() <= 1e-3
            && (a.bottom_left - b.bottom_left).abs() <= 1e-3
    }

    unsafe fn set_element_framework_name(ptr: *mut c_void, name: &str) -> bool {
        let fe_com = qi_interface(ptr, &IID_IFRAMEWORKELEMENT);
        if fe_com.is_null() {
            return false;
        }
        let fe = fe_com.as_ptr().cast::<RawIFrameworkElement>();
        let hs = HSTRING::from(name);
        let abi_hs: core::mem::MaybeUninit<HSTRING> = core::mem::transmute_copy(&hs);
        let ok = ((*(*fe).lp_vtbl).put_name)(fe_com.as_ptr(), abi_hs).is_ok();
        drop(hs);
        ok
    }

    unsafe fn sync_overlay_border_geometry(src_insp: *mut c_void, dst_insp: *mut c_void) {
        let src_border_com = qi_interface(src_insp, &IID_IBORDER);
        let dst_border_com = qi_interface(dst_insp, &IID_IBORDER);
        if !src_border_com.is_null() && !dst_border_com.is_null() {
            let src_b = src_border_com.as_ptr().cast::<RawIBorder>();
            let dst_b = dst_border_com.as_ptr().cast::<RawIBorder>();

            let mut src_bt = RawThickness::default();
            let mut dst_bt = RawThickness::default();
            if ((*(*src_b).lp_vtbl).get_border_thickness)(src_border_com.as_ptr(), &mut src_bt)
                .is_ok()
                && (((*(*dst_b).lp_vtbl).get_border_thickness)(
                    dst_border_com.as_ptr(),
                    &mut dst_bt,
                )
                .is_err()
                    || !approx_eq_thickness(src_bt, dst_bt))
            {
                let _ = ((*(*dst_b).lp_vtbl).put_border_thickness)(dst_border_com.as_ptr(), src_bt);
            }

            let mut src_cr = RawCornerRadius::default();
            let mut dst_cr = RawCornerRadius::default();
            if ((*(*src_b).lp_vtbl).get_corner_radius)(src_border_com.as_ptr(), &mut src_cr).is_ok()
                && (((*(*dst_b).lp_vtbl).get_corner_radius)(dst_border_com.as_ptr(), &mut dst_cr)
                    .is_err()
                    || !approx_eq_corner_radius(src_cr, dst_cr))
            {
                let _ = ((*(*dst_b).lp_vtbl).put_corner_radius)(dst_border_com.as_ptr(), src_cr);
            }
        }

        let src_fe_com = qi_interface(src_insp, &IID_IFRAMEWORKELEMENT);
        let dst_fe_com = qi_interface(dst_insp, &IID_IFRAMEWORKELEMENT);
        if !src_fe_com.is_null() && !dst_fe_com.is_null() {
            let src_fe = src_fe_com.as_ptr().cast::<RawIFrameworkElement>();
            let dst_fe = dst_fe_com.as_ptr().cast::<RawIFrameworkElement>();
            let mut src_margin = RawThickness::default();
            let mut dst_margin = RawThickness::default();
            if ((*(*src_fe).lp_vtbl).get_margin)(src_fe_com.as_ptr(), &mut src_margin).is_ok()
                && (((*(*dst_fe).lp_vtbl).get_margin)(dst_fe_com.as_ptr(), &mut dst_margin)
                    .is_err()
                    || !approx_eq_thickness(src_margin, dst_margin))
            {
                let _ = ((*(*dst_fe).lp_vtbl).put_margin)(dst_fe_com.as_ptr(), src_margin);
            }
        }
    }

    unsafe fn sync_overlay_border_brushes(
        dst_border_ptr: *mut c_void,
        bg_color: RawUiColor,
        border_color: RawUiColor,
    ) {
        let dst_b = dst_border_ptr.cast::<RawIBorder>();

        let mut cur_bg = ComRaw::null();
        if ((*(*dst_b).lp_vtbl).get_background)(dst_border_ptr, &mut cur_bg.0).is_err()
            || !is_solid_brush_with_color(cur_bg.as_ptr(), bg_color)
        {
            if let Some(tb) = cached_solid_brush(bg_color) {
                let _ = ((*(*dst_b).lp_vtbl).put_background)(dst_border_ptr, tb.as_ptr());
            }
        }

        let mut cur_bb = ComRaw::null();
        if ((*(*dst_b).lp_vtbl).get_border_brush)(dst_border_ptr, &mut cur_bb.0).is_err()
            || !is_solid_brush_with_color(cur_bb.as_ptr(), border_color)
        {
            if let Some(tb) = cached_solid_brush(border_color) {
                let _ = ((*(*dst_b).lp_vtbl).put_border_brush)(dst_border_ptr, tb.as_ptr());
            }
        }
    }

    unsafe fn activate_alttab_overlay_border() -> Option<ComRaw> {
        let class_id = HSTRING::from("Windows.UI.Xaml.Controls.Border");
        let insp: IInspectable = RoActivateInstance(&class_id).ok()?;
        let raw = insp.into_raw();
        (!raw.is_null()).then_some(ComRaw(raw))
    }

    enum ParentPanelLookup {
        Panel(ComRaw),
        Detached,
        NonPanel,
    }

    unsafe fn resolve_parent_panel(
        bg_border_insp: *mut c_void,
        parent_hint: Option<*mut c_void>,
    ) -> ParentPanelLookup {
        if let Some(p) = parent_hint.filter(|p| !p.is_null()) {
            let panel_com = qi_interface(p, &IID_IPANEL);
            return if panel_com.is_null() {
                ParentPanelLookup::NonPanel
            } else {
                ParentPanelLookup::Panel(panel_com)
            };
        }
        let Some(vth) = cached_visual_tree_helper() else {
            return ParentPanelLookup::Detached;
        };
        let dep = qi_interface(bg_border_insp, &IID_IDEPENDENCYOBJECT);
        if dep.is_null() {
            return ParentPanelLookup::Detached;
        }
        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
        let mut parent_dep = ComRaw::null();
        if ((*(*vth_iface).lp_vtbl).get_parent)(vth.as_ptr(), dep.as_ptr(), &mut parent_dep.0)
            .is_err()
            || parent_dep.is_null()
        {
            return ParentPanelLookup::Detached;
        }
        let panel_com = qi_interface(parent_dep.as_ptr(), &IID_IPANEL);
        if panel_com.is_null() {
            ParentPanelLookup::NonPanel
        } else {
            ParentPanelLookup::Panel(panel_com)
        }
    }

    unsafe fn find_overlay_in_panel_children(children_ptr: *mut c_void) -> Option<ComRaw> {
        let vec = children_ptr.cast::<RawIUIElementVector>();
        let mut count = 0u32;
        if ((*(*vec).lp_vtbl).get_size)(children_ptr, &mut count).is_err() {
            return None;
        }
        for i in 0..count.min(64) {
            let mut child = ComRaw::null();
            if ((*(*vec).lp_vtbl).get_at)(children_ptr, i, &mut child.0).is_ok()
                && !child.is_null()
                && element_framework_name(child.as_ptr()).as_deref()
                    == Some(ALTTAB_ITEM_OVERLAY_NAME)
            {
                return Some(child);
            }
        }
        None
    }

    unsafe fn remove_all_overlays_in_panel_children(children_ptr: *mut c_void) {
        let vec = children_ptr.cast::<RawIUIElementVector>();
        let mut count = 0u32;
        if ((*(*vec).lp_vtbl).get_size)(children_ptr, &mut count).is_err() {
            return;
        }
        for i in (0..count.min(64)).rev() {
            let mut child = ComRaw::null();
            if ((*(*vec).lp_vtbl).get_at)(children_ptr, i, &mut child.0).is_ok()
                && !child.is_null()
                && element_framework_name(child.as_ptr()).as_deref()
                    == Some(ALTTAB_ITEM_OVERLAY_NAME)
            {
                let _ = ((*(*vec).lp_vtbl).remove_at)(children_ptr, i);
            }
        }
    }

    unsafe fn record_and_hide_alttab_item_visual(key: u64, bg_border_insp: *mut c_void) -> bool {
        let Some(ecp) = cached_element_composition_preview() else {
            return false;
        };
        let ui_elem = qi_interface(bg_border_insp, &IID_IUIELEMENT);
        if ui_elem.is_null() {
            return false;
        }
        let ecp_iface = ecp.as_ptr().cast::<RawIElementCompositionPreviewStatics>();
        let mut vis_com = ComRaw::null();
        if ((*(*ecp_iface).lp_vtbl).get_element_visual)(
            ecp.as_ptr(),
            ui_elem.as_ptr(),
            &mut vis_com.0,
        )
        .is_err()
            || vis_com.is_null()
        {
            return false;
        }
        let vis = vis_com.as_ptr().cast::<RawIVisual>();
        let mut cur_vis = 1u8;
        let hr_get = ((*(*vis).lp_vtbl).get_is_visible)(vis_com.as_ptr(), &mut cur_vis);

        UI_BRUSH_STORE.with(|cell| {
            if let Ok(mut s) = cell.try_borrow_mut() {
                if !s.alttab_item_orig_comp_visible.contains_key(&key) {
                    let orig_vis = if hr_get.is_ok() && cur_vis != 0 {
                        cur_vis
                    } else {
                        1u8
                    };
                    s.alttab_item_orig_comp_visible.insert(key, orig_vis);
                }
            }
        });

        if hr_get.is_err() || cur_vis != 0 {
            ((*(*vis).lp_vtbl).put_is_visible)(vis_com.as_ptr(), 0u8).is_ok()
        } else {
            true
        }
    }

    unsafe fn restore_alttab_item_visual(key: u64, bg_border_insp: *mut c_void) {
        let saved_vis = UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow_mut()
                .ok()
                .and_then(|mut s| s.alttab_item_orig_comp_visible.remove(&key))
        });
        let Some(ecp) = cached_element_composition_preview() else {
            return;
        };
        let ui_elem = qi_interface(bg_border_insp, &IID_IUIELEMENT);
        if ui_elem.is_null() {
            return;
        }
        let ecp_iface = ecp.as_ptr().cast::<RawIElementCompositionPreviewStatics>();
        let mut vis_com = ComRaw::null();
        if ((*(*ecp_iface).lp_vtbl).get_element_visual)(
            ecp.as_ptr(),
            ui_elem.as_ptr(),
            &mut vis_com.0,
        )
        .is_err()
            || vis_com.is_null()
        {
            return;
        }
        let vis = vis_com.as_ptr().cast::<RawIVisual>();
        let mut cur_vis = 1u8;
        let hr_get = ((*(*vis).lp_vtbl).get_is_visible)(vis_com.as_ptr(), &mut cur_vis);
        if saved_vis.is_some() || (hr_get.is_ok() && cur_vis == 0) {
            let target_vis = saved_vis.unwrap_or(1u8);
            if hr_get.is_err() || cur_vis != target_vis {
                let _ = ((*(*vis).lp_vtbl).put_is_visible)(vis_com.as_ptr(), target_vis);
            }
        }
    }

    fn has_saved_fallback_brushes(key: u64) -> bool {
        UI_BRUSH_STORE.with(|cell| {
            cell.try_borrow().ok().is_some_and(|s| {
                s.saved.contains_key(&key)
                    || s.watches.contains_key(&(key, 0))
                    || s.watches.contains_key(&(key, 1))
            })
        })
    }

    unsafe fn apply_alttab_item_border_appearance(
        rt: &Arc<TapRuntime>,
        bg_border_insp: *mut c_void,
        parent_hint: Option<*mut c_void>,
        bg_color: RawUiColor,
        border_color: RawUiColor,
        desired_hide: bool,
    ) {
        if bg_border_insp.is_null() {
            return;
        }
        let key = com_identity_key(bg_border_insp);
        if desired_hide {
            track_alttab_subtarget_weak(key, TargetKind::AltTabItemBorder, bg_border_insp);
            match resolve_parent_panel(bg_border_insp, parent_hint) {
                ParentPanelLookup::Detached => return,
                ParentPanelLookup::Panel(panel_com) => {
                    if let Some(vth) = cached_visual_tree_helper() {
                        let root_dep = qi_interface(panel_com.as_ptr(), &IID_IDEPENDENCYOBJECT);
                        if !root_dep.is_null() {
                            let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
                            let mut lv_dep = ComRaw::null();
                            if ((*(*vth_iface).lp_vtbl).get_parent)(
                                vth.as_ptr(),
                                root_dep.as_ptr(),
                                &mut lv_dep.0,
                            )
                            .is_ok()
                                && !lv_dep.is_null()
                            {
                                apply_focus_secondary_appearance(rt, lv_dep.as_ptr(), true);
                            }
                        }
                    }
                    let panel = panel_com.as_ptr().cast::<RawIPanel>();
                    let mut children_com = ComRaw::null();
                    if ((*(*panel).lp_vtbl).get_children)(panel_com.as_ptr(), &mut children_com.0)
                        .is_ok()
                        && !children_com.is_null()
                    {
                        let overlay_ready = if let Some(existing) =
                            find_overlay_in_panel_children(children_com.as_ptr())
                        {
                            sync_overlay_border_geometry(bg_border_insp, existing.as_ptr());
                            let ov_border = qi_interface(existing.as_ptr(), &IID_IBORDER);
                            if !ov_border.is_null() {
                                sync_overlay_border_brushes(
                                    ov_border.as_ptr(),
                                    bg_color,
                                    border_color,
                                );
                            }
                            true
                        } else if let Some(overlay) = activate_alttab_overlay_border() {
                            let _ = set_element_framework_name(
                                overlay.as_ptr(),
                                ALTTAB_ITEM_OVERLAY_NAME,
                            );
                            let ov_ui = qi_interface(overlay.as_ptr(), &IID_IUIELEMENT);
                            if !ov_ui.is_null() {
                                let ui_iface = ov_ui.as_ptr().cast::<RawIUIElement>();
                                let _ = ((*(*ui_iface).lp_vtbl).put_is_hit_test_visible)(
                                    ov_ui.as_ptr(),
                                    0u8,
                                );
                            }
                            sync_overlay_border_geometry(bg_border_insp, overlay.as_ptr());
                            let ov_border = qi_interface(overlay.as_ptr(), &IID_IBORDER);
                            if !ov_border.is_null() {
                                sync_overlay_border_brushes(
                                    ov_border.as_ptr(),
                                    bg_color,
                                    border_color,
                                );
                            }
                            if !ov_ui.is_null() {
                                let vec = children_com.as_ptr().cast::<RawIUIElementVector>();
                                ((*(*vec).lp_vtbl).insert_at)(
                                    children_com.as_ptr(),
                                    0,
                                    ov_ui.as_ptr(),
                                )
                                .is_ok()
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if overlay_ready {
                            if record_and_hide_alttab_item_visual(key, bg_border_insp) {
                                rt.diag_flags
                                    .fetch_or(DIAG_ALTTAB_ITEM_APPLIED, Ordering::Relaxed);
                                return;
                            }
                            remove_all_overlays_in_panel_children(children_com.as_ptr());
                        } else {
                            return;
                        }
                    }
                }
                ParentPanelLookup::NonPanel => {}
            }

            apply_border_target_appearance(
                rt,
                key,
                bg_border_insp,
                TargetKind::AltTabItemBorder,
                bg_color,
                border_color,
                true,
                DIAG_ALTTAB_ITEM_APPLIED,
            );
        } else {
            if let ParentPanelLookup::Panel(panel_com) =
                resolve_parent_panel(bg_border_insp, parent_hint)
            {
                let panel = panel_com.as_ptr().cast::<RawIPanel>();
                let mut children_com = ComRaw::null();
                if ((*(*panel).lp_vtbl).get_children)(panel_com.as_ptr(), &mut children_com.0)
                    .is_ok()
                    && !children_com.is_null()
                {
                    remove_all_overlays_in_panel_children(children_com.as_ptr());
                }
            }
            restore_alttab_item_visual(key, bg_border_insp);
            if has_saved_fallback_brushes(key) {
                apply_border_target_appearance(
                    rt,
                    key,
                    bg_border_insp,
                    TargetKind::AltTabItemBorder,
                    bg_color,
                    border_color,
                    false,
                    DIAG_ALTTAB_ITEM_APPLIED,
                );
            }
        }
    }

    unsafe fn sync_alttab_subtree_items(
        rt: &Arc<TapRuntime>,
        bg_element_ptr: *mut c_void,
        desired_hide: bool,
    ) {
        if bg_element_ptr.is_null() {
            return;
        }
        let Some(vth) = cached_visual_tree_helper() else {
            return;
        };
        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
        let root_insp = qi_interface(bg_element_ptr, &IID_IINSPECTABLE);
        if root_insp.is_null() {
            return;
        }

        let bg_color = RawUiColor::from_argb(rt.alt_tab_item_bg_argb.load(Ordering::Acquire));
        let border_color =
            RawUiColor::from_argb(rt.alt_tab_item_border_argb.load(Ordering::Acquire));

        let mut queue: VecDeque<(ComRaw, Option<ComRaw>, Option<ComRaw>)> = VecDeque::new();
        queue.push_back((root_insp, None, None));
        let mut visited = 0usize;

        while let Some((node_com, parent_com, gp_com)) = queue.pop_front() {
            if visited >= 600 {
                break;
            }
            visited += 1;

            let cls_name = element_runtime_class_name(node_com.as_ptr());
            let elem_name = element_framework_name(node_com.as_ptr());

            if elem_name.as_deref() == Some(ALTTAB_ITEM_OVERLAY_NAME) {
                continue;
            }

            if cls_name.as_deref().is_some_and(|c| c.contains("ListViewItem")) {
                apply_focus_secondary_appearance(rt, node_com.as_ptr(), desired_hide);
            }

            if elem_name.as_deref() == Some("BackgroundBorder")
                && !qi_interface(node_com.as_ptr(), &IID_IBORDER).is_null()
            {
                apply_alttab_item_border_appearance(
                    rt,
                    node_com.as_ptr(),
                    parent_com.as_ref().map(ComRaw::as_ptr),
                    bg_color,
                    border_color,
                    desired_hide,
                );
                if let Some(ref gp) = gp_com {
                    let gp_key = com_identity_key(gp.as_ptr());
                    if !is_alttab_subtarget_tracked(gp_key) {
                        apply_focus_secondary_appearance(rt, gp.as_ptr(), desired_hide);
                    }
                }
                continue;
            }

            let skip_children = elem_name
                .as_deref()
                .is_some_and(|n| matches!(n, "ThumbnailHost" | "CloseButton"))
                || cls_name.as_deref().is_some_and(|c| {
                    c.contains("SwitchItemControl")
                        || c.contains("SwitchItemElement")
                        || c.contains("Button")
                        || c.contains("ScrollBar")
                });
            if skip_children {
                continue;
            }

            let dep = qi_interface(node_com.as_ptr(), &IID_IDEPENDENCYOBJECT);
            if dep.is_null() {
                continue;
            }
            let mut count = 0i32;
            if ((*(*vth_iface).lp_vtbl).get_children_count)(vth.as_ptr(), dep.as_ptr(), &mut count)
                .is_err()
                || count <= 0
            {
                continue;
            }
            for i in 0..count.min(128) {
                let mut child_dep = ComRaw::null();
                if ((*(*vth_iface).lp_vtbl).get_child)(
                    vth.as_ptr(),
                    dep.as_ptr(),
                    i,
                    &mut child_dep.0,
                )
                .is_ok()
                    && !child_dep.is_null()
                {
                    queue.push_back((
                        child_dep,
                        Some(node_com.clone_ref()),
                        parent_com.as_ref().map(ComRaw::clone_ref),
                    ));
                }
            }
        }
    }

    unsafe fn restore_all_alttab_subtargets_for_current_thread(rt: &Arc<TapRuntime>) {
        let entries: Vec<(u64, TargetKind, ComRaw)> = UI_BRUSH_STORE.with(|cell| {
            let Ok(mut s) = cell.try_borrow_mut() else {
                return Vec::new();
            };
            s.alttab_subtargets
                .drain()
                .map(|(k, (kind, weak_obj))| (k, kind, weak_obj))
                .collect()
        });
        if entries.is_empty() {
            return;
        }
        let bg_color = RawUiColor::from_argb(rt.alt_tab_item_bg_argb.load(Ordering::Acquire));
        let border_color =
            RawUiColor::from_argb(rt.alt_tab_item_border_argb.load(Ordering::Acquire));
        let mut keys = Vec::with_capacity(entries.len());
        for (key, kind, weak_com) in entries {
            keys.push(key);
            let wr = weak_com.as_ptr().cast::<RawIWeakReference>();
            let mut insp = ComRaw::null();
            if ((*(*wr).lp_vtbl).resolve)(weak_com.as_ptr(), &IID_IINSPECTABLE, &mut insp.0)
                .is_ok()
                && !insp.is_null()
            {
                match kind {
                    TargetKind::AltTabItemBorder => {
                        apply_alttab_item_border_appearance(
                            rt,
                            insp.as_ptr(),
                            None,
                            bg_color,
                            border_color,
                            false,
                        );
                    }
                    TargetKind::AltTabItemFocus => {
                        apply_focus_secondary_appearance(rt, insp.as_ptr(), false);
                    }
                    _ => {}
                }
            }
        }
        purge_saved_brushes_for_current_thread(&keys);
    }

    unsafe fn sync_hover_flyout_bg_shapes(
        rt: &Arc<TapRuntime>,
        bg_border_ptr: *mut c_void,
        desired_hide: bool,
    ) {
        if bg_border_ptr.is_null() {
            return;
        }
        let Some(vth) = cached_visual_tree_helper() else {
            return;
        };
        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
        let root_insp = qi_interface(bg_border_ptr, &IID_IINSPECTABLE);
        if root_insp.is_null() {
            return;
        }

        let mut queue: VecDeque<ComRaw> = VecDeque::new();
        queue.push_back(root_insp);
        let mut visited = 0usize;

        while let Some(node_com) = queue.pop_front() {
            if visited >= 32 {
                break;
            }
            visited += 1;

            let elem_name = element_framework_name(node_com.as_ptr());
            if elem_name
                .as_deref()
                .is_some_and(|n| matches!(n, "BackgroundFill" | "BackgroundStroke" | "ScreenEdgeStroke"))
            {
                let key = com_identity_key(node_com.as_ptr());
                let cached_orig = UI_BRUSH_STORE.with(|cell| {
                    cell.try_borrow()
                        .ok()
                        .and_then(|s| s.hover_flyout_shapes.get(&key).map(|(_, op, vis)| (*op, *vis)))
                });
                let (orig_opacity, orig_visibility) = if let Some(pair) = cached_orig {
                    pair
                } else {
                    let mut orig_op = 1.0f64;
                    let mut orig_vis = VISIBILITY_VISIBLE;
                    let ui_elem = qi_interface(node_com.as_ptr(), &IID_IUIELEMENT);
                    if !ui_elem.is_null() {
                        let ui = ui_elem.as_ptr().cast::<RawIUIElement>();
                        if ((*(*ui).lp_vtbl).get_opacity)(ui_elem.as_ptr(), &mut orig_op).is_err()
                            || !orig_op.is_finite()
                            || orig_op <= 0.01
                        {
                            orig_op = 1.0;
                        }
                        if ((*(*ui).lp_vtbl).get_visibility)(ui_elem.as_ptr(), &mut orig_vis).is_err() {
                            orig_vis = VISIBILITY_VISIBLE;
                        }
                    }
                    if desired_hide {
                        let weak_src = qi_interface(node_com.as_ptr(), &IID_IWEAKREFERENCESOURCE);
                        if !weak_src.is_null() {
                            let src_iface = weak_src.as_ptr().cast::<RawIWeakReferenceSource>();
                            let mut weak_ref = ComRaw::null();
                            if ((*(*src_iface).lp_vtbl).get_weak_reference)(
                                weak_src.as_ptr(),
                                &mut weak_ref.0,
                            )
                            .is_ok()
                                && !weak_ref.is_null()
                            {
                                let displaced = UI_BRUSH_STORE.with(|cell| {
                                    if let Ok(mut s) = cell.try_borrow_mut() {
                                        if !s.hover_flyout_shapes.contains_key(&key) {
                                            if s.hover_flyout_shapes.len() >= 64 {
                                                let dead: Vec<u64> = s
                                                    .hover_flyout_shapes
                                                    .iter()
                                                    .filter_map(|(&k, (w, _, _))| {
                                                        let wr = w.as_ptr().cast::<RawIWeakReference>();
                                                        let mut resolved = ComRaw::null();
                                                        let alive = ((*(*wr).lp_vtbl).resolve)(
                                                            w.as_ptr(),
                                                            &IID_IINSPECTABLE,
                                                            &mut resolved.0,
                                                        )
                                                        .is_ok()
                                                            && !resolved.is_null();
                                                        (!alive).then_some(k)
                                                    })
                                                    .collect();
                                                for dk in dead {
                                                    s.hover_flyout_shapes.remove(&dk);
                                                }
                                            }
                                            s.hover_flyout_shapes
                                                .insert(key, (weak_ref, orig_op, orig_vis));
                                            return None;
                                        }
                                    }
                                    Some(weak_ref)
                                });
                                drop(displaced);
                            }
                        }
                    }
                    (orig_op, orig_vis)
                };
                apply_shape_appearance(rt, node_com.as_ptr(), desired_hide, orig_opacity, orig_visibility);
                if desired_hide {
                    rt.diag_flags
                        .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
                }
                continue;
            }

            if elem_name
                .as_deref()
                .is_some_and(|n| n.contains("HoverFlyoutContent") || n.contains("Thumbnail"))
            {
                continue;
            }

            let dep = qi_interface(node_com.as_ptr(), &IID_IDEPENDENCYOBJECT);
            if dep.is_null() {
                continue;
            }
            let mut count = 0i32;
            if ((*(*vth_iface).lp_vtbl).get_children_count)(vth.as_ptr(), dep.as_ptr(), &mut count)
                .is_err()
                || count <= 0
            {
                continue;
            }
            for i in 0..count.min(32) {
                let mut child_dep = ComRaw::null();
                if ((*(*vth_iface).lp_vtbl).get_child)(
                    vth.as_ptr(),
                    dep.as_ptr(),
                    i,
                    &mut child_dep.0,
                )
                .is_ok()
                    && !child_dep.is_null()
                {
                    queue.push_back(child_dep);
                }
            }
        }
    }

    unsafe fn restore_all_hover_flyout_shapes_for_current_thread(rt: &Arc<TapRuntime>) {
        let entries: Vec<(ComRaw, f64, i32)> = UI_BRUSH_STORE.with(|cell| {
            let Ok(mut s) = cell.try_borrow_mut() else {
                return Vec::new();
            };
            s.hover_flyout_shapes
                .drain()
                .map(|(_, entry)| entry)
                .collect()
        });
        for (weak_com, orig_opacity, orig_visibility) in entries {
            let wr = weak_com.as_ptr().cast::<RawIWeakReference>();
            let mut insp = ComRaw::null();
            if ((*(*wr).lp_vtbl).resolve)(weak_com.as_ptr(), &IID_IINSPECTABLE, &mut insp.0)
                .is_ok()
                && !insp.is_null()
            {
                apply_shape_appearance(rt, insp.as_ptr(), false, orig_opacity, orig_visibility);
            }
        }
    }

    unsafe fn apply_hover_flyout_border_appearance(
        rt: &Arc<TapRuntime>,
        handle: u64,
        insp_ptr: *mut c_void,
        desired_hide: bool,
    ) {
        let border_com = qi_interface(insp_ptr, &IID_IBORDER);
        let saved = if desired_hide {
            SavedTargetBrushes::default()
        } else {
            take_saved_target_brushes(handle)
        };
        if !border_com.is_null() {
            sync_overflow_brush_prop(
                rt,
                handle,
                insp_ptr,
                border_com.as_ptr(),
                BrushPropKind::BorderBackground,
                RawUiColor::TRANSPARENT,
                desired_hide,
                saved.background,
                DIAG_HOVER_FLYOUT_APPLIED,
            );
            sync_overflow_brush_prop(
                rt,
                handle,
                insp_ptr,
                border_com.as_ptr(),
                BrushPropKind::BorderBrush,
                RawUiColor::TRANSPARENT,
                desired_hide,
                saved.border_brush,
                DIAG_HOVER_FLYOUT_APPLIED,
            );
        }
        sync_element_shadow_prop(
            rt,
            handle,
            insp_ptr,
            desired_hide,
            saved.shadow,
            DIAG_HOVER_FLYOUT_SHADOW_APPLIED,
            false,
        );
        if desired_hide {
            if !border_com.is_null() {
                rt.diag_flags
                    .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
                ensure_prop_watch_registered(
                    rt,
                    handle,
                    TargetKind::HoverFlyoutBorder,
                    BrushPropKind::BorderBackground,
                    insp_ptr,
                );
                ensure_prop_watch_registered(
                    rt,
                    handle,
                    TargetKind::HoverFlyoutBorder,
                    BrushPropKind::BorderBrush,
                    insp_ptr,
                );
            }
            ensure_prop_watch_registered(
                rt,
                handle,
                TargetKind::HoverFlyoutBorder,
                BrushPropKind::ElementShadow,
                insp_ptr,
            );
        }
    }

    unsafe fn apply_osd_panel_appearance(
        rt: &Arc<TapRuntime>,
        handle: u64,
        insp_ptr: *mut c_void,
        desired_hide: bool,
    ) {
        let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
        let grid2_com = qi_interface(insp_ptr, &IID_IGRID2);
        let saved = if desired_hide {
            SavedTargetBrushes::default()
        } else {
            take_saved_target_brushes(handle)
        };
        let (target_bg, target_border) = rt.osd_target_colors();
        if !panel_com.is_null() {
            sync_overflow_brush_prop(
                rt,
                handle,
                insp_ptr,
                panel_com.as_ptr(),
                BrushPropKind::PanelBackground,
                target_bg,
                desired_hide,
                saved.background,
                DIAG_OSD_APPLIED,
            );
        }
        if !grid2_com.is_null() {
            sync_overflow_brush_prop(
                rt,
                handle,
                insp_ptr,
                grid2_com.as_ptr(),
                BrushPropKind::GridBorderBrush,
                target_border,
                desired_hide,
                saved.border_brush,
                DIAG_OSD_APPLIED,
            );
        }
        sync_element_shadow_prop(
            rt,
            handle,
            insp_ptr,
            desired_hide,
            saved.shadow,
            DIAG_OSD_SHADOW_APPLIED,
            false,
        );
        if desired_hide {
            if !panel_com.is_null() {
                ensure_prop_watch_registered(
                    rt,
                    handle,
                    TargetKind::OsdPanel,
                    BrushPropKind::PanelBackground,
                    insp_ptr,
                );
            }
            if !grid2_com.is_null() {
                ensure_prop_watch_registered(
                    rt,
                    handle,
                    TargetKind::OsdPanel,
                    BrushPropKind::GridBorderBrush,
                    insp_ptr,
                );
            }
            ensure_prop_watch_registered(
                rt,
                handle,
                TargetKind::OsdPanel,
                BrushPropKind::ElementShadow,
                insp_ptr,
            );
        }
    }

    unsafe fn find_parent_tooltip_control(insp_ptr: *mut c_void) -> Option<(u64, ComRaw)> {
        let vth = cached_visual_tree_helper()?;
        let dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        if dep.is_null() {
            return None;
        }
        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
        let mut parent_dep = ComRaw::null();
        if ((*(*vth_iface).lp_vtbl).get_parent)(vth.as_ptr(), dep.as_ptr(), &mut parent_dep.0)
            .is_err()
            || parent_dep.is_null()
        {
            return None;
        }
        let tt_com = qi_interface(parent_dep.as_ptr(), &IID_ITOOLTIP);
        let ctrl_com = qi_interface(parent_dep.as_ptr(), &IID_ICONTROL);
        if tt_com.is_null() || ctrl_com.is_null() {
            return None;
        }
        let parent_insp = qi_interface(parent_dep.as_ptr(), &IID_IINSPECTABLE);
        if parent_insp.is_null() {
            return None;
        }
        let parent_key = com_identity_key(parent_insp.as_ptr());
        Some((parent_key, parent_insp))
    }

    unsafe fn find_parent_menu_flyout_control(insp_ptr: *mut c_void) -> Option<(u64, ComRaw)> {
        let vth = cached_visual_tree_helper()?;
        let mut cur_dep = qi_interface(insp_ptr, &IID_IDEPENDENCYOBJECT);
        if cur_dep.is_null() {
            return None;
        }
        let vth_iface = vth.as_ptr().cast::<RawIVisualTreeHelperStatics>();
        for _ in 0..4 {
            let mut parent_dep = ComRaw::null();
            if ((*(*vth_iface).lp_vtbl).get_parent)(
                vth.as_ptr(),
                cur_dep.as_ptr(),
                &mut parent_dep.0,
            )
            .is_err()
                || parent_dep.is_null()
            {
                break;
            }
            let ctrl_com = qi_interface(parent_dep.as_ptr(), &IID_ICONTROL);
            if !ctrl_com.is_null() {
                let parent_insp = qi_interface(parent_dep.as_ptr(), &IID_IINSPECTABLE);
                if !parent_insp.is_null() {
                    if let Some(cls) = element_runtime_class_name(parent_insp.as_ptr()) {
                        if cls.contains("MenuFlyoutPresenter")
                            || ((cls.contains("JumpList") || cls.contains("JumpView"))
                                && !cls.contains("Item"))
                        {
                            let parent_key = com_identity_key(parent_insp.as_ptr());
                            return Some((parent_key, parent_insp));
                        }
                    }
                }
            }
            cur_dep = parent_dep;
        }
        None
    }

    unsafe fn sync_control_corner_radius_in_map(
        ctrl_key: u64,
        ctrl_ptr: *mut c_void,
        apply_round: bool,
        get_map: fn(&mut UiThreadBrushStore) -> &mut HashMap<u64, SavedCornerRadius>,
        read_map: fn(&UiThreadBrushStore) -> &HashMap<u64, SavedCornerRadius>,
    ) {
        if ctrl_ptr.is_null() {
            return;
        }
        let ctrl7_com = qi_interface(ctrl_ptr, &IID_ICONTROL7);
        if ctrl7_com.is_null() {
            return;
        }
        let ctrl7 = ctrl7_com.as_ptr().cast::<RawIControl7>();
        if apply_round {
            let already_saved = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .is_some_and(|s| read_map(&s).contains_key(&ctrl_key))
            });
            if !already_saved {
                let mut saved_cr: Option<SavedCornerRadius> = None;
                let dp = ensure_cached_com(
                    |s| s.control_corner_radius_dp.as_ref(),
                    |s, v| s.control_corner_radius_dp = Some(v),
                    fetch_control_corner_radius_dp,
                );
                let unset = ensure_cached_com(
                    |s| s.unset_value.as_ref(),
                    |s, v| s.unset_value = Some(v),
                    fetch_unset_value,
                );
                if let (Some(dp_com), Some(unset_com)) = (dp, unset) {
                    let dep_com = qi_interface(ctrl_ptr, &IID_IDEPENDENCYOBJECT);
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
                            && com_same_identity(local_val.as_ptr(), unset_com.as_ptr())
                        {
                            saved_cr = Some(SavedCornerRadius::Unset);
                        }
                    }
                }
                if saved_cr.is_none() {
                    let mut cur_cr = RawCornerRadius::default();
                    if ((*(*ctrl7).lp_vtbl).get_corner_radius)(ctrl7_com.as_ptr(), &mut cur_cr)
                        .is_ok()
                    {
                        saved_cr = Some(SavedCornerRadius::Local(cur_cr));
                    }
                }
                if let Some(orig) = saved_cr {
                    let _ = UI_BRUSH_STORE.with(|cell| {
                        if let Ok(mut s) = cell.try_borrow_mut() {
                            get_map(&mut s).entry(ctrl_key).or_insert(orig);
                        }
                    });
                }
            }
            let target_cr = RawCornerRadius {
                top_left: 8.0,
                top_right: 8.0,
                bottom_right: 8.0,
                bottom_left: 8.0,
            };
            let mut cur_cr = RawCornerRadius::default();
            if ((*(*ctrl7).lp_vtbl).get_corner_radius)(ctrl7_com.as_ptr(), &mut cur_cr).is_err()
                || !approx_eq_corner_radius(cur_cr, target_cr)
            {
                let _ = ((*(*ctrl7).lp_vtbl).put_corner_radius)(ctrl7_com.as_ptr(), target_cr);
            }
        } else {
            let orig = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| get_map(&mut s).remove(&ctrl_key))
            });
            match orig {
                Some(SavedCornerRadius::Unset) => {
                    let dp = ensure_cached_com(
                        |s| s.control_corner_radius_dp.as_ref(),
                        |s, v| s.control_corner_radius_dp = Some(v),
                        fetch_control_corner_radius_dp,
                    );
                    let _ = clear_dp_value(ctrl_ptr, dp.as_ref());
                }
                Some(SavedCornerRadius::Local(orig_cr)) => {
                    let _ = ((*(*ctrl7).lp_vtbl).put_corner_radius)(ctrl7_com.as_ptr(), orig_cr);
                }
                None => {}
            }
        }
    }

    unsafe fn sync_tooltip_control_corner_radius(
        tooltip_key: u64,
        tt_ptr: *mut c_void,
        apply_round: bool,
    ) {
        sync_control_corner_radius_in_map(
            tooltip_key,
            tt_ptr,
            apply_round,
            |s| &mut s.tooltip_orig_corner_radius,
            |s| &s.tooltip_orig_corner_radius,
        );
    }

    unsafe fn sync_menu_flyout_control_corner_radius(
        ctrl_key: u64,
        ctrl_ptr: *mut c_void,
        apply_round: bool,
    ) {
        sync_control_corner_radius_in_map(
            ctrl_key,
            ctrl_ptr,
            apply_round,
            |s| &mut s.menu_flyout_orig_corner_radius,
            |s| &s.menu_flyout_orig_corner_radius,
        );
    }

    unsafe fn sync_tooltip_presenter_corner_radius_fallback(
        presenter_key: u64,
        cp_ptr: *mut c_void,
        apply_round: bool,
    ) {
        if cp_ptr.is_null() {
            return;
        }
        let cp = cp_ptr.cast::<RawIContentPresenter4>();
        if apply_round {
            let target_cr = RawCornerRadius {
                top_left: 8.0,
                top_right: 8.0,
                bottom_right: 8.0,
                bottom_left: 8.0,
            };
            let mut cur_cr = RawCornerRadius::default();
            if ((*(*cp).lp_vtbl).get_corner_radius)(cp_ptr, &mut cur_cr).is_ok()
                && approx_eq_corner_radius(cur_cr, target_cr)
            {
                return;
            }
            let already_saved = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .is_some_and(|s| s.tooltip_orig_corner_radius.contains_key(&presenter_key))
            });
            if !already_saved
                && ((*(*cp).lp_vtbl).get_corner_radius)(cp_ptr, &mut cur_cr).is_ok()
            {
                let _ = UI_BRUSH_STORE.with(|cell| {
                    if let Ok(mut s) = cell.try_borrow_mut() {
                        s.tooltip_orig_corner_radius
                            .entry(presenter_key)
                            .or_insert(SavedCornerRadius::Local(cur_cr));
                    }
                });
            }
            let _ = ((*(*cp).lp_vtbl).put_corner_radius)(cp_ptr, target_cr);
        } else {
            let orig = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| s.tooltip_orig_corner_radius.remove(&presenter_key))
            });
            if let Some(SavedCornerRadius::Local(orig_cr)) = orig {
                let _ = ((*(*cp).lp_vtbl).put_corner_radius)(cp_ptr, orig_cr);
            }
        }
    }

    unsafe fn restore_tooltip_pair(
        rt: &Arc<TapRuntime>,
        presenter_key: u64,
        presenter_insp: Option<*mut c_void>,
        tooltip_key_opt: Option<u64>,
        tooltip_insp: Option<*mut c_void>,
    ) {
        let tk_eff = tooltip_insp
            .map(|p| com_identity_key(p))
            .or(tooltip_key_opt);

        let mut keys_to_clean = Vec::with_capacity(2);
        if presenter_key != 0 {
            keys_to_clean.push(presenter_key);
        }
        if let Some(tk) = tk_eff.filter(|&k| k != 0 && !keys_to_clean.contains(&k)) {
            keys_to_clean.push(tk);
        }
        unregister_watches_for_keys(&keys_to_clean);

        let (target_bg, target_border) = rt.tooltip_target_colors();

        if let (Some(tk), Some(tt_ptr)) = (tk_eff, tooltip_insp) {
            let tt_saved = take_saved_target_brushes(tk);
            let ctrl_com = qi_interface(tt_ptr, &IID_ICONTROL);
            if !ctrl_com.is_null() {
                sync_overflow_brush_prop(
                    rt,
                    tk,
                    tt_ptr,
                    ctrl_com.as_ptr(),
                    BrushPropKind::ControlBackground,
                    target_bg,
                    false,
                    tt_saved.background,
                    DIAG_TOOLTIP_APPLIED,
                );
                sync_overflow_brush_prop(
                    rt,
                    tk,
                    tt_ptr,
                    ctrl_com.as_ptr(),
                    BrushPropKind::ControlBorderBrush,
                    target_border,
                    false,
                    tt_saved.border_brush,
                    DIAG_TOOLTIP_APPLIED,
                );
            }
            sync_tooltip_control_corner_radius(tk, tt_ptr, false);
            if tt_saved.shadow.is_some() {
                sync_element_shadow_prop(
                    rt,
                    tk,
                    tt_ptr,
                    false,
                    tt_saved.shadow,
                    DIAG_TOOLTIP_SHADOW_APPLIED,
                    false,
                );
            }
        }

        if let Some(cp_ptr) = presenter_insp {
            let cp_saved = take_saved_target_brushes(presenter_key);
            if cp_saved.shadow.is_some() {
                sync_element_shadow_prop(
                    rt,
                    presenter_key,
                    cp_ptr,
                    false,
                    cp_saved.shadow,
                    DIAG_TOOLTIP_SHADOW_APPLIED,
                    true,
                );
            }
            let cp_com = qi_interface(cp_ptr, &IID_ICONTENTPRESENTER4);
            if !cp_com.is_null() {
                if let Some(sb_bg) = cp_saved.background {
                    let _ = sync_overflow_brush_prop_with_mode(
                        rt,
                        presenter_key,
                        cp_ptr,
                        cp_com.as_ptr(),
                        BrushPropKind::ContentPresenterBackground,
                        target_bg,
                        false,
                        Some(sb_bg),
                        DIAG_TOOLTIP_APPLIED,
                        true,
                    );
                }
                if let Some(sb_bb) = cp_saved.border_brush {
                    let _ = sync_overflow_brush_prop_with_mode(
                        rt,
                        presenter_key,
                        cp_ptr,
                        cp_com.as_ptr(),
                        BrushPropKind::ContentPresenterBorderBrush,
                        target_border,
                        false,
                        Some(sb_bb),
                        DIAG_TOOLTIP_APPLIED,
                        true,
                    );
                }
                sync_tooltip_presenter_corner_radius_fallback(
                    presenter_key,
                    cp_com.as_ptr(),
                    false,
                );
            }
        }

        purge_saved_brushes_for_current_thread(&keys_to_clean);
    }

    unsafe fn restore_all_tooltip_targets_for_current_thread(rt: &Arc<TapRuntime>) {
        let entries: Vec<(u64, TooltipTrackedEntry)> = UI_BRUSH_STORE.with(|cell| {
            let Ok(mut s) = cell.try_borrow_mut() else {
                return Vec::new();
            };
            s.tooltip_modified_targets.drain().collect()
        });
        for (presenter_key, entry) in entries {
            let cp_insp = resolve_weak_inspectable(&entry.presenter_weak);
            let tt_insp = entry
                .tooltip_weak
                .as_ref()
                .and_then(|tw| resolve_weak_inspectable(tw));
            restore_tooltip_pair(
                rt,
                presenter_key,
                cp_insp.as_ref().map(ComRaw::as_ptr),
                entry.tooltip_key,
                tt_insp.as_ref().map(ComRaw::as_ptr),
            );
        }
    }

    unsafe fn apply_tooltip_presenter_appearance(
        rt: &Arc<TapRuntime>,
        _handle: u64,
        insp_ptr: *mut c_void,
        desired_hide: bool,
    ) {
        if insp_ptr.is_null() {
            return;
        }
        let presenter_key = com_identity_key(insp_ptr);
        let parent_tooltip = find_parent_tooltip_control(insp_ptr);

        if desired_hide {
            let parent_pair = parent_tooltip
                .as_ref()
                .map(|(tk, insp)| (*tk, insp.as_ptr()));
            track_tooltip_modified_target(presenter_key, insp_ptr, parent_pair);

            let (target_bg, target_border) = rt.tooltip_target_colors();
            let apply_round = rt.is_tooltip_suppressed();

            if let Some((tooltip_key, ref tt_insp)) = parent_tooltip {
                let tt_ptr = tt_insp.as_ptr();
                let ctrl_com = qi_interface(tt_ptr, &IID_ICONTROL);
                if !ctrl_com.is_null() {
                    sync_overflow_brush_prop(
                        rt,
                        tooltip_key,
                        tt_ptr,
                        ctrl_com.as_ptr(),
                        BrushPropKind::ControlBackground,
                        target_bg,
                        true,
                        None,
                        DIAG_TOOLTIP_APPLIED,
                    );
                    sync_overflow_brush_prop(
                        rt,
                        tooltip_key,
                        tt_ptr,
                        ctrl_com.as_ptr(),
                        BrushPropKind::ControlBorderBrush,
                        target_border,
                        true,
                        None,
                        DIAG_TOOLTIP_APPLIED,
                    );
                    ensure_prop_watch_registered(
                        rt,
                        tooltip_key,
                        TargetKind::ToolTipPresenter,
                        BrushPropKind::ControlBackground,
                        tt_ptr,
                    );
                    ensure_prop_watch_registered(
                        rt,
                        tooltip_key,
                        TargetKind::ToolTipPresenter,
                        BrushPropKind::ControlBorderBrush,
                        tt_ptr,
                    );
                }
                sync_tooltip_control_corner_radius(tooltip_key, tt_ptr, apply_round);
                sync_element_shadow_prop(
                    rt,
                    tooltip_key,
                    tt_ptr,
                    true,
                    None,
                    DIAG_TOOLTIP_SHADOW_APPLIED,
                    false,
                );
                ensure_prop_watch_registered(
                    rt,
                    tooltip_key,
                    TargetKind::ToolTipPresenter,
                    BrushPropKind::ElementShadow,
                    tt_ptr,
                );
            }

            sync_element_shadow_prop(
                rt,
                presenter_key,
                insp_ptr,
                true,
                None,
                DIAG_TOOLTIP_SHADOW_APPLIED,
                true,
            );
            ensure_prop_watch_registered(
                rt,
                presenter_key,
                TargetKind::ToolTipPresenter,
                BrushPropKind::ElementShadow,
                insp_ptr,
            );

            // Preserve {TemplateBinding Background/BorderBrush/CornerRadius} on ContentPresenter#LayoutRoot:
            // only fall back to local mutation if LayoutRoot did not already inherit ToolTip's values.
            let cp_com = qi_interface(insp_ptr, &IID_ICONTENTPRESENTER4);
            if !cp_com.is_null() {
                let wrote_cp_bg = sync_overflow_brush_prop_with_mode(
                    rt,
                    presenter_key,
                    insp_ptr,
                    cp_com.as_ptr(),
                    BrushPropKind::ContentPresenterBackground,
                    target_bg,
                    true,
                    None,
                    DIAG_TOOLTIP_APPLIED,
                    true,
                );
                if wrote_cp_bg
                    || parent_tooltip.is_none()
                    || has_saved_brush_slot(presenter_key, BrushSlot::Background)
                {
                    ensure_prop_watch_registered(
                        rt,
                        presenter_key,
                        TargetKind::ToolTipPresenter,
                        BrushPropKind::ContentPresenterBackground,
                        insp_ptr,
                    );
                }
                let wrote_cp_bb = sync_overflow_brush_prop_with_mode(
                    rt,
                    presenter_key,
                    insp_ptr,
                    cp_com.as_ptr(),
                    BrushPropKind::ContentPresenterBorderBrush,
                    target_border,
                    true,
                    None,
                    DIAG_TOOLTIP_APPLIED,
                    true,
                );
                if wrote_cp_bb
                    || parent_tooltip.is_none()
                    || has_saved_brush_slot(presenter_key, BrushSlot::BorderBrush)
                {
                    ensure_prop_watch_registered(
                        rt,
                        presenter_key,
                        TargetKind::ToolTipPresenter,
                        BrushPropKind::ContentPresenterBorderBrush,
                        insp_ptr,
                    );
                }
                if apply_round
                    || UI_BRUSH_STORE.with(|cell| {
                        cell.try_borrow()
                            .ok()
                            .is_some_and(|s| s.tooltip_orig_corner_radius.contains_key(&presenter_key))
                    })
                {
                    sync_tooltip_presenter_corner_radius_fallback(
                        presenter_key,
                        cp_com.as_ptr(),
                        apply_round,
                    );
                }
            }
        } else {
            let tracked = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| s.tooltip_modified_targets.remove(&presenter_key))
            });
            let resolved_parent = if parent_tooltip.is_some() {
                parent_tooltip
            } else {
                tracked.as_ref().and_then(|e| {
                    e.tooltip_weak
                        .as_ref()
                        .and_then(|tw| resolve_weak_inspectable(tw))
                        .map(|insp| (e.tooltip_key.unwrap_or_else(|| com_identity_key(insp.as_ptr())), insp))
                })
            };
            let tk_opt = resolved_parent
                .as_ref()
                .map(|(tk, _)| *tk)
                .or_else(|| tracked.as_ref().and_then(|e| e.tooltip_key));
            restore_tooltip_pair(
                rt,
                presenter_key,
                Some(insp_ptr),
                tk_opt,
                resolved_parent.as_ref().map(|(_, insp)| insp.as_ptr()),
            );
        }
    }

    unsafe fn sync_menu_flyout_border_corner_radius_fallback(
        surface_key: u64,
        border_ptr: *mut c_void,
        apply_round: bool,
    ) {
        if border_ptr.is_null() {
            return;
        }
        let border = border_ptr.cast::<RawIBorder>();
        if apply_round {
            let target_cr = RawCornerRadius {
                top_left: 8.0,
                top_right: 8.0,
                bottom_right: 8.0,
                bottom_left: 8.0,
            };
            let mut cur_cr = RawCornerRadius::default();
            if ((*(*border).lp_vtbl).get_corner_radius)(border_ptr, &mut cur_cr).is_ok()
                && approx_eq_corner_radius(cur_cr, target_cr)
            {
                return;
            }
            let already_saved = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .is_some_and(|s| s.menu_flyout_orig_corner_radius.contains_key(&surface_key))
            });
            if !already_saved
                && ((*(*border).lp_vtbl).get_corner_radius)(border_ptr, &mut cur_cr).is_ok()
            {
                let _ = UI_BRUSH_STORE.with(|cell| {
                    if let Ok(mut s) = cell.try_borrow_mut() {
                        s.menu_flyout_orig_corner_radius
                            .entry(surface_key)
                            .or_insert(SavedCornerRadius::Local(cur_cr));
                    }
                });
            }
            let _ = ((*(*border).lp_vtbl).put_corner_radius)(border_ptr, target_cr);
        } else {
            let orig = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| s.menu_flyout_orig_corner_radius.remove(&surface_key))
            });
            if let Some(SavedCornerRadius::Local(orig_cr)) = orig {
                let _ = ((*(*border).lp_vtbl).put_corner_radius)(border_ptr, orig_cr);
            }
        }
    }

    unsafe fn restore_menu_flyout_pair(
        rt: &Arc<TapRuntime>,
        surface_key: u64,
        surface_insp: Option<*mut c_void>,
        ctrl_key_opt: Option<u64>,
        ctrl_insp: Option<*mut c_void>,
    ) {
        let ck_eff = ctrl_insp.map(|p| com_identity_key(p)).or(ctrl_key_opt);

        let mut keys_to_clean = Vec::with_capacity(2);
        if surface_key != 0 {
            keys_to_clean.push(surface_key);
        }
        if let Some(ck) = ck_eff.filter(|&k| k != 0 && !keys_to_clean.contains(&k)) {
            keys_to_clean.push(ck);
        }
        unregister_watches_for_keys(&keys_to_clean);

        let (target_bg, target_border) = rt.menu_flyout_target_colors();

        if let (Some(ck), Some(ctrl_ptr)) = (ck_eff, ctrl_insp) {
            let ctrl_saved = take_saved_target_brushes(ck);
            let ctrl_com = qi_interface(ctrl_ptr, &IID_ICONTROL);
            if !ctrl_com.is_null() {
                sync_overflow_brush_prop(
                    rt,
                    ck,
                    ctrl_ptr,
                    ctrl_com.as_ptr(),
                    BrushPropKind::ControlBackground,
                    target_bg,
                    false,
                    ctrl_saved.background,
                    DIAG_MENU_FLYOUT_APPLIED,
                );
                sync_overflow_brush_prop(
                    rt,
                    ck,
                    ctrl_ptr,
                    ctrl_com.as_ptr(),
                    BrushPropKind::ControlBorderBrush,
                    target_border,
                    false,
                    ctrl_saved.border_brush,
                    DIAG_MENU_FLYOUT_APPLIED,
                );
            }
            sync_menu_flyout_control_corner_radius(ck, ctrl_ptr, false);
            if ctrl_saved.shadow.is_some() {
                sync_element_shadow_prop(
                    rt,
                    ck,
                    ctrl_ptr,
                    false,
                    ctrl_saved.shadow,
                    DIAG_MENU_FLYOUT_SHADOW_APPLIED,
                    false,
                );
            }
        }

        if let Some(surf_ptr) = surface_insp {
            let surf_saved = take_saved_target_brushes(surface_key);
            if surf_saved.shadow.is_some() {
                sync_element_shadow_prop(
                    rt,
                    surface_key,
                    surf_ptr,
                    false,
                    surf_saved.shadow,
                    DIAG_MENU_FLYOUT_SHADOW_APPLIED,
                    false,
                );
            }
            let border_com = qi_interface(surf_ptr, &IID_IBORDER);
            if !border_com.is_null() {
                if let Some(sb_bg) = surf_saved.background {
                    let _ = sync_overflow_brush_prop_with_mode(
                        rt,
                        surface_key,
                        surf_ptr,
                        border_com.as_ptr(),
                        BrushPropKind::BorderBackground,
                        target_bg,
                        false,
                        Some(sb_bg),
                        DIAG_MENU_FLYOUT_APPLIED,
                        true,
                    );
                }
                if let Some(sb_bb) = surf_saved.border_brush {
                    let _ = sync_overflow_brush_prop_with_mode(
                        rt,
                        surface_key,
                        surf_ptr,
                        border_com.as_ptr(),
                        BrushPropKind::BorderBrush,
                        target_border,
                        false,
                        Some(sb_bb),
                        DIAG_MENU_FLYOUT_APPLIED,
                        true,
                    );
                }
                sync_menu_flyout_border_corner_radius_fallback(
                    surface_key,
                    border_com.as_ptr(),
                    false,
                );
            } else {
                let panel_com = qi_interface(surf_ptr, &IID_IPANEL);
                if !panel_com.is_null() {
                    if let Some(sb_bg) = surf_saved.background {
                        sync_overflow_brush_prop(
                            rt,
                            surface_key,
                            surf_ptr,
                            panel_com.as_ptr(),
                            BrushPropKind::PanelBackground,
                            target_bg,
                            false,
                            Some(sb_bg),
                            DIAG_MENU_FLYOUT_APPLIED,
                        );
                    }
                }
                let grid2_com = qi_interface(surf_ptr, &IID_IGRID2);
                if !grid2_com.is_null() {
                    if let Some(sb_bb) = surf_saved.border_brush {
                        sync_overflow_brush_prop(
                            rt,
                            surface_key,
                            surf_ptr,
                            grid2_com.as_ptr(),
                            BrushPropKind::GridBorderBrush,
                            target_border,
                            false,
                            Some(sb_bb),
                            DIAG_MENU_FLYOUT_APPLIED,
                        );
                    }
                }
            }
        }

        purge_saved_brushes_for_current_thread(&keys_to_clean);
    }

    unsafe fn restore_all_menu_flyout_targets_for_current_thread(rt: &Arc<TapRuntime>) {
        let entries: Vec<(u64, TooltipTrackedEntry)> = UI_BRUSH_STORE.with(|cell| {
            let Ok(mut s) = cell.try_borrow_mut() else {
                return Vec::new();
            };
            s.menu_flyout_modified_targets.drain().collect()
        });
        for (surface_key, entry) in entries {
            let surf_insp = resolve_weak_inspectable(&entry.presenter_weak);
            let ctrl_insp = entry
                .tooltip_weak
                .as_ref()
                .and_then(|cw| resolve_weak_inspectable(cw));
            restore_menu_flyout_pair(
                rt,
                surface_key,
                surf_insp.as_ref().map(ComRaw::as_ptr),
                entry.tooltip_key,
                ctrl_insp.as_ref().map(ComRaw::as_ptr),
            );
        }
    }

    unsafe fn apply_menu_flyout_surface_appearance(
        rt: &Arc<TapRuntime>,
        _handle: u64,
        insp_ptr: *mut c_void,
        desired_hide: bool,
    ) {
        if insp_ptr.is_null() {
            return;
        }
        let surface_key = com_identity_key(insp_ptr);
        let parent_ctrl = find_parent_menu_flyout_control(insp_ptr);

        if desired_hide {
            let parent_pair = parent_ctrl
                .as_ref()
                .map(|(ck, insp)| (*ck, insp.as_ptr()));
            track_menu_flyout_modified_target(surface_key, insp_ptr, parent_pair);

            let (target_bg, target_border) = rt.menu_flyout_target_colors();
            let apply_round = rt.is_menu_flyout_suppressed();

            if let Some((ctrl_key, ref ctrl_insp)) = parent_ctrl {
                let ctrl_ptr = ctrl_insp.as_ptr();
                let ctrl_com = qi_interface(ctrl_ptr, &IID_ICONTROL);
                if !ctrl_com.is_null() {
                    sync_overflow_brush_prop(
                        rt,
                        ctrl_key,
                        ctrl_ptr,
                        ctrl_com.as_ptr(),
                        BrushPropKind::ControlBackground,
                        target_bg,
                        true,
                        None,
                        DIAG_MENU_FLYOUT_APPLIED,
                    );
                    sync_overflow_brush_prop(
                        rt,
                        ctrl_key,
                        ctrl_ptr,
                        ctrl_com.as_ptr(),
                        BrushPropKind::ControlBorderBrush,
                        target_border,
                        true,
                        None,
                        DIAG_MENU_FLYOUT_APPLIED,
                    );
                    ensure_prop_watch_registered(
                        rt,
                        ctrl_key,
                        TargetKind::MenuFlyoutSurface,
                        BrushPropKind::ControlBackground,
                        ctrl_ptr,
                    );
                    ensure_prop_watch_registered(
                        rt,
                        ctrl_key,
                        TargetKind::MenuFlyoutSurface,
                        BrushPropKind::ControlBorderBrush,
                        ctrl_ptr,
                    );
                }
                sync_menu_flyout_control_corner_radius(ctrl_key, ctrl_ptr, apply_round);
                sync_element_shadow_prop(
                    rt,
                    ctrl_key,
                    ctrl_ptr,
                    true,
                    None,
                    DIAG_MENU_FLYOUT_SHADOW_APPLIED,
                    false,
                );
                ensure_prop_watch_registered(
                    rt,
                    ctrl_key,
                    TargetKind::MenuFlyoutSurface,
                    BrushPropKind::ElementShadow,
                    ctrl_ptr,
                );
            }

            sync_element_shadow_prop(
                rt,
                surface_key,
                insp_ptr,
                true,
                None,
                DIAG_MENU_FLYOUT_SHADOW_APPLIED,
                false,
            );
            ensure_prop_watch_registered(
                rt,
                surface_key,
                TargetKind::MenuFlyoutSurface,
                BrushPropKind::ElementShadow,
                insp_ptr,
            );

            // Preserve {TemplateBinding Background/BorderBrush/CornerRadius} on child Border
            // (e.g. MenuFlyoutPresenterRoot) when inherited from parent Control.
            let border_com = qi_interface(insp_ptr, &IID_IBORDER);
            if !border_com.is_null() {
                let wrote_b_bg = sync_overflow_brush_prop_with_mode(
                    rt,
                    surface_key,
                    insp_ptr,
                    border_com.as_ptr(),
                    BrushPropKind::BorderBackground,
                    target_bg,
                    true,
                    None,
                    DIAG_MENU_FLYOUT_APPLIED,
                    true,
                );
                if wrote_b_bg
                    || parent_ctrl.is_none()
                    || has_saved_brush_slot(surface_key, BrushSlot::Background)
                {
                    ensure_prop_watch_registered(
                        rt,
                        surface_key,
                        TargetKind::MenuFlyoutSurface,
                        BrushPropKind::BorderBackground,
                        insp_ptr,
                    );
                }
                let wrote_b_bb = sync_overflow_brush_prop_with_mode(
                    rt,
                    surface_key,
                    insp_ptr,
                    border_com.as_ptr(),
                    BrushPropKind::BorderBrush,
                    target_border,
                    true,
                    None,
                    DIAG_MENU_FLYOUT_APPLIED,
                    true,
                );
                if wrote_b_bb
                    || parent_ctrl.is_none()
                    || has_saved_brush_slot(surface_key, BrushSlot::BorderBrush)
                {
                    ensure_prop_watch_registered(
                        rt,
                        surface_key,
                        TargetKind::MenuFlyoutSurface,
                        BrushPropKind::BorderBrush,
                        insp_ptr,
                    );
                }
                if apply_round
                    || UI_BRUSH_STORE.with(|cell| {
                        cell.try_borrow()
                            .ok()
                            .is_some_and(|s| s.menu_flyout_orig_corner_radius.contains_key(&surface_key))
                    })
                {
                    sync_menu_flyout_border_corner_radius_fallback(
                        surface_key,
                        border_com.as_ptr(),
                        apply_round,
                    );
                }
            } else {
                let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
                if !panel_com.is_null() {
                    sync_overflow_brush_prop(
                        rt,
                        surface_key,
                        insp_ptr,
                        panel_com.as_ptr(),
                        BrushPropKind::PanelBackground,
                        target_bg,
                        true,
                        None,
                        DIAG_MENU_FLYOUT_APPLIED,
                    );
                    ensure_prop_watch_registered(
                        rt,
                        surface_key,
                        TargetKind::MenuFlyoutSurface,
                        BrushPropKind::PanelBackground,
                        insp_ptr,
                    );
                }
                let grid2_com = qi_interface(insp_ptr, &IID_IGRID2);
                if !grid2_com.is_null() {
                    sync_overflow_brush_prop(
                        rt,
                        surface_key,
                        insp_ptr,
                        grid2_com.as_ptr(),
                        BrushPropKind::GridBorderBrush,
                        target_border,
                        true,
                        None,
                        DIAG_MENU_FLYOUT_APPLIED,
                    );
                    ensure_prop_watch_registered(
                        rt,
                        surface_key,
                        TargetKind::MenuFlyoutSurface,
                        BrushPropKind::GridBorderBrush,
                        insp_ptr,
                    );
                }
            }
        } else {
            let tracked = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| s.menu_flyout_modified_targets.remove(&surface_key))
            });
            let resolved_parent = if parent_ctrl.is_some() {
                parent_ctrl
            } else {
                tracked.as_ref().and_then(|e| {
                    e.tooltip_weak
                        .as_ref()
                        .and_then(|cw| resolve_weak_inspectable(cw))
                        .map(|insp| (e.tooltip_key.unwrap_or_else(|| com_identity_key(insp.as_ptr())), insp))
                })
            };
            let ck_opt = resolved_parent
                .as_ref()
                .map(|(ck, _)| *ck)
                .or_else(|| tracked.as_ref().and_then(|e| e.tooltip_key));
            restore_menu_flyout_pair(
                rt,
                surface_key,
                Some(insp_ptr),
                ck_opt,
                resolved_parent.as_ref().map(|(_, insp)| insp.as_ptr()),
            );
        }
    }

    unsafe fn sync_shell_card_border_corner_radius(
        surface_key: u64,
        border_ptr: *mut c_void,
        apply_round: bool,
    ) {
        if border_ptr.is_null() {
            return;
        }
        let border = border_ptr.cast::<RawIBorder>();
        if apply_round {
            let target_cr = RawCornerRadius {
                top_left: 8.0,
                top_right: 8.0,
                bottom_right: 8.0,
                bottom_left: 8.0,
            };
            let mut cur_cr = RawCornerRadius::default();
            if ((*(*border).lp_vtbl).get_corner_radius)(border_ptr, &mut cur_cr).is_ok()
                && approx_eq_corner_radius(cur_cr, target_cr)
            {
                return;
            }
            let already_saved = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .is_some_and(|s| s.shell_card_orig_corner_radius.contains_key(&surface_key))
            });
            if !already_saved
                && ((*(*border).lp_vtbl).get_corner_radius)(border_ptr, &mut cur_cr).is_ok()
            {
                let _ = UI_BRUSH_STORE.with(|cell| {
                    if let Ok(mut s) = cell.try_borrow_mut() {
                        s.shell_card_orig_corner_radius
                            .entry(surface_key)
                            .or_insert(SavedCornerRadius::Local(cur_cr));
                    }
                });
            }
            let _ = ((*(*border).lp_vtbl).put_corner_radius)(border_ptr, target_cr);
        } else {
            let orig = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| s.shell_card_orig_corner_radius.remove(&surface_key))
            });
            if let Some(SavedCornerRadius::Local(orig_cr)) = orig {
                let _ = ((*(*border).lp_vtbl).put_corner_radius)(border_ptr, orig_cr);
            }
        }
    }

    unsafe fn restore_shell_card_target(
        rt: &Arc<TapRuntime>,
        surface_key: u64,
        surface_insp: Option<*mut c_void>,
    ) {
        if surface_key != 0 {
            unregister_watches_for_keys(&[surface_key]);
        }
        let surf_saved = take_saved_target_brushes(surface_key);
        let (target_bg, target_border) = rt.menu_flyout_target_colors();

        if let Some(surf_ptr) = surface_insp {
            if surf_saved.shadow.is_some() {
                sync_element_shadow_prop(
                    rt,
                    surface_key,
                    surf_ptr,
                    false,
                    surf_saved.shadow,
                    DIAG_SHELL_CARD_SHADOW_APPLIED,
                    false,
                );
            }
            let border_com = qi_interface(surf_ptr, &IID_IBORDER);
            if !border_com.is_null() {
                if let Some(sb_bg) = surf_saved.background {
                    let _ = sync_overflow_brush_prop_with_mode(
                        rt,
                        surface_key,
                        surf_ptr,
                        border_com.as_ptr(),
                        BrushPropKind::BorderBackground,
                        target_bg,
                        false,
                        Some(sb_bg),
                        DIAG_SHELL_CARD_APPLIED,
                        true,
                    );
                }
                if let Some(sb_bb) = surf_saved.border_brush {
                    let _ = sync_overflow_brush_prop_with_mode(
                        rt,
                        surface_key,
                        surf_ptr,
                        border_com.as_ptr(),
                        BrushPropKind::BorderBrush,
                        target_border,
                        false,
                        Some(sb_bb),
                        DIAG_SHELL_CARD_APPLIED,
                        true,
                    );
                }
                sync_shell_card_border_corner_radius(surface_key, border_com.as_ptr(), false);
            } else {
                let panel_com = qi_interface(surf_ptr, &IID_IPANEL);
                if !panel_com.is_null() {
                    if let Some(sb_bg) = surf_saved.background {
                        sync_overflow_brush_prop(
                            rt,
                            surface_key,
                            surf_ptr,
                            panel_com.as_ptr(),
                            BrushPropKind::PanelBackground,
                            target_bg,
                            false,
                            Some(sb_bg),
                            DIAG_SHELL_CARD_APPLIED,
                        );
                    }
                }
                let grid2_com = qi_interface(surf_ptr, &IID_IGRID2);
                if !grid2_com.is_null() {
                    if let Some(sb_bb) = surf_saved.border_brush {
                        sync_overflow_brush_prop(
                            rt,
                            surface_key,
                            surf_ptr,
                            grid2_com.as_ptr(),
                            BrushPropKind::GridBorderBrush,
                            target_border,
                            false,
                            Some(sb_bb),
                            DIAG_SHELL_CARD_APPLIED,
                        );
                    }
                }
            }
        }
        if surface_key != 0 {
            purge_saved_brushes_for_current_thread(&[surface_key]);
        }
    }

    unsafe fn restore_all_shell_card_targets_for_current_thread(rt: &Arc<TapRuntime>) {
        let entries: Vec<(u64, TooltipTrackedEntry)> = UI_BRUSH_STORE.with(|cell| {
            let Ok(mut s) = cell.try_borrow_mut() else {
                return Vec::new();
            };
            s.shell_card_modified_targets.drain().collect()
        });
        for (surface_key, entry) in entries {
            let surf_insp = resolve_weak_inspectable(&entry.presenter_weak);
            restore_shell_card_target(
                rt,
                surface_key,
                surf_insp.as_ref().map(ComRaw::as_ptr),
            );
        }
    }

    unsafe fn apply_shell_card_surface_appearance(
        rt: &Arc<TapRuntime>,
        _handle: u64,
        insp_ptr: *mut c_void,
        kind: TargetKind,
        desired_hide: bool,
    ) {
        if insp_ptr.is_null() {
            return;
        }
        let surface_key = com_identity_key(insp_ptr);
        if desired_hide {
            track_shell_card_modified_target(surface_key, insp_ptr);
            let is_subsurface = kind == TargetKind::ShellCardSubSurface;
            let (target_bg, target_border) = if is_subsurface {
                (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
            } else {
                rt.menu_flyout_target_colors()
            };
            let apply_round = !is_subsurface && rt.is_menu_flyout_suppressed();

            sync_element_shadow_prop(
                rt,
                surface_key,
                insp_ptr,
                true,
                None,
                DIAG_SHELL_CARD_SHADOW_APPLIED,
                false,
            );
            ensure_prop_watch_registered(
                rt,
                surface_key,
                kind,
                BrushPropKind::ElementShadow,
                insp_ptr,
            );

            let border_com = qi_interface(insp_ptr, &IID_IBORDER);
            if !border_com.is_null() {
                let _ = sync_overflow_brush_prop_with_mode(
                    rt,
                    surface_key,
                    insp_ptr,
                    border_com.as_ptr(),
                    BrushPropKind::BorderBackground,
                    target_bg,
                    true,
                    None,
                    DIAG_SHELL_CARD_APPLIED,
                    true,
                );
                let _ = sync_overflow_brush_prop_with_mode(
                    rt,
                    surface_key,
                    insp_ptr,
                    border_com.as_ptr(),
                    BrushPropKind::BorderBrush,
                    target_border,
                    true,
                    None,
                    DIAG_SHELL_CARD_APPLIED,
                    true,
                );
                ensure_prop_watch_registered(
                    rt,
                    surface_key,
                    kind,
                    BrushPropKind::BorderBackground,
                    insp_ptr,
                );
                ensure_prop_watch_registered(
                    rt,
                    surface_key,
                    kind,
                    BrushPropKind::BorderBrush,
                    insp_ptr,
                );
                sync_shell_card_border_corner_radius(surface_key, border_com.as_ptr(), apply_round);
            } else {
                let panel_com = qi_interface(insp_ptr, &IID_IPANEL);
                if !panel_com.is_null() {
                    sync_overflow_brush_prop(
                        rt,
                        surface_key,
                        insp_ptr,
                        panel_com.as_ptr(),
                        BrushPropKind::PanelBackground,
                        target_bg,
                        true,
                        None,
                        DIAG_SHELL_CARD_APPLIED,
                    );
                    ensure_prop_watch_registered(
                        rt,
                        surface_key,
                        kind,
                        BrushPropKind::PanelBackground,
                        insp_ptr,
                    );
                }
                let grid2_com = qi_interface(insp_ptr, &IID_IGRID2);
                if !grid2_com.is_null() {
                    sync_overflow_brush_prop(
                        rt,
                        surface_key,
                        insp_ptr,
                        grid2_com.as_ptr(),
                        BrushPropKind::GridBorderBrush,
                        target_border,
                        true,
                        None,
                        DIAG_SHELL_CARD_APPLIED,
                    );
                    ensure_prop_watch_registered(
                        rt,
                        surface_key,
                        kind,
                        BrushPropKind::GridBorderBrush,
                        insp_ptr,
                    );
                }
            }
        } else {
            let removed = UI_BRUSH_STORE.with(|cell| {
                cell.try_borrow_mut()
                    .ok()
                    .and_then(|mut s| s.shell_card_modified_targets.remove(&surface_key))
            });
            if removed.is_some()
                || has_saved_brush_slot(surface_key, BrushSlot::Background)
                || has_saved_brush_slot(surface_key, BrushSlot::BorderBrush)
                || has_saved_brush_slot(surface_key, BrushSlot::Shadow)
            {
                restore_shell_card_target(rt, surface_key, Some(insp_ptr));
            }
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
            TargetKind::HoverFlyoutShape => {
                apply_shape_appearance(rt, insp_ptr, desired_hide, orig_opacity, orig_visibility);
                if desired_hide {
                    rt.diag_flags
                        .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
                }
            }
            TargetKind::HoverFlyoutBorder => {
                apply_hover_flyout_border_appearance(rt, handle, insp_ptr, desired_hide);
            }
            TargetKind::OverflowBorder => {
                apply_border_target_appearance(
                    rt,
                    handle,
                    insp_ptr,
                    TargetKind::OverflowBorder,
                    RawUiColor::TRANSPARENT,
                    RawUiColor::TRANSPARENT,
                    desired_hide,
                    DIAG_OVERFLOW_APPLIED,
                );
            }
            TargetKind::AltTabBorder => {
                apply_border_target_appearance(
                    rt,
                    handle,
                    insp_ptr,
                    TargetKind::AltTabBorder,
                    RawUiColor::TRANSPARENT,
                    RawUiColor::TRANSPARENT,
                    desired_hide,
                    DIAG_ALTTAB_APPLIED,
                );
            }
            TargetKind::AltTabItemBorder => {
                let bg_color =
                    RawUiColor::from_argb(rt.alt_tab_item_bg_argb.load(Ordering::Acquire));
                let border_color =
                    RawUiColor::from_argb(rt.alt_tab_item_border_argb.load(Ordering::Acquire));
                apply_alttab_item_border_appearance(
                    rt,
                    insp_ptr,
                    None,
                    bg_color,
                    border_color,
                    desired_hide,
                );
            }
            TargetKind::AltTabItemFocus => {
                apply_focus_secondary_appearance(rt, insp_ptr, desired_hide);
            }
            TargetKind::OsdPanel => {
                apply_osd_panel_appearance(rt, handle, insp_ptr, desired_hide);
            }
            TargetKind::ToolTipPresenter => {
                apply_tooltip_presenter_appearance(rt, handle, insp_ptr, desired_hide);
            }
            TargetKind::MenuFlyoutSurface => {
                apply_menu_flyout_surface_appearance(rt, handle, insp_ptr, desired_hide);
            }
            TargetKind::ShellCardSurface | TargetKind::ShellCardSubSurface => {
                apply_shell_card_surface_appearance(rt, handle, insp_ptr, kind, desired_hide);
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
                    RawUiColor::TRANSPARENT,
                    desired_hide,
                    saved.background,
                    DIAG_OVERFLOW_APPLIED,
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
        tooltip_dispatchers: Vec<(u32, OwnedDispatcher)>,
        menu_flyout_dispatchers: Vec<(u32, OwnedDispatcher)>,
    }

    unsafe fn detect_host_scope_and_pipe(is_appcontainer: bool) -> (TapHostScope, Option<String>) {
        let pid = GetCurrentProcessId();
        if is_appcontainer {
            return (
                TapHostScope::ShellExperience,
                Some(appcontainer_pipe_name_for_pid(pid)),
            );
        }
        let mut buf = [0u16; 512];
        let len = GetModuleFileNameW(None, &mut buf) as usize;
        if len > 0 {
            let path = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
            let exe_name = path.rsplit(['\\', '/']).next().unwrap_or("");
            if exe_name.eq_ignore_ascii_case("ShellHost.exe") {
                return (
                    TapHostScope::ShellHost,
                    Some(shell_host_pipe_name_for_pid(pid)),
                );
            }
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
        alt_tab_item_bg_argb: AtomicU32,
        alt_tab_item_border_argb: AtomicU32,
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
                    tooltip_dispatchers: Vec::new(),
                    menu_flyout_dispatchers: Vec::new(),
                }),
                is_appcontainer,
                host_scope,
                reverse_pipe_name,
                alt_tab_bounds_dip: Mutex::new(None),
                hover_flyout_bounds_dip: Mutex::new(None),
                osd_bounds_dip: Mutex::new(None),
                osd_suppressed: AtomicBool::new(false),
                osd_fallback_bg_argb: AtomicU32::new(0x3E_141822),
                osd_fallback_border_argb: AtomicU32::new(0x52_FFFFFF),
                tooltip_bounds_dip: Mutex::new(None),
                tooltip_suppressed: AtomicBool::new(false),
                tooltip_fallback_bg_argb: AtomicU32::new(0x44_141822),
                tooltip_fallback_border_argb: AtomicU32::new(0x54_FFFFFF),
                menu_flyout_bounds_by_tid: Mutex::new(HashMap::new()),
                shell_card_bounds_by_handle: Mutex::new(HashMap::new()),
                menu_flyout_suppressed: AtomicBool::new(false),
                menu_flyout_fallback_bg_argb: AtomicU32::new(0x48_141822),
                menu_flyout_fallback_border_argb: AtomicU32::new(0x56_FFFFFF),
                alt_tab_item_bg_argb: AtomicU32::new(0x38_1A1D24),
                alt_tab_item_border_argb: AtomicU32::new(0x42_FFFFFF),
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
                    (_, OwnedDispatcher::Core(cookie)) if cookie != 0 => {
                        let key = DispatchKey::CoreCookie(cookie);
                        if !seen_keys.contains(&key) {
                            seen_keys.push(key);
                            dispatch_list.push(DispatchTarget::Core { cookie });
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
                    for &(tid, disp) in &g.tooltip_dispatchers {
                        push_unique(&mut seen_keys, &mut dispatch_list, Some(tid), disp);
                    }
                    for &(tid, disp) in &g.menu_flyout_dispatchers {
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

        fn query_alt_tab_bounds_dip(self: &Arc<Self>, wait_ms: u64) -> Option<(u32, [f32; 4])> {
            let dispatched = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::AltTabBorder));
            if dispatched == 0 {
                if let Ok(mut b) = self.alt_tab_bounds_dip.lock() {
                    *b = None;
                }
                return None;
            }
            self.alt_tab_bounds_dip.lock().ok().and_then(|b| *b)
        }

        fn query_hover_flyout_bounds_dip(
            self: &Arc<Self>,
            wait_ms: u64,
        ) -> Option<(u32, [f32; 4])> {
            let dispatched =
                self.dispatch_apply_filtered(wait_ms, Some(TargetKind::HoverFlyoutBorder));
            if dispatched == 0 {
                if let Ok(mut b) = self.hover_flyout_bounds_dip.lock() {
                    *b = None;
                }
                return None;
            }
            self.hover_flyout_bounds_dip.lock().ok().and_then(|b| *b)
        }

        fn query_osd_bounds_dip(self: &Arc<Self>, wait_ms: u64) -> Option<(u32, [f32; 4])> {
            if self.is_osd_suppressed() {
                if let Ok(mut b) = self.osd_bounds_dip.lock() {
                    *b = None;
                }
                return None;
            }
            let dispatched = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::OsdPanel));
            if dispatched == 0 {
                if let Ok(mut b) = self.osd_bounds_dip.lock() {
                    *b = None;
                }
                return None;
            }
            self.osd_bounds_dip.lock().ok().and_then(|b| *b)
        }

        #[inline]
        fn osd_target_colors(&self) -> (RawUiColor, RawUiColor) {
            if self.is_osd_suppressed() {
                (
                    RawUiColor::from_argb(self.osd_fallback_bg_argb.load(Ordering::Acquire)),
                    RawUiColor::from_argb(self.osd_fallback_border_argb.load(Ordering::Acquire)),
                )
            } else {
                (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
            }
        }

        fn set_osd_suppressed(self: &Arc<Self>, suppressed: bool) {
            let prev = self.osd_suppressed.swap(suppressed, Ordering::AcqRel);
            if suppressed {
                if let Ok(mut b) = self.osd_bounds_dip.lock() {
                    *b = None;
                }
            }
            if prev != suppressed && self.desired_hide() {
                self.dispatch_apply_filtered(0, Some(TargetKind::OsdPanel));
            }
        }

        fn set_osd_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
            let prev_bg = self.osd_fallback_bg_argb.swap(bg_argb, Ordering::AcqRel);
            let prev_bd = self
                .osd_fallback_border_argb
                .swap(border_argb, Ordering::AcqRel);
            if (prev_bg != bg_argb || prev_bd != border_argb)
                && self.desired_hide()
                && self.is_osd_suppressed()
            {
                self.dispatch_apply_filtered(0, Some(TargetKind::OsdPanel));
            }
        }

        fn query_tooltip_bounds_dip(self: &Arc<Self>, wait_ms: u64) -> Option<(u32, [f32; 4])> {
            if self.is_tooltip_suppressed() {
                if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                    *b = None;
                }
                return None;
            }
            let dispatched = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::ToolTipPresenter));
            if dispatched == 0 {
                if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                    *b = None;
                }
                return None;
            }
            self.tooltip_bounds_dip.lock().ok().and_then(|b| *b)
        }

        #[inline]
        fn tooltip_target_colors(&self) -> (RawUiColor, RawUiColor) {
            if self.is_tooltip_suppressed() {
                (
                    RawUiColor::from_argb(self.tooltip_fallback_bg_argb.load(Ordering::Acquire)),
                    RawUiColor::from_argb(self.tooltip_fallback_border_argb.load(Ordering::Acquire)),
                )
            } else {
                (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
            }
        }

        fn set_tooltip_suppressed(self: &Arc<Self>, suppressed: bool) {
            let prev = self.tooltip_suppressed.swap(suppressed, Ordering::AcqRel);
            if suppressed {
                if let Ok(mut b) = self.tooltip_bounds_dip.lock() {
                    *b = None;
                }
            }
            if prev != suppressed && self.desired_hide() {
                self.dispatch_apply_filtered(0, Some(TargetKind::ToolTipPresenter));
            }
        }

        fn set_tooltip_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
            let prev_bg = self.tooltip_fallback_bg_argb.swap(bg_argb, Ordering::AcqRel);
            let prev_bd = self
                .tooltip_fallback_border_argb
                .swap(border_argb, Ordering::AcqRel);
            if (prev_bg != bg_argb || prev_bd != border_argb)
                && self.desired_hide()
                && self.is_tooltip_suppressed()
            {
                self.dispatch_apply_filtered(0, Some(TargetKind::ToolTipPresenter));
            }
        }

        fn query_menu_flyout_bounds_dip(
            self: &Arc<Self>,
            wait_ms: u64,
        ) -> Vec<(u32, [f32; 4])> {
            if self.is_menu_flyout_suppressed() {
                if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                    b.clear();
                }
                return Vec::new();
            }
            let dispatched =
                self.dispatch_apply_filtered(wait_ms, Some(TargetKind::MenuFlyoutSurface));
            if dispatched == 0 {
                if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                    b.clear();
                }
                return Vec::new();
            }
            let (active_tids, has_core_menu) = if let Ok(g) = self.state.lock() {
                let mut tids = Vec::new();
                let mut has_core = false;
                for t in g.targets.values() {
                    if t.kind == TargetKind::MenuFlyoutSurface {
                        if let Some(tid) = t.thread_id {
                            if !tids.contains(&tid) {
                                tids.push(tid);
                            }
                        } else {
                            has_core = true;
                        }
                    }
                }
                (tids, has_core)
            } else {
                (Vec::new(), true)
            };
            let mut out = Vec::new();
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                if !has_core_menu {
                    b.retain(|tid, _| active_tids.contains(tid));
                }
                for (&tid, rects) in b.iter() {
                    for &r in rects {
                        out.push((tid, r));
                    }
                }
            }
            out.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.1[1].total_cmp(&b.1[1]))
                    .then_with(|| a.1[0].total_cmp(&b.1[0]))
            });
            out.truncate(MAX_MENU_FLYOUT_RECTS);
            out
        }

        fn query_shell_card_bounds_dip(
            self: &Arc<Self>,
            wait_ms: u64,
        ) -> Vec<(u32, [f32; 4])> {
            if self.is_menu_flyout_suppressed() {
                if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                    b.clear();
                }
                if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                    b.clear();
                }
                return Vec::new();
            }

            let (menu_tids, shell_handles, has_core_menu) =
                if let Ok(g) = self.state.lock() {
                    let mut m_tids = Vec::new();
                    let mut s_handles = Vec::new();
                    let mut core_menu = false;
                    for (&h, t) in &g.targets {
                        match t.kind {
                            TargetKind::MenuFlyoutSurface => {
                                if let Some(tid) = t.thread_id {
                                    if !m_tids.contains(&tid) {
                                        m_tids.push(tid);
                                    }
                                } else {
                                    core_menu = true;
                                }
                            }
                            TargetKind::ShellCardSurface => {
                                s_handles.push(h);
                            }
                            _ => {}
                        }
                    }
                    (m_tids, s_handles, core_menu)
                } else {
                    (Vec::new(), Vec::new(), false)
                };

            let need_menu_dispatch = has_core_menu || !menu_tids.is_empty();
            let need_shell_dispatch = !shell_handles.is_empty();
            if !need_menu_dispatch && !need_shell_dispatch {
                if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                    b.clear();
                }
                if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                    b.clear();
                }
                return Vec::new();
            }

            if need_menu_dispatch {
                let _ = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::MenuFlyoutSurface));
            }
            if need_shell_dispatch {
                let _ = self.dispatch_apply_filtered(wait_ms, Some(TargetKind::ShellCardSurface));
            }

            let mut out: Vec<(u32, [f32; 4])> = Vec::new();
            if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                if !need_menu_dispatch {
                    b.clear();
                } else {
                    if !has_core_menu {
                        b.retain(|tid, _| menu_tids.contains(tid));
                    }
                    for (&tid, rects) in b.iter() {
                        for &r in rects {
                            out.push((tid, r));
                        }
                    }
                }
            }
            if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                if !need_shell_dispatch {
                    b.clear();
                } else {
                    b.retain(|h, _| shell_handles.contains(h));
                    for &(tid, r) in b.values() {
                        let is_dup = out.iter().any(|&(pt, prev)| {
                            pt == tid
                                && (prev[0] - r[0]).abs() <= 4.0
                                && (prev[1] - r[1]).abs() <= 4.0
                                && (prev[2] - r[2]).abs() <= 4.0
                                && (prev[3] - r[3]).abs() <= 4.0
                        });
                        if !is_dup {
                            out.push((tid, r));
                        }
                    }
                }
            }
            out.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.1[1].total_cmp(&b.1[1]))
                    .then_with(|| a.1[0].total_cmp(&b.1[0]))
                    .then_with(|| a.1[3].total_cmp(&b.1[3]))
                    .then_with(|| a.1[2].total_cmp(&b.1[2]))
            });
            out.truncate(MAX_SHELL_CARD_RECTS);
            out
        }

        #[inline]
        fn menu_flyout_target_colors(&self) -> (RawUiColor, RawUiColor) {
            if self.is_menu_flyout_suppressed() {
                (
                    RawUiColor::from_argb(
                        self.menu_flyout_fallback_bg_argb.load(Ordering::Acquire),
                    ),
                    RawUiColor::from_argb(
                        self.menu_flyout_fallback_border_argb
                            .load(Ordering::Acquire),
                    ),
                )
            } else {
                (RawUiColor::TRANSPARENT, RawUiColor::TRANSPARENT)
            }
        }

        fn set_menu_flyout_suppressed(self: &Arc<Self>, suppressed: bool) {
            let prev = self.menu_flyout_suppressed.swap(suppressed, Ordering::AcqRel);
            if suppressed {
                if let Ok(mut b) = self.menu_flyout_bounds_by_tid.lock() {
                    b.clear();
                }
                if let Ok(mut b) = self.shell_card_bounds_by_handle.lock() {
                    b.clear();
                }
            }
            if prev != suppressed && self.desired_hide() {
                self.dispatch_apply_filtered(0, Some(TargetKind::MenuFlyoutSurface));
                self.dispatch_apply_filtered(0, Some(TargetKind::ShellCardSurface));
            }
        }

        fn set_menu_flyout_fallback_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
            let prev_bg = self
                .menu_flyout_fallback_bg_argb
                .swap(bg_argb, Ordering::AcqRel);
            let prev_bd = self
                .menu_flyout_fallback_border_argb
                .swap(border_argb, Ordering::AcqRel);
            if (prev_bg != bg_argb || prev_bd != border_argb)
                && self.desired_hide()
                && self.is_menu_flyout_suppressed()
            {
                self.dispatch_apply_filtered(0, Some(TargetKind::MenuFlyoutSurface));
                self.dispatch_apply_filtered(0, Some(TargetKind::ShellCardSurface));
            }
        }

        fn set_alt_tab_item_colors(self: &Arc<Self>, bg_argb: u32, border_argb: u32) {
            let prev_bg = self.alt_tab_item_bg_argb.swap(bg_argb, Ordering::AcqRel);
            let prev_bd = self
                .alt_tab_item_border_argb
                .swap(border_argb, Ordering::AcqRel);
            if (prev_bg != bg_argb || prev_bd != border_argb) && self.desired_hide() {
                self.dispatch_apply_filtered(0, Some(TargetKind::AltTabItemBorder));
                self.dispatch_apply_filtered(0, Some(TargetKind::AltTabBorder));
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

        if desired_hide && items.is_empty() {
            return;
        }
        if !items.is_empty() {
            rt.diag_flags.fetch_or(DIAG_APPLY_RAN, Ordering::Relaxed);
        }
        if desired_hide {
            unsafe {
                prewarm_overflow_brush_store_for_current_thread();
            }
        }

        let mut stale_handles = Vec::new();
        let mut saw_alt_tab = false;
        let mut measured_alt_tab: Option<[f32; 4]> = None;
        let mut saw_hover_flyout = false;
        let mut measured_hover_flyout: Option<([f32; 4], f32)> = None;
        let mut saw_osd = false;
        let mut measured_osd: Option<([f32; 4], f32)> = None;
        let osd_suppressed = rt.is_osd_suppressed();
        let mut saw_tooltip = false;
        let mut measured_tooltip: Option<([f32; 4], f32)> = None;
        let tooltip_suppressed = rt.is_tooltip_suppressed();
        let mut saw_menu_flyout = false;
        let mut measured_menu_flyouts: Vec<[f32; 4]> = Vec::new();
        let menu_flyout_suppressed = rt.is_menu_flyout_suppressed();
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

                if item.kind == TargetKind::AltTabBorder {
                    saw_alt_tab = true;
                    sync_alttab_subtree_items(rt, insp.as_ptr(), desired_hide);
                    if desired_hide {
                        if let Some(bounds) =
                            measure_element_bounds_dip(insp.as_ptr(), 80.0, 60.0, 0.01)
                        {
                            measured_alt_tab = Some(bounds);
                        }
                    }
                } else if item.kind == TargetKind::HoverFlyoutBorder {
                    saw_hover_flyout = true;
                    sync_hover_flyout_bg_shapes(rt, insp.as_ptr(), desired_hide);
                    if desired_hide {
                        if let Some(bounds) = measure_hover_flyout_bounds_dip(insp.as_ptr()) {
                            let area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
                            if measured_hover_flyout
                                .is_none_or(|(_, best_area)| area > best_area)
                            {
                                measured_hover_flyout = Some((bounds, area));
                            }
                        }
                    }
                } else if item.kind == TargetKind::OsdPanel {
                    saw_osd = true;
                    if desired_hide && !osd_suppressed {
                        if let Some(bounds) =
                            measure_element_bounds_dip(insp.as_ptr(), 80.0, 32.0, 0.05)
                        {
                            let area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
                            if measured_osd.is_none_or(|(_, best_area)| area > best_area) {
                                measured_osd = Some((bounds, area));
                            }
                        }
                    }
                } else if item.kind == TargetKind::ToolTipPresenter {
                    saw_tooltip = true;
                    if desired_hide && !tooltip_suppressed {
                        if let Some(bounds) = measure_tooltip_bounds_dip(insp.as_ptr()) {
                            let area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
                            if measured_tooltip.is_none_or(|(_, best_area)| area > best_area) {
                                measured_tooltip = Some((bounds, area));
                            }
                        }
                    }
                } else if item.kind == TargetKind::MenuFlyoutSurface {
                    saw_menu_flyout = true;
                    if desired_hide && !menu_flyout_suppressed {
                        if let Some(bounds) = measure_menu_flyout_bounds_dip(insp.as_ptr()) {
                            let is_dup = measured_menu_flyouts.iter().any(|prev| {
                                (prev[0] - bounds[0]).abs() <= 4.0
                                    && (prev[1] - bounds[1]).abs() <= 4.0
                                    && (prev[2] - bounds[2]).abs() <= 4.0
                                    && (prev[3] - bounds[3]).abs() <= 4.0
                            });
                            if !is_dup && measured_menu_flyouts.len() < MAX_MENU_FLYOUT_RECTS {
                                measured_menu_flyouts.push(bounds);
                            }
                        }
                    }
                } else if item.kind == TargetKind::ShellCardSurface {
                    let cur_tid = GetCurrentThreadId();
                    let bounds_opt = if desired_hide && !menu_flyout_suppressed {
                        measure_element_bounds_dip(insp.as_ptr(), 64.0, 28.0, 0.05)
                    } else {
                        None
                    };
                    if let Ok(mut b) = rt.shell_card_bounds_by_handle.lock() {
                        if let Some(bounds) = bounds_opt {
                            b.insert(item.handle, (cur_tid, bounds));
                        } else {
                            b.remove(&item.handle);
                        }
                    }
                }
            }
        }

        if saw_alt_tab {
            let cur_tid = unsafe { GetCurrentThreadId() };
            if let Ok(mut b) = rt.alt_tab_bounds_dip.lock() {
                *b = if desired_hide {
                    measured_alt_tab.map(|r| (cur_tid, r))
                } else {
                    None
                };
            }
        }
        if saw_hover_flyout {
            let cur_tid = unsafe { GetCurrentThreadId() };
            if let Ok(mut b) = rt.hover_flyout_bounds_dip.lock() {
                *b = if desired_hide {
                    measured_hover_flyout.map(|(r, _)| (cur_tid, r))
                } else {
                    None
                };
            }
        }
        if saw_osd {
            let cur_tid = unsafe { GetCurrentThreadId() };
            if let Ok(mut b) = rt.osd_bounds_dip.lock() {
                *b = if desired_hide && !osd_suppressed {
                    measured_osd.map(|(r, _)| (cur_tid, r))
                } else {
                    None
                };
            }
        }
        if saw_tooltip {
            let cur_tid = unsafe { GetCurrentThreadId() };
            if let Ok(mut b) = rt.tooltip_bounds_dip.lock() {
                *b = if desired_hide && !tooltip_suppressed {
                    measured_tooltip.map(|(r, _)| (cur_tid, r))
                } else {
                    None
                };
            }
        }
        if saw_menu_flyout {
            let cur_tid = unsafe { GetCurrentThreadId() };
            if let Ok(mut b) = rt.menu_flyout_bounds_by_tid.lock() {
                if desired_hide && !menu_flyout_suppressed && !measured_menu_flyouts.is_empty() {
                    b.insert(cur_tid, measured_menu_flyouts);
                } else {
                    b.remove(&cur_tid);
                }
            }
        }

        if !stale_handles.is_empty() {
            purge_saved_brushes_for_current_thread(&stale_handles);
            if let Ok(mut b) = rt.shell_card_bounds_by_handle.lock() {
                for &h in &stale_handles {
                    b.remove(&h);
                }
            }
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

        if !desired_hide {
            unsafe {
                restore_all_alttab_subtargets_for_current_thread(rt);
                restore_all_hover_flyout_shapes_for_current_thread(rt);
                restore_all_tooltip_targets_for_current_thread(rt);
                restore_all_menu_flyout_targets_for_current_thread(rt);
                restore_all_shell_card_targets_for_current_thread(rt);
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
        if matches!(kind, TargetKind::ShapeElement | TargetKind::HoverFlyoutShape) {
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

        let want_shape_hide_initial =
            matches!(kind, TargetKind::ShapeElement | TargetKind::HoverFlyoutShape)
                && rt.desired_hide();
        let want_focus_hide_initial = kind == TargetKind::AltTabItemFocus && rt.desired_hide();
        if want_shape_hide_initial {
            apply_shape_appearance(rt, insp.as_ptr(), true, orig, orig_vis);
            if kind == TargetKind::HoverFlyoutShape {
                rt.diag_flags
                    .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
            }
        } else if want_focus_hide_initial {
            let key = com_identity_key(insp.as_ptr());
            sync_focus_secondary_thickness_prop(rt, key, insp.as_ptr(), true, false);
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

        let cur_tid = GetCurrentThreadId();
        let mut tooltip_extra_disp: Option<OwnedDispatcher> = None;
        let mut menu_flyout_extra_disp: Option<OwnedDispatcher> = None;
        let (thread_id, dispatcher, async_dispatch) =
            if let Ok(dq) = DispatcherQueue::GetForCurrentThread() {
                let dq_for_enqueue = dq.clone();
                if kind == TargetKind::ToolTipPresenter {
                    tooltip_extra_disp =
                        Some(OwnedDispatcher::Queue(dq.clone().into_raw() as usize));
                } else if kind == TargetKind::MenuFlyoutSurface {
                    menu_flyout_extra_disp =
                        Some(OwnedDispatcher::Queue(dq.clone().into_raw() as usize));
                }
                (
                    Some(cur_tid),
                    OwnedDispatcher::Queue(dq.into_raw() as usize),
                    DispatchTarget::Queue {
                        thread_id: cur_tid,
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
                if kind == TargetKind::ToolTipPresenter {
                    if let Ok(extra_cookie) =
                        git.RegisterInterfaceInGlobal(cd_unk, &IID_ICOREDISPATCHER)
                    {
                        tooltip_extra_disp = Some(OwnedDispatcher::Core(extra_cookie));
                    }
                } else if matches!(
                    kind,
                    TargetKind::MenuFlyoutSurface
                        | TargetKind::ShellCardSurface
                        | TargetKind::ShellCardSubSurface
                ) {
                    if let Ok(extra_cookie) =
                        git.RegisterInterfaceInGlobal(cd_unk, &IID_ICOREDISPATCHER)
                    {
                        menu_flyout_extra_disp = Some(OwnedDispatcher::Core(extra_cookie));
                    }
                }
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
                if let Some(extra) = tooltip_extra_disp.take() {
                    if !g.tooltip_dispatchers.iter().any(|(tid, _)| *tid == cur_tid) {
                        g.tooltip_dispatchers.push((cur_tid, extra));
                    } else {
                        tooltip_extra_disp = Some(extra);
                    }
                }
                if let Some(extra) = menu_flyout_extra_disp.take() {
                    if !g.menu_flyout_dispatchers.iter().any(|(tid, _)| *tid == cur_tid) {
                        g.menu_flyout_dispatchers.push((cur_tid, extra));
                    } else {
                        menu_flyout_extra_disp = Some(extra);
                    }
                }
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

        if let Some(unused_extra) = tooltip_extra_disp {
            release_owned_dispatcher(unused_extra, Some(&git));
        }
        if let Some(unused_extra) = menu_flyout_extra_disp {
            release_owned_dispatcher(unused_extra, Some(&git));
        }
        if let Some(old) = evicted {
            release_target_resources(old, Some(&git));
        }

        if shutdown_after {
            if want_shape_hide_initial {
                apply_shape_appearance(rt, insp.as_ptr(), false, orig, orig_vis);
            } else if want_focus_hide_initial {
                let key = com_identity_key(insp.as_ptr());
                sync_focus_secondary_thickness_prop(rt, key, insp.as_ptr(), false, false);
            }
        } else if matches!(kind, TargetKind::ShapeElement | TargetKind::HoverFlyoutShape) {
            if want_hide_after != want_shape_hide_initial {
                apply_shape_appearance(rt, insp.as_ptr(), want_hide_after, orig, orig_vis);
                if want_hide_after && kind == TargetKind::HoverFlyoutShape {
                    rt.diag_flags
                        .fetch_or(DIAG_HOVER_FLYOUT_APPLIED, Ordering::Relaxed);
                }
            }
        } else if want_hide_after {
            let core_git = matches!(async_dispatch, DispatchTarget::Core { .. }).then_some(&git);
            rt.enqueue_target_dispatch(async_dispatch, core_git, None);
        } else if want_focus_hide_initial {
            let key = com_identity_key(insp.as_ptr());
            sync_focus_secondary_thickness_prop(rt, key, insp.as_ptr(), false, false);
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
            let hint_container_root = type_name.contains("TaskbarFrame")
                || type_name.contains("TaskbarBackground")
                || type_name.contains("FlyoutFrame")
                || matches!(
                    name.as_str(),
                    "HoverFlyoutCanvas"
                        | "HoverFlyoutGrid"
                        | "HoverFlyoutBackground"
                        | "HoverFlyoutBackgroundControl"
                )
                || name == "OverflowRootGrid"
                || name == "ModalRootGrid"
                || name == "AltTabRoot"
                || type_name.contains("Switcher.AltTab")
                || name == "BackgroundElement"
                || type_name.contains("SwitchItemList")
                || name == "CustomFocusItemPanel"
                || type_name.contains("NotificationAreaOverflow")
                || type_name.contains("NotifyIconOverflow")
                || type_name.contains("OverflowView")
                || name == "ConfirmatorFrameCanvas"
                || name == "ConfirmatorMainGrid"
                || type_name.contains("HWConfirmatorUI")
                || type_name.contains("ConfirmatorHostControl")
                || type_name.ends_with(".ToolTip")
                || type_name == "ToolTip"
                || type_name.contains("MenuFlyoutPresenter")
                || type_name.contains("JumpList")
                || type_name.contains("JumpView")
                || name == "JumpListRestyledAcrylic"
                || name == "JumpListGrid"
                || type_name.contains("TaskbarJumpListFrame")
                || type_name.contains("ControlCenter")
                || type_name.contains("NotificationCenter")
                || type_name.contains("CalendarCenter")
                || type_name.contains("ActionCenter")
                || type_name.contains("ToastNotificationView")
                || type_name.contains("FlexibleToastView")
                || type_name.contains("FlexibleNormalToastView")
                || type_name.contains("FlexiblePriorityToastView")
                || matches!(
                    name.as_str(),
                    "NotificationCenterGrid"
                        | "NotificationCenterView"
                        | "NotificationCenterPage"
                        | "CalendarCenterGrid"
                        | "CalendarHeaderMinimizedOverlay"
                        | "ControlCenterRegion"
                        | "RootGridBorder"
                        | "L1Grid"
                        | "L2Frame"
                        | "MainGrid"
                        | "MediaTransportControlsRegion"
                        | "MediaTransportControlsGrid"
                        | "BackgroundBlur"
                        | "FooterGrid"
                        | "PaneFooter"
                        | "Footer"
                        | "ToastBackgroundBorder2"
                        | "ToastBackgroundBorder"
                );
            let may_be_alttab_item_container = type_name.contains("ListViewItem");
            let mut is_container_root = hint_container_root;
            if hint_container_root {
                rt.diag_flags.fetch_or(DIAG_SAW_FRAME_OR_BG, Ordering::Relaxed);
            }
            if matches!(
                name.as_str(),
                "BackgroundFill" | "BackgroundStroke" | "ScreenEdgeStroke"
            ) {
                rt.diag_flags.fetch_or(DIAG_SAW_TARGET_NAME, Ordering::Relaxed);
            }
            let existing_flyout_on_tid: Vec<(u64, TargetKind)> = if is_container_root {
                rt.state
                    .lock()
                    .map(|g| {
                        g.targets
                            .iter()
                            .filter_map(|(&h, t)| {
                                (t.thread_id == Some(cur_tid)
                                    && matches!(
                                        t.kind,
                                        TargetKind::MenuFlyoutSurface
                                            | TargetKind::ShellCardSurface
                                            | TargetKind::ShellCardSubSurface
                                    ))
                                .then_some((h, t.kind))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let (matched_kind, extra_candidates, reclass_actions) = if let Ok(mut idx) =
                rt.tree.lock()
            {
                idx.on_add_on_thread(relation.parent, handle, type_name, name, cur_tid);
                if !is_container_root && may_be_alttab_item_container {
                    is_container_root = idx.thread_has_alttab_container(cur_tid);
                }
                let matched_kind = idx.classify_target_in_scope(handle, rt.host_scope);
                let extra = if is_container_root {
                    idx.collect_classified_targets_for_thread_in_scope(cur_tid, rt.host_scope)
                } else {
                    Vec::new()
                };
                let mut reclass = Vec::new();
                for (eh, old_kind) in existing_flyout_on_tid {
                    let new_kind = idx.classify_target_in_scope(eh, rt.host_scope);
                    if new_kind != Some(old_kind) {
                        reclass.push((eh, old_kind, new_kind));
                    }
                }
                (matched_kind, extra, reclass)
            } else {
                (None, Vec::new(), Vec::new())
            };
            if !reclass_actions.is_empty() {
                let mut dropped_targets = Vec::new();
                let mut downgraded_handles = Vec::new();
                let mut clear_bounds_handles = Vec::new();
                if let Ok(mut g) = rt.state.lock() {
                    for (eh, old_kind, new_kind) in reclass_actions {
                        if old_kind == TargetKind::ShellCardSurface
                            && new_kind != Some(TargetKind::ShellCardSurface)
                        {
                            clear_bounds_handles.push(eh);
                        }
                        match new_kind {
                            Some(nk) => {
                                if let Some(t) = g.targets.get_mut(&eh) {
                                    t.kind = nk;
                                    downgraded_handles.push((eh, nk));
                                }
                            }
                            None => {
                                if let Some(old) = g.targets.remove(&eh) {
                                    dropped_targets.push((eh, old));
                                }
                            }
                        }
                    }
                }
                if !clear_bounds_handles.is_empty() {
                    if let Ok(mut b) = rt.shell_card_bounds_by_handle.lock() {
                        for h in clear_bounds_handles {
                            b.remove(&h);
                        }
                    }
                }
                for (eh, nk) in downgraded_handles {
                    if let Ok(g) = rt.state.lock() {
                        if let Some(t) = g.targets.get(&eh) {
                            let hide_now = g.desired_hide && !g.shutdown;
                            let weak_raw = t.weak_ref_raw;
                            let op = t.orig_opacity;
                            let vis = t.orig_visibility;
                            drop(g);
                            if let Some(insp) = resolve_weak_raw_inspectable(weak_raw) {
                                apply_target_appearance(
                                    rt,
                                    eh,
                                    insp.as_ptr(),
                                    nk,
                                    hide_now,
                                    op,
                                    vis,
                                );
                            }
                        }
                    }
                }
                if !dropped_targets.is_empty() {
                    let git = create_git().ok();
                    for (eh, old) in dropped_targets {
                        if let Some(insp) = resolve_weak_raw_inspectable(old.weak_ref_raw) {
                            apply_target_appearance(
                                rt,
                                eh,
                                insp.as_ptr(),
                                old.kind,
                                false,
                                old.orig_opacity,
                                old.orig_visibility,
                            );
                        }
                        release_target_resources(old, git.as_ref());
                    }
                }
            }
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
                let mut removed_alt_tab = false;
                let mut removed_hover_flyout = false;
                let mut removed_osd = false;
                let mut removed_tooltip = false;
                let mut removed_menu_flyout = false;
                let mut remaining_menu_on_tid = false;
                let mut removed_shell_handles: Vec<u64> = Vec::new();
                let rem_tid = GetCurrentThreadId();
                if let Ok(mut g) = rt.state.lock() {
                    for rh in removed {
                        if let Some(old) = g.targets.remove(&rh) {
                            if old.kind == TargetKind::AltTabBorder {
                                removed_alt_tab = true;
                            } else if old.kind == TargetKind::HoverFlyoutBorder {
                                removed_hover_flyout = true;
                            } else if old.kind == TargetKind::OsdPanel {
                                removed_osd = true;
                            } else if old.kind == TargetKind::ToolTipPresenter {
                                removed_tooltip = true;
                            } else if old.kind == TargetKind::MenuFlyoutSurface {
                                removed_menu_flyout = true;
                            } else if old.kind == TargetKind::ShellCardSurface {
                                removed_shell_handles.push(rh);
                            }
                            evicted.push(old);
                        }
                    }
                    if removed_menu_flyout {
                        remaining_menu_on_tid = g.targets.values().any(|t| {
                            t.kind == TargetKind::MenuFlyoutSurface
                                && t.thread_id.is_none_or(|tid| tid == rem_tid)
                        });
                    }
                }
                if removed_alt_tab {
                    if let Ok(mut b) = rt.alt_tab_bounds_dip.lock() {
                        *b = None;
                    }
                }
                if removed_hover_flyout {
                    if let Ok(mut b) = rt.hover_flyout_bounds_dip.lock() {
                        *b = None;
                    }
                }
                if removed_osd {
                    if let Ok(mut b) = rt.osd_bounds_dip.lock() {
                        *b = None;
                    }
                }
                if removed_tooltip {
                    if let Ok(mut b) = rt.tooltip_bounds_dip.lock() {
                        *b = None;
                    }
                }
                if removed_menu_flyout && !remaining_menu_on_tid {
                    if let Ok(mut b) = rt.menu_flyout_bounds_by_tid.lock() {
                        b.remove(&rem_tid);
                    }
                }
                if !removed_shell_handles.is_empty() {
                    if let Ok(mut b) = rt.shell_card_bounds_by_handle.lock() {
                        for rh in removed_shell_handles {
                            b.remove(&rh);
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
            if let Some(reverse_pipe) = rt_pipe.reverse_pipe_name.clone() {
                run_reverse_pipe_client_loop(rt_pipe, reverse_pipe);
            } else {
                run_pipe_server_loop(rt_pipe);
            }
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

            let (leftover, leftover_tooltip_dispatchers, leftover_menu_dispatchers): (
                Vec<TrackedTarget>,
                Vec<OwnedDispatcher>,
                Vec<OwnedDispatcher>,
            ) = if let Ok(mut g) = rt.state.lock() {
                g.shutdown = true;
                (
                    g.targets.drain().map(|(_, t)| t).collect(),
                    g.tooltip_dispatchers
                        .drain(..)
                        .map(|(_, disp)| disp)
                        .collect(),
                    g.menu_flyout_dispatchers
                        .drain(..)
                        .map(|(_, disp)| disp)
                        .collect(),
                )
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };
            for t in leftover {
                release_target_resources(t, git.as_ref());
            }
            for disp in leftover_tooltip_dispatchers {
                release_owned_dispatcher(disp, git.as_ref());
            }
            for disp in leftover_menu_dispatchers {
                release_owned_dispatcher(disp, git.as_ref());
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

    unsafe fn read_pipe_exact_with_stop(
        pipe: HANDLE,
        event: HANDLE,
        stop_event: Option<HANDLE>,
        buf: &mut [u8],
        timeout_ms: u32,
    ) -> bool {
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
                let signaled_ok = match stop_event {
                    Some(se) if !se.is_invalid() => {
                        WaitForMultipleObjects(&[event, se], false, timeout_ms) == WAIT_OBJECT_0
                    }
                    _ => WaitForSingleObject(event, timeout_ms) == WAIT_OBJECT_0,
                };
                if !signaled_ok {
                    let _ = CancelIo(pipe);
                    let mut tr = 0u32;
                    let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
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

    unsafe fn read_pipe_exact(pipe: HANDLE, event: HANDLE, buf: &mut [u8]) -> bool {
        read_pipe_exact_with_stop(pipe, event, None, buf, 1000)
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
                    let mut tr = 0u32;
                    let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
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

    /// Processes a single incoming IPC command and returns `true` if `CMD_SHUTDOWN` was handled.
    unsafe fn handle_pipe_command(
        rt: &Arc<TapRuntime>,
        pipe: HANDLE,
        io_event: HANDLE,
        cmd: u8,
        host_pid: u32,
    ) -> bool {
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
                let _ = write_pipe_exact(pipe, io_event, &[CMD_VERSION, TAP_PROTOCOL_VERSION]);
            }
            CMD_ALTTAB_RECT => {
                let bounds = if rt.desired_hide() {
                    rt.query_alt_tab_bounds_dip(40)
                } else {
                    None
                };
                let mut resp = [0u8; 22];
                resp[0] = CMD_ALTTAB_RECT;
                if let Some((tid, r)) = bounds {
                    resp[1] = 1;
                    resp[2..6].copy_from_slice(&tid.to_le_bytes());
                    resp[6..10].copy_from_slice(&r[0].to_le_bytes());
                    resp[10..14].copy_from_slice(&r[1].to_le_bytes());
                    resp[14..18].copy_from_slice(&r[2].to_le_bytes());
                    resp[18..22].copy_from_slice(&r[3].to_le_bytes());
                }
                let _ = write_pipe_exact(pipe, io_event, &resp);
            }
            CMD_HOVER_FLYOUT_RECT => {
                let bounds = if rt.desired_hide() {
                    rt.query_hover_flyout_bounds_dip(40)
                } else {
                    None
                };
                let mut resp = [0u8; 22];
                resp[0] = CMD_HOVER_FLYOUT_RECT;
                if let Some((tid, r)) = bounds {
                    resp[1] = 1;
                    resp[2..6].copy_from_slice(&tid.to_le_bytes());
                    resp[6..10].copy_from_slice(&r[0].to_le_bytes());
                    resp[10..14].copy_from_slice(&r[1].to_le_bytes());
                    resp[14..18].copy_from_slice(&r[2].to_le_bytes());
                    resp[18..22].copy_from_slice(&r[3].to_le_bytes());
                }
                let _ = write_pipe_exact(pipe, io_event, &resp);
            }
            CMD_OSD_RECT | CMD_TOOLTIP_RECT => {
                let bounds = if cmd == CMD_OSD_RECT {
                    if rt.desired_hide() && !rt.is_osd_suppressed() {
                        rt.query_osd_bounds_dip(40)
                    } else {
                        None
                    }
                } else if rt.desired_hide() && !rt.is_tooltip_suppressed() {
                    rt.query_tooltip_bounds_dip(40)
                } else {
                    None
                };
                let mut resp = [0u8; 22];
                resp[0] = cmd;
                if let Some((tid, r)) = bounds {
                    resp[1] = 1;
                    resp[2..6].copy_from_slice(&tid.to_le_bytes());
                    resp[6..10].copy_from_slice(&r[0].to_le_bytes());
                    resp[10..14].copy_from_slice(&r[1].to_le_bytes());
                    resp[14..18].copy_from_slice(&r[2].to_le_bytes());
                    resp[18..22].copy_from_slice(&r[3].to_le_bytes());
                }
                let _ = write_pipe_exact(pipe, io_event, &resp);
            }
            CMD_MENU_FLYOUT_RECTS => {
                let rects = if rt.desired_hide() && !rt.is_menu_flyout_suppressed() {
                    rt.query_menu_flyout_bounds_dip(40)
                } else {
                    Vec::new()
                };
                let mut resp = [0u8; 2 + MAX_MENU_FLYOUT_RECTS * 20];
                resp[0] = CMD_MENU_FLYOUT_RECTS;
                let count = rects.len().min(MAX_MENU_FLYOUT_RECTS);
                resp[1] = count as u8;
                for (slot, &(tid, r)) in rects.iter().take(count).enumerate() {
                    let base = 2 + slot * 20;
                    resp[base..base + 4].copy_from_slice(&tid.to_le_bytes());
                    resp[base + 4..base + 8].copy_from_slice(&r[0].to_le_bytes());
                    resp[base + 8..base + 12].copy_from_slice(&r[1].to_le_bytes());
                    resp[base + 12..base + 16].copy_from_slice(&r[2].to_le_bytes());
                    resp[base + 16..base + 20].copy_from_slice(&r[3].to_le_bytes());
                }
                let _ = write_pipe_exact(pipe, io_event, &resp);
            }
            CMD_SHELL_CARD_RECTS => {
                let rects = if rt.desired_hide() && !rt.is_menu_flyout_suppressed() {
                    rt.query_shell_card_bounds_dip(40)
                } else {
                    Vec::new()
                };
                let mut resp = [0u8; 2 + MAX_SHELL_CARD_RECTS * 20];
                resp[0] = CMD_SHELL_CARD_RECTS;
                let count = rects.len().min(MAX_SHELL_CARD_RECTS);
                resp[1] = count as u8;
                for (slot, &(tid, r)) in rects.iter().take(count).enumerate() {
                    let base = 2 + slot * 20;
                    resp[base..base + 4].copy_from_slice(&tid.to_le_bytes());
                    resp[base + 4..base + 8].copy_from_slice(&r[0].to_le_bytes());
                    resp[base + 8..base + 12].copy_from_slice(&r[1].to_le_bytes());
                    resp[base + 12..base + 16].copy_from_slice(&r[2].to_le_bytes());
                    resp[base + 16..base + 20].copy_from_slice(&r[3].to_le_bytes());
                }
                let _ = write_pipe_exact(pipe, io_event, &resp);
            }
            CMD_SET_OSD_SUPPRESS | CMD_SET_TOOLTIP_SUPPRESS | CMD_SET_MENU_FLYOUT_SUPPRESS => {
                let mut extra = [0u8; 1];
                if read_pipe_exact(pipe, io_event, &mut extra) {
                    match cmd {
                        CMD_SET_OSD_SUPPRESS => {
                            rt.set_osd_suppressed(extra[0] != 0);
                        }
                        CMD_SET_TOOLTIP_SUPPRESS => {
                            rt.set_tooltip_suppressed(extra[0] != 0);
                        }
                        _ => {
                            rt.set_menu_flyout_suppressed(extra[0] != 0);
                        }
                    }
                    let _ = write_pipe_exact(pipe, io_event, &[cmd]);
                }
            }
            CMD_SET_ALTTAB_ITEM_STYLE
            | CMD_SET_OSD_FALLBACK_STYLE
            | CMD_SET_TOOLTIP_FALLBACK_STYLE
            | CMD_SET_MENU_FLYOUT_FALLBACK_STYLE => {
                let mut extra = [0u8; 8];
                if read_pipe_exact(pipe, io_event, &mut extra) {
                    let bg_argb = u32::from_le_bytes([extra[0], extra[1], extra[2], extra[3]]);
                    let border_argb = u32::from_le_bytes([extra[4], extra[5], extra[6], extra[7]]);
                    match cmd {
                        CMD_SET_ALTTAB_ITEM_STYLE => {
                            rt.set_alt_tab_item_colors(bg_argb, border_argb);
                        }
                        CMD_SET_OSD_FALLBACK_STYLE => {
                            rt.set_osd_fallback_colors(bg_argb, border_argb);
                        }
                        CMD_SET_TOOLTIP_FALLBACK_STYLE => {
                            rt.set_tooltip_fallback_colors(bg_argb, border_argb);
                        }
                        _ => {
                            rt.set_menu_flyout_fallback_colors(bg_argb, border_argb);
                        }
                    }
                    let _ = write_pipe_exact(pipe, io_event, &[cmd]);
                }
            }
            CMD_SHUTDOWN => {
                rt.trigger_shutdown(600);
                let _ = write_pipe_exact(pipe, io_event, &[cmd]);
                return true;
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
                let op_hr = rt.last_put_opacity_hr.load(Ordering::Acquire).to_le_bytes();
                let vis_hr = rt
                    .last_put_visibility_hr
                    .load(Ordering::Acquire)
                    .to_le_bytes();
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
        false
    }

    fn run_reverse_pipe_client_loop(rt: Arc<TapRuntime>, pipe_name: String) {
        unsafe {
            let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

            let Ok(io_event) = CreateEventW(None, true, false, None) else {
                if com_ok {
                    CoUninitialize();
                }
                return;
            };

            'outer: while !rt.stop_requested.load(Ordering::SeqCst) {
                let pipe = match CreateFileW(
                    PCWSTR(wide_name.as_ptr()),
                    GENERIC_READ.0 | GENERIC_WRITE.0,
                    FILE_SHARE_NONE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    None,
                ) {
                    Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE => h,
                    _ => {
                        if GetLastError() == ERROR_PIPE_BUSY {
                            let _ = WaitNamedPipeW(PCWSTR(wide_name.as_ptr()), 150);
                        } else {
                            let _ = WaitForSingleObject(rt.stop_event_handle(), 100);
                        }
                        continue;
                    }
                };

                loop {
                    if rt.stop_requested.load(Ordering::SeqCst) {
                        break;
                    }
                    let mut req = [0u8; 5];
                    if !read_pipe_exact_with_stop(
                        pipe,
                        io_event,
                        Some(rt.stop_event_handle()),
                        &mut req,
                        INFINITE,
                    ) {
                        break;
                    }
                    let cmd = req[0];
                    let host_pid = u32::from_le_bytes([req[1], req[2], req[3], req[4]]);
                    if handle_pipe_command(&rt, pipe, io_event, cmd, host_pid) {
                        let _ = CloseHandle(pipe);
                        break 'outer;
                    }
                }

                let _ = CloseHandle(pipe);
                if !rt.stop_requested.load(Ordering::SeqCst) {
                    rt.on_host_exited();
                }
            }

            let _ = CloseHandle(io_event);
            if com_ok {
                CoUninitialize();
            }
        }
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
                                let mut tr = 0u32;
                                let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
                                break false;
                            }
                            let w = WaitForMultipleObjects(&wait_handles[..count], false, INFINITE);
                            if w == WAIT_OBJECT_0 {
                                let _ = &ov;
                                let mut tr = 0u32;
                                break GetOverlappedResult(pipe, &ov, &mut tr, false).is_ok();
                            } else if w == WAIT_EVENT_INDEX(1) || rt.stop_requested.load(Ordering::SeqCst) {
                                let _ = CancelIo(pipe);
                                let mut tr = 0u32;
                                let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
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
                                let mut tr = 0u32;
                                let _ = GetOverlappedResult(pipe, &ov, &mut tr, true);
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
                    if handle_pipe_command(&rt, pipe, io_event, cmd, host_pid) {
                        let _ = DisconnectNamedPipe(pipe);
                        break;
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
