// config.rs

//! User preferences (theme, page size, debug). webex-tui holds no Webex
//! credentials: the webex-bridge daemon owns the Webex connection. A legacy
//! `client.yml` / token cache is ignored, never read or deleted.

use color_eyre::eyre::eyre;
use serde::{Deserialize, Serialize};
use std::{env, fs};

const USER_FILE_NAME: &str = "config.yml";
const CONFIG_DIR: &str = ".config";
const APP_CONFIG_DIR: &str = "webex-tui";

fn default_theme_name() -> String {
    "default".to_string()
}

fn default_messages_to_load() -> u32 {
    10
}

/// Messages per room page accepted by the bridge UI API.
pub(crate) const MESSAGES_TO_LOAD_RANGE: std::ops::RangeInclusive<u32> = 1..=100;

/// Page size: `--messages` (already range-checked by clap) overrides the
/// config's `messages_to_load`; an out-of-range config value is clamped, with
/// a warning to print.
pub(crate) fn resolve_messages_to_load(cli: Option<u32>, config: u32) -> (u32, Option<String>) {
    if let Some(value) = cli {
        return (value, None);
    }
    let (min, max) = (
        *MESSAGES_TO_LOAD_RANGE.start(),
        *MESSAGES_TO_LOAD_RANGE.end(),
    );
    let value = config.clamp(min, max);
    let warning = (value != config).then(|| {
        format!(
            "messages_to_load = {config} in the config is outside {min}..={max}; using {value}."
        )
    });
    (value, warning)
}

/// User preferences configuration (managed by nix/user)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct UserConfig {
    /// Theme name to load from themes directory
    #[serde(default = "default_theme_name")]
    pub(crate) theme: String,
    /// Number of messages to load per room
    #[serde(default = "default_messages_to_load")]
    pub(crate) messages_to_load: u32,
    /// Enable debug logging by default
    #[serde(default)]
    pub(crate) debug: bool,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            theme: default_theme_name(),
            messages_to_load: default_messages_to_load(),
            debug: false,
        }
    }
}

/// A note from loading the config, shown once logging is set up; warnings
/// are also printed before the terminal UI starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigNote {
    pub(crate) warning: bool,
    pub(crate) text: String,
}

impl ConfigNote {
    fn info(text: String) -> Self {
        Self {
            warning: false,
            text,
        }
    }
    fn warn(text: String) -> Self {
        Self {
            warning: true,
            text,
        }
    }
}

impl UserConfig {
    /// Load user configuration: `WEBEX_TUI_CONFIG`, else
    /// `~/.config/webex-tui/config.yml` (created with defaults if missing).
    /// Runs before logging is set up, so it returns its notes.
    pub(crate) fn load() -> (Self, Vec<ConfigNote>) {
        let mut notes = Vec::new();
        // Try environment variable first (nix can set this)
        if let Ok(config_path) = env::var("WEBEX_TUI_CONFIG") {
            notes.push(ConfigNote::info(format!(
                "Loading user config from WEBEX_TUI_CONFIG: {config_path}"
            )));
            match fs::read_to_string(&config_path) {
                Ok(content) => return (Self::parse(&content, &config_path, &mut notes), notes),
                Err(e) => notes.push(ConfigNote::warn(format!(
                    "Cannot read WEBEX_TUI_CONFIG {config_path} ({e}); trying the default location"
                ))),
            }
        }

        // Try standard location
        if let Some(home) = dirs::home_dir() {
            let config_path = home
                .join(CONFIG_DIR)
                .join(APP_CONFIG_DIR)
                .join(USER_FILE_NAME);
            let shown = config_path.display().to_string();
            notes.push(ConfigNote::info(format!(
                "Loading user config from: {shown}"
            )));

            // If config file doesn't exist, create it with defaults
            if !config_path.exists() {
                let default_config = Self::default();
                match default_config.save() {
                    Ok(()) => notes.push(ConfigNote::info(format!(
                        "Created default config file: {shown}"
                    ))),
                    Err(e) => notes.push(ConfigNote::warn(format!(
                        "Failed to create default config file: {e}"
                    ))),
                }
                // Copy bundled theme files to user's config directory
                match crate::theme::copy_bundled_themes() {
                    Ok(()) => notes.push(ConfigNote::info(
                        "Copied theme files to user config directory".into(),
                    )),
                    Err(e) => {
                        notes.push(ConfigNote::warn(format!("Failed to copy theme files: {e}")))
                    }
                }
                return (default_config, notes);
            }

            match fs::read_to_string(&config_path) {
                Ok(content) => return (Self::parse(&content, &shown, &mut notes), notes),
                Err(e) => notes.push(ConfigNote::warn(format!(
                    "Cannot read {shown} ({e}); using defaults"
                ))),
            }
        }

        notes.push(ConfigNote::info(
            "No user config found, using defaults".into(),
        ));
        (Self::default(), notes)
    }

