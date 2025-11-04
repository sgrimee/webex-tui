// teams/client.rs

//! Obtain a `Webex` client

use gethostname::gethostname;
use log::*;
use oauth2::AccessToken;
use webex::Webex;

/// Return a `Webex` client from the provided `AccessToken`
///
/// The hostname is used to identify the client. A single client should
/// run on a given host at one time, but several clients can run on
/// different hosts.
pub(crate) async fn get_webex_client(token: AccessToken) -> Webex {
    let secret: &str = token.secret();
    let device_name = gethostname()
        .into_string()
        .unwrap_or_else(|_| String::from("unknown"));

    // Use pre-configured mercury URL to skip discovery
    let mercury_url = "https://wdm-a.wbx2.com/wdm/api/v1";
    let client = Webex::new_with_mercury_url(&device_name, secret, mercury_url).await;
    debug!("Authenticated with pre-configured mercury URL.");
    client
}
