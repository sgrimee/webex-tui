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

/// Create and authorize a client with the given `ClientCredentials`.
/// A browser is opened for user authentication.
/// Returns a token, or an error if any authentication step fail.
async fn get_integration_token_browser(
    credentials: ClientCredentials,
    port: u16,
) -> Result<AccessToken> {
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

    // Use required scopes for webex-tui functionality - must match Webex integration configuration
    let scopes = vec![
        "spark:applications_token",     // Retrieve Service App token
        "spark:messages_write",         // Post and delete messages on your behalf
        "spark:messages_read",          // Read the content of rooms that you are in
        "spark:memberships_write",      // Invite people to rooms on your behalf
        "spark:memberships_read",       // List people in the rooms you are in
        "spark:people_write",           // Write user directory information
        "spark:people_read",            // Read your users' company directory
        "spark:rooms_write",            // Manage rooms on your behalf
        "spark:rooms_read",             // List the titles of rooms that you are in
        "spark:teams_write",            // Create teams on your users' behalf
        "spark:teams_read",             // List the teams your user's a member of
        "spark:team_memberships_write", // Add people to teams on your users' behalf
        "spark:team_memberships_read",  // List the people in the teams your user belongs to
        "spark:devices_write",          // Modify and delete your devices
        "spark:devices_read",           // See details for your devices
        "spark:organizations_read",     // Access to read your user's organizations
        "application:webhooks_write",   // Register Service App authorization webhook
        "application:webhooks_read",    // List webhooks for Service App authorization
    ];

    // Generate the authorization URL
    let mut auth_url_builder = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(pkce_challenge);

    // Add all required scopes
    for scope in &scopes {
        auth_url_builder = auth_url_builder.add_scope(Scope::new(scope.to_string()));
    }

    let (auth_url, csrf_token) = auth_url_builder.url();

    println!("Requesting authorization for required scopes...");
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

    let http_client = reqwest::ClientBuilder::new()
        // Following redirects opens the client up to SSRF vulnerabilities.
        // .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Client should build");

    let token_result = client
        .exchange_code(code)
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await?;

    Ok(token_result.access_token().clone())
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
                                eyre!("OAuth scope error: {}. Your Webex integration must be configured with these exact scopes: spark:applications_token, spark:messages_write, spark:messages_read, spark:memberships_write, spark:memberships_read, spark:people_write, spark:people_read, spark:rooms_write, spark:rooms_read, spark:teams_write, spark:teams_read, spark:team_memberships_write, spark:team_memberships_read, spark:devices_write, spark:devices_read, spark:organizations_read, application:webhooks_write, application:webhooks_read. Please update your integration at https://developer.webex.com/my-apps", error_msg)
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
