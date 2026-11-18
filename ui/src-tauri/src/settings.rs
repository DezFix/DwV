use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorySettings {
    pub category: String,
    pub enabled: bool,
    pub path: String,
    pub auto_extract: bool,
}

/// Every field falls back to [`Settings::default`], so a file written by an
/// older version (or missing a field) keeps the rest of the user's choices.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub download_dir: String,
    pub max_concurrent_downloads: u32,
    pub speed_limit_mbps: f64,
    pub auto_start_downloads: bool,
    #[serde(default = "default_connections")]
    pub connections_per_download: u32,
    #[serde(default = "default_true")]
    pub ask_save_path: bool,
    #[serde(default = "default_autostart")]
    pub autostart: bool,
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
    /// Quality key used when a video arrives without one (extension, deep link).
    pub default_video_quality: String,
}

fn default_autostart() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_connections() -> u32 {
    8
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            max_concurrent_downloads: 3,
            speed_limit_mbps: 2.0,
            auto_start_downloads: true,
            connections_per_download: 8,
            ask_save_path: true,
            autostart: true,
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
            default_video_quality: "best".to_string(),
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
            Ok(json) => {
                serde_json::from_str(json.trim_start_matches('\u{feff}')).unwrap_or_else(|_| {
                    // keep the broken file for the user instead of overwriting it silently
                    let _ = std::fs::copy(file, file.with_extension("json.bak"));
                    Self::default()
                })
            }
            Err(_) => Self::default(),
        }
    }

    /// Writes through a temporary file, so a crash mid-write cannot leave
    /// half a JSON document behind.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, file).map_err(|e| e.to_string())
    }

    /// Proxy URL for HTTP clients and yt-dlp, or `None` when disabled.
    /// The host may carry a scheme (`socks5://host`); plain hosts mean HTTP.
    pub fn proxy_url(&self) -> Option<String> {
        if !self.proxy_enabled {
            return None;
        }
        let raw = self.proxy_host.trim();
        if raw.is_empty() {
            return None;
        }
        let (scheme, host) = match raw.split_once("://") {
            Some((scheme, host)) => (scheme.to_ascii_lowercase(), host),
            None => ("http".to_string(), raw),
        };
        let host = host.trim_end_matches('/');
        if host.is_empty() {
            return None;
        }
        let credentials = if self.proxy_username.is_empty() {
            String::new()
        } else {
            format!(
                "{}:{}@",
                encode_userinfo(&self.proxy_username),
                encode_userinfo(&self.proxy_password)
            )
        };
        let port = if self.proxy_port > 0 && !host.contains(']') && !host.contains(':') {
            format!(":{}", self.proxy_port)
        } else {
            String::new()
        };
        Some(format!("{scheme}://{credentials}{host}{port}"))
    }

    /// The proxy for torrents: librqbit tunnels peers only through SOCKS5.
    pub fn socks_proxy_url(&self) -> Option<String> {
        self.proxy_url()
            .filter(|url| url.starts_with("socks5://") || url.starts_with("socks5h://"))
            .map(|url| url.replacen("socks5h://", "socks5://", 1))
    }

    /// The custom User-Agent, if the user set one.
    pub fn user_agent(&self) -> Option<&str> {
        Some(self.custom_user_agent.trim()).filter(|value| !value.is_empty())
    }
}

fn encode_userinfo(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn old_settings_get_new_defaults() {
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("ask_save_path");
        object.remove("connections_per_download");
        let settings: Settings = serde_json::from_value(value).unwrap();
        assert!(settings.ask_save_path);
        assert_eq!(settings.connections_per_download, 8);
    }

    #[test]
    fn missing_fields_keep_the_rest() {
        // 0.2.x had no default_video_quality; before #[serde(default)] the
        // whole file was replaced with defaults.
        let json = r#"{"download_dir":"D:\\Downloads","speed_limit_mbps":7.5,"theme":"dark"}"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.download_dir, "D:\\Downloads");
        assert_eq!(settings.speed_limit_mbps, 7.5);
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.default_video_quality, "best");
        assert_eq!(settings.max_concurrent_downloads, 3);
    }

    #[test]
    fn proxy_urls() {
        let mut settings = Settings::default();
        assert_eq!(settings.proxy_url(), None);
        settings.proxy_enabled = true;
        settings.proxy_host = "proxy.local".to_string();
        settings.proxy_port = 3128;
        assert_eq!(
            settings.proxy_url().as_deref(),
            Some("http://proxy.local:3128")
        );
        assert_eq!(settings.socks_proxy_url(), None);

        settings.proxy_host = "socks5://127.0.0.1".to_string();
        settings.proxy_port = 9050;
        settings.proxy_username = "me".to_string();
        settings.proxy_password = "p@ss:w".to_string();
        assert_eq!(
            settings.proxy_url().as_deref(),
            Some("socks5://me:p%40ss%3Aw@127.0.0.1:9050")
        );
        assert_eq!(settings.socks_proxy_url(), settings.proxy_url());

        settings.proxy_enabled = false;
        assert_eq!(settings.proxy_url(), None);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("dwv-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("settings.json");
        let settings = Settings {
            default_video_quality: "720p".to_string(),
            ..Default::default()
        };
        settings.save(&file).unwrap();
        assert!(!file.with_extension("json.tmp").exists());
        assert_eq!(Settings::load(&file).default_video_quality, "720p");

        std::fs::write(&file, "{ broken").unwrap();
        assert_eq!(Settings::load(&file).default_video_quality, "best");
        assert!(file.with_extension("json.bak").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
