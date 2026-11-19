use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub max_background: usize,
    pub prefix: String,
    pub jump_modifier: String,
    pub mac_option_symbols: bool,
    /// capture the mouse for wheel scrolling and clicks (shift+drag selects text natively)
    pub mouse: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            max_background: 5,
            prefix: "ctrl+space".into(),
            jump_modifier: "alt".into(),
            mac_option_symbols: true,
            mouse: true,
        }
    }
}

/// Parse toml text; anything invalid yields defaults.
pub fn parse_config(text: &str) -> Config {
    toml::from_str(text).unwrap_or_default()
}

pub fn config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join("claude-term"))
}

/// Missing or bad file yields defaults.
pub fn load_config() -> Config {
    config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("config.toml")).ok())
        .map(|t| parse_config(&t))
        .unwrap_or_default()
}
