// taken from https://github.com/Nabushika/webexterm

use oauth2::basic::BasicClient;
use oauth2::reqwest::async_http_client as http_client;
use oauth2::url::Url;
use oauth2::{
    AccessToken, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, RedirectUrl, Scope,
    TokenResponse, TokenUrl,
};
use open;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

const INTEGRATION_CLIENT_ID: &str = "INTEGRATION_CLIENT_ID";
const INTEGRATION_CLIENT_SECRET: &str = "INTEGRATION_CLIENT_SECRET";

pub async fn get_webex_client() -> webex::Webex {
    let client_id = env::var(INTEGRATION_CLIENT_ID)
        .unwrap_or_else(|_| panic!("{} not specified in environment", INTEGRATION_CLIENT_ID));
    let client_secret = env::var(INTEGRATION_CLIENT_SECRET)
        .unwrap_or_else(|_| panic!("{} not specified in environment", INTEGRATION_CLIENT_SECRET));

    // println!("Authenticating to webex");
    let token = get_integration_token(client_id, client_secret)
        .await
        .expect("Need token to continue");
    let token: &str = token.secret();
    webex::Webex::new(token).await
}

async fn get_integration_token(
    client_id: String,
    client_secret: String,
) -> Result<AccessToken, Box<dyn std::error::Error + Send + Sync>> {
    let client = BasicClient::new(
        ClientId::new(client_id),
        Some(ClientSecret::new(client_secret)),
        AuthUrl::new("http://webexapis.com/v1/authorize".to_string())?,
        Some(TokenUrl::new(
            "https://webexapis.com/v1/access_token".to_string(),
        )?),
    )
    .set_redirect_uri(
        RedirectUrl::new("http://localhost:8080".to_string()).expect("Invalid redirect url"),
    );

    let (auth_url, csrf_state) = client
        .authorize_url(CsrfToken::new_random)
        // This example is requesting access to the user's public repos and email.
        .add_scope(Scope::new("spark:all".to_string()))
        .url();

    // println!("Browse to: {}", auth_url);
    open::that(auth_url.as_str()).expect("opening browser for authentication");

    let listener = TcpListener::bind("127.0.0.1:8080").unwrap();
    if let Some(mut stream) = listener.incoming().flatten().next() {
        let code;
        let state;
        {
            let mut reader = BufReader::new(&stream);

            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();

            let redirect_url = request_line.split_whitespace().nth(1).unwrap();
            let url = Url::parse(&("http://localhost".to_string() + redirect_url)).unwrap();

            let code_pair = url
                .query_pairs()
                .find(|pair| {
                    let &(ref key, _) = pair;
                    key == "code"
                })
                .unwrap();

            let (_, value) = code_pair;
            code = AuthorizationCode::new(value.into_owned());

            let state_pair = url
                .query_pairs()
                .find(|pair| {
                    let &(ref key, _) = pair;
                    key == "state"
                })
                .unwrap();

            let (_, value) = state_pair;
            state = CsrfToken::new(value.into_owned());
        }

        let message = "Go back to your terminal :)";
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{}",
            message.len(),
            message
        );
        stream.write_all(response.as_bytes()).unwrap();

        if state.secret() != csrf_state.secret() {
            return Err("returned state != csrf_state".into());
        }

        // Exchange the code with a token.
        let token_res = client.exchange_code(code).request_async(http_client).await;

        if let Ok(token) = token_res {
            return Ok(token.access_token().clone());
        }
    }
    Err("Error".into())
}
