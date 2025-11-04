// teams/auth.rs

//! Handles OAuth authentication to the user's `Webex` integration.
//!
//! Inspired by `https://github.com/Nabushika/webexterm`

use super::token_cache::{clear_token_cache, load_token_cache, save_token_cache, TokenCache};
use crate::ClientCredentials;
use color_eyre::eyre::{eyre, Result};
use log::*;
use oauth2::basic::BasicClient;
use oauth2::url::Url;
use oauth2::{
    AccessToken, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    RedirectUrl, Scope, TokenResponse, TokenUrl,
};

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::net::TcpStream;

/// Get an integration token, using cached token if available and valid,
/// otherwise falling back to browser authentication.
pub(crate) async fn get_integration_token_cached(
    credentials: ClientCredentials,
    port: u16,
) -> Result<AccessToken> {
    // First, try to load and use cached token
    match load_token_cache() {
        Ok(cache) => {
            if cache.is_likely_valid() {
                info!("Using cached authentication token");
                return Ok(cache.to_access_token());
            } else {
                info!("Cached token expired or invalid, clearing cache");
                if let Err(e) = clear_token_cache() {
                    warn!("Failed to clear expired token cache: {e}");
                }
            }
        }
        Err(e) => {
            debug!("No valid cached token found: {e}");
        }
    }

    // No valid cached token, perform browser authentication
    info!("No valid cached token, starting browser authentication");
    let token = get_integration_token_browser(credentials, port).await?;

    // Cache the new token for future use
    let cache = TokenCache::new(token.clone(), None, None);
    if let Err(e) = save_token_cache(&cache) {
        warn!("Failed to cache authentication token: {e}");
        // Don't fail the authentication if caching fails
    } else {
        info!("Authentication token cached successfully");
    }

    Ok(token)
}

/// Try to authenticate with a specific set of scopes
async fn try_auth_with_scopes(
    credentials: &ClientCredentials,
    scopes: &[&str],
    port: u16,
) -> Result<(AccessToken, Vec<String>)> {
    let client = BasicClient::new(ClientId::new(credentials.client_id.clone()))
        .set_client_secret(ClientSecret::new(credentials.client_secret.clone()))
        .set_auth_uri(AuthUrl::new(
            "https://webexapis.com/v1/authorize".to_string(),
        )?)
        .set_token_uri(TokenUrl::new(
            "https://webexapis.com/v1/access_token".to_string(),
        )?)
        .set_redirect_uri(RedirectUrl::new(format!("http://localhost:{port}"))?);

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let mut auth_url_builder = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(pkce_challenge);

    for scope in scopes {
        auth_url_builder = auth_url_builder.add_scope(Scope::new(scope.to_string()));
    }

    let (auth_url, csrf_token) = auth_url_builder.url();

    println!("Requesting authorization...");
    debug!("Requesting scopes: {scopes:?}");

    if webbrowser::open(auth_url.as_str()).is_err() {
        let msg = format!("We were unable to open a browser. You may quit with Ctrl+C and try again after setting 
the BROWSER environment variable, or open the following url manually (on this computer):\n{auth_url}\n");
        println!("{msg}");
    }

    let mut stream = await_authorization_callback(port).await?;
    let (code, state) = parse_authorization_response(&mut stream)?;
    send_success_response(&mut stream)?;

    if state.secret() != csrf_token.secret() {
        return Err(eyre!(
            "Invalid CSRF authorization code received on callback"
        ));
    }

    let http_client = reqwest::ClientBuilder::new().build()?;
    let token_result = client
        .exchange_code(code)
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await?;

    // Get the actual scopes granted (if available in the response)
    let granted_scopes = token_result
        .scopes()
        .map(|scopes| scopes.iter().map(|s| s.as_str().to_string()).collect())
        .unwrap_or_else(|| scopes.iter().map(|s| s.to_string()).collect());

    Ok((token_result.access_token().clone(), granted_scopes))
}

/// Create and authorize a client with the given `ClientCredentials`.
/// A browser is opened for user authentication.
/// Returns a token, or an error if any authentication step fail.
async fn get_integration_token_browser(
    credentials: ClientCredentials,
    port: u16,
) -> Result<AccessToken> {
    // Desired scopes for full webex-tui functionality (scope, description, critical)
    let desired_scopes = vec![
        (
            "spark:all",
            "All spark permissions (alternative to granular scopes)",
            false,
        ),
        (
            "spark:applications_token",
            "Retrieve Service App token",
            true,
        ),
        ("spark:messages_write", "Post and delete messages", true),
        ("spark:messages_read", "Read room messages", true),
        ("spark:memberships_write", "Invite people to rooms", true),
        ("spark:memberships_read", "List room members", true),
        ("spark:people_write", "Write user directory", false),
        ("spark:people_read", "Read user directory", true),
        ("spark:rooms_write", "Manage rooms", true),
        ("spark:rooms_read", "List rooms", true),
        ("spark:teams_write", "Create teams", false),
        ("spark:teams_read", "List teams", true),
        (
            "spark:team_memberships_write",
            "Add people to teams (leave team)",
            false,
        ),
        ("spark:team_memberships_read", "List team members", false),
        (
            "spark:devices_write",
            "Register device for real-time events",
            true,
        ),
        (
            "spark:devices_read",
            "Read device info for real-time events",
            true,
        ),
        ("spark:organizations_read", "Read organization info", false),
        ("application:webhooks_write", "Register webhooks", false),
        ("application:webhooks_read", "List webhooks", false),
    ];

    let scopes_list: Vec<&str> = desired_scopes.iter().map(|(s, _, _)| *s).collect();

    // Try authentication with granular scopes
    println!("Attempting authentication with granular scopes...");
    let result = try_auth_with_scopes(&credentials, &scopes_list, port).await;

    let (token, granted_scopes) = match result {
        Ok((token, scopes)) => (token, scopes),
        Err(e) if e.to_string().contains("invalid_scope") || e.to_string().contains("scope") => {
            // Scope error - try with spark:all instead
            println!();
            println!("Granular scopes not supported by this integration.");
            println!("Trying with spark:all scope...");
            try_auth_with_scopes(&credentials, &["spark:all"], port).await?
        }
        Err(e) => return Err(e),
    };

    // Analyze granted scopes and warn about missing functionality
    println!();
    println!("========================================================================");
    println!("Authentication successful!");
    println!("------------------------------------------------------------------------");
    println!("Granted scopes: {}", granted_scopes.join(", "));
    println!("------------------------------------------------------------------------");

    // Check for missing critical scopes
    let mut missing_critical = Vec::new();
    let mut missing_optional = Vec::new();

    let has_spark_all = granted_scopes.iter().any(|s| s == "spark:all");

    if !has_spark_all {
        for (scope, description, critical) in &desired_scopes {
            if !granted_scopes.iter().any(|s| s == scope) {
                if *critical {
                    missing_critical.push((*scope, *description));
                } else {
                    missing_optional.push((*scope, *description));
                }
            }
        }
    }

    if !missing_critical.is_empty() {
        println!("WARNING: Missing critical scopes:");
        for (scope, desc) in &missing_critical {
            println!("  - {scope}: {desc}");
        }
        println!();
    }

    if !missing_optional.is_empty() {
        println!("INFO: Missing optional scopes (reduced functionality):");
        for (scope, desc) in &missing_optional {
            println!("  - {scope}: {desc}");
        }
        println!();
    }

    if missing_critical.is_empty() && missing_optional.is_empty() && !has_spark_all {
        println!("All desired scopes granted!");
    }

    if has_spark_all {
        println!("Using spark:all scope (grants most permissions)");
        println!("NOTE: spark:all may conflict with team_memberships scopes");
        println!("      You may not be able to leave teams/rooms");
    }

    println!("========================================================================");
    println!();

    Ok(token)
}

/// Backward compatibility alias for the cached authentication function
pub(crate) async fn get_integration_token(
    credentials: ClientCredentials,
    port: u16,
) -> Result<AccessToken> {
    get_integration_token_cached(credentials, port).await
}

/// Listen on local port for OAuth callback and return the TCP stream
async fn await_authorization_callback(port: u16) -> Result<TcpStream> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))?;
    let stream = listener.incoming().flatten().next().unwrap();
    Ok(stream)
}

