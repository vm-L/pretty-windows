//! Configuration persistence, box specifications, and tray action schema for DeskBox.

use super::layout::{BOX_DEFAULT_HEIGHT_DIP, BOX_DEFAULT_WIDTH_DIP};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Storage behavior mode of a desktop organizer box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BoxKind {
    /// Stores lightweight path references in `DeskBox.toml` without moving original files.
    #[default]
    Collection,
    /// Mirrors a real directory on disk with live bidirectional updates and in-place subfolder drill-down.
    FolderPortal,
}

impl BoxKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Collection => "collection",
            Self::FolderPortal => "folder_portal",
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "collection" | "free" => Some(Self::Collection),
            "folder_portal" | "folder" | "portal" => Some(Self::FolderPortal),
            _ => None,
        }
    }
}

/// Visual layout mode for items inside a desktop organizer box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BoxViewMode {
    LargeIcons,
    #[default]
    MediumIcons,
    SmallIcons,
    CompactList,
}

impl BoxViewMode {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const ALL: [BoxViewMode; 4] = [
        BoxViewMode::LargeIcons,
        BoxViewMode::MediumIcons,
        BoxViewMode::SmallIcons,
        BoxViewMode::CompactList,
    ];

    #[cfg_attr(not(windows), allow(dead_code))]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LargeIcons => "大图标",
            Self::MediumIcons => "中图标",
            Self::SmallIcons => "小图标",
            Self::CompactList => "紧凑列表",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LargeIcons => "large_icons",
            Self::MediumIcons => "medium_icons",
            Self::SmallIcons => "small_icons",
            Self::CompactList => "compact_list",
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "large_icons" | "large" => Some(Self::LargeIcons),
            "medium_icons" | "medium" => Some(Self::MediumIcons),
            "small_icons" | "small" => Some(Self::SmallIcons),
            "compact_list" | "list" | "compact" => Some(Self::CompactList),
            _ => None,
        }
    }
}

/// One-shot item sort command applied on demand to entries inside a box (never persisted).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BoxSortMode {
    #[default]
    TypeAsc,
    NameAsc,
    NameDesc,
    ModifiedDesc,
}

impl BoxSortMode {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const ALL: [BoxSortMode; 4] = [
        BoxSortMode::TypeAsc,
        BoxSortMode::NameAsc,
        BoxSortMode::NameDesc,
        BoxSortMode::ModifiedDesc,
    ];

    #[cfg_attr(not(windows), allow(dead_code))]
    pub const fn label(self) -> &'static str {
        match self {
            Self::TypeAsc => "按类型排序",
            Self::NameAsc => "按名称升序",
            Self::NameDesc => "按名称降序",
            Self::ModifiedDesc => "按修改时间",
        }
    }
}

/// Actions exposed in the system tray cascading submenu for DeskBox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum DeskBoxTrayAction {
    ToggleEnabled,
    ToggleHideDesktopIcons,
    AutoFitAll,
    LockAll,
    UnlockAll,
    SetDefaultView(BoxViewMode),
}

impl DeskBoxTrayAction {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const ALL: [DeskBoxTrayAction; 9] = [
        DeskBoxTrayAction::ToggleEnabled,
        DeskBoxTrayAction::ToggleHideDesktopIcons,
        DeskBoxTrayAction::AutoFitAll,
        DeskBoxTrayAction::LockAll,
        DeskBoxTrayAction::UnlockAll,
        DeskBoxTrayAction::SetDefaultView(BoxViewMode::LargeIcons),
        DeskBoxTrayAction::SetDefaultView(BoxViewMode::MediumIcons),
        DeskBoxTrayAction::SetDefaultView(BoxViewMode::SmallIcons),
        DeskBoxTrayAction::SetDefaultView(BoxViewMode::CompactList),
    ];

    #[cfg_attr(not(windows), allow(dead_code))]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::ToggleEnabled => 1,
            Self::ToggleHideDesktopIcons => 2,
            Self::AutoFitAll => 3,
            Self::LockAll => 4,
            Self::UnlockAll => 5,
            Self::SetDefaultView(BoxViewMode::LargeIcons) => 10,
            Self::SetDefaultView(BoxViewMode::MediumIcons) => 11,
            Self::SetDefaultView(BoxViewMode::SmallIcons) => 12,
            Self::SetDefaultView(BoxViewMode::CompactList) => 13,
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::ToggleEnabled),
            2 => Some(Self::ToggleHideDesktopIcons),
            3 => Some(Self::AutoFitAll),
            4 => Some(Self::LockAll),
            5 => Some(Self::UnlockAll),
            10 => Some(Self::SetDefaultView(BoxViewMode::LargeIcons)),
            11 => Some(Self::SetDefaultView(BoxViewMode::MediumIcons)),
            12 => Some(Self::SetDefaultView(BoxViewMode::SmallIcons)),
            13 => Some(Self::SetDefaultView(BoxViewMode::CompactList)),
            _ => None,
        }
    }
}

