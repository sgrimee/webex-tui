//! Token caching module for persisting OAuth tokens between sessions
//!
//! This module provides secure disk-based caching of access and refresh tokens
//! to avoid repeated authentication flows, significantly improving startup time.

use color_eyre::eyre::{eyre, Result};
use oauth2::{AccessToken, RefreshToken};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Cached token data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TokenCache {
    /// The OAuth access token
    pub access_token: String,
    /// Optional refresh token for token renewal
    pub refresh_token: Option<String>,
    /// Unix timestamp when the token expires (if known)
    pub expires_at: Option<u64>,
    /// Unix timestamp when this cache entry was created
    pub cached_at: u64,
}

impl TokenCache {
    /// Create a new token cache entry
    pub fn new(
        access_token: AccessToken,
        refresh_token: Option<RefreshToken>,
        expires_in: Option<Duration>,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        let expires_at = expires_in.map(|duration| now + duration.as_secs());

        Self {
            access_token: access_token.secret().to_string(),
            refresh_token: refresh_token.map(|token| token.secret().to_string()),
            expires_at,
            cached_at: now,
        }
    }

    /// Check if the cached token is likely still valid
    pub fn is_likely_valid(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        // If we have an expiration time, check if we're within a 5-minute buffer
        if let Some(expires_at) = self.expires_at {
            return now + 300 < expires_at; // 5 minute buffer
        }

        // If no expiration info, consider valid if cached within last 12 hours
        // OAuth tokens typically last much longer, but we err on the side of caution
        now - self.cached_at < 12 * 60 * 60
    }

    /// Convert back to OAuth AccessToken
    pub fn to_access_token(&self) -> AccessToken {
        AccessToken::new(self.access_token.clone())
    }

    /// Convert back to OAuth RefreshToken if available
    #[allow(dead_code)]
    pub fn to_refresh_token(&self) -> Option<RefreshToken> {
        self.refresh_token
            .as_ref()
            .map(|token| RefreshToken::new(token.clone()))
    }
}

/// Get the path to the token cache file
fn get_cache_file_path() -> Result<PathBuf> {
    get_cache_file_path_impl(None)
}

/// Internal implementation that supports custom suffixes for testing
fn get_cache_file_path_impl(suffix: Option<&str>) -> Result<PathBuf> {
    let cache_dir = dirs::cache_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))
        .ok_or_else(|| eyre!("Could not determine cache directory"))?;

    let app_cache_dir = cache_dir.join("webex-tui");

    // Ensure the cache directory exists
    fs::create_dir_all(&app_cache_dir)?;

    let filename = match suffix {
        Some(suffix) => format!("tokens-{suffix}.json"),
        None => "tokens.json".to_string(),
    };

    Ok(app_cache_dir.join(filename))
}

/// Set restrictive file permissions (user read/write only)
#[cfg(unix)]
fn set_secure_permissions(path: &PathBuf) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600); // User read/write only
    fs::set_permissions(path, permissions)?;
    Ok(())
}

/// Set restrictive file permissions on Windows (best effort)
#[cfg(windows)]
fn set_secure_permissions(_path: &PathBuf) -> Result<()> {
    // On Windows, files in user directories are typically secure by default
    // Additional ACL manipulation would require windows-specific crates
    log::debug!("File permissions on Windows rely on default user directory security");
    Ok(())
}

/// Save token cache to disk securely
pub(crate) fn save_token_cache(cache: &TokenCache) -> Result<()> {
    let cache_path = get_cache_file_path()?;

    log::debug!("Saving token cache to: {cache_path:?}");

    // Create the file with restrictive permissions from the start
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&cache_path)?;

    // Set secure permissions
    set_secure_permissions(&cache_path)?;

    // Write the cache data
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, cache)?;

    log::info!("Token cache saved successfully");
    Ok(())
}

/// Load token cache from disk
pub(crate) fn load_token_cache() -> Result<TokenCache> {
    let cache_path = get_cache_file_path()?;

    if !cache_path.exists() {
        return Err(eyre!("Token cache file does not exist"));
    }

    log::debug!("Loading token cache from: {cache_path:?}");

    let file = File::open(&cache_path)?;
    let reader = BufReader::new(file);
    let cache: TokenCache =
        serde_json::from_reader(reader).map_err(|e| eyre!("Failed to parse token cache: {}", e))?;

    log::info!("Token cache loaded successfully");
    Ok(cache)
}

