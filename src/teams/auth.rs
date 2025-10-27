// teams/auth.rs

//! Handles OAuth authentication to the user's `Webex` integration.
//!
//! Inspired by `https://github.com/Nabushika/webexterm`

use super::token_cache::{load_token_cache, save_token_cache, clear_token_cache, TokenCache};
use super::ClientCredentials;
use color_eyre::eyre::{eyre, Result};
use log::*;
use oauth2::basic::BasicClient;
use oauth2::url::Url;
use oauth2::{
    reqwest, AccessToken, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    PkceCodeChallenge, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::net::TcpStream;

/// Get an integration token, using cached token if available and valid,
/// otherwise falling back to browser authentication.
pub(crate) async fn get_integration_token_cached(credentials: ClientCredentials, port: u16) -> Result<AccessToken> {
    // First, try to load and use cached token
    match load_token_cache() {
        Ok(cache) => {
            if cache.is_likely_valid() {
                info!("Using cached authentication token");
                return Ok(cache.to_access_token());
            } else {
                info!("Cached token expired or invalid, clearing cache");
                if let Err(e) = clear_token_cache() {
                    warn!("Failed to clear expired token cache: {}", e);
                }
            }
        }
        Err(e) => {
            debug!("No valid cached token found: {}", e);
        }
    }

    // No valid cached token, perform browser authentication
    info!("No valid cached token, starting browser authentication");
    let token = get_integration_token_browser(credentials, port).await?;
    
    // Cache the new token for future use
    let cache = TokenCache::new(token.clone(), None, None);
    if let Err(e) = save_token_cache(&cache) {
        warn!("Failed to cache authentication token: {}", e);
        // Don't fail the authentication if caching fails
    } else {
        info!("Authentication token cached successfully");
    }
    
    Ok(token)
}

/// Create and authorize a client with the given `ClientCredentials`.
/// A browser is opened for user authentication.
/// Returns a token, or an error if any authentication step fail.
async fn get_integration_token_browser(credentials: ClientCredentials, port: u16) -> Result<AccessToken> {
    let client = BasicClient::new(ClientId::new(credentials.client_id))
        .set_client_secret(ClientSecret::new(credentials.client_secret))
        .set_auth_uri(
            AuthUrl::new("https://webexapis.com/v1/authorize".to_string())
                .expect("Invalid auth uri"),
        )
        .set_token_uri(
            TokenUrl::new("https://webexapis.com/v1/access_token".to_string())
                .expect("Invalid token uri"),
        )
        .set_redirect_uri(
            RedirectUrl::new(format!("http://localhost:{}", port)).expect("Invalid redirect url"),
        );

    // Generate a PKCE challenge.
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    // Generate the full authorization URL.
    let (auth_url, csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("spark:all".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    if webbrowser::open(auth_url.as_str()).is_err() {
        let msg = format!("We were unable to open a browser. You may quit with Ctrl+C and try again after setting 
the BROWSER environment variable, or open the following url manually (on this computer):\n{}\n",
        auth_url
    );
        println!("{}", msg);
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

/// Clear cached tokens (useful for logout or when authentication fails)
pub(crate) fn clear_cached_tokens() -> Result<()> {
    clear_token_cache()
}

/// Backward compatibility alias for the cached authentication function
pub(crate) async fn get_integration_token(credentials: ClientCredentials, port: u16) -> Result<AccessToken> {
    get_integration_token_cached(credentials, port).await
}

/// Listen on local port for OAuth callback and return the TCP stream
async fn await_authorization_callback(port: u16) -> Result<TcpStream> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))?;
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

    let code_pair = url
        .query_pairs()
        .find(|pair| {
            let (key, _) = pair;
            key == "code"
        })
        .expect("Could not find code param in incoming redirect call.");

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
