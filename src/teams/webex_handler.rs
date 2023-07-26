use log::{debug, info};

use super::Teams;

pub async fn process_webex_events(teams: &mut Teams) {
    if let Some(event) = teams.next_event().await {
        debug!(
            "Webex event in Teams thread with type: {:#?}",
            event.activity_type()
        );

        if event.activity_type() == webex::ActivityType::Message(webex::MessageActivity::Posted) {
            // The event stream doesn't contain the message -- you have to go fetch it
            if let Ok(msg) = teams
                .client
                .get::<webex::Message>(&event.get_global_id())
                .await
            {
                info!("Message: {:#?}", msg);
            }
        }
    }
}
