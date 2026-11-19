use std::fs;
use std::path::{Path, PathBuf};

/// Visual styles supported for Windows 11 multi-monitor taskbars.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TaskbarStyle {
    Default,
    Transparent,
    Blur,
    Acrylic,
    #[default]
    LiquidGlass,
}

impl TaskbarStyle {
    /// Ordered entries displayed in the cascading tray submenu.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const ALL: [Self; 5] = [
        Self::Default,
        Self::Transparent,
        Self::Blur,
        Self::Acrylic,
        Self::LiquidGlass,
    ];

    /// User-facing Chinese label shown in the menu.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "默认",
            Self::Transparent => "透明",
            Self::Blur => "模糊",
            Self::Acrylic => "亚克力",
            Self::LiquidGlass => "液态玻璃",
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Default => 0,
            Self::Transparent => 1,
            Self::Blur => 2,
            Self::Acrylic => 3,
            Self::LiquidGlass => 4,
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Default),
            1 => Some(Self::Transparent),
            2 => Some(Self::Blur),
            3 => Some(Self::Acrylic),
            4 => Some(Self::LiquidGlass),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Transparent => "transparent",
            Self::Blur => "blur",
            Self::Acrylic => "acrylic",
            Self::LiquidGlass => "liquid_glass",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim() {
            "default" => Some(Self::Default),
            "transparent" => Some(Self::Transparent),
            "blur" => Some(Self::Blur),
            "acrylic" => Some(Self::Acrylic),
            "liquid_glass" => Some(Self::LiquidGlass),
            _ => None,
        }
    }
}

/// Persistent configuration stored next to the executable (`Taskbar.toml`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TaskbarConfig {
    pub style: TaskbarStyle,
}

impl TaskbarConfig {
    /// Parses configuration from TOML-formatted text, falling back to `LiquidGlass` for unknown values.
    pub fn parse(content: &str) -> Self {
        let mut cfg = Self::default();
        for raw_line in content.lines() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim() == "style" {
                let unquoted = value.trim().trim_matches('"').trim_matches('\'');
                if let Some(parsed) = TaskbarStyle::from_str(unquoted) {
                    cfg.style = parsed;
                }
            }
        }
        cfg
    }

    /// Serializes configuration to human-readable TOML.
    pub fn serialize(&self) -> String {
        format!(
            "# Pretty Windows 任务栏配置文件\nstyle = \"{}\"\n",
            self.style.as_str()
        )
    }

    /// Loads configuration from disk, returning defaults if missing or unreadable.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn load_from_path(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => Self::parse(&content),
            Err(_) => Self::default(),
        }
    }

    /// Saves configuration to disk next to the executable.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn save_to_path(&self, path: &Path) -> std::io::Result<()> {
        fs::write(path, self.serialize())
    }
}

/// Resolves the portable configuration file path next to the current executable (`Taskbar.toml`).
#[allow(dead_code)]
pub fn portable_config_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("Taskbar.toml");
        }
    }
    PathBuf::from("Taskbar.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_roundtrips_across_u8_and_string_representations() {
        assert_eq!(TaskbarStyle::ALL.len(), 5);
        for (idx, style) in TaskbarStyle::ALL.into_iter().enumerate() {
            assert_eq!(style.to_u8() as usize, idx);
            assert_eq!(TaskbarStyle::from_u8(style.to_u8()), Some(style));
            assert_eq!(TaskbarStyle::from_str(style.as_str()), Some(style));
            assert!(!style.label().is_empty());
        }
        assert_eq!(TaskbarStyle::from_u8(99), None);
        assert_eq!(TaskbarStyle::from_str("invalid"), None);
    }

    #[test]
    fn config_parse_and_serialize_roundtrip() {
        for style in TaskbarStyle::ALL {
            let cfg = TaskbarConfig { style };
            let serialized = cfg.serialize();
            let parsed = TaskbarConfig::parse(&serialized);
            assert_eq!(parsed, cfg);
        }

        let noisy = r#"
            # header comment
            unknown = true
            style = "liquid_glass" # inline note
        "#;
        assert_eq!(
            TaskbarConfig::parse(noisy),
            TaskbarConfig {
                style: TaskbarStyle::LiquidGlass
            }
        );

        let bad = "style = \"nonexistent\"";
        assert_eq!(TaskbarStyle::default(), TaskbarStyle::LiquidGlass);
        assert_eq!(TaskbarConfig::default().style, TaskbarStyle::LiquidGlass);
        assert_eq!(TaskbarConfig::parse(bad), TaskbarConfig::default());
        assert_eq!(
            TaskbarConfig::load_from_path(Path::new("/nonexistent/Taskbar_missing.toml")),
            TaskbarConfig::default()
        );
    }
}
