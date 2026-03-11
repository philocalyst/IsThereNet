use color::palette::css::{GREEN, LIGHT_GREEN, RED, YELLOW};
use color::{AlphaColor, Srgb};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_ping_ip")]
    pub ping_ip: String,
    #[serde(default = "default_ping_interval")]
    pub ping_interval_seconds: f64,
    #[serde(default = "default_ping_timeout")]
    pub ping_timeout_seconds: f64,
    #[serde(default = "default_slow_threshold")]
    pub ping_slow_threshold_milliseconds: f64,
    #[serde(default)]
    pub shell_command_on_status_change: Option<String>,
    #[serde(default)]
    pub fade_seconds: FadeSecondsConfig,
    #[serde(default)]
    pub colors: ColorsConfig,
    #[serde(default)]
    pub screen: ScreenConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ping_ip: default_ping_ip(),
            ping_interval_seconds: default_ping_interval(),
            ping_timeout_seconds: default_ping_timeout(),
            ping_slow_threshold_milliseconds: default_slow_threshold(),
            shell_command_on_status_change: None,
            fade_seconds: FadeSecondsConfig::default(),
            colors: ColorsConfig::default(),
            screen: ScreenConfig::default(),
        }
    }
}

impl Config {
    pub fn ping_interval(&self) -> Duration {
        Duration::from_secs_f64(self.ping_interval_seconds)
    }

    pub fn ping_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.ping_timeout_seconds)
    }
}

fn default_ping_ip() -> String {
    "1.1.1.1".to_string()
}
fn default_ping_interval() -> f64 {
    5.0
}
fn default_ping_timeout() -> f64 {
    1.0
}
fn default_slow_threshold() -> f64 {
    300.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FadeSecondsConfig {
    #[serde(default = "default_connected_fade")]
    pub connected: f64,
    #[serde(default = "default_disconnected_fade")]
    pub disconnected: f64,
    #[serde(default = "default_slow_fade")]
    pub slow: f64,
}

impl Default for FadeSecondsConfig {
    fn default() -> Self {
        Self {
            connected: default_connected_fade(),
            disconnected: default_disconnected_fade(),
            slow: default_slow_fade(),
        }
    }
}

fn default_connected_fade() -> f64 {
    5.0
}
fn default_disconnected_fade() -> f64 {
    0.0
}
fn default_slow_fade() -> f64 {
    10.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColorsConfig {
    pub connected: AlphaColor<Srgb>,
    pub disconnected: AlphaColor<Srgb>,
    pub slow: AlphaColor<Srgb>,
}

impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            connected: LIGHT_GREEN,
            disconnected: RED,
            slow: YELLOW,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ScreenConfig {
    Name(String),
}

impl Default for ScreenConfig {
    fn default() -> Self {
        ScreenConfig::Name("all".to_string())
    }
}

impl ScreenConfig {
    pub fn matches(&self, screen_name: &str, is_main: bool) -> bool {
        match self {
            ScreenConfig::Name(name) => {
                let name_lower = name.to_lowercase();
                if name_lower == "all" {
                    true
                } else if name_lower == "main" {
                    is_main
                } else {
                    screen_name.to_lowercase().contains(&name_lower)
                }
            }
        }
    }
}

pub fn config_path() -> PathBuf {
    let home = dirs::home_dir().expect("no home directory");
    home.join(".config").join("istherenet").join("config.json")
}

pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let path = config_path();

    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let config = Config::default();
        let json = serde_json::to_string_pretty(&config)?;
        std::fs::write(&path, json)?;
        return Ok(config);
    }

    let data = std::fs::read_to_string(&path)?;
    let config: Config = serde_json::from_str(&data)?;
    Ok(config)
}
