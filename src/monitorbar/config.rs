//! Configuration persistence and schema for the MonitorBar taskbar widget.

use std::fs;
use std::path::{Path, PathBuf};

/// Toggleable monitoring groups displayed in MonitorBar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum MetricItem {
    Network,
    Cpu,
    Memory,
    Gpu,
    Disk,
}

impl MetricItem {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const ALL: [MetricItem; 5] = [
        MetricItem::Network,
        MetricItem::Memory,
        MetricItem::Cpu,
        MetricItem::Disk,
        MetricItem::Gpu,
    ];

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn label(self) -> &'static str {
        match self {
            MetricItem::Network => "网速",
            MetricItem::Cpu => "CPU",
            MetricItem::Memory => "内存",
            MetricItem::Gpu => "显卡",
            MetricItem::Disk => "硬盘",
        }
    }
}

/// Configurable action executed on a non-drag left click on MonitorBar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum LeftClickAction {
    Menu,
    TaskManager,
    ShowDesktop,
    Settings,
    CustomApp,
}

impl LeftClickAction {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const ALL: [LeftClickAction; 5] = [
        LeftClickAction::Menu,
        LeftClickAction::TaskManager,
        LeftClickAction::ShowDesktop,
        LeftClickAction::Settings,
        LeftClickAction::CustomApp,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LeftClickAction::Menu => "menu",
            LeftClickAction::TaskManager => "task_manager",
            LeftClickAction::ShowDesktop => "show_desktop",
            LeftClickAction::Settings => "settings",
            LeftClickAction::CustomApp => "custom_app",
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "menu" => Some(LeftClickAction::Menu),
            "task_manager" | "taskmanager" | "taskmgr" => Some(LeftClickAction::TaskManager),
            "show_desktop" | "showdesktop" | "desktop" => Some(LeftClickAction::ShowDesktop),
            "settings" | "system_settings" => Some(LeftClickAction::Settings),
            "custom_app" | "custom" => Some(LeftClickAction::CustomApp),
            _ => None,
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn label(self) -> &'static str {
        match self {
            LeftClickAction::Menu => "呼出菜单",
            LeftClickAction::TaskManager => "任务管理器",
            LeftClickAction::ShowDesktop => "显示桌面",
            LeftClickAction::Settings => "系统设置",
            LeftClickAction::CustomApp => "自定义程序",
        }
    }
}

/// Persistent user configuration for MonitorBar, stored in `MonitorBar.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorConfig {
    pub enabled: bool,
    pub locked: bool,
    pub offset_x: i32,
    pub left_click: LeftClickAction,
    pub custom_app_path: String,
    pub show_network: bool,
    pub show_cpu: bool,
    pub show_memory: bool,
    pub show_gpu: bool,
    pub show_disk: bool,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            locked: true,
            offset_x: 0,
            left_click: LeftClickAction::Menu,
            custom_app_path: String::new(),
            show_network: true,
            show_cpu: true,
            show_memory: true,
            show_gpu: true,
            show_disk: true,
        }
    }
}

impl MonitorConfig {
    /// Returns whether a given metric group is enabled.
    pub fn is_visible(&self, item: MetricItem) -> bool {
        match item {
            MetricItem::Network => self.show_network,
            MetricItem::Cpu => self.show_cpu,
            MetricItem::Memory => self.show_memory,
            MetricItem::Gpu => self.show_gpu,
            MetricItem::Disk => self.show_disk,
        }
    }

    /// Sets visibility for a metric group without bypassing the 1-metric minimum guard.
    fn set_visible_raw(&mut self, item: MetricItem, visible: bool) {
        match item {
            MetricItem::Network => self.show_network = visible,
            MetricItem::Cpu => self.show_cpu = visible,
            MetricItem::Memory => self.show_memory = visible,
            MetricItem::Gpu => self.show_gpu = visible,
            MetricItem::Disk => self.show_disk = visible,
        }
    }

    /// Counts currently enabled metric groups.
    pub fn enabled_metric_count(&self) -> usize {
        MetricItem::ALL
            .iter()
            .filter(|&&item| self.is_visible(item))
            .count()
    }

    /// Ensures at least one metric group remains visible (falls back to `show_network = true`).
    pub fn ensure_at_least_one_metric(&mut self) {
        if self.enabled_metric_count() == 0 {
            self.show_network = true;
        }
    }

    /// Toggles a metric group, refusing to turn off the last remaining visible metric.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn toggle_metric(&mut self, item: MetricItem) -> bool {
        if self.is_visible(item) {
            if self.enabled_metric_count() <= 1 {
                return false;
            }
            self.set_visible_raw(item, false);
        } else {
            self.set_visible_raw(item, true);
        }
        true
    }
}

/// Returns the portable configuration path (`<exe_dir>/MonitorBar.toml`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn portable_config_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe_dir.join("MonitorBar.toml")
}

/// Loads `MonitorBar.toml` or writes defaults if missing/unreadable.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn load_or_create(path: &Path) -> MonitorConfig {
    if let Ok(content) = fs::read_to_string(path) {
        let cfg = parse_toml(&content);
        return cfg;
    }
    let cfg = MonitorConfig::default();
    let _ = save_config(path, &cfg);
    cfg
}

