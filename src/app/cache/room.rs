use crate::frontend::data::{RoomKind, RoomSnapshot};
use chrono::{DateTime, Duration, Utc};
use color_eyre::{eyre::eyre, Result};
pub(crate) type RoomId = String;
/// Neutral room presentation plus local ordering/read state.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct Room {
    pub(crate) id: String,
    pub(crate) title: Option<String>,
    pub(crate) room_type: String,
    pub(crate) is_locked: bool,
    pub(crate) lock_state: Option<bool>,
    pub(crate) team_id: Option<String>,
    pub(crate) last_activity: DateTime<Utc>,
    pub(crate) unread: bool,
}
impl Room {
    pub(crate) fn from_snapshot(value: &RoomSnapshot) -> Result<Self> {
        let last_activity = value
            .last_activity
            .as_deref()
            .ok_or(eyre!("Room has no activity timestamp"))?;
        Ok(Self {
            id: value.id.as_str().into(),
            title: value.title.clone(),
            room_type: match value.kind {
                RoomKind::Direct => "direct",
                RoomKind::Group => "group",
                RoomKind::Unknown => "unknown",
            }
            .into(),
            is_locked: value.is_locked.unwrap_or(true),
            lock_state: value.is_locked,
            team_id: value.team_id.as_ref().map(|id| id.as_str().into()),
            last_activity: DateTime::parse_from_rfc3339(last_activity)
                .map_err(|_| eyre!("Room has invalid activity timestamp"))?
                .with_timezone(&Utc),
            unread: false,
        })
    }
    pub(crate) fn is_direct(&self) -> bool {
        self.room_type == "direct"
    }
    pub(crate) fn is_space(&self) -> bool {
        self.room_type == "group"
    }
    pub(crate) fn is_moderated(&self) -> bool {
        self.lock_state.unwrap_or(self.is_locked)
    }
    pub(crate) fn has_activity_since(&self, duration: Duration) -> bool {
        self.last_activity > Utc::now() - duration
    }
    pub(crate) fn update_last_activity(&mut self, time: DateTime<Utc>) {
        if time > self.last_activity {
            self.last_activity = time
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::data::{RoomId as Id, TeamId};
    #[test]
    fn unknown_lock_fails_closed() {
        let room = Room::from_snapshot(&RoomSnapshot {
            id: Id::new("room"),
            title: None,
            kind: RoomKind::Group,
            team_id: Some(TeamId::new("team")),
            last_activity: Some("2026-01-01T00:00:00Z".into()),
            is_member: true,
            is_locked: None,
        })
        .unwrap();
        assert!(room.is_moderated());
    }
}
