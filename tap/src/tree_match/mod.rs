use std::collections::{HashMap, HashSet, VecDeque};

mod explorer_rules;
mod shell_rules;

#[cfg(test)]
mod tests_explorer;
#[cfg(test)]
mod tests_shell;


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
pub const CMD_SET_VIRTUAL_DESKTOP_BAR_STYLE: u8 = 0x13;
pub const CMD_INPUT_SWITCHER_RECT: u8 = 0x14;
pub const CMD_SET_INPUT_SWITCHER_SUPPRESS: u8 = 0x15;
pub const CMD_SET_INPUT_SWITCHER_FALLBACK_STYLE: u8 = 0x16;
pub const CMD_SET_START_MODAL_STYLE: u8 = 0x17;
pub const MAX_MENU_FLYOUT_RECTS: usize = 3;
pub const MAX_SHELL_CARD_RECTS: usize = 8;
pub const TAP_PROTOCOL_VERSION: u8 = 15;
pub const ALTTAB_ITEM_OVERLAY_NAME: &str = "PrettyWindowsAltTabItemBg";
pub const START_MENU_OVERLAY_NAME: &str = "PrettyWindowsStartMenuOverlay";
pub const START_MODAL_OVERLAY_NAME: &str = "PrettyWindowsStartModalBg";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapHostScope {
    Explorer,
    ShellExperience,
    ShellHost,
    StartMenu,
    SearchHost,
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
    StartMenuModalSurface,
    VirtualDesktopBarSurface,
    InputSwitcherPanel,
    InputSwitcherSubPanel,
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
    children: HashMap<u64, Vec<u64>>,
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
        let prev_parent = self.nodes.get(&child).map(|m| m.parent);
        if let Some(old_parent) = prev_parent {
            if old_parent != parent && old_parent != 0 {
                if let Some(list) = self.children.get_mut(&old_parent) {
                    list.retain(|&c| c != child);
                    if list.is_empty() {
                        self.children.remove(&old_parent);
                    }
                }
            }
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
        if parent != 0 && parent != child && prev_parent != Some(parent) {
            let list = self.children.entry(parent).or_default();
            if !list.contains(&child) {
                list.push(child);
            }
        }
    }

    pub fn on_remove(&mut self, handle: u64) -> Vec<u64> {
        let mut removed = Vec::new();
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();
        if let Some(meta) = self.nodes.remove(&handle) {
            if meta.parent != 0 {
                if let Some(siblings) = self.children.get_mut(&meta.parent) {
                    siblings.retain(|&c| c != handle);
                    if siblings.is_empty() {
                        self.children.remove(&meta.parent);
                    }
                }
            }
            removed.push(handle);
            queue.push_back(handle);
            seen.insert(handle);
        } else {
            self.children.remove(&handle);
        }
        while let Some(parent_h) = queue.pop_front() {
            let Some(children) = self.children.remove(&parent_h) else {
                continue;
            };
            for ch in children {
                if seen.insert(ch) && self.nodes.remove(&ch).is_some() {
                    removed.push(ch);
                    queue.push_back(ch);
                }
            }
        }
        removed
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
            TapHostScope::StartMenu => self.classify_start_menu_target(handle),
            TapHostScope::SearchHost => self.classify_search_host_target(handle),
        }
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

pub fn start_menu_pipe_name_for_pid(pid: u32) -> String {
    format!(r"\\.\pipe\PrettyWindows_StartMenuTap_{pid}")
}

pub fn search_host_pipe_name_for_pid(pid: u32) -> String {
    format!(r"\\.\pipe\PrettyWindows_SearchHostTap_{pid}")
}
