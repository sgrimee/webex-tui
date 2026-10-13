// config.rs

//! User preferences (theme, page size, debug). webex-tui holds no Webex
//! credentials: the webex-bridge daemon owns the Webex connection. A legacy
//! `client.yml` / token cache is ignored, never read or deleted.

use color_eyre::eyre::eyre;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::Path};

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

impl UserConfig {
    /// Load user configuration with fallback methods
    pub(crate) fn load() -> Self {
        // Try environment variable first (nix can set this)
        if let Ok(config_path) = env::var("WEBEX_TUI_CONFIG") {
            info!("Loading user config from WEBEX_TUI_CONFIG: {config_path}");
            if let Ok(config) = Self::load_from_file(std::path::Path::new(&config_path)) {
                return config;
            }
            warn!("Failed to load config from WEBEX_TUI_CONFIG, trying default location");
        }

        // Try standard location
        if let Some(home) = dirs::home_dir() {
            let config_path = home
                .join(CONFIG_DIR)
                .join(APP_CONFIG_DIR)
                .join(USER_FILE_NAME);
            info!("Loading user config from: {}", config_path.display());

            // If config file doesn't exist, create it with defaults
            if !config_path.exists() {
                info!(
                    "No user config found, creating default configuration at: {}",
                    config_path.display()
                );
                let default_config = Self::default();

                // Save default config
                if let Err(e) = default_config.save() {
                    warn!("Failed to create default config file: {e}");
                } else {
                    info!("Created default config file: {}", config_path.display());
                }

                // Copy bundled theme files to user's config directory
                if let Err(e) = crate::theme::copy_bundled_themes() {
                    warn!("Failed to copy theme files: {e}");
                } else {
                    info!("Copied theme files to user config directory");
                }

                return default_config;
            }

            if let Ok(config) = Self::load_from_file(&config_path) {
                return config;
            }
        }

        // Use defaults if no config found (shouldn't reach here normally)
        info!("No user config found, using defaults");
        Self::default()
    }

    /// Load configuration from a specific file
    fn load_from_file(path: &Path) -> color_eyre::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: UserConfig = serde_yaml::from_str(&content)?;
        Ok(config)
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

    use super::*;

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