/// Persistent definition of one desktop organizer box.
#[derive(Clone, Debug, PartialEq)]
pub struct DeskBoxSpec {
    pub id: u64,
    pub kind: BoxKind,
    pub title: String,
    pub folder_path: String,
    pub monitor_id: String,
    pub x_dip: f32,
    pub y_dip: f32,
    pub w_dip: f32,
    pub h_dip: f32,
    pub view_mode: BoxViewMode,
    pub collapsed: bool,
    pub edge_auto_hide: bool,
    pub auto_hide_header: bool,
    pub locked: bool,
    pub items: Vec<String>,
}

impl DeskBoxSpec {
    pub fn new_collection(id: u64, title: impl Into<String>, x_dip: f32, y_dip: f32) -> Self {
        Self {
            id,
            kind: BoxKind::Collection,
            title: title.into(),
            folder_path: String::new(),
            monitor_id: String::new(),
            x_dip,
            y_dip,
            w_dip: BOX_DEFAULT_WIDTH_DIP,
            h_dip: BOX_DEFAULT_HEIGHT_DIP,
            view_mode: BoxViewMode::MediumIcons,
            collapsed: false,
            edge_auto_hide: false,
            auto_hide_header: false,
            locked: false,
            items: Vec::new(),
        }
    }

    pub fn new_folder_portal(
        id: u64,
        folder_path: impl Into<String>,
        x_dip: f32,
        y_dip: f32,
    ) -> Self {
        let folder_path = folder_path.into();
        let title = folder_portal_display_name(&folder_path);
        Self {
            id,
            kind: BoxKind::FolderPortal,
            title,
            folder_path,
            monitor_id: String::new(),
            x_dip,
            y_dip,
            w_dip: BOX_DEFAULT_WIDTH_DIP,
            h_dip: BOX_DEFAULT_HEIGHT_DIP,
            view_mode: BoxViewMode::MediumIcons,
            collapsed: false,
            edge_auto_hide: false,
            auto_hide_header: false,
            locked: false,
            items: Vec::new(),
        }
    }

    /// Appends unique normalized paths into a collection box and returns how many were added.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn dedup_add_items(&mut self, paths: &[String]) -> usize {
        if self.kind != BoxKind::Collection {
            return 0;
        }
        let mut seen: HashSet<String> = self
            .items
            .iter()
            .map(|p| normalize_path_key(p))
            .collect();
        let mut added = 0;
        for raw in paths {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let key = normalize_path_key(trimmed);
            if seen.insert(key) {
                self.items.push(trimmed.to_string());
                added += 1;
            }
        }
        added
    }

    /// Removes matching paths from a collection box and returns the removed count.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn remove_items(&mut self, paths: &[String]) -> usize {
        if self.kind != BoxKind::Collection || paths.is_empty() {
            return 0;
        }
        let targets: HashSet<String> = paths.iter().map(|p| normalize_path_key(p)).collect();
        let before = self.items.len();
        self.items
            .retain(|item| !targets.contains(&normalize_path_key(item)));
        before - self.items.len()
    }

    /// Moves item at `from_idx` to `to_idx` inside `ordered_paths` and stores the resulting order.
    /// Returns `Some(new_order)` when the order actually changed.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn reorder_item_paths(
        ordered_paths: &[String],
        from_idx: usize,
        to_idx: usize,
    ) -> Option<Vec<String>> {
        if from_idx >= ordered_paths.len() || ordered_paths.len() < 2 {
            return None;
        }
        let clamped_to = to_idx.min(ordered_paths.len() - 1);
        if from_idx == clamped_to {
            return None;
        }
        let mut next = ordered_paths.to_vec();
        let moved = next.remove(from_idx);
        next.insert(clamped_to, moved);
        Some(next)
    }
}

/// Global user configuration for the DeskBox module, stored in `<exe_dir>/DeskBox.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct DeskBoxConfig {
    pub enabled: bool,
    pub hide_desktop_icons: bool,
    pub default_view_mode: BoxViewMode,
    pub next_box_id: u64,
    pub boxes: Vec<DeskBoxSpec>,
}

