// teams/mod.rs

//! Handles IO for Webex, including making network calls
//! and listening to events.

pub(crate) mod app_handler;
pub(crate) mod auth;
mod client;
pub(crate) mod token_cache;
mod webex_handler;

use self::{app_handler::AppCmdEvent, client::get_webex_client};
use crate::app::App;

use log::*;
use oauth2::AccessToken;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;

use webex::{error::Error as WebexError, Webex, WebexEventStream};

/// ClientCredentials obtained when creating the Webex integration
#[derive(Clone)]
pub(crate) struct ClientCredentials {
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
}

/// `Teams` is meant to run in a separate thread from the `App`.
/// It makes API calls to Webex.
pub(crate) struct Teams<'a> {
    client: Webex,
    app: Arc<tokio::sync::Mutex<App<'a>>>,
}

impl<'a> Teams<'a> {
    pub(crate) async fn new(
        token: AccessToken,
        app: Arc<tokio::sync::Mutex<App<'a>>>,
    ) -> Teams<'a> {
        let client = get_webex_client(token).await;
        Self { client, app }
    }

    /// Spawns a new thread to receive events from Webex
    /// and send them to the `Teams` thread for handling
    pub(crate) async fn handle_events(
        &mut self,
        mut app_to_teams_rx_lowpri: UnboundedReceiver<AppCmdEvent>,
        mut app_to_teams_rx_highpri: UnboundedReceiver<AppCmdEvent>,
    ) {
        // Webex events
        let client = self.client.clone();
        let (wbx_stream_to_teams_tx, mut wbx_stream_to_teams_rx) =
            tokio::sync::mpsc::channel::<webex::Event>(100);

        tokio::spawn(async move {
            // This is the webex events stream thread
            loop {
                let event_stream_opt = initialize_event_stream(&client).await;
                if let Some(mut event_stream) = event_stream_opt {
                    loop {
                        match event_stream.next().await {
                            Ok(event) => wbx_stream_to_teams_tx
                                .send(event)
                                .await
                                .expect("sending event from event stream thread to teams thread"),
                            Err(e) => {
                                if !event_stream.is_open {
                                    warn!("Event stream closed, reopening.");
                                    break;
                                }
                                error!("Error received from event stream: {e}");
                                break;
                            }
                        }
                    }
                } else {
                    // Event stream initialization failed due to permissions
                    // Don't retry, just exit this thread
                    error!("Event stream thread exiting - permissions issue");
                    break;
                }
            }
        });

        loop {
            tokio::select! {
                Some(app_event) = app_to_teams_rx_highpri.recv() => {
                    trace!("Got high priority app event: {app_event:#?}");
                    self.handle_app_event(app_event).await;
                },
                Some(webex_event) = wbx_stream_to_teams_rx.recv() => {
                    trace!("Got webex event: {webex_event:#?}" );
                    if let Err(err) = self.handle_webex_event(webex_event).await {
                        error!("Error handling webex event: {err}");
                    }
                },
                Some(app_event) = app_to_teams_rx_lowpri.recv() => {
                    trace!("Got low priority app event: {app_event:#?}");
                    self.handle_app_event(app_event).await;
                }
            }
        }
    }
}

// Run this in the webex events stream thread.

/// Tries hard to get an event stream. Can block up to 60 seconds.
/// Returns None if event stream cannot be established due to permission errors.
async fn initialize_event_stream(client: &Webex) -> Option<WebexEventStream> {
    loop {
        let es = client.event_stream().await;
        match es {
            Ok(stream) => {
                return Some(stream);
            }
            Err(e) => {
                // Check if it's a 403 error (likely missing required scopes)
                if let WebexError::StatusText(status, msg) = &e {
                    if status.as_u16() == 403 {
                        error!("========================================================================");
                        error!("CRITICAL: Event stream initialization failed with 403 Forbidden");
                        error!("Error details: {msg}");
                        error!("");
                        error!("This usually means your Webex integration is missing required OAuth scopes:");
                        error!("  - spark:devices_write");
                        error!("  - spark:devices_read");
                        error!("");
                        error!("The event stream is ESSENTIAL for real-time message updates.");
                        error!("Without it, you must manually reload rooms to see new messages.");
                        error!("");
                        error!("To fix this:");
                        error!("  1. Go to https://developer.webex.com/my-apps");
                        error!("  2. Select your integration");
                        error!("  3. Add the missing scopes");
                        error!("  4. Delete token cache: rm ~/.cache/webex-tui/tokens.json");
                        error!("  5. Re-authenticate webex-tui");
                        error!("========================================================================");
                        // Return None - don't retry on scope errors
                        error!("Continuing without event stream (manual reload required)");
                        return None;
                    } else {
                        error!("Failed to start event stream, trying again in 1 minute: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    }
                } else {
                    error!("Failed to start event stream, trying again in 1 minute: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            }
        };
    }
}
