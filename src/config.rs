/*!
 * Config System — S06
 *
 * Stores user-facing REPL preferences in .localcoder/settings.json:
 *   ui.theme
 *   ui.tips
 */

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const DEFAULT_OLLAMA_MODEL: &str = "qwen3.5:4b";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Default,
    Light,
    Dark,
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Theme::Default => write!(f, "default"),
            Theme::Light => write!(f, "light"),
            Theme::Dark => write!(f, "dark"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "default_tips")]
    pub tips: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self::Default
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: Theme::Default,
            tips: true,
        }
    }
}

fn default_tips() -> bool {
    true
}

impl AppConfig {
    pub fn load(project_dir: &Path) -> Result<Self> {
        let path = resolve_settings_path(project_dir)?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read settings file: {}", path.display()))?;
        let root: Value = serde_json::from_str(&raw)
            .with_context(|| format!("invalid settings JSON: {}", path.display()))?;

        let ui = root.get("ui").cloned().unwrap_or_else(|| json!({}));
        let cfg = serde_json::from_value::<Self>(ui).unwrap_or_default();
        Ok(cfg)
    }

    pub fn save(&self, project_dir: &Path) -> Result<PathBuf> {
        let path = resolve_settings_path(project_dir)?;

        let mut root: Value = if path.exists() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("failed to read settings file: {}", path.display()))?;
            serde_json::from_str(&raw)
                .with_context(|| format!("invalid settings JSON: {}", path.display()))?
        } else {
            default_settings_json()
        };

        root["ui"] = serde_json::to_value(self).context("failed to serialize ui config")?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create settings dir: {}", parent.display()))?;
        }

        fs::write(
            &path,
            serde_json::to_string_pretty(&root).context("failed to serialize settings")?,
        )
        .with_context(|| format!("failed to write settings file: {}", path.display()))?;

        Ok(path)
    }
}

fn default_settings_json() -> Value {
    json!({
        "ollama": {
            "url": DEFAULT_OLLAMA_URL,
            "model": DEFAULT_OLLAMA_MODEL
        }
    })
}

fn resolve_settings_path(project_dir: &Path) -> Result<PathBuf> {
    let cwd_path = project_dir.join(".localcoder/settings.json");
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    if let Some(home) = std::env::var_os("HOME") {
        let home_path = PathBuf::from(home).join(".localcoder/settings.json");
        if home_path.exists() {
            return Ok(home_path);
        }
    }

    Ok(cwd_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_defaults_when_missing_ui() {
        let project = TempDir::new().unwrap();
        let dir = project.path().join(".localcoder");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("settings.json"),
            r#"{"ollama":{"url":"http://localhost:11434","model":"qwen"}}"#,
        )
        .unwrap();

        let cfg = AppConfig::load(project.path()).unwrap();
        assert_eq!(cfg.theme, Theme::Default);
        assert!(cfg.tips);
    }

    #[test]
    fn save_and_reload_ui_config() {
        let project = TempDir::new().unwrap();
        let cfg = AppConfig {
            theme: Theme::Dark,
            tips: false,
        };

        let path = cfg.save(project.path()).unwrap();
        assert!(path.exists());

        let loaded = AppConfig::load(project.path()).unwrap();
        assert_eq!(loaded.theme, Theme::Dark);
        assert!(!loaded.tips);
    }
}
