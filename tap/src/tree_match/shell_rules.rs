use super::{
    NodeMeta, TargetKind, VisualTreeIndex, START_MENU_OVERLAY_NAME, START_MODAL_OVERLAY_NAME,
};

impl VisualTreeIndex {
    pub(super) fn is_forbidden_shell_card_item_node(meta: &NodeMeta) -> bool {
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

    pub(super) fn has_forbidden_shell_card_item_ancestor(&self, handle: u64) -> bool {
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

    pub(super) fn classify_shell_experience_target(&self, handle: u64) -> Option<TargetKind> {
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

    pub(super) fn classify_shell_host_target(&self, handle: u64) -> Option<TargetKind> {
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

    fn is_forbidden_start_menu_overlay_node(meta: &NodeMeta) -> bool {
        meta.name.starts_with("PrettyWindows")
            || meta.name == START_MENU_OVERLAY_NAME
            || meta.name == START_MODAL_OVERLAY_NAME
    }

    fn is_start_menu_modal_card_border(&self, node: &NodeMeta) -> bool {
        if node.name.starts_with("PrettyWindows")
            || !(node.name.is_empty()
                || node.name.contains("Background")
                || node.name.contains("Border"))
        {
            return false;
        }
        let Some(parent) = self.nodes.get(&node.parent) else {
            return false;
        };
        let parent_is_root_grid = parent.name == "Root"
            && (parent.type_name.ends_with(".Grid") || parent.type_name == "Grid");
        if !parent_is_root_grid || parent.parent == 0 {
            return false;
        }
        let Some(grandparent) = self.nodes.get(&parent.parent) else {
            return false;
        };
        grandparent.type_name.contains("FolderModal")
            || grandparent.type_name.contains("ExpandedFolderList")
            || matches!(
                grandparent.name.as_str(),
                "StartFolderModal" | "FolderModal" | "ExpandedFolderList"
            )
    }

    pub(super) fn classify_start_menu_target(&self, handle: u64) -> Option<TargetKind> {
        let node = self.nodes.get(&handle)?;
        if node.name.starts_with("PrettyWindows")
            || !self.has_ancestor_matching(
                handle,
                |m| {
                    Self::is_forbidden_shell_card_item_node(m)
                        || Self::is_forbidden_start_menu_overlay_node(m)
                },
                |_| false,
                false,
            )
        {
            return None;
        }
        let is_border = node.type_name.ends_with(".Border") || node.type_name == "Border";
        let is_grid = node.type_name.ends_with(".Grid") || node.type_name == "Grid";
        if !is_border && !is_grid {
            return None;
        }
        if is_border
            && self
                .nodes
                .get(&node.parent)
                .is_some_and(|p| p.type_name.contains("MenuFlyoutPresenter"))
        {
            return Some(TargetKind::MenuFlyoutSurface);
        }
        if is_border && self.is_start_menu_modal_card_border(node) {
            return Some(TargetKind::StartMenuModalSurface);
        }
        if node.name == "AcrylicBorder" {
            return Some(TargetKind::ShellCardSurface);
        }
        if matches!(
            node.name.as_str(),
            "AcrylicOverlay"
                | "StartDropShadow"
                | "LeftCompanionDropShadow"
                | "RightCompanionDropShadow"
        ) {
            return Some(TargetKind::ShellCardSubSurface);
        }
        None
    }

    fn is_forbidden_search_host_node(meta: &NodeMeta) -> bool {
        let t = meta.type_name.as_str();
        let n = meta.name.as_str();
        t.contains("RichSearchBox")
            || n.contains("RichSearchBox")
            || t.contains("SearchBox")
            || n.contains("SearchBox")
            || n == "TaskbarSearchBackground"
            || t.contains("RichComposerBox")
            || n.contains("ComposerBox")
            || t.contains("QueryFormulation")
            || n.contains("QueryFormulation")
            || t.contains("HostedWebView")
            || n.contains("HostedWebView")
            || n == "SuggestionsHost"
            || n == "AgentListView"
            || n == "HCOuterBorderGrid"
            || n == "HCBorder"
    }

    pub(super) fn classify_search_host_target(&self, handle: u64) -> Option<TargetKind> {
        let node = self.nodes.get(&handle)?;
        if self.has_forbidden_shell_card_item_ancestor(handle)
            || !self.has_ancestor_matching(
                handle,
                Self::is_forbidden_search_host_node,
                |_| false,
                false,
            )
        {
            return None;
        }
        let is_border = node.type_name.ends_with(".Border") || node.type_name == "Border";
        let is_grid = node.type_name.ends_with(".Grid") || node.type_name == "Grid";
        if !is_border && !is_grid {
            return None;
        }
        if is_border
            && self
                .nodes
                .get(&node.parent)
                .is_some_and(|p| p.type_name.contains("MenuFlyoutPresenter"))
        {
            return Some(TargetKind::MenuFlyoutSurface);
        }
        if matches!(node.name.as_str(), "AppBorder" | "AccentAppBorder") {
            return Some(TargetKind::ShellCardSurface);
        }
        if matches!(
            node.name.as_str(),
            "LayerBorder" | "AccentLayerBorder" | "dropshadow"
        ) {
            return Some(TargetKind::ShellCardSubSurface);
        }
        None
    }
}
