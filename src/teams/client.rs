use super::{auth::get_integration_token, ClientCredentials};
use log::*;
use webex::Webex;

pub async fn get_webex_client(credentials: ClientCredentials) -> Webex {
    trace!("Getting OAuth token");
    let token = get_integration_token(credentials)
        .await
        .expect("Need token to continue");
    let token: &str = token.secret();
    let client = Webex::new(token).await;
    debug!("Authenticated.");
    client
}
