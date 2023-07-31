/// The teams module handles IO for Webex, including making
/// network calls and listening to events.
pub mod app_handler;
mod auth;
mod client;
mod webex_handler;
use self::{app_handler::AppCmdEvent, client::get_webex_client};
use crate::app::App;
use log::{debug, info};
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use webex::Webex;

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
        credentials: ClientCredentials,
        app: Arc<tokio::sync::Mutex<App<'a>>>,
    ) -> Teams<'a> {
        let client = get_webex_client(credentials).await;

        if let Ok(me) = client.me().await {
            info!("We are: {}", me.display_name);
            let mut app = app.lock().await;
            app.set_me_user(me);
        }

        Self { client, app }
    }

    // pub async fn handle_events(&mut self, app_to_teams_rx: Receiver<AppCmdEvent>) {
    pub async fn handle_events(&mut self, mut app_to_teams_rx: Receiver<AppCmdEvent>) {
        // Webex events
        let mut event_stream = self
            .client
            .event_stream()
            .await
            .expect("Creating webex event stream");

        let (wbx_stream_to_teams_tx, mut wbx_stream_to_teams_rx) =
            tokio::sync::mpsc::channel::<webex::Event>(100);

        tokio::spawn(async move {
            while let Ok(event) = event_stream.next().await {
                wbx_stream_to_teams_tx
                    .send(event)
                    .await
                    .expect("creating webex event stream")
            }
        });

        loop {
            tokio::select! {
                Some(webex_event) = wbx_stream_to_teams_rx.recv() => {
                debug!("Got webex msg: {:#?}", webex_event );
                self.handle_webex_event(webex_event).await;
                },
                Some(app_event) = app_to_teams_rx.recv() => {
                    debug!("Got app event");
                    self.handle_app_event(app_event).await;
                }
            }
        }
    }
}
