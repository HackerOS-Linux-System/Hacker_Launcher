use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default = "default_runner")]
    pub default_runner: String,
    #[serde(default = "default_auto_update")]
    pub auto_update: String,
    #[serde(default = "default_true")]
    pub enable_esync: bool,
    #[serde(default = "default_true")]
    pub enable_fsync: bool,
    #[serde(default)]
    pub enable_dxvk_async: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// When true, games with `use_shared_prefix` set will all share the
    /// single prefix at `shared_prefix_path` instead of getting their own.
    #[serde(default)]
    pub use_shared_prefix_default: bool,
    /// Empty means "use `<prefixes_dir>/shared`" (computed by GameManager).
    #[serde(default)]
    pub shared_prefix_path: String,
    /// "List" or "Grid". Read once at app startup by the frontend, so
    /// changing it takes effect after restarting the app.
    #[serde(default = "default_library_view")]
    pub library_view: String,
}

fn default_runner() -> String {
    "Proton".to_string()
}
fn default_auto_update() -> String {
    "Enabled".to_string()
}
fn default_true() -> bool {
    true
}
fn default_theme() -> String {
    "Dark (Default)".to_string()
}
fn default_library_view() -> String {
    "List".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fullscreen: false,
            default_runner: default_runner(),
            auto_update: default_auto_update(),
            enable_esync: true,
            enable_fsync: true,
            enable_dxvk_async: false,
            theme: default_theme(),
            use_shared_prefix_default: false,
            shared_prefix_path: String::new(),
            library_view: default_library_view(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paths {
    pub prefixes_dir: String,
    pub protons_dir: String,
    pub logs_dir: String,
}

pub struct ConfigManager {
    pub base_dir: PathBuf,
    pub protons_dir: PathBuf,
    pub prefixes_dir: PathBuf,
    pub config_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub games_file: PathBuf,
    pub settings_file: PathBuf,
    pub settings: Settings,
}

impl ConfigManager {
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("Cannot find home directory")?;
        let base_dir = home.join(".hackeros").join("Hacker-Launcher");
        let config_dir = base_dir.join("Config");
        let prefixes_dir = base_dir.join("Prefixes");
        let protons_dir = base_dir.join("Protons");
        let logs_dir = base_dir.join("Logs");
        let games_file = config_dir.join("games.json");
        let settings_file = config_dir.join("settings.json");

        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(&prefixes_dir)?;
        fs::create_dir_all(&protons_dir)?;
        fs::create_dir_all(&logs_dir)?;

        let settings = Self::load_settings_from(&settings_file);

        Ok(Self {
            base_dir,
            protons_dir,
            prefixes_dir,
            config_dir,
            logs_dir,
            games_file,
            settings_file,
            settings,
        })
    }

    fn load_settings_from(path: &PathBuf) -> Settings {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(s) = serde_json::from_str::<Settings>(&content) {
                    return s;
                }
            }
        }
        Settings::default()
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let content = serde_json::to_string_pretty(settings)?;
        fs::write(&self.settings_file, content)?;
        Ok(())
    }
}
