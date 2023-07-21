pub mod auth;
use std::env;

const INTEGRATION_CLIENT_ID: &str = "INTEGRATION_CLIENT_ID";
const INTEGRATION_CLIENT_SECRET: &str = "INTEGRATION_CLIENT_SECRET";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // get integration credentials from environment variables
    let client_id = env::var(INTEGRATION_CLIENT_ID)
        .unwrap_or_else(|_| panic!("{} not specified in environment", INTEGRATION_CLIENT_ID));
    let client_secret = env::var(INTEGRATION_CLIENT_SECRET)
        .unwrap_or_else(|_| panic!("{} not specified in environment", INTEGRATION_CLIENT_SECRET));
    // obtain token with OAuth2
    println!("Authenticating to webex");
    let token = auth::get_integration_token(client_id, client_secret)
        .await
        .expect("Need token to continue");
    let token: &str = token.secret();

    let webex = webex::Webex::new(token).await;

    println!("Getting list of rooms");
    let rooms = webex.get_all_rooms().await.expect("obtaining rooms");

    println!("{rooms:#?}");

    Ok(())
}
