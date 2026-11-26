//! Collection & FolderPortal item listing, search filtering, sorting, async scanning, and file watcher.

use super::{
    BoxDisplayItem, BoxInner, DirScanPayload, DragAction, InlineEditMode, FILE_ATTR_HIDDEN,
    FILE_ATTR_SYSTEM, WM_APP_DESKBOX_DIR_CHANGED, WM_APP_DESKBOX_DIR_LOADED,
};
use crate::deskbox::config::{
    compare_item_for_sort, normalize_path_key, BoxKind, BoxSortMode, DeskBoxSpec,
};
use crate::deskbox::icons::IconService;
use crate::deskbox::watcher::FolderWatcher;
use crate::ui::Spring1D;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, PostMessageW};

impl BoxInner {
    pub(super) fn apply_search_filter(&mut self) {
        self.bump_content_rev();
        let query = self.search_query.trim().to_lowercase();
        if !self.search_active || query.is_empty() {
            self.items = self.all_items.clone();
        } else {
            self.items = self
                .all_items
                .iter()
                .filter(|it| it.name_lower.contains(&query))
                .cloned()
                .collect();
        }
        self.selected.retain(|&idx| idx < self.items.len());
        if self.selection_anchor.is_some_and(|a| a >= self.items.len()) {
            self.selection_anchor = None;
        }
        let layout = self.current_layout();
        self.scroll_y = layout.scroll_y;
        self.scroll_target_y = layout.scroll_y;
        self.scroll_spring = Spring1D::still(layout.scroll_y);
        self.scroll_animating = false;
    }

    pub(super) fn clear_search(&mut self) {
        let was_search_edit = self.inline_edit_mode == Some(InlineEditMode::Search);
        if was_search_edit {
            self.cancel_inline_rename();
        }
        if self.search_active || !self.search_query.is_empty() {
            self.search_active = false;
            self.search_query.clear();
            self.apply_search_filter();
            self.sync_header_opacity_target();
            let _ = self.render();
        }
    }

    pub(super) fn has_missing_items(&self) -> bool {
        self.spec.kind == BoxKind::Collection && self.all_items.iter().any(|it| !it.exists)
    }

    fn current_dir_saved_order(&self) -> Vec<String> {
        match self.spec.kind {
            BoxKind::Collection => self.spec.items.clone(),
            BoxKind::FolderPortal => {
                if self.nav_stack.is_empty() {
                    self.spec.items.clone()
                } else {
                    let key = normalize_path_key(&self.current_portal_dir().to_string_lossy());
                    self.dir_manual_orders.get(&key).cloned().unwrap_or_default()
                }
            }
        }
    }

    fn save_current_dir_order(&mut self, ordered_paths: Vec<String>) {
        match self.spec.kind {
            BoxKind::Collection => {
                self.spec.items = ordered_paths;
                self.persist_to_host();
            }
            BoxKind::FolderPortal => {
                if self.nav_stack.is_empty() {
                    self.spec.items = ordered_paths;
                    self.persist_to_host();
                } else {
                    let key = normalize_path_key(&self.current_portal_dir().to_string_lossy());
                    self.dir_manual_orders.insert(key, ordered_paths);
                }
            }
        }
    }

    /// Executes a one-shot sort on all current items and updates the active item order without
    /// persisting any sticky sort mode.
    pub(super) fn apply_one_shot_sort(&mut self, sort: BoxSortMode) {
        self.all_items.sort_by(|a, b| {
            compare_item_for_sort(
                sort,
                a.is_dir,
                &a.path,
                &a.name,
                a.modified_secs,
                b.is_dir,
                &b.path,
                &b.name,
                b.modified_secs,
            )
        });
        let ordered_paths: Vec<String> = self
            .all_items
            .iter()
            .map(|it| it.path.to_string_lossy().into_owned())
            .collect();
        self.save_current_dir_order(ordered_paths);
        self.apply_search_filter();
        let _ = self.render();
    }

    /// Reorders one item within the current box (works for both Collection and FolderPortal).
    pub(super) fn reorder_item_in_place(&mut self, from_idx: usize, to_idx: usize) -> bool {
        let ordered_paths: Vec<String> = self
            .all_items
            .iter()
            .map(|it| it.path.to_string_lossy().into_owned())
            .collect();
        let Some(new_order) = DeskBoxSpec::reorder_item_paths(&ordered_paths, from_idx, to_idx)
        else {
            return false;
        };
        let clamped_to = to_idx.min(self.all_items.len() - 1);
        let moved = self.all_items.remove(from_idx);
        self.all_items.insert(clamped_to, moved);
        self.save_current_dir_order(new_order);
        self.apply_search_filter();
        if clamped_to < self.items.len() {
            self.selected.clear();
            self.selected.insert(clamped_to);
            self.selection_anchor = Some(clamped_to);
        }
        true
    }

