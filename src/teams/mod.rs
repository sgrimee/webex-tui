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
                let mut event_stream = initialize_event_stream(&client).await;
                loop {
                    match event_stream.next().await {
                        Ok(event) => wbx_stream_to_teams_tx
                            .send(event)
                            .await
                            .expect("sending event from event stream thread to teams thread"),
                        Err(e) => {
                            if !event_stream.is_open {
                                warn!("Even stream closed, reopening.");
                                break;
                            }
                            error!("Error received from event stream: {e}");
                            break;
                        }
                    }
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
async fn initialize_event_stream(client: &Webex) -> WebexEventStream {
    let event_stream: WebexEventStream;
    loop {
        let es = client.event_stream().await;
        match es {
            Ok(stream) => {
                event_stream = stream;
                break;
            }
            Err(e) => {
                // Check if it's a 403 error (expected for third-party integrations without device permissions)
                if let WebexError::StatusText(status, _) = &e {
                    if status.as_u16() == 403 {
                        debug!("Device registration returned 403 (expected for integrations without spark:devices_write), retrying in 1 minute");
                    } else {
                        error!("Failed to start event stream, trying again in 1 minute: {e}");
                    }
                } else {
                    error!("Failed to start event stream, trying again in 1 minute: {e}");
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        };
    }
    event_stream
}
