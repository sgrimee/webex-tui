use chrono::{DateTime, Duration, Utc};
use webex::Room as WebexRoom;

pub type RoomId = String;

/// `Room` is a wrapper around the webex Room type, adding some extra information.

#[derive(Debug)]
pub struct Room {
    id: String,
    title: Option<String>,
    room_type: String,
    // is_locked: bool,
    // team_id: Option<String>,
    last_activity: DateTime<Utc>,
    // creator_id: String,
    // created: String,
    unread: bool,
}

impl Room {
    /// Returns whether a room is a 1-1 chat
    pub fn is_direct(&self) -> bool {
        self.room_type == "direct"
    }

    /// Returns whether a room is a space.
    pub fn is_space(&self) -> bool {
        self.room_type == "group"
    }

    pub fn id(&self) -> &RoomId {
        &self.id
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn last_activity(&self) -> DateTime<Utc> {
        self.last_activity
    }

    pub fn unread(&self) -> bool {
        self.unread
    }

    pub fn set_unread(&mut self, unread: bool) {
        self.unread = unread;
    }

    /// Returns whether the room has seen any activity in the past specified period.
    /// Panics if room is not known.
    pub fn has_activity_since(&self, duration: Duration) -> bool {
        self.last_activity() > (Utc::now() - duration)
    }

    /// Updates the last activity of the room if the new activity is more recent.
    pub fn update_last_activity(&mut self, last_activity: DateTime<Utc>) {
        if last_activity > self.last_activity {
            self.last_activity = last_activity;
        }
    }
}

impl From<WebexRoom> for Room {
    fn from(webex_room: WebexRoom) -> Self {
        Room {
            // original webex::Room fields
            id: webex_room.id,
            title: webex_room.title,
            room_type: webex_room.room_type,
            // is_locked: webex_room.is_locked,
            // team_id: webex_room.team_id,
            // creator_id: webex_room.creator_id,
            // created: webex_room.created,
            // fields added or modified by this crate
            last_activity: DateTime::parse_from_rfc3339(&webex_room.last_activity)
                .unwrap()
                .with_timezone(&Utc),
            unread: false,
        }
    }
}