    pub(super) fn apply_dir_scan_result(&mut self, payload: DirScanPayload) {
        if self.spec.kind != BoxKind::FolderPortal || payload.gen_id != self.dir_scan_gen {
            return;
        }
        let was_loading = self.dir_scan_loading;
        let prev_missing = self.portal_dir_missing;
        self.dir_scan_loading = false;
        self.portal_dir_missing = payload.dir_missing;

        // Remember currently selected paths so background directory refreshes do not drop selection.
        let prev_selected_keys: HashSet<String> = self
            .selected
            .iter()
            .filter_map(|&idx| self.items.get(idx))
            .map(|it| normalize_path_key(&it.path.to_string_lossy()))
            .collect();

        let saved = self.current_dir_saved_order();
        let next_items = if saved.is_empty() {
            payload.items
        } else {
            let order_map: HashMap<String, usize> = saved
                .iter()
                .enumerate()
                .map(|(i, p)| (normalize_path_key(p), i))
                .collect();
            let mut items = payload.items;
            items.sort_by(|a, b| {
                let ka = normalize_path_key(&a.path.to_string_lossy());
                let kb = normalize_path_key(&b.path.to_string_lossy());
                match (order_map.get(&ka), order_map.get(&kb)) {
                    (Some(ia), Some(ib)) => ia.cmp(ib),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => compare_item_for_sort(
                        BoxSortMode::TypeAsc,
                        a.is_dir,
                        &a.path,
                        &a.name,
                        a.modified_secs,
                        b.is_dir,
                        &b.path,
                        &b.name,
                        b.modified_secs,
                    ),
                }
            });
            items
        };

        let unchanged = !was_loading
            && prev_missing == self.portal_dir_missing
            && self.all_items.len() == next_items.len()
            && self
                .all_items
                .iter()
                .zip(next_items.iter())
                .all(|(a, b)| a.path == b.path && a.name == b.name && a.is_dir == b.is_dir);
        if unchanged {
            return;
        }

        // Cancel item-index-based drags so a background directory refresh cannot reorder stale indices.
        if matches!(
            self.drag,
            Some(
                DragAction::ReorderItem { .. }
                    | DragAction::PendingItemDrag { .. }
                    | DragAction::PendingRemoveClick { .. }
            )
        ) {
            self.drag = None;
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            }
        }