impl Default for DeskBoxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            hide_desktop_icons: false,
            default_view_mode: BoxViewMode::MediumIcons,
            next_box_id: 1,
            boxes: Vec::new(),
        }
    }
}

impl DeskBoxConfig {
    /// Allocates the next unique non-zero box ID.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn alloc_box_id(&mut self) -> u64 {
        let max_existing = self.boxes.iter().map(|b| b.id).max().unwrap_or(0);
        let id = self.next_box_id.max(max_existing.saturating_add(1)).max(1);
        self.next_box_id = id.saturating_add(1);
        id
    }

    /// Computes a gentle cascade `(x_dip, y_dip)` origin within `[work_left..work_right, work_top..work_bottom]`.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn default_cascade_origin(
        box_count: usize,
        work_left: i32,
        work_top: i32,
        work_right: i32,
        work_bottom: i32,
        dpi: u32,
    ) -> (f32, f32) {
        let s = (dpi.max(96) as f32) / 96.0;
        let work_w_dip = ((work_right - work_left).max(320) as f32) / s;
        let work_h_dip = ((work_bottom - work_top).max(240) as f32) / s;
        let col = (box_count / 4) as f32;
        let row = (box_count % 4) as f32;
        let raw_x = 42.0 + row * 28.0 + col * 368.0;
        let raw_y = 42.0 + row * 28.0;
        let max_x = (work_w_dip - 360.0).max(16.0);
        let max_y = (work_h_dip - 320.0).max(16.0);
        (raw_x.clamp(16.0, max_x), raw_y.clamp(16.0, max_y))
    }
}

#[inline]
pub(crate) fn normalize_path_key(raw: &str) -> String {
    let mut s = raw.trim().replace('/', "\\");
    while s.ends_with('\\') {
        // Keep drive roots such as `C:\` and bare root `\` intact.
        let bytes = s.as_bytes();
        if s.len() == 1 || (s.len() == 3 && bytes[1] == b':') {
            break;
        }
        s.pop();
    }
    s.to_lowercase()
}

/// Derives the fixed display title of a `FolderPortal` box directly from its linked `folder_path`.
pub fn folder_portal_display_name(folder_path: &str) -> String {
    let trimmed = folder_path.trim();
    if trimmed.is_empty() {
        return "关联文件夹".to_string();
    }
    let stripped = trimmed.trim_end_matches(['\\', '/']);
    if stripped.is_empty() {
        return trimmed.to_string();
    }
    // Preserve drive roots such as `C:\` cleanly.
    if stripped.len() == 2 && stripped.as_bytes()[1] == b':' {
        return format!("{stripped}\\");
    }
    if let Some(leaf) = stripped.rsplit(['\\', '/']).find(|seg| !seg.is_empty()) {
        return leaf.to_string();
    }
    stripped.to_string()
}

/// Compares two DeskBox items according to one-shot sort `mode`.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(windows), allow(dead_code))]
pub fn compare_item_for_sort(
    mode: BoxSortMode,
    a_is_dir: bool,
    a_path: &Path,
    a_name: &str,
    a_modified_secs: u64,
    b_is_dir: bool,
    b_path: &Path,
    b_name: &str,
    b_modified_secs: u64,
) -> std::cmp::Ordering {
    match mode {
        BoxSortMode::TypeAsc => {
            compare_item_type_and_name(a_is_dir, a_path, a_name, b_is_dir, b_path, b_name)
        }
        BoxSortMode::NameAsc => b_is_dir
            .cmp(&a_is_dir)
            .then_with(|| compare_natural_ci(a_name, b_name))
            .then_with(|| a_name.cmp(b_name))
            .then_with(|| {
                normalize_path_key(&a_path.to_string_lossy())
                    .cmp(&normalize_path_key(&b_path.to_string_lossy()))
            }),
        BoxSortMode::NameDesc => b_is_dir
            .cmp(&a_is_dir)
            .then_with(|| compare_natural_ci(b_name, a_name))
            .then_with(|| b_name.cmp(a_name))
            .then_with(|| {
                normalize_path_key(&b_path.to_string_lossy())
                    .cmp(&normalize_path_key(&a_path.to_string_lossy()))
            }),
        BoxSortMode::ModifiedDesc => b_is_dir
            .cmp(&a_is_dir)
            .then_with(|| b_modified_secs.cmp(&a_modified_secs))
            .then_with(|| compare_natural_ci(a_name, b_name))
            .then_with(|| a_name.cmp(b_name)),
    }
}

