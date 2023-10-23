/// The teams module handles IO for Webex, including making
/// network calls and listening to events.
pub mod app_handler;
pub mod auth;
mod client;
mod webex_handler;

use self::{app_handler::AppCmdEvent, client::get_webex_client};
use crate::app::teams_store::RoomId;
use crate::app::App;

// use color_eyre::eyre::Result;
use log::*;
use oauth2::AccessToken;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;

use webex::{GlobalId, GlobalIdType, Person, Room, Webex, WebexEventStream};

#[derive(Clone)]
pub struct ClientCredentials {
    pub client_id: String,
    pub client_secret: String,
}

pub struct Teams<'a> {
    client: Webex,
    app: Arc<tokio::sync::Mutex<App<'a>>>,
    // app_cmd_handler: AppCmdHandler<'a>,
}

impl<'a> Teams<'a> {
    pub async fn new(
        // credentials: ClientCredentials,
        token: AccessToken,
        app: Arc<tokio::sync::Mutex<App<'a>>>,
    ) -> Teams<'a> {
        let client = get_webex_client(token).await;

        // Retrieve the logged in user
        // TODO: should we do this after initialisation to reduce startup time
        if let Ok(me) = client
            .get::<Person>(&GlobalId::new_with_cluster_unchecked(
                GlobalIdType::Person,
                "me".to_string(),
                None,
            ))
            .await
        {
            debug!("We are: {}", me.display_name);
            let mut app = app.lock().await;
            app.set_me_user(me);
        }

        Self { client, app }
    }

    // pub async fn handle_events(&mut self, app_to_teams_rx: Receiver<AppCmdEvent>) {
    pub async fn handle_events(&mut self, mut app_to_teams_rx: UnboundedReceiver<AppCmdEvent>) {
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
                            error!("Error received from event stream: {}", e);
                            break;
                        }
                    }
                }
            }
        });

        loop {
            tokio::select! {
                Some(webex_event) = wbx_stream_to_teams_rx.recv() => {
                trace!("Got webex event: {:#?}", webex_event );
                self.handle_webex_event(webex_event).await;
                },
                Some(app_event) = app_to_teams_rx.recv() => {
                    trace!("Got app event: {:#?}", app_event);
                    self.handle_app_event(app_event).await;
                }
            }
        }
    }

    pub async fn refresh_room_roomid(&mut self, id: &RoomId) {
        debug!("Getting room with local id: {}", id);
        let global_id = GlobalId::new(GlobalIdType::Room, id.to_owned()).unwrap();
        self.refresh_room_globalid(global_id).await
    }

    pub async fn refresh_room_globalid(&mut self, global_id: GlobalId) {
        debug!("Getting room with global id: {:?}", global_id);
        match self.client.get::<Room>(&global_id).await {
            Ok(room) => {
                let mut app = self.app.lock().await;
                app.room_updated(room);
            }
            Err(error) => error!("Error retrieving room: {}", error),
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
                error!(
                    "Failed to start event stream, trying again in 1 minute: {}",
                    e
                );
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        };
    }
    event_stream
}
