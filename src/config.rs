// config.rs

// credits: iRigellute/spotify-tui

use color_eyre::eyre::{eyre, Error, Result};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs,
    io::{stdin, Write},
    path::{Path, PathBuf},
};

const DEFAULT_PORT: u16 = 8080;
const FILE_NAME: &str = "client.yml";
const USER_FILE_NAME: &str = "config.yml";
const CONFIG_DIR: &str = ".config";
const APP_CONFIG_DIR: &str = "webex-tui";

fn default_theme_name() -> String {
    "default".to_string()
}

fn default_messages_to_load() -> u32 {
    10
}
// const TOKEN_CACHE_FILE: &str = ".webex_token_cache.json";

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ClientConfig {
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    // FIXME: port should be defined in `user_config` not in here
    pub(crate) port: Option<u16>,
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

struct ConfigPaths {
    pub(crate) config_file_path: PathBuf,
    // pub(crate) token_cache_path: PathBuf,
}

impl ClientConfig {
    pub(crate) fn new() -> ClientConfig {
        ClientConfig {
            client_id: "".to_string(),
            client_secret: "".to_string(),
            port: None,
        }
    }

    /// Returns the path(s) to config files, creating them if needed.
    fn get_or_build_paths(&self) -> Result<ConfigPaths> {
        match dirs::home_dir() {
            Some(home) => {
                let path = Path::new(&home);
                let home_config_dir = path.join(CONFIG_DIR);
                let app_config_dir = home_config_dir.join(APP_CONFIG_DIR);

                if !home_config_dir.exists() {
                    fs::create_dir(&home_config_dir)?;
                }

                if !app_config_dir.exists() {
                    fs::create_dir(&app_config_dir)?;
                }

                let config_file_path = &app_config_dir.join(FILE_NAME);
                // let token_cache_path = &app_config_dir.join(TOKEN_CACHE_FILE);

                let paths = ConfigPaths {
                    config_file_path: config_file_path.to_path_buf(),
                    // token_cache_path: token_cache_path.to_path_buf(),
                };

                Ok(paths)
            }
            None => Err(eyre!("No $HOME directory found for client config")),
        }
    }

    /// Reads the configuration from the config file if it exists.
    /// If it doesn't, prompt the user to enter the integration credentials
    /// and save them to the configuration file.
    pub(crate) fn load_config(&mut self) -> Result<()> {
        let paths = self.get_or_build_paths()?;
        if paths.config_file_path.exists() {
            let config_string = fs::read_to_string(&paths.config_file_path)?;
            let config_yml: ClientConfig = serde_yaml::from_str(&config_string)?;

            self.client_id = config_yml.client_id;
            self.client_secret = config_yml.client_secret;
            self.port = config_yml.port;

            Ok(())
        } else {
            println!(
                "Config will be saved to {}",
                paths.config_file_path.display()
            );

            println!("\nHow to get setup:\n");

            let instructions = [
                "Go to the webex integrations page - https://developer.webex.com/docs/integrations",
                "Click `Create an Integration`",
                "Set the integration name to webex-tui",
                "Pick any icon",
                "Add a description of 10+ characters (for example sgrimee/webex-tui)",
                &format!(
                    "Add `http://localhost:{}` to the Redirect URI(s)",
                    DEFAULT_PORT
                ),
                "Under `Scopes`, check `spark:all`",
                "At the bottom, click `Add Integration`",
                "Enter the `Client ID` and `Client Secret` you will get below.",
                "Then your browser should open for the authentication to Webex.",
            ];

            let mut number = 1;
            for item in instructions.iter() {
                println!("  {}. {}", number, item);
                number += 1;
            }

            const EXPECTED_CID_LEN: usize = 65;
            let client_id = ClientConfig::get_client_key_from_input("Client ID", EXPECTED_CID_LEN)?;
            const EXPECTED_CS_LEN: usize = 64;
            let client_secret =
                ClientConfig::get_client_key_from_input("Client Secret", EXPECTED_CS_LEN)?;

            let port = DEFAULT_PORT;

            let config_yml = ClientConfig {
                client_id,
                client_secret,
                port: Some(port),
            };

            let content_yml = serde_yaml::to_string(&config_yml)?;

            let mut new_config = fs::File::create(&paths.config_file_path)?;
            write!(new_config, "{}", content_yml)?;

            self.client_id = config_yml.client_id;
            self.client_secret = config_yml.client_secret;
            self.port = config_yml.port;

            Ok(())
        }
    }

    /// Gets a string typed by the user on the terminal and performs
    /// basic validation.
    fn get_client_key_from_input(
        type_label: &'static str,
        expected_length: usize,
    ) -> Result<String> {
        let mut client_key = String::new();
        const MAX_RETRIES: u8 = 5;
        let mut num_retries = 0;
        loop {
            println!("\nEnter your {}: ", type_label);
            stdin().read_line(&mut client_key)?;
            client_key = client_key.trim().to_string();
            match ClientConfig::validate_client_key(&client_key, expected_length) {
                Ok(_) => return Ok(client_key),
                Err(error_string) => {
                    println!("{}", error_string);
                    client_key.clear();
                    num_retries += 1;
                    if num_retries == MAX_RETRIES {
                        return Err(Error::from(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Maximum retries ({}) exceeded.", MAX_RETRIES),
                        )));
                    }
                }
            };
        }
    }

    /// Performs basic validation on a key, ensuring it is an hexadecimal string
    /// of given `expected_length`.
    fn validate_client_key(key: &str, expected_length: usize) -> Result<()> {
        if key.len() != expected_length {
            Err(Error::from(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "invalid length: {} (must be {})",
                    key.len(),
                    expected_length,
                ),
            )))
        } else if !key.chars().all(|c| c.is_ascii_hexdigit()) {
            Err(Error::from(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid character found (must be hex digits)",
            )))
        } else {
            Ok(())
        }
    }
}

impl UserConfig {
    /// Load user configuration with fallback methods
    pub(crate) fn load() -> Self {
        // Try environment variable first (nix can set this)
        if let Ok(config_path) = env::var("WEBEX_TUI_CONFIG") {
            info!("Loading user config from WEBEX_TUI_CONFIG: {}", config_path);
            if let Ok(config) = Self::load_from_file(&std::path::Path::new(&config_path)) {
                return config;
            }
            warn!("Failed to load config from WEBEX_TUI_CONFIG, trying default location");
        }

        // Try standard location
        if let Some(home) = dirs::home_dir() {
            let config_path = home.join(CONFIG_DIR).join(APP_CONFIG_DIR).join(USER_FILE_NAME);
            info!("Loading user config from: {}", config_path.display());
            if let Ok(config) = Self::load_from_file(&config_path) {
                return config;
            }
        }

        // Use defaults if no config found
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
