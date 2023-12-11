use chrono::{DateTime, Duration, Utc};
use webex::Room as WebexRoom;

pub(crate) type RoomId = String;

/// `Room` is a wrapper around the webex Room type, adding some extra information.

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct Room {
    pub(crate) id: String,
    pub(crate) title: Option<String>,
    pub(crate) room_type: String,
    // is_locked: bool,
    pub(crate) team_id: Option<String>,
    pub(crate) last_activity: DateTime<Utc>,
    // creator_id: String,
    // created: String,
    pub(crate) unread: bool,
}

impl Room {
    /// Returns whether a room is a 1-1 chat
    pub(crate) fn is_direct(&self) -> bool {
        self.room_type == "direct"
    }

    /// Returns whether a room is a space.
    pub(crate) fn is_space(&self) -> bool {
        self.room_type == "group"
    }

    /// Returns whether the room has seen any activity in the past specified period.
    /// Panics if room is not known.
    pub(crate) fn has_activity_since(&self, duration: Duration) -> bool {
        self.last_activity > (Utc::now() - duration)
    }

    /// Updates the last activity of the room if the new activity is more recent.
    pub(crate) fn update_last_activity(&mut self, last_activity: DateTime<Utc>) {
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
            team_id: webex_room.team_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use std::thread;

    #[test]
    fn test_room_update_last_activity() {
        let mut room = Room {
            id: "id".to_string(),
            room_type: "group".to_string(),
            last_activity: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
            unread: false,
            ..Default::default()
        };
        room.update_last_activity(Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 1).unwrap());
        assert_eq!(
            room.last_activity,
            Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 1).unwrap()
        );
        room.update_last_activity(Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap());
        assert_eq!(
            room.last_activity,
            Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 1).unwrap()
        );
    }

    #[test]
    fn test_room_has_activity_since() {
        let room = Room {
            id: "id".to_string(),
            room_type: "group".to_string(),
            last_activity: Utc::now(),
            unread: false,
            ..Default::default()
        };
        assert!(room.has_activity_since(Duration::seconds(5)));
        thread::sleep(std::time::Duration::from_secs(1));
        assert!(!room.has_activity_since(Duration::seconds(1)));
    }
}
