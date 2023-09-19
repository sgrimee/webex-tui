// credits: iRigellute/spotify-tui

use super::banner::BANNER;
use color_eyre::eyre::{eyre, Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{stdin, Write},
    path::{Path, PathBuf},
};

const DEFAULT_PORT: u16 = 8080;
const FILE_NAME: &str = "client.yml";
const CONFIG_DIR: &str = ".config";
const APP_CONFIG_DIR: &str = "webex-tui";
const TOKEN_CACHE_FILE: &str = ".webex_token_cache.json";

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientConfig {
    pub client_id: String,
    pub client_secret: String,
    // FIXME: port should be defined in `user_config` not in here
    pub port: Option<u16>,
}

pub struct ConfigPaths {
    pub config_file_path: PathBuf,
    pub token_cache_path: PathBuf,
}

impl ClientConfig {
    pub fn new() -> ClientConfig {
        ClientConfig {
            client_id: "".to_string(),
            client_secret: "".to_string(),
            port: None,
        }
    }

    // pub fn get_redirect_uri(&self) -> String {
    //     format!("http://localhost:{}", self.get_port())
    // }

    // pub fn get_port(&self) -> u16 {
    //     self.port.unwrap_or(DEFAULT_PORT)
    // }

    pub fn get_or_build_paths(&self) -> Result<ConfigPaths> {
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
                let token_cache_path = &app_config_dir.join(TOKEN_CACHE_FILE);

                let paths = ConfigPaths {
                    config_file_path: config_file_path.to_path_buf(),
                    token_cache_path: token_cache_path.to_path_buf(),
                };

                Ok(paths)
            }
            None => Err(eyre!("No $HOME directory found for client config")),
        }
    }

    pub fn load_config(&mut self) -> Result<()> {
        let paths = self.get_or_build_paths()?;
        if paths.config_file_path.exists() {
            let config_string = fs::read_to_string(&paths.config_file_path)?;
            let config_yml: ClientConfig = serde_yaml::from_str(&config_string)?;

            self.client_id = config_yml.client_id;
            self.client_secret = config_yml.client_secret;
            self.port = config_yml.port;

            Ok(())
        } else {
            println!("{}", BANNER);

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

            // let mut port = String::new();
            // println!("\nEnter port of redirect uri (default {}): ", DEFAULT_PORT);
            // stdin().read_line(&mut port)?;
            // let port = port.trim().parse::<u16>().unwrap_or(DEFAULT_PORT);
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