/// Compares two DeskBox items in ascending `type > name` order:
/// 1. Directories before files.
/// 2. Files grouped by lowercase extension ascending (`""` < `"docx"` < `"exe"` < `"png"` < `"txt"`).
/// 3. Case-insensitive natural ascending order on display name (`file2` < `file10`).
/// 4. Deterministic tie-breakers on exact display name and normalized path.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn compare_item_type_and_name(
    a_is_dir: bool,
    a_path: &Path,
    a_name: &str,
    b_is_dir: bool,
    b_path: &Path,
    b_name: &str,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match b_is_dir.cmp(&a_is_dir) {
        Ordering::Equal => {}
        ord => return ord,
    }

    if !a_is_dir {
        let ext_a = a_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let ext_b = b_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext_a.cmp(&ext_b) {
            Ordering::Equal => {}
            ord => return ord,
        }
    }

    match compare_natural_ci(a_name, b_name) {
        Ordering::Equal => {}
        ord => return ord,
    }

    match a_name.cmp(b_name) {
        Ordering::Equal => normalize_path_key(&a_path.to_string_lossy())
            .cmp(&normalize_path_key(&b_path.to_string_lossy())),
        ord => ord,
    }
}

fn compare_natural_ci(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let mut ia = a.chars().peekable();
    let mut ib = b.chars().peekable();

    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let mut da = String::new();
                while let Some(&c) = ia.peek() {
                    if c.is_ascii_digit() {
                        da.push(c);
                        ia.next();
                    } else {
                        break;
                    }
                }
                let mut db = String::new();
                while let Some(&c) = ib.peek() {
                    if c.is_ascii_digit() {
                        db.push(c);
                        ib.next();
                    } else {
                        break;
                    }
                }
                let ta = da.trim_start_matches('0');
                let tb = db.trim_start_matches('0');
                match ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)) {
                    Ordering::Equal => match da.len().cmp(&db.len()) {
                        Ordering::Equal => {}
                        ord => return ord,
                    },
                    ord => return ord,
                }
            }
            (Some(ca), Some(cb)) => {
                ia.next();
                ib.next();
                let mut la = ca.to_lowercase();
                let mut lb = cb.to_lowercase();
                loop {
                    match (la.next(), lb.next()) {
                        (None, None) => break,
                        (None, Some(_)) => return Ordering::Less,
                        (Some(_), None) => return Ordering::Greater,
                        (Some(xa), Some(xb)) if xa != xb => return xa.cmp(&xb),
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Returns the portable configuration path (`<exe_dir>/DeskBox.toml`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn portable_config_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe_dir.join("DeskBox.toml")
}

/// Loads `DeskBox.toml`, creating defaults only when missing.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn load_or_create(path: &Path) -> DeskBoxConfig {
    match fs::read_to_string(path) {
        Ok(content) => parse_toml(&content),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let cfg = DeskBoxConfig::default();
            let _ = save_config(path, &cfg);
            cfg
        }
        Err(_) => DeskBoxConfig::default(),
    }
}

/// Serializes and atomically writes `DeskBox.toml`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn save_config(path: &Path, cfg: &DeskBoxConfig) -> bool {
    crate::system::atomic_write_string(path, &to_toml_string(cfg)).is_ok()
}

