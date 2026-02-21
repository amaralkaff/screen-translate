use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub api_url: String,
    pub api_key: Option<String>,
    pub target_lang: String,
    pub source_lang: String,
    pub poll_interval_ms: u64,
    pub popup_duration_secs: u64,
    pub max_text_length: usize,
    pub python_path: Option<String>,
    pub api_port: u16,
    pub load_languages: String,
    pub auto_update: bool,
    pub start_local_server: bool,
}

impl Default for Config {
    fn default() -> Self {
        // macOS uses port 5000 for AirPlay Receiver, so default to 5001
        #[cfg(target_os = "macos")]
        let default_port: u16 = 5001;
        #[cfg(not(target_os = "macos"))]
        let default_port: u16 = 5000;

        Self {
            api_url: format!("http://127.0.0.1:{}/translate", default_port),
            api_key: None,
            target_lang: "id".into(),
            source_lang: "auto".into(),
            poll_interval_ms: 100,
            popup_duration_secs: 5,
            max_text_length: 5000,
            python_path: None,
            api_port: default_port,
            load_languages: "en,zh,ja,es,ar,id".into(),
            auto_update: true,
            start_local_server: true,
        }
    }
}

impl Config {
    pub fn app_dir() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
            PathBuf::from(appdata).join("screen-translate")
        }
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("screen-translate")
        }
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let contents = std::fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&contents)?;
            tracing::info!("Loaded config from {}", path.display());
            Ok(config)
        } else {
            // Auto-create config directory and default config
            let dir = Self::app_dir();
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!("Failed to create config directory: {}", e);
            } else {
                let defaults = Config::default();
                let default_config = format!("\
# Screen Translate configuration
# See https://github.com/amaralkaff/screen-translate for details

# LibreTranslate API endpoint (default: local server, no internet needed)
# api_url = \"http://127.0.0.1:{}/translate\"

# API key - NOT NEEDED for local usage! Only for remote APIs.
# api_key = \"\"

# Target language for translations (ISO 639 code)
# target_lang = \"id\"

# Source language (\"auto\" for auto-detection)
# source_lang = \"auto\"

# Debounce interval in milliseconds (minimum 50)
# poll_interval_ms = 100

# Maximum text length to translate (characters)
# max_text_length = 5000

# LibreTranslate server port (macOS defaults to 5001 to avoid AirPlay conflict)
# api_port = {}

# Languages to load (comma-separated ISO 639 codes)
# load_languages = \"en,zh,ja,es,ar,id\"

# Path to Python executable (for starting LibreTranslate)
# python_path = \"\"

# Automatically check for updates on startup
# auto_update = true

# Start local LibreTranslate server (disable if using remote API)
# start_local_server = true
", defaults.api_port, defaults.api_port);
                if let Err(e) = std::fs::write(&path, default_config) {
                    tracing::warn!("Failed to write default config: {}", e);
                } else {
                    tracing::info!("Created default config at {}", path.display());
                }
            }
            Ok(Config::default())
        }
    }

    fn config_path() -> PathBuf {
        Self::app_dir().join("config.toml")
    }
}