        self.all_items = next_items;
        self.apply_search_filter();
        if !prev_selected_keys.is_empty() {
            self.selected.clear();
            for (idx, it) in self.items.iter().enumerate() {
                if prev_selected_keys.contains(&normalize_path_key(&it.path.to_string_lossy())) {
                    self.selected.insert(idx);
                }
            }
            self.selection_anchor = self.selected.iter().copied().min();
        }
        let _ = self.render();
    }

    pub(super) fn reload_items(&mut self) {
        self.portal_dir_missing = false;

        match self.spec.kind {
            BoxKind::Collection => {
                self.selected.clear();
                self.selection_anchor = None;
                self.dir_scan_loading = false;
                self.all_items.clear();
                self.items.clear();
                self.drop_shared.is_collection.store(true, Ordering::Relaxed);
                for raw in &self.spec.items {
                    let path = PathBuf::from(raw);
                    let meta = fs::metadata(&path).ok();
                    let exists = meta.is_some();
                    let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
                    let modified_secs = meta
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_secs());
                    let name = display_name_for_path(&path, raw);
                    let name_lower = name.to_lowercase();
                    let (icon_key_l, icon_key_j) = IconService::cache_key_pair(&path);
                    self.all_items.push(BoxDisplayItem {
                        path,
                        name,
                        name_lower,
                        icon_key_l,
                        icon_key_j,
                        is_dir,
                        exists,
                        modified_secs,
                    });
                }
                self.apply_search_filter();
            }
            BoxKind::FolderPortal => {
                self.drop_shared.is_collection.store(false, Ordering::Relaxed);
                let dir = self.current_portal_dir();
                self.dir_scan_gen = self.dir_scan_gen.wrapping_add(1);
                let gen_id = self.dir_scan_gen;
                let hwnd_raw = self.hwnd.0 as isize;
                let result_slot = Arc::clone(&self.dir_scan_result);
                if self.all_items.is_empty() {
                    self.dir_scan_loading = true;
                }
                let _ = std::thread::Builder::new()
                    .name("DeskBoxDirScan".into())
                    .spawn(move || {
                        let (mut loaded, dir_missing) = scan_portal_directory(&dir);
                        loaded.sort_by(|a, b| {
                            compare_item_for_sort(
                                BoxSortMode::TypeAsc,
                                a.is_dir,
                                &a.path,
                                &a.name,
                                a.modified_secs,
                                b.is_dir,
                                &b.path,
                                &b.name,
                                b.modified_secs,
                            )
                        });
                        if let Ok(mut guard) = result_slot.lock() {
                            if guard.as_ref().is_none_or(|prev| prev.gen_id <= gen_id) {
                                *guard = Some(DirScanPayload {
                                    gen_id,
                                    items: loaded,
                                    dir_missing,
                                });
                            }
                        }
                        let target = HWND(hwnd_raw as *mut core::ffi::c_void);
                        unsafe {
                            if !target.is_invalid() && IsWindow(target).as_bool() {
                                let _ = PostMessageW(
                                    target,
                                    WM_APP_DESKBOX_DIR_LOADED,
                                    WPARAM(gen_id),
                                    LPARAM(0),
                                );
                            }
                        }
                    });
            }
        }
    }

    pub(super) fn restart_watcher(&mut self) {
        self.watcher = None;
        if self.spec.kind != BoxKind::FolderPortal {
            return;
        }
        self.watch_gen = self.watch_gen.wrapping_add(1);
        let dir = self.current_portal_dir();
        self.watcher =
            FolderWatcher::start(&dir, self.hwnd, WM_APP_DESKBOX_DIR_CHANGED, self.watch_gen);
    }

    pub(super) fn deduplicate_and_add_collection(&mut self, incoming: &[PathBuf]) -> bool {
        if self.spec.kind != BoxKind::Collection {
            return false;
        }
        let strings: Vec<String> = incoming
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let added = self.spec.dedup_add_items(&strings);
        if added > 0 {
            self.reload_items();
            self.persist_to_host();
            let _ = self.render();
            true
        } else {
            false
        }
    }

    pub(super) fn remove_collection_indices(&mut self, indices: &[usize]) -> bool {
        if self.spec.kind != BoxKind::Collection || indices.is_empty() {
            return false;
        }
        let targets: Vec<String> = indices
            .iter()
            .filter_map(|&idx| self.items.get(idx))
            .map(|it| it.path.to_string_lossy().into_owned())
            .collect();
        if self.spec.remove_items(&targets) > 0 {
            self.reload_items();
            self.persist_to_host();
            let _ = self.render();
            true
        } else {
            false
        }
    }

    pub(super) fn clean_missing_collection_items(&mut self) -> bool {
        if self.spec.kind != BoxKind::Collection {
            return false;
        }
        let before = self.spec.items.len();
        self.spec.items.retain(|s| Path::new(s).exists());
        if self.spec.items.len() != before {
            self.reload_items();
            self.persist_to_host();
            let _ = self.render();
            true
        } else {
            false
        }
    }
}

fn scan_portal_directory(dir: &Path) -> (Vec<BoxDisplayItem>, bool) {
    match fs::read_dir(dir) {
        Ok(rd) => {
            let mut loaded = Vec::new();
            for entry in rd.flatten() {
                let path = entry.path();
                let os_name = entry.file_name();
                let name_lossy = os_name.to_string_lossy();
                if name_lossy.eq_ignore_ascii_case("desktop.ini")
                    || name_lossy.eq_ignore_ascii_case("thumbs.db")
                {
                    continue;
                }
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let attrs = meta.file_attributes();
                if (attrs & (FILE_ATTR_HIDDEN | FILE_ATTR_SYSTEM)) != 0 {
                    continue;
                }
                let is_dir = meta.is_dir();
                let modified_secs = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                let name = display_name_for_path(&path, &name_lossy);
                let name_lower = name.to_lowercase();
                let (icon_key_l, icon_key_j) = IconService::cache_key_pair(&path);
                loaded.push(BoxDisplayItem {
                    path,
                    name,
                    name_lower,
                    icon_key_l,
                    icon_key_j,
                    is_dir,
                    exists: true,
                    modified_secs,
                });
            }
            (loaded, false)
        }
        Err(_) => (Vec::new(), true),
    }
}

fn display_name_for_path(path: &Path, fallback: &str) -> String {
    let raw = path
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback);
    let lower = raw.to_ascii_lowercase();
    if (lower.ends_with(".lnk") || lower.ends_with(".url")) && raw.len() > 4 {
        raw[..raw.len() - 4].to_string()
    } else {
        raw.to_string()
    }
}
