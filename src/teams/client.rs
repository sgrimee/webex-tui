// use super::{auth::get_integration_token, ClientCredentials};
use log::*;
use oauth2::AccessToken;
use webex::Webex;

pub async fn get_webex_client(token: AccessToken) -> Webex {
    let secret: &str = token.secret();
    let client = Webex::new(secret).await;
    debug!("Authenticated.");
    client
}
