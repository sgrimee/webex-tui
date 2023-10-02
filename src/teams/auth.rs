// taken from https://github.com/Nabushika/webexterm

use super::ClientCredentials;
use color_eyre::eyre::{eyre, Result};
use oauth2::basic::BasicClient;
use oauth2::reqwest::async_http_client as http_client;
use oauth2::url::Url;
use oauth2::{
    AccessToken, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, RedirectUrl, Scope,
    TokenResponse, TokenUrl,
};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::net::TcpStream;
use webbrowser;

pub async fn get_integration_token(credentials: ClientCredentials) -> Result<AccessToken> {
    let client = create_basic_client(credentials)?;

    let (auth_url, csrf_state) = get_authorize_url(&client)?;

    if open_web_browser(&auth_url).is_err() {
        let msg = format!("We were unable to open a browser. You may try again after setting the BROWSER environment variable,
or open the following url manually:\n{}\n",
        auth_url
    );
        println!("{}", msg);
        qr2term::print_qr(auth_url.as_str()).unwrap();
    }

    let mut stream = await_authorization_callback().await?;
    println!("Got callback");

    let (code, state) = parse_authorization_response(&mut stream)?;
    send_success_response(&mut stream)?;

    if state.secret() != csrf_state.secret() {
        return Err(eyre!(
            "Invalid CSRF authorization code received on callback"
        ));
    }
    let token_res = client
        .exchange_code(code)
        .request_async(http_client)
        .await?;

    return Ok(token_res.access_token().clone());
}

fn create_basic_client(credentials: ClientCredentials) -> Result<BasicClient> {
    let client = BasicClient::new(
        ClientId::new(credentials.client_id),
        Some(ClientSecret::new(credentials.client_secret)),
        AuthUrl::new("http://webexapis.com/v1/authorize".to_string())?,
        Some(TokenUrl::new(
            "https://webexapis.com/v1/access_token".to_string(),
        )?),
    )
    .set_redirect_uri(
        RedirectUrl::new("http://127.0.0.1:8080".to_string()).expect("Invalid redirect url"),
    );

    Ok(client)
}

fn get_authorize_url(client: &BasicClient) -> Result<(Url, CsrfToken)> {
    let (auth_url, csrf_state) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("spark:all".to_string()))
        .url();

    Ok((auth_url, csrf_state))
}

fn open_web_browser(auth_url: &Url) -> Result<()> {
    webbrowser::open(auth_url.as_str())?;
    Ok(())
}

async fn await_authorization_callback() -> Result<TcpStream> {
    let listener = TcpListener::bind("0.0.0.0:8080")?;
    let stream = listener.incoming().flatten().next().unwrap();
    Ok(stream)
}

fn parse_authorization_response(stream: &mut TcpStream) -> Result<(AuthorizationCode, CsrfToken)> {
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    println!("parse_authorization_response line: {}", &request_line);

    let redirect_url = request_line.split_whitespace().nth(1).unwrap();
    let url = Url::parse(redirect_url).unwrap();

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