/// Serializes `DeskBoxConfig` to a clean human-editable TOML document.
pub fn to_toml_string(cfg: &DeskBoxConfig) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("# Pretty Windows DeskBox configuration\n");
    out.push_str(&format!("enabled = {}\n", cfg.enabled));
    out.push_str(&format!("hide_desktop_icons = {}\n", cfg.hide_desktop_icons));
    out.push_str(&format!(
        "default_view_mode = \"{}\"\n",
        cfg.default_view_mode.as_str()
    ));
    let next_id = cfg.next_box_id.max(
        cfg.boxes
            .iter()
            .map(|b| b.id.saturating_add(1))
            .max()
            .unwrap_or(1),
    );
    out.push_str(&format!("next_box_id = {next_id}\n"));

    for spec in &cfg.boxes {
        out.push_str("\n[[boxes]]\n");
        out.push_str(&format!("id = {}\n", spec.id));
        out.push_str(&format!("kind = \"{}\"\n", spec.kind.as_str()));
        out.push_str(&format!("title = \"{}\"\n", escape_toml_str(&spec.title)));
        out.push_str(&format!(
            "folder_path = \"{}\"\n",
            escape_toml_str(&spec.folder_path)
        ));
        out.push_str(&format!(
            "monitor_id = \"{}\"\n",
            escape_toml_str(&spec.monitor_id)
        ));
        out.push_str(&format!("x_dip = {:.1}\n", spec.x_dip));
        out.push_str(&format!("y_dip = {:.1}\n", spec.y_dip));
        out.push_str(&format!(
            "w_dip = {:.1}\n",
            spec.w_dip.max(super::layout::BOX_MIN_WIDTH_DIP)
        ));
        out.push_str(&format!(
            "h_dip = {:.1}\n",
            spec.h_dip.max(super::layout::BOX_MIN_HEIGHT_DIP)
        ));
        out.push_str(&format!("view_mode = \"{}\"\n", spec.view_mode.as_str()));
        out.push_str(&format!("collapsed = {}\n", spec.collapsed));
        out.push_str(&format!("edge_auto_hide = {}\n", spec.edge_auto_hide));
        out.push_str(&format!("auto_hide_header = {}\n", spec.auto_hide_header));
        out.push_str(&format!("locked = {}\n", spec.locked));
        if spec.items.is_empty() {
            out.push_str("items = []\n");
        } else {
            out.push_str("items = [\n");
            for item in &spec.items {
                out.push_str(&format!("    \"{}\",\n", escape_toml_str(item)));
            }
            out.push_str("]\n");
        }
    }
    out
}

/// Parses `DeskBox.toml`, ignoring unrecognized keys gracefully.
pub fn parse_toml(input: &str) -> DeskBoxConfig {
    let input = input.trim_start_matches('\u{feff}');
    let mut cfg = DeskBoxConfig::default();
    let mut cur_box: Option<DeskBoxSpec> = None;
    let mut in_items_array = false;

    for raw_line in input.lines() {
        let stripped = strip_toml_comment(raw_line);
        let line = stripped.trim();
        if line.is_empty() {
            continue;
        }

        if line == "[[boxes]]" {
            in_items_array = false;
            if let Some(prev) = cur_box.take() {
                cfg.boxes.push(prev);
            }
            cur_box = Some(DeskBoxSpec::new_collection(0, "桌面收纳", 48.0, 48.0));
            continue;
        }

        if in_items_array {
            if let Some(ref mut spec) = cur_box {
                let ended = parse_array_fragment(line, &mut spec.items);
                if ended {
                    in_items_array = false;
                }
            } else {
                in_items_array = !line.contains(']');
            }
            continue;
        }
        if line.starts_with('[') {
            continue;
        }

        let Some((k_raw, v_raw)) = line.split_once('=') else {
            continue;
        };
        let key = k_raw.trim();
        let val = v_raw.trim();

        if let Some(ref mut spec) = cur_box {
            match key {
                "id" => {
                    if let Ok(v) = val.parse::<u64>() {
                        spec.id = v;
                    }
                }
                "kind" => {
                    if let Some(s) = parse_toml_string_literal(val) {
                        if let Some(k) = BoxKind::from_str(&s) {
                            spec.kind = k;
                        }
                    }
                }
                "title" => {
                    if let Some(s) = parse_toml_string_literal(val) {
                        spec.title = s;
                    }
                }
                "folder_path" => {
                    if let Some(s) = parse_toml_string_literal(val) {
                        spec.folder_path = s;
                    }
                }
                "monitor_id" => {
                    if let Some(s) = parse_toml_string_literal(val) {
                        spec.monitor_id = s;
                    }
                }
                "x_dip" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            spec.x_dip = v;
                        }
                    }
                }
                "y_dip" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            spec.y_dip = v;
                        }
                    }
                }
                "w_dip" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            spec.w_dip = v.max(super::layout::BOX_MIN_WIDTH_DIP);
                        }
                    }
                }
                "h_dip" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            spec.h_dip = v.max(super::layout::BOX_MIN_HEIGHT_DIP);
                        }
                    }
                }
                "view_mode" => {
                    if let Some(s) = parse_toml_string_literal(val) {
                        if let Some(m) = BoxViewMode::from_str(&s) {
                            spec.view_mode = m;
                        }
                    }
                }
                "collapsed" => {
                    if let Some(b) = parse_bool(val) {
                        spec.collapsed = b;
                    }
                }
                "edge_auto_hide" => {
                    if let Some(b) = parse_bool(val) {
                        spec.edge_auto_hide = b;
                    }
                }
                "auto_hide_header" => {
                    if let Some(b) = parse_bool(val) {
                        spec.auto_hide_header = b;
                    }
                }
                "locked" => {
                    if let Some(b) = parse_bool(val) {
                        spec.locked = b;
                    }
                }
                "items" => {
                    if let Some(after_open) = val.strip_prefix('[') {
                        spec.items.clear();
                        let ended = parse_array_fragment(after_open, &mut spec.items);
                        if !ended {
                            in_items_array = true;
                        }
                    }
                }
                _ => {}
            }
        } else {
            match key {
                "enabled" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.enabled = b;
                    }
                }
                "hide_desktop_icons" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.hide_desktop_icons = b;
                    }
                }
                "default_view_mode" => {
                    if let Some(s) = parse_toml_string_literal(val) {
                        if let Some(m) = BoxViewMode::from_str(&s) {
                            cfg.default_view_mode = m;
                        }
                    }
                }
                "next_box_id" => {
                    if let Ok(v) = val.parse::<u64>() {
                        cfg.next_box_id = v.max(1);
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(last) = cur_box.take() {
        cfg.boxes.push(last);
    }

    // Repair missing or duplicate box IDs cleanly.
    let mut next_id = cfg
        .next_box_id
        .max(
            cfg.boxes
                .iter()
                .map(|b| b.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        )
        .max(1);
    let mut seen_ids = HashSet::new();
    for spec in &mut cfg.boxes {
        if spec.id == 0 || !seen_ids.insert(spec.id) {
            spec.id = next_id;
            seen_ids.insert(spec.id);
            next_id = next_id.saturating_add(1);
        }
        if spec.kind == BoxKind::FolderPortal {
            spec.title = folder_portal_display_name(&spec.folder_path);
        } else if spec.title.trim().is_empty() {
            spec.title = "桌面收纳".to_string();
        }
    }
    cfg.next_box_id = next_id;
    cfg
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_double && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
        } else if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '#' && !in_double && !in_single {
            return &line[..idx];
        }
    }
    line
}