/// Parse a TCP stream for OAuth callback content, return the `AuthorizationCode` and `CsrfToken`
/// Panics if the stream is not correctly formatted.
fn parse_authorization_response(stream: &mut TcpStream) -> Result<(AuthorizationCode, CsrfToken)> {
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();

    let redirect_url = request_line.split_whitespace().nth(1).unwrap();
    let url = Url::parse(&("http://localhost".to_string() + redirect_url)).unwrap();

    // Debug: print all query parameters
    debug!("OAuth redirect URL: {redirect_url}");
    debug!("Parsed URL query parameters:");
    for (key, value) in url.query_pairs() {
        debug!("  {key}: {value}");
    }

    let code_pair = url
        .query_pairs()
        .find(|pair| {
            let (key, _) = pair;
            key == "code"
        })
                        .ok_or_else(|| {
                            let params: Vec<String> = url.query_pairs()
                                .map(|(key, value)| format!("{key}={value}"))
                                .collect();

                            // Check if this is a scope-related error
                            let error_msg = url.query_pairs()
                                .find(|(key, _)| key == "error_description")
                                .map(|(_, value)| value.to_string())
                                .or_else(|| url.query_pairs()
                                    .find(|(key, _)| key == "error")
                                    .map(|(_, value)| value.to_string()))
                                .unwrap_or_else(|| "Unknown OAuth error".to_string());

                            if error_msg.contains("scope") || error_msg.contains("invalid_scope") {
                                eyre!("OAuth scope error: {}. Your Webex integration may be configured with different scopes than requested. Common alternatives: spark:all (grants most permissions but may conflict with team_memberships scopes), or individual scopes. Please check your integration configuration at https://developer.webex.com/my-apps", error_msg)
                            } else {
                                eyre!("OAuth authentication failed: {}. Available parameters: [{}]", error_msg, params.join(", "))
                            }
                        })?;

    let (_, value) = code_pair;
    let code = AuthorizationCode::new(value.into_owned());

    let state_pair = url
        .query_pairs()
        .find(|pair| {
            let (key, _) = pair;
            key == "state"
        })
        .expect("Could not find state param in incoming redirect call.");

    let (_, value) = state_pair;
    let state = CsrfToken::new(value.into_owned());

    Ok((code, state))
}

/// Send an HTTP response on the TCP stream indicating success.
fn send_success_response(stream: &mut TcpStream) -> Result<()> {
    let message = "Webex authentication complete. You can close this and enjoy webex-tui.";
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{}",
        message.len(),
        message
    );
    let _ = stream.write_all(response.as_bytes());
    Ok(())
}