/// Clear the token cache (e.g., on logout or authentication failure)
pub(crate) fn clear_token_cache() -> Result<()> {
    let cache_path = get_cache_file_path()?;

    if cache_path.exists() {
        fs::remove_file(&cache_path)?;
        log::info!("Token cache cleared");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oauth2::{AccessToken, RefreshToken};
    use std::time::Duration;

    /// Test-specific function to save cache with a unique suffix
    fn save_test_cache(cache: &TokenCache, test_suffix: &str) -> Result<()> {
        let cache_path = get_cache_file_path_impl(Some(test_suffix))?;

        log::debug!("Saving test token cache to: {cache_path:?}");

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&cache_path)?;

        set_secure_permissions(&cache_path)?;

        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, cache)?;

        Ok(())
    }

    /// Test-specific function to load cache with a unique suffix
    fn load_test_cache(test_suffix: &str) -> Result<TokenCache> {
        let cache_path = get_cache_file_path_impl(Some(test_suffix))?;

        if !cache_path.exists() {
            return Err(eyre!("Token cache file does not exist"));
        }

        let file = File::open(&cache_path)?;
        let reader = BufReader::new(file);
        let cache: TokenCache = serde_json::from_reader(reader)
            .map_err(|e| eyre!("Failed to parse token cache: {}", e))?;

        Ok(cache)
    }

    /// Test-specific function to clear cache with a unique suffix
    fn clear_test_cache(test_suffix: &str) -> Result<()> {
        let cache_path = get_cache_file_path_impl(Some(test_suffix))?;

        if cache_path.exists() {
            fs::remove_file(&cache_path)?;
        }

        Ok(())
    }

    #[test]
    fn test_token_cache_creation() {
        let access_token = AccessToken::new("test_access_token".to_string());
        let refresh_token = Some(RefreshToken::new("test_refresh_token".to_string()));
        let expires_in = Some(Duration::from_secs(3600)); // 1 hour

        let cache = TokenCache::new(access_token, refresh_token, expires_in);

        assert_eq!(cache.access_token, "test_access_token");
        assert_eq!(cache.refresh_token, Some("test_refresh_token".to_string()));
        assert!(cache.expires_at.is_some());
    }

    #[test]
    fn test_token_validity_with_expiration() {
        let access_token = AccessToken::new("test_token".to_string());

        // Token that expires in 1 hour (should be valid)
        let future_cache =
            TokenCache::new(access_token.clone(), None, Some(Duration::from_secs(3600)));
        assert!(future_cache.is_likely_valid());

        // Simulate expired token
        let mut expired_cache = TokenCache::new(access_token, None, Some(Duration::from_secs(100)));
        expired_cache.expires_at = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                - 100, // Expired 100 seconds ago
        );
        assert!(!expired_cache.is_likely_valid());
    }

    #[test]
    fn test_token_validity_without_expiration() {
        let access_token = AccessToken::new("test_token".to_string());

        // Recent token without expiration info (should be valid)
        let recent_cache = TokenCache::new(access_token.clone(), None, None);
        assert!(recent_cache.is_likely_valid());

        // Old token without expiration info
        let mut old_cache = TokenCache::new(access_token, None, None);
        old_cache.cached_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 24 * 60 * 60; // 24 hours ago
        assert!(!old_cache.is_likely_valid());
    }

    #[test]
    fn test_cache_round_trip() {
        // Test saving and loading a cache with unique suffix to avoid conflicts
        let test_suffix = "round_trip";
        let access_token = AccessToken::new("test_round_trip_token".to_string());
        let original_cache = TokenCache::new(access_token, None, Some(Duration::from_secs(3600)));

        // Clear any existing test cache first
        let _ = clear_test_cache(test_suffix);

        // Save the cache
        save_test_cache(&original_cache, test_suffix).expect("Should save cache");

        // Load the cache back
        let loaded_cache = load_test_cache(test_suffix).expect("Should load cache");

        // Verify they match
        assert_eq!(original_cache.access_token, loaded_cache.access_token);
        assert_eq!(original_cache.refresh_token, loaded_cache.refresh_token);
        assert_eq!(original_cache.expires_at, loaded_cache.expires_at);
        assert!(loaded_cache.is_likely_valid());

        // Clean up
        let _ = clear_test_cache(test_suffix);
    }

    #[test]
    fn test_cache_clear() {
        // Test clearing a cache with unique suffix to avoid conflicts
        let test_suffix = "clear";
        let access_token = AccessToken::new("test_clear_token".to_string());
        let cache = TokenCache::new(access_token, None, None);

        // Clear any existing test cache first
        let _ = clear_test_cache(test_suffix);

        // Save the cache
        save_test_cache(&cache, test_suffix).expect("Should save cache");

        // Verify it exists
        assert!(load_test_cache(test_suffix).is_ok());

        // Clear it
        clear_test_cache(test_suffix).expect("Should clear cache");

        // Verify it's gone
        assert!(load_test_cache(test_suffix).is_err());
    }
}