fn escape_toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn parse_toml_string_literal(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return Some(unescape_toml_inner(inner));
    }
    trimmed
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .map(ToString::to_string)
}

fn unescape_toml_inner(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Extracts quoted items from one line fragment of a TOML array; returns `true` when `]` closes it.
fn parse_array_fragment(fragment: &str, out: &mut Vec<String>) -> bool {
    let mut chars = fragment.char_indices().peekable();
    let mut closed = false;
    while let Some((idx, ch)) = chars.next() {
        match ch {
            '"' => {
                let start = idx + 1;
                let mut end = fragment.len();
                let mut escaped = false;
                for (j, c2) in chars.by_ref() {
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    if c2 == '\\' {
                        escaped = true;
                        continue;
                    }
                    if c2 == '"' {
                        end = j;
                        break;
                    }
                }
                let item = unescape_toml_inner(&fragment[start..end]);
                if !item.trim().is_empty() {
                    out.push(item);
                }
            }
            '\'' => {
                let start = idx + 1;
                let mut end = fragment.len();
                for (j, c2) in chars.by_ref() {
                    if c2 == '\'' {
                        end = j;
                        break;
                    }
                }
                let item = fragment[start..end].to_string();
                if !item.trim().is_empty() {
                    out.push(item);
                }
            }
            ']' => {
                closed = true;
                break;
            }
            _ => {}
        }
    }
    closed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trip_preserves_windows_paths_cjk_titles_and_multiple_boxes() {
        let mut cfg = DeskBoxConfig {
            enabled: true,
            hide_desktop_icons: true,
            default_view_mode: BoxViewMode::SmallIcons,
            next_box_id: 5,
            boxes: Vec::new(),
        };

        let mut b1 = DeskBoxSpec::new_collection(1, "常用 \"工作\" 工具 #1", 60.5, 92.0);
        b1.w_dip = 380.0;
        b1.h_dip = 320.0;
        b1.view_mode = BoxViewMode::LargeIcons;
        b1.collapsed = true;
        b1.items = vec![
            r"C:\Program Files\Git\git-cmd.exe".to_string(),
            r"D:\Docs\设计 # 方案\readme.md".to_string(),
        ];

        let mut b2 = DeskBoxSpec::new_folder_portal(
            4,
            r"C:\Users\Tester\Downloads",
            480.0,
            100.0,
        );
        assert_eq!(b2.title, "Downloads");
        b2.view_mode = BoxViewMode::CompactList;
        b2.edge_auto_hide = true;
        b2.locked = true;
        b2.monitor_id = r"\\.\DISPLAY1".to_string();

        cfg.boxes.push(b1);
        cfg.boxes.push(b2);

        let serialized = to_toml_string(&cfg);
        assert!(!serialized.contains("all_visible"));
        assert!(!serialized.contains("sort_mode"));
        let parsed = parse_toml(&serialized);

        assert_eq!(parsed.enabled, cfg.enabled);
        assert_eq!(parsed.hide_desktop_icons, cfg.hide_desktop_icons);
        assert_eq!(parsed.default_view_mode, cfg.default_view_mode);
        assert_eq!(parsed.boxes.len(), 2);
        assert_eq!(parsed.boxes[0], cfg.boxes[0]);
        assert!(parsed.boxes[0].collapsed);
        assert_eq!(parsed.boxes[1], cfg.boxes[1]);
        assert!(!parsed.boxes[1].collapsed);
    }

    #[test]
    fn dedup_add_and_remove_collection_items_work_case_insensitively() {
        let mut spec = DeskBoxSpec::new_collection(1, "Test", 40.0, 40.0);
        let added = spec.dedup_add_items(&[
            r"C:\Foo\Bar.txt".to_string(),
            r"c:/foo/bar.txt".to_string(),
            r"C:\Foo\Baz.exe".to_string(),
            "   ".to_string(),
        ]);
        assert_eq!(added, 2);
        assert_eq!(spec.items.len(), 2);

        let removed = spec.remove_items(&[r"C:/FOO/bar.txt".to_string()]);
        assert_eq!(removed, 1);
        assert_eq!(spec.items, vec![r"C:\Foo\Baz.exe".to_string()]);

        // Folder portal rejects collection item additions and derives title from its folder name.
        let mut portal = DeskBoxSpec::new_folder_portal(2, r"C:\Dl", 40.0, 40.0);
        assert_eq!(portal.title, "Dl");
        assert_eq!(portal.dedup_add_items(&[r"C:\a.txt".to_string()]), 0);
    }

    #[test]
    fn folder_portal_always_uses_linked_folder_name() {
        assert_eq!(folder_portal_display_name(r"D:\nginx2"), "nginx2");
        assert_eq!(folder_portal_display_name(r"D:\nginx2\"), "nginx2");
        assert_eq!(folder_portal_display_name(r"D:\"), r"D:\");
        assert_eq!(folder_portal_display_name(r"\\server\share\"), "share");
        assert_eq!(folder_portal_display_name(""), "关联文件夹");

        let parsed = parse_toml(
            r#"
            [[boxes]]
            id = 5
            kind = "folder_portal"
            title = "自定义旧名称"
            folder_path = "D:\\Projects\\nginx2\\"
            "#,
        );
        assert_eq!(parsed.boxes.len(), 1);
        assert_eq!(parsed.boxes[0].title, "nginx2");
    }

    #[test]
    fn corrupt_or_empty_toml_repairs_ids_and_defaults_safely() {
        let parsed = parse_toml(
            r#"
            enabled = true
            all_visible = false
            [[boxes]]
            id = 0
            title = ""
            items = ["C:\\a.txt", "D:\\b.png"]
            [[boxes]]
            id = 0
            kind = "folder_portal"
            folder_path = "C:\\Temp"
            w_dip = 100.0
            h_dip = 50.0
            collapsed = true
            "#,
        );
        assert!(parsed.enabled);
        assert_eq!(parsed.boxes.len(), 2);
        assert_ne!(parsed.boxes[0].id, 0);
        assert_ne!(parsed.boxes[0].id, parsed.boxes[1].id);
        assert_eq!(parsed.boxes[0].title, "桌面收纳");
        assert!(!parsed.boxes[0].collapsed);
        assert_eq!(parsed.boxes[0].items.len(), 2);
        assert_eq!(parsed.boxes[1].kind, BoxKind::FolderPortal);
        assert!(parsed.boxes[1].collapsed);
        assert!(parsed.boxes[1].w_dip >= super::super::layout::BOX_MIN_WIDTH_DIP);
        assert!(parsed.boxes[1].h_dip >= super::super::layout::BOX_MIN_HEIGHT_DIP);
    }

    #[test]
    fn compare_item_type_and_name_sorts_dirs_then_extension_then_natural_name() {
        let mut items = vec![
            (false, PathBuf::from(r"C:\a\note10.txt"), "note10.txt"),
            (false, PathBuf::from(r"C:\a\note2.txt"), "note2.txt"),
            (false, PathBuf::from(r"C:\a\App.LNK"), "App"),
            (true, PathBuf::from(r"C:\a\杂项"), "杂项"),
            (false, PathBuf::from(r"C:\a\photo.png"), "photo.png"),
            (true, PathBuf::from(r"C:\a\GPT"), "GPT"),
            (false, PathBuf::from(r"C:\a\Beta.txt"), "Beta.txt"),
            (false, PathBuf::from(r"C:\a\alpha.TXT"), "alpha.TXT"),
            (false, PathBuf::from(r"C:\a\README"), "README"),
        ];
        items.sort_by(|a, b| compare_item_type_and_name(a.0, &a.1, a.2, b.0, &b.1, b.2));
        let ordered: Vec<&str> = items.into_iter().map(|(_, _, n)| n).collect();
        assert_eq!(
            ordered,
            vec![
                "GPT",
                "杂项",
                "README",
                "App",
                "photo.png",
                "alpha.TXT",
                "Beta.txt",
                "note2.txt",
                "note10.txt",
            ]
        );
    }

    #[test]
    fn sort_modes_and_collection_manual_reorder_work_cleanly() {
        let a_path = PathBuf::from(r"C:\a\old.txt");
        let b_path = PathBuf::from(r"C:\a\new.md");
        assert_eq!(
            compare_item_for_sort(
                BoxSortMode::ModifiedDesc,
                false,
                &b_path,
                "new.md",
                200,
                false,
                &a_path,
                "old.txt",
                100
            ),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_item_for_sort(
                BoxSortMode::NameDesc,
                false,
                &a_path,
                "old.txt",
                100,
                false,
                &b_path,
                "new.md",
                200
            ),
            std::cmp::Ordering::Less
        );

        let ordered: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let reordered = DeskBoxSpec::reorder_item_paths(&ordered, 0, 2).unwrap();
        assert_eq!(reordered, vec!["b", "c", "a"]);
        assert!(DeskBoxSpec::reorder_item_paths(&ordered, 1, 1).is_none());
    }

    #[test]
    fn normalize_path_key_strips_trailing_slash_preserves_root_and_folds_unicode() {
        assert_eq!(normalize_path_key(r"C:\Users\Test\"), r"c:\users\test");
        assert_eq!(normalize_path_key(r"C:/Users/Test///"), r"c:\users\test");
        assert_eq!(normalize_path_key(r"C:\"), r"c:\");
        assert_eq!(normalize_path_key(r"C:/"), r"c:\");
        assert_eq!(normalize_path_key(r"D:\ÄÖÜ\File"), r"d:\äöü\file");
    }

    #[test]
    fn parse_toml_preserves_auto_hide_header_single_quote_hash_and_saturates_ids() {
        let raw = r#"
enabled = true
next_box_id = 18446744073709551615

[[boxes]]
id = 18446744073709551615
kind = "folder_portal"
folder_path = 'D:\Work#2026\Sub' # trailing comment
auto_hide_header = true
"#;
        let parsed = parse_toml(raw);
        assert_eq!(parsed.boxes.len(), 1);
        assert_eq!(parsed.boxes[0].folder_path, r"D:\Work#2026\Sub");
        assert!(parsed.boxes[0].auto_hide_header);
        assert_eq!(parsed.next_box_id, u64::MAX);
    }

    #[test]
    fn parse_toml_handles_single_quoted_items_with_brackets_and_unclosed_array() {
        let raw = r#"
enabled = true

[[boxes]]
id = 1
kind = "collection"
title = "Music"
items = [
    'D:\Music\[2026]\track1.mp3',
    "D:\\Docs\\readme.txt",
]

[[boxes]]
id = 2
kind = "collection"
title = "CorruptedUnclosed"
items = [
    'C:\Broken\a.txt',

[[boxes]]
id = 3
kind = "collection"
title = "RecoveredAfterUnclosed"
items = ['C:\Recovered\[ok]\b.txt']
"#;
        let parsed = parse_toml(raw);
        assert_eq!(parsed.boxes.len(), 3);
        assert_eq!(
            parsed.boxes[0].items,
            vec![r"D:\Music\[2026]\track1.mp3", r"D:\Docs\readme.txt"]
        );
        assert_eq!(parsed.boxes[1].title, "CorruptedUnclosed");
        assert_eq!(parsed.boxes[1].items, vec![r"C:\Broken\a.txt"]);
        assert_eq!(parsed.boxes[2].title, "RecoveredAfterUnclosed");
        assert_eq!(parsed.boxes[2].items, vec![r"C:\Recovered\[ok]\b.txt"]);
    }
}
