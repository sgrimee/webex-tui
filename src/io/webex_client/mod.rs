mod auth;
use auth::get_integration_token;
use std::env;
use webex::Webex;

const INTEGRATION_CLIENT_ID: &str = "INTEGRATION_CLIENT_ID";
const INTEGRATION_CLIENT_SECRET: &str = "INTEGRATION_CLIENT_SECRET";

pub async fn get_webex_client() -> Webex {
    let client_id = env::var(INTEGRATION_CLIENT_ID)
        .unwrap_or_else(|_| panic!("{} not specified in environment", INTEGRATION_CLIENT_ID));
    let client_secret = env::var(INTEGRATION_CLIENT_SECRET)
        .unwrap_or_else(|_| panic!("{} not specified in environment", INTEGRATION_CLIENT_SECRET));

    // println!("Authenticating to webex");
    let token = get_integration_token(client_id, client_secret)
        .await
        .expect("Need token to continue");
    let token: &str = token.secret();
    Webex::new(token).await
}