/// Serializes and writes `MonitorBar.toml`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn save_config(path: &Path, cfg: &MonitorConfig) -> bool {
    let mut normalized = cfg.clone();
    normalized.ensure_at_least_one_metric();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, to_toml_string(&normalized)).is_ok()
}

/// Parses MonitorBar TOML text, falling back to defaults for missing or invalid fields.
pub fn parse_toml(input: &str) -> MonitorConfig {
    let mut cfg = MonitorConfig::default();

    for raw_line in input.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let Some((key, val_raw)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val_raw.trim();

        match key {
            "enabled" => {
                if let Some(b) = parse_bool(val) {
                    cfg.enabled = b;
                }
            }
            "locked" => {
                if let Some(b) = parse_bool(val) {
                    cfg.locked = b;
                }
            }
            "offset_x" => {
                if let Ok(n) = val.parse::<i32>() {
                    cfg.offset_x = n;
                }
            }
            "left_click" => {
                if let Some(s) = parse_toml_string(val) {
                    if let Some(action) = LeftClickAction::from_str(&s) {
                        cfg.left_click = action;
                    }
                }
            }
            "custom_app_path" => {
                if let Some(s) = parse_toml_string(val) {
                    cfg.custom_app_path = s;
                }
            }
            "show_network" => {
                if let Some(b) = parse_bool(val) {
                    cfg.show_network = b;
                }
            }
            "show_cpu" => {
                if let Some(b) = parse_bool(val) {
                    cfg.show_cpu = b;
                }
            }
            "show_memory" => {
                if let Some(b) = parse_bool(val) {
                    cfg.show_memory = b;
                }
            }
            "show_gpu" => {
                if let Some(b) = parse_bool(val) {
                    cfg.show_gpu = b;
                }
            }
            "show_disk" => {
                if let Some(b) = parse_bool(val) {
                    cfg.show_disk = b;
                }
            }
            _ => {}
        }
    }

    cfg.ensure_at_least_one_metric();
    cfg
}

/// Formats `MonitorConfig` into clean human-readable TOML.
pub fn to_toml_string(cfg: &MonitorConfig) -> String {
    let mut normalized = cfg.clone();
    normalized.ensure_at_least_one_metric();
    format!(
        "# MonitorBar 配置文件\n\
         enabled = {}\n\
         locked = {}\n\
         offset_x = {}\n\
         left_click = \"{}\"\n\
         custom_app_path = \"{}\"\n\
         show_network = {}\n\
         show_cpu = {}\n\
         show_memory = {}\n\
         show_gpu = {}\n\
         show_disk = {}\n",
        normalized.enabled,
        normalized.locked,
        normalized.offset_x,
        normalized.left_click.as_str(),
        escape_toml_basic_string(&normalized.custom_app_path),
        normalized.show_network,
        normalized.show_cpu,
        normalized.show_memory,
        normalized.show_gpu,
        normalized.show_disk,
    )
}

fn parse_bool(val: &str) -> Option<bool> {
    match val.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn strip_comment(line: &str) -> &str {
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_basic {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_basic = false;
            }
        } else if in_literal {
            if ch == '\'' {
                in_literal = false;
            }
        } else {
            match ch {
                '"' => in_basic = true,
                '\'' => in_literal = true,
                '#' => return &line[..idx],
                _ => {}
            }
        }
    }
    line
}

fn escape_toml_basic_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn parse_toml_string(val: &str) -> Option<String> {
    let val = val.trim();
    if val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2 {
        return Some(val[1..val.len() - 1].to_string());
    }
    if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
        let inner = &val[1..val.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch != '\\' {
                out.push(ch);
                continue;
            }
            match chars.next()? {
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            }
        }
        return Some(out);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_toml_with_windows_path_and_actions() {
        let cfg = MonitorConfig {
            enabled: false,
            locked: false,
            offset_x: -144,
            left_click: LeftClickAction::CustomApp,
            custom_app_path: r#"C:\Program Files\MyTools\Demo #1\app.exe"#.to_string(),
            show_network: false,
            show_cpu: true,
            show_memory: false,
            show_gpu: true,
            show_disk: false,
        };
        let toml = to_toml_string(&cfg);
        let parsed = parse_toml(&toml);
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn all_metrics_disabled_restores_network() {
        let parsed = parse_toml(
            "show_network = false\n\
             show_cpu = false\n\
             show_memory = false\n\
             show_gpu = false\n\
             show_disk = false\n",
        );
        assert!(parsed.show_network);
        assert_eq!(parsed.enabled_metric_count(), 1);
    }

    #[test]
    fn toggle_metric_refuses_last_remaining() {
        let mut cfg = MonitorConfig {
            show_network: true,
            show_cpu: false,
            show_memory: false,
            show_gpu: false,
            show_disk: false,
            ..MonitorConfig::default()
        };
        assert!(!cfg.toggle_metric(MetricItem::Network));
        assert!(cfg.show_network);

        assert!(cfg.toggle_metric(MetricItem::Cpu));
        assert!(cfg.show_cpu);
        assert!(cfg.toggle_metric(MetricItem::Network));
        assert!(!cfg.show_network);
        assert!(!cfg.toggle_metric(MetricItem::Cpu));
        assert!(cfg.show_cpu);
    }
}
