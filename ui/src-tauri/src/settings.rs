use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorySettings {
    pub category: String,
    pub enabled: bool,
    pub path: String,
    pub auto_extract: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub download_dir: String,
    pub max_concurrent_downloads: u32,
    pub speed_limit_mbps: f64,
    pub auto_start_downloads: bool,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub show_notifications: bool,
    pub theme: String,
    pub proxy_enabled: bool,
    pub proxy_host: String,
    pub proxy_port: u16,
    pub proxy_username: String,
    pub proxy_password: String,
    pub categories: Vec<CategorySettings>,
    pub custom_user_agent: String,
    pub ui_scale: f64,
    pub animations_enabled: bool,
    pub truncate_filenames: bool,
    pub language: String,
    pub torrent_dir: String,
    pub torrent_autostart: bool,
    pub speed_limit_enabled: bool,
    pub torrent_upload_limit_kbps: u32,
    pub torrent_seed_after_complete: bool,
    pub torrent_max_peers: u32,
    pub torrent_dht_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            max_concurrent_downloads: 3,
            speed_limit_mbps: 2.0,
            auto_start_downloads: true,
            minimize_to_tray: true,
            close_to_tray: true,
            show_notifications: true,
            theme: "system".to_string(),
            proxy_enabled: false,
            proxy_host: String::new(),
            proxy_port: 0,
            proxy_username: String::new(),
            proxy_password: String::new(),
            categories: vec![
                category("video", "Videos", false),
                category("audio", "Music", false),
                category("document", "Documents", false),
                category("archive", "Archives", true),
                category("program", "Programs", false),
                category("other", "Other", false),
            ],
            custom_user_agent: String::new(),
            ui_scale: 1.0,
            animations_enabled: true,
            truncate_filenames: false,
            language: "auto".to_string(),
            torrent_dir: String::new(),
            torrent_autostart: true,
            speed_limit_enabled: true,
            torrent_upload_limit_kbps: 0,
            torrent_seed_after_complete: false,
            torrent_max_peers: 50,
            torrent_dht_enabled: true,
        }
    }
}

fn category(name: &str, path: &str, auto_extract: bool) -> CategorySettings {
    CategorySettings {
        category: name.to_string(),
        enabled: true,
        path: path.to_string(),
        auto_extract,
    }
}

impl Settings {
    pub fn load(file: &Path) -> Self {
        match std::fs::read_to_string(file) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, file: &Path) -> Result<(), String> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(file, json).map_err(|e| e.to_string())
    }
}
