use super::{NodeMeta, TargetKind, VisualTreeIndex, ALTTAB_ITEM_OVERLAY_NAME};

impl VisualTreeIndex {
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

    pub(super) fn has_ancestor_matching(
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

    fn is_forbidden_virtual_desktop_bar_node(meta: &NodeMeta) -> bool {
        let t = meta.type_name.as_str();
        let n = meta.name.as_str();
        t.contains("VirtualDesktopListViewItem")
            || t.contains("VirtualDesktopElementThemed")
            || t.contains("NewVirtualDesktopElementThemed")
            || t.contains("RemoteVirtualDesktopElementThemed")
            || t.contains("SwitchItem")
            || n.contains("SwitchItem")
            || t.contains("Button")
            || t.contains("MenuFlyout")
            || t.contains("ToolTip")
            || t.contains("Tooltip")
            || t.contains("AltTab")
            || n.contains("AltTab")
            || t.contains("StartMenu")
            || t.contains("TaskbarFrame")
    }

    fn is_virtual_desktop_bar_container_node(meta: &NodeMeta) -> bool {
        meta.type_name.contains("VirtualDesktopBarElement")
            || meta.name == "VirtualDesktopBar"
            || meta.type_name.contains("Switcher.TaskView")
            || matches!(meta.name.as_str(), "TaskViewTimelineRoot" | "TaskViewRoot")
    }

    pub fn has_valid_virtual_desktop_bar_ancestor(&self, handle: u64) -> bool {
        self.has_ancestor_matching(
            handle,
            Self::is_forbidden_virtual_desktop_bar_node,
            Self::is_virtual_desktop_bar_container_node,
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

    fn is_forbidden_input_switcher_node(meta: &NodeMeta) -> bool {
        let t = meta.type_name.as_str();
        let n = meta.name.as_str();
        t.contains("InputSwitchListItem")
            || n.contains("InputSwitchListItem")
            || t.contains("ListViewItem")
            || n.contains("ListViewItem")
            || t.contains("ListView")
            || n.contains("ListView")
            || t.contains("Button")
            || n.contains("Button")
            || t.contains("ScrollViewer")
            || n.contains("ScrollViewer")
            || t.contains("ScrollBar")
            || n.contains("ScrollBar")
            || t.contains("ToolTip")
            || n.contains("ToolTip")
            || t.contains("Tooltip")
            || n.contains("Tooltip")
            || t.contains("MenuFlyout")
            || n.contains("MenuFlyout")
            || t.contains("Popup")
            || n.contains("Popup")
            || n == "HeaderPanel"
            || n == "Footer"
            || t.contains("StartMenu")
            || n.contains("StartMenu")
            || t.contains("TaskView")
            || n.contains("TaskView")
            || t.contains("AltTab")
            || n.contains("AltTab")
            || t.contains("Confirmator")
            || n.contains("Confirmator")
            || t.contains("HoverFlyout")
            || n.contains("HoverFlyout")
            || n == "OverflowRootGrid"
            || t.contains("TaskbarFrame")
            || n.contains("TaskbarFrame")
    }

    fn is_input_switcher_container_node(meta: &NodeMeta) -> bool {
        meta.type_name.contains("InputSwitcher") || meta.name == "FlatContent"
    }

    pub fn has_valid_input_switcher_ancestor(&self, handle: u64) -> bool {
        self.has_ancestor_matching(
            handle,
            Self::is_forbidden_input_switcher_node,
            Self::is_input_switcher_container_node,
            true,
        )
    }

    fn has_direct_child_named(&self, handle: u64, child_name: &str) -> bool {
        self.children.get(&handle).is_some_and(|list| {
            list.iter()
                .any(|ch| self.nodes.get(ch).is_some_and(|m| m.name == child_name))
        })
    }

    fn has_overlay_panel_ancestor(&self, handle: u64) -> bool {
        let mut cur = self.nodes.get(&handle).map(|n| n.parent).unwrap_or(0);
        let mut steps = 0usize;
        while cur != 0 && steps < 48 {
            if cur == handle {
                return false;
            }
            let Some(meta) = self.nodes.get(&cur) else {
                break;
            };
            if meta.name == "OverlayPanel" {
                return true;
            }
            cur = meta.parent;
            steps += 1;
        }
        false
    }



    pub(super) fn classify_explorer_target(&self, handle: u64) -> Option<TargetKind> {
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
        if matches!(
            node.name.as_str(),
            "VirtualDesktopSwitcherBackground" | "VirtualDesktopBarBackground"
        ) && (node.type_name.ends_with(".Border")
            || node.type_name == "Border"
            || node.type_name.ends_with(".Grid")
            || node.type_name == "Grid")
            && self.has_valid_virtual_desktop_bar_ancestor(handle)
        {
            return Some(TargetKind::VirtualDesktopBarSurface);
        }
        {
            let is_border = node.type_name.ends_with(".Border") || node.type_name == "Border";
            let is_grid = node.type_name.ends_with(".Grid") || node.type_name == "Grid";
            if (is_grid || is_border)
                && self.has_valid_input_switcher_ancestor(handle)
                && !self.has_overlay_panel_ancestor(handle)
            {
                if node.name == "OverlayPanel" {
                    return Some(TargetKind::InputSwitcherSubPanel);
                }
                if self.has_direct_child_named(handle, "OverlayPanel")
                    || (self.has_direct_child_named(handle, "HeaderPanel")
                        && self.has_direct_child_named(handle, "Footer"))
                {
                    return Some(TargetKind::InputSwitcherPanel);
                }
            }
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

    pub(super) fn classify_jumplist_target(&self, handle: u64) -> Option<TargetKind> {
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

}
