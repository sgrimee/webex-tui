// use super::{auth::get_integration_token, ClientCredentials};
use gethostname::gethostname;
use log::*;
use oauth2::AccessToken;
use webex::Webex;

pub async fn get_webex_client(token: AccessToken) -> Webex {
    let secret: &str = token.secret();
    let device_name = gethostname()
        .into_string()
        .unwrap_or_else(|_| String::from("unknown"));
    let client = Webex::new_with_device_name(&device_name, secret).await;
    debug!("Authenticated.");
    client
}
