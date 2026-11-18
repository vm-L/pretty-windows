use std::fs;
use std::path::{Path, PathBuf};

/// Persistent configuration stored next to the executable (`MediaBar.toml`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Whether the taskbar bar position is locked against accidental dragging.
    pub locked: bool,
    /// Horizontal offset in physical pixels relative to the default tray-left anchor.
    /// Negative values shift MediaBar further left; positive values shift right (clamped).
    pub offset_x: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            locked: true,
            offset_x: 0,
        }
    }
}

impl Config {
    /// Parses configuration from TOML-formatted text, falling back to defaults for missing/invalid fields.
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
            let key = key.trim();
            let value = value.trim();
            match key {
                "locked" => {
                    if let Ok(parsed) = value.parse::<bool>() {
                        cfg.locked = parsed;
                    }
                }
                "offset_x" => {
                    if let Ok(parsed) = value.parse::<i32>() {
                        cfg.offset_x = parsed;
                    }
                }
                _ => {}
            }
        }
        cfg
    }

    /// Serializes configuration to human-readable TOML.
    pub fn serialize(&self) -> String {
        format!(
            "# MediaBar 配置文件\nlocked = {}\noffset_x = {}\n",
            self.locked, self.offset_x
        )
    }

    /// Loads configuration from disk, returning defaults if the file does not exist or cannot be read.
    pub fn load_from_path(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => Self::parse(&content),
            Err(_) => Self::default(),
        }
    }

    /// Saves configuration to disk next to the executable.
    pub fn save_to_path(&self, path: &Path) -> std::io::Result<()> {
        fs::write(path, self.serialize())
    }
}

/// Resolves the portable configuration file path next to the current executable.
#[allow(dead_code)]
pub fn portable_config_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("MediaBar.toml");
        }
    }
    PathBuf::from("MediaBar.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_roundtrip() {
        let cfg = Config {
            locked: false,
            offset_x: -148,
        };
        let toml = cfg.serialize();
        let parsed = Config::parse(&toml);
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn parse_ignores_comments_and_invalid_values() {
        let raw = r#"
            # comment line
            locked = invalid_bool
            unknown_key = 42
            offset_x = -24 # inline comment
        "#;
        let parsed = Config::parse(raw);
        assert!(parsed.locked);
        assert_eq!(parsed.offset_x, -24);
    }

    #[test]
    fn missing_file_returns_default() {
        let cfg = Config::load_from_path(Path::new("/nonexistent/path/MediaBar_does_not_exist.toml"));
        assert_eq!(cfg, Config::default());
    }
}
