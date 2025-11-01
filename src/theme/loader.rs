//! Theme loading functionality

use super::models::Theme;
use color_eyre::Result;
use log::{debug, info, warn};
use std::fs;
use std::path::PathBuf;

/// Load a theme by name, with fallback to default theme
pub fn load_theme(theme_name: &str) -> Theme {
    match try_load_theme(theme_name) {
        Ok(theme) => {
            info!("Successfully loaded theme: {}", theme.name);
            theme
        }
        Err(e) => {
            warn!("Failed to load theme '{theme_name}': {e}. Using default theme.");
            Theme::default()
        }
    }
}

/// Attempt to load a theme file, returning errors for handling
fn try_load_theme(theme_name: &str) -> Result<Theme> {
    let theme_path = get_theme_path(theme_name)?;

    debug!("Loading theme from: {}", theme_path.display());

    let content = fs::read_to_string(&theme_path)?;
    let mut theme: Theme = serde_yaml::from_str(&content)?;

    // Ensure the theme name matches (in case it's different in the file)
    if theme.name.is_empty() {
        theme.name = theme_name.to_string();
    }

    // Validate user_colors array size to prevent memory issues
    if theme.user_colors.len() > 64 {
        warn!(
            "Theme '{}' has too many user_colors ({}), truncating to 64",
            theme.name,
            theme.user_colors.len()
        );
        theme.user_colors.truncate(64);
    }

    // If no user colors specified, use defaults
    if theme.user_colors.is_empty() {
        warn!("Theme '{}' has no user_colors, using defaults", theme.name);
        theme.user_colors = Theme::default().user_colors;
    }

    Ok(theme)
}

/// Get the path to a theme file
fn get_theme_path(theme_name: &str) -> Result<PathBuf> {
    let home_dir =
        dirs::home_dir().ok_or_else(|| color_eyre::eyre::eyre!("Could not find home directory"))?;

    let theme_dir = home_dir.join(".config").join("webex-tui").join("themes");
    let theme_file = format!("{theme_name}.yml");
    let theme_path = theme_dir.join(theme_file);

    if !theme_path.exists() {
        return Err(color_eyre::eyre::eyre!(
            "Theme file not found: {}",
            theme_path.display()
        ));
    }

    Ok(theme_path)
}

/// Create the themes directory if it doesn't exist
#[allow(dead_code)]
pub fn ensure_themes_directory() -> Result<PathBuf> {
    let home_dir =
        dirs::home_dir().ok_or_else(|| color_eyre::eyre::eyre!("Could not find home directory"))?;

    let theme_dir = home_dir.join(".config").join("webex-tui").join("themes");

    if !theme_dir.exists() {
        fs::create_dir_all(&theme_dir)?;
        debug!("Created themes directory: {}", theme_dir.display());
    }

    Ok(theme_dir)
}

/// List available themes in the themes directory
#[allow(dead_code)]
pub fn list_available_themes() -> Result<Vec<String>> {
    let theme_dir = match dirs::home_dir() {
        Some(home) => home.join(".config").join("webex-tui").join("themes"),
        None => return Ok(vec![]),
    };

    if !theme_dir.exists() {
        return Ok(vec![]);
    }

    let mut themes = Vec::new();

    for entry in fs::read_dir(theme_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("yml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                themes.push(stem.to_string());
            }
        }
    }

    themes.sort();
    Ok(themes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_default_theme() {
        let theme = load_theme("nonexistent");
        assert_eq!(theme.name, "Default");
        assert!(!theme.user_colors.is_empty());
    }

    #[test]
    fn test_theme_path_generation() {
        // This test will fail if HOME is not set, which is acceptable for unit tests
        if dirs::home_dir().is_some() {
            let path = get_theme_path("test");
            // Should return an error since file doesn't exist, but path should be correctly formed
            assert!(path.is_err());
        }
    }
}