    /// Field-tolerant parse: a bad or unknown entry is reported and skipped,
    /// the others still apply (an invalid value never resets the theme).
    fn parse(content: &str, source: &str, notes: &mut Vec<ConfigNote>) -> Self {
        let mut config = Self::default();
        let value: serde_yaml::Value = match serde_yaml::from_str(content) {
            Ok(value) => value,
            Err(e) => {
                notes.push(ConfigNote::warn(format!(
                    "{source} is not valid YAML ({e}); using defaults"
                )));
                return config;
            }
        };
        let map = match value {
            serde_yaml::Value::Mapping(map) => map,
            serde_yaml::Value::Null => return config, // empty file
            _ => {
                notes.push(ConfigNote::warn(format!(
                    "{source} is not a mapping of settings; using defaults"
                )));
                return config;
            }
        };
        for (key, value) in &map {
            let key = key.as_str().unwrap_or("?");
            let bad = |expected: &str| {
                ConfigNote::warn(format!(
                    "{source}: ignoring {key} (expected {expected}); using the default"
                ))
            };
            match key {
                "theme" => match value.as_str() {
                    Some(theme) => config.theme = theme.to_owned(),
                    None => notes.push(bad("a theme name")),
                },
                "messages_to_load" => match value.as_u64().and_then(|n| u32::try_from(n).ok()) {
                    Some(n) => config.messages_to_load = n,
                    None => notes.push(bad("a number")),
                },
                "debug" => match value.as_bool() {
                    Some(debug) => config.debug = debug,
                    None => notes.push(bad("true or false")),
                },
                _ => notes.push(ConfigNote::warn(format!(
                    "{source}: ignoring unknown setting {key}"
                ))),
            }
        }
        config
    }

    /// Save user configuration to the standard location
    pub(crate) fn save(&self) -> color_eyre::Result<()> {
        if let Some(home) = dirs::home_dir() {
            let config_dir = home.join(CONFIG_DIR).join(APP_CONFIG_DIR);

            // Create directories if they don't exist
            if !config_dir.exists() {
                fs::create_dir_all(&config_dir)?;
            }

            let config_path = config_dir.join(USER_FILE_NAME);
            let content = serde_yaml::to_string(self)?;
            fs::write(config_path, content)?;
            Ok(())
        } else {
            Err(eyre!("Could not determine home directory"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigNote, UserConfig};

    fn parse(content: &str) -> (UserConfig, Vec<ConfigNote>) {
        let mut notes = Vec::new();
        (UserConfig::parse(content, "config.yml", &mut notes), notes)
    }

    #[test]
    fn one_invalid_value_keeps_the_other_settings_and_warns() {
        let (config, notes) = parse("theme: dracula\nmessages_to_load: lots\ndebug: true\n");
        assert_eq!(config.theme, "dracula");
        assert_eq!(config.messages_to_load, 10);
        assert!(config.debug);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].warning && notes[0].text.contains("messages_to_load"));
    }

    #[test]
    fn unknown_keys_and_broken_yaml_are_reported_not_silent() {
        let (config, notes) = parse("theme: dracula\nclient_id: abc\n");
        assert_eq!(config.theme, "dracula");
        assert!(notes
            .iter()
            .any(|n| n.warning && n.text.contains("client_id")));
        let (config, notes) = parse("theme: [unclosed\n");
        assert_eq!(config, UserConfig::default());
        assert!(notes[0].warning && notes[0].text.contains("not valid YAML"));
        let (config, notes) = parse("");
        assert_eq!(config, UserConfig::default());
        assert!(notes.is_empty());
    }

    #[test]
    fn the_cli_page_size_overrides_the_config_and_config_values_are_clamped() {
        use super::resolve_messages_to_load as resolve;
        assert_eq!(resolve(Some(42), 10), (42, None));
        assert_eq!(resolve(None, 25), (25, None));
        let (value, warning) = resolve(None, 0);
        assert_eq!(value, 1);
        assert!(warning.unwrap().contains("1..=100"));
        assert_eq!(resolve(None, 500).0, 100);
    }

    #[test]
    fn test_user_config_default() {
        let config = UserConfig::default();
        assert_eq!(config.theme, "default");
        assert_eq!(config.messages_to_load, 10);
        assert!(!config.debug);
    }

    #[test]
    fn test_user_config_serialization() {
        let config = UserConfig::default();
        let yaml = serde_yaml::to_string(&config).expect("Failed to serialize");

        // Check that default values are present in YAML
        assert!(yaml.contains("theme:"));
        assert!(yaml.contains("default"));
        assert!(yaml.contains("messages_to_load:"));
        assert!(yaml.contains("10"));
        assert!(yaml.contains("debug:"));
        assert!(yaml.contains("false"));
    }
}
