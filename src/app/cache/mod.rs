//! Frontend-neutral message cache; legacy Webex values are projected in `bridge`.
//!
//! Each loaded room holds either legacy thread-grouped content (the current
//! direct-Webex runtime) or a canonical bridge `RoomMessageView`. A room never
//! mixes the two, and only the dormant bridge glue creates canonical rooms, so
//! the legacy runtime is unchanged. The read API below serves both.
use crate::frontend::{
    data::{MessageSnapshot, PersonSnapshot},
    message_view::{RoomMessageView, ViewRow},
};
use chrono::{DateTime, Utc};
use color_eyre::{eyre::eyre, Result};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use std::collections::HashMap;
pub(crate) mod msg_thread;
pub(crate) mod persons;
pub(crate) mod room;
pub(crate) mod room_and_team_title;
pub(crate) mod room_content;
pub(crate) mod room_list_filter;
pub(crate) mod rooms;
pub(crate) mod teams;
use self::{room_and_team_title::RoomAndTeamTitle, room_content::RoomContent};
use persons::Persons;
use room::RoomId;
use rooms::Rooms;
use teams::Teams;
pub(crate) type MessageId = String;

/// Per-room message storage; see the module documentation.
enum RoomMessages {
    Legacy(RoomContent),
    // Dormant until the bridge glue is started (Step 5 cutover).
    #[allow(dead_code)]
    Canonical(RoomMessageView),
}

impl RoomMessages {
    /// Displayed rows in order; the selection index addresses these. Legacy
    /// rooms are messages only (legacy parent repair fetches missing roots).
    /// Canonical rooms add a deleted-root tombstone row and a missing-root
    /// placeholder row (a reply whose root is not loaded or is unknown); the
    /// placeholder is replaced when the root arrives, since the view rebuilds.
    fn rows(&self) -> Box<dyn Iterator<Item = ViewRow<'_>> + '_> {
        match self {
            Self::Legacy(content) => Box::new(content.messages().map(ViewRow::Message)),
            Self::Canonical(view) => Box::new(view.rows()),
        }
    }
    /// Displayed messages in order. Canonical deleted-root placeholders are
    /// included as their (body-purged) tombstone; missing-root placeholders
    /// have no snapshot and are not listed here, so this is not row-indexed.
    fn messages(&self) -> Box<dyn Iterator<Item = &MessageSnapshot> + '_> {
        match self {
            Self::Legacy(content) => Box::new(content.messages()),
            Self::Canonical(view) => Box::new(view.rows().filter_map(|row| match row {
                ViewRow::Message(message) | ViewRow::DeletedRoot(message) => Some(message),
                ViewRow::MissingRoot(_) => None,
            })),
        }
    }
    fn len(&self) -> usize {
        match self {
            Self::Legacy(content) => content.len(),
            Self::Canonical(view) => view.len(),
        }
    }
    fn is_empty(&self) -> bool {
        match self {
            Self::Legacy(content) => content.is_empty(),
            Self::Canonical(view) => view.is_empty(),
        }
    }
    /// Message at a row index. A missing-root placeholder has no message to
    /// act on (reply, edit, copy, delete), so selecting it is an error.
    fn nth(&self, index: usize) -> Result<&MessageSnapshot> {
        match self {
            Self::Legacy(content) => content.nth_message(index),
            Self::Canonical(view) => match view.row(index) {
                Some(ViewRow::Message(message) | ViewRow::DeletedRoot(message)) => Ok(message),
                Some(ViewRow::MissingRoot(_)) => Err(eyre!(
                    "Row {} is a placeholder for a thread start that is not loaded",
                    index
                )),
                None => Err(eyre!("Message {} not found in room", index)),
            },
        }
    }
    /// Row index of a loaded message (active or deleted root). A missing-root
    /// placeholder is not a loaded message, so its ID is never found here.
    fn index_of(&self, id: &MessageId) -> Option<usize> {
        match self {
            Self::Legacy(content) => content.index_of_message(id),
            Self::Canonical(_) => self.rows().position(|row| match row {
                ViewRow::Message(message) | ViewRow::DeletedRoot(message) => {
                    message.id.as_str() == id
                }
                ViewRow::MissingRoot(_) => false,
            }),
        }
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    pub(crate) rooms: Rooms,
    rooms_content: HashMap<RoomId, RoomMessages>,
    pub(crate) teams: Teams,
    pub(crate) me: Option<PersonSnapshot>,
    pub(crate) persons: Persons,
}
impl Cache {
    pub(crate) fn set_me(&mut self, me: PersonSnapshot) {
        self.me = Some(me)
    }
    pub(crate) fn is_me(&self, id: &Option<String>) -> bool {
        matches!((&self.me,id),(Some(me),Some(id)) if me.id.as_str()==id)
    }
    pub(crate) fn add_message(&mut self, msg: &MessageSnapshot) -> Result<()> {
        let room_id = msg.room_id.as_str().to_owned();
        match self
            .rooms_content
            .entry(room_id.clone())
            .or_insert_with(|| RoomMessages::Legacy(RoomContent::default()))
        {
            RoomMessages::Legacy(content) => content.add(msg)?,
            RoomMessages::Canonical(_) => {
                return Err(eyre!("legacy message for a canonical bridge room"))
            }
        }
        if let Some(room) = self.rooms.room_with_id_mut(&room_id) {
            let timestamp = msg
                .updated_at
                .as_deref()
                .or(msg.created_at.as_deref())
                .ok_or(eyre!("message has no timestamp"))?;
            room.update_last_activity(DateTime::parse_from_rfc3339(timestamp)?.with_timezone(&Utc));
            self.rooms.reposition_room(&room_id);
        }
        Ok(())
    }
    pub(crate) fn messages_in_room(
        &self,
        id: &RoomId,
    ) -> Box<dyn Iterator<Item = &MessageSnapshot> + '_> {
        self.rooms_content
            .get(id)
            .map(RoomMessages::messages)
            .unwrap_or_else(|| Box::new(std::iter::empty()))
    }
    /// Displayed rows, in the order the selection index addresses.
    pub(crate) fn rows_in_room(&self, id: &RoomId) -> Box<dyn Iterator<Item = ViewRow<'_>> + '_> {
        self.rooms_content
            .get(id)
            .map(RoomMessages::rows)
            .unwrap_or_else(|| Box::new(std::iter::empty()))
    }
    pub(crate) fn room_is_empty(&self, id: &RoomId) -> bool {
        self.rooms_content
            .get(id)
            .is_none_or(RoomMessages::is_empty)
    }
    pub(crate) fn nb_messages_in_room(&self, id: &RoomId) -> usize {
        self.rooms_content.get(id).map_or(0, RoomMessages::len)
    }
    #[allow(dead_code)]
    pub(crate) fn delete_message(&mut self, id: &MessageId, room: &RoomId) -> Result<()> {
        match self.rooms_content.get_mut(room) {
            Some(RoomMessages::Legacy(content)) => content.delete_message(id)?,
            // Canonical rooms only change through daemon tombstones.
            Some(RoomMessages::Canonical(_)) => {
                return Err(eyre!("canonical bridge rooms apply tombstones only"))
            }
            None => {}
        }
        Ok(())
    }
    pub(crate) fn nth_message_in_room(&self, index: usize, room: &str) -> Result<&MessageSnapshot> {
        self.rooms_content
            .get(room)
            .ok_or(eyre!("Room {} not found", index))?
            .nth(index)
    }
    pub(crate) fn index_of_message_in_room(&self, id: &MessageId, room: &RoomId) -> Option<usize> {
        self.rooms_content.get(room)?.index_of(id)
    }

    /// Canonical bridge view for a room, created on first use. Refuses a room
    /// already holding legacy content (never mixes backends in one room).
    #[allow(dead_code)] // dormant bridge glue
    pub(crate) fn canonical_view_mut(&mut self, id: &RoomId) -> Option<&mut RoomMessageView> {
        let entry = self.rooms_content.entry(id.clone()).or_insert_with(|| {
            RoomMessages::Canonical(RoomMessageView::new(crate::frontend::data::RoomId::new(
                id.as_str(),
            )))
        });
        match entry {
            RoomMessages::Canonical(view) => Some(view),
            RoomMessages::Legacy(_) => None,
        }
    }

    #[allow(dead_code)] // dormant bridge glue
    pub(crate) fn canonical_view(&self, id: &RoomId) -> Option<&RoomMessageView> {
        match self.rooms_content.get(id)? {
            RoomMessages::Canonical(view) => Some(view),
            RoomMessages::Legacy(_) => None,
        }
    }

    /// Whether `root` is known deleted in this room's canonical view (a
    /// stored tombstone or an ID-only delete fence). Unknown roots and legacy
    /// rooms are not known deleted.
    pub(crate) fn thread_root_deleted(
        &self,
        room: &RoomId,
        root: &crate::frontend::data::MessageId,
    ) -> bool {
        self.canonical_view(room).is_some_and(|view| {
            view.is_tombstoned(root)
                || view.get(root).is_some_and(|message| {
                    message.status != crate::frontend::data::MessageStatus::Active
                })
        })
    }

    /// Rooms whose canonical view has been created (loaded or requested).
    #[allow(dead_code)] // dormant bridge glue
    pub(crate) fn canonical_rooms(&self) -> Vec<RoomId> {
        let mut rooms: Vec<RoomId> = self
            .rooms_content
            .iter()
            .filter(|(_, content)| matches!(content, RoomMessages::Canonical(_)))
            .map(|(id, _)| id.clone())
            .collect();
        rooms.sort();
        rooms
    }
    pub(crate) fn message_exists_in_room(&self, id: &MessageId, room: &RoomId) -> bool {
        self.index_of_message_in_room(id, room).is_some()
    }
    pub(crate) fn room_and_team_title(&self, id: &RoomId) -> Result<RoomAndTeamTitle> {
        let room = self.rooms.room_with_id(id).ok_or(eyre!("Room not found"))?;
        let title = room.title.clone().unwrap_or_else(|| "No room title".into());
        let team = room.team_id.clone().and_then(|id| {
            self.teams
                .team_with_id(&id)
                .and_then(|team| team.name.clone())
        });
        Ok(match team {
            None => RoomAndTeamTitle {
                room_title: title,
                team_name: None,
            },
            Some(team) if team == title => RoomAndTeamTitle {
                room_title: "General".into(),
                team_name: Some(team),
            },
            Some(team) => RoomAndTeamTitle {
                room_title: title,
                team_name: Some(team),
            },
        })
    }
    pub(crate) fn wipe_messages_in_room(&mut self, id: &str) -> Result<usize> {
        match self.rooms_content.get_mut(id) {
            Some(RoomMessages::Legacy(content)) => Ok(content.wipe()),
            Some(RoomMessages::Canonical(view)) => Ok(view.clear_messages()),
            None => Err(eyre!("Could not wipe messages, room {} not found", id)),
        }
    }
    pub(crate) fn remove_room(&mut self, id: &RoomId) {
        self.rooms_content.remove(id);
        self.rooms.remove_room(id)
    }
    pub(crate) fn rooms_matching_search<'a>(
        &'a self,
        query: &'a str,
        filter: &'a room_list_filter::RoomsListFilter,
    ) -> impl Iterator<Item = (&'a room::Room, i64)> {
        let matcher = SkimMatcherV2::default();
        let mut rooms: Vec<_> = self
            .rooms
            .rooms_filtered_by(filter)
            .filter_map(|room| {
                let title = self.room_and_team_title(&room.id).ok()?;
                matcher
                    .fuzzy_match(
                        &format!(
                            "{} {}",
                            title.room_title,
                            title.team_name.unwrap_or_default()
                        ),
                        query,
                    )
                    .map(|score| (room, score))
            })
            .collect();
        rooms.sort_by(|a, b| b.1.cmp(&a.1));
        rooms.into_iter()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::data::{MessageId as Id, MessageStatus, OwnedMessageBody, RoomId};
    fn message(id: &str, parent: Option<&str>, created: &str) -> MessageSnapshot {
        MessageSnapshot {
            id: Id::new(id),
            room_id: RoomId::new("room"),
            revision: None,
            parent_id: parent.map(Id::new),
            sender_id: None,
            sender_email: None,
            created_at: Some(created.into()),
            updated_at: None,
            status: MessageStatus::Active,
            body: Some(OwnedMessageBody::LegacyUnknown(id.into())),
            attachment_count: None,
        }
    }
    #[test]
    fn cache_orders_neutral_messages() {
        let mut cache = Cache::default();
        cache
            .add_message(&message("second", None, "2026-01-01T00:02:00Z"))
            .unwrap();
        cache
            .add_message(&message("root", None, "2026-01-01T00:01:00Z"))
            .unwrap();
        cache
            .add_message(&message("reply", Some("root"), "2026-01-01T00:03:00Z"))
            .unwrap();
        assert_eq!(
            cache
                .messages_in_room(&"room".into())
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["root", "reply", "second"]
        );
    }
    #[test]
    fn unknown_room_is_created() {
        let mut cache = Cache::default();
        cache
            .add_message(&message("a", None, "2026-01-01T00:00:00Z"))
            .unwrap();
        assert_eq!(cache.nb_messages_in_room(&"room".into()), 1);
    }
    #[test]
    fn duplicate_id_revises_without_duplication() {
        let mut cache = Cache::default();
        cache
            .add_message(&message("a", None, "2026-01-01T00:00:00Z"))
            .unwrap();
        let mut revised = message("a", None, "2026-01-01T00:01:00Z");
        revised.revision = Some(2);
        cache.add_message(&revised).unwrap();
        assert_eq!(cache.nb_messages_in_room(&"room".into()), 1);
        assert_eq!(
            cache.nth_message_in_room(0, "room").unwrap().revision,
            Some(2)
        );
    }
    #[test]
    fn delete_removes_existing_message() {
        let mut cache = Cache::default();
        cache
            .add_message(&message("a", None, "2026-01-01T00:00:00Z"))
            .unwrap();
        cache.delete_message(&"a".into(), &"room".into()).unwrap();
        assert!(cache.room_is_empty(&"room".into()));
    }
    #[test]
    fn existence_tracks_message_id() {
        let mut cache = Cache::default();
        cache
            .add_message(&message("a", None, "2026-01-01T00:00:00Z"))
            .unwrap();
        assert!(cache.message_exists_in_room(&"a".into(), &"room".into()));
        assert!(!cache.message_exists_in_room(&"b".into(), &"room".into()));
    }
    #[test]
    fn nth_and_index_follow_display_order() {
        let mut cache = Cache::default();
        cache
            .add_message(&message("b", None, "2026-01-01T00:02:00Z"))
            .unwrap();
        cache
            .add_message(&message("a", None, "2026-01-01T00:01:00Z"))
            .unwrap();
        assert_eq!(
            cache.nth_message_in_room(0, "room").unwrap().id.as_str(),
            "a"
        );
        assert_eq!(
            cache.index_of_message_in_room(&"b".into(), &"room".into()),
            Some(1)
        );
    }
    #[test]
    fn empty_unknown_room_is_empty() {
        assert!(Cache::default().room_is_empty(&"missing".into()));
    }
    #[test]
    fn count_unknown_room_is_zero() {
        assert_eq!(Cache::default().nb_messages_in_room(&"missing".into()), 0);
    }
    #[test]
    fn missing_created_is_rejected() {
        let mut cache = Cache::default();
        let mut message = message("a", None, "2026-01-01T00:00:00Z");
        message.created_at = None;
        assert!(cache.add_message(&message).is_err());
    }
    #[test]
    fn wipe_missing_room_is_an_error() {
        assert!(Cache::default().wipe_messages_in_room("missing").is_err());
    }

    fn canonical(id: &str, parent: Option<&str>, created: &str, revision: u64) -> MessageSnapshot {
        let mut message = message(id, parent, created);
        message.revision = Some(revision);
        message
    }

    #[test]
    fn canonical_rooms_serve_the_same_read_api_and_never_mix_with_legacy() {
        let mut cache = Cache::default();
        let room: String = "room".into();
        let view = cache.canonical_view_mut(&room).unwrap();
        view.upsert(&canonical("root", None, "2026-01-01T00:01:00Z", 1))
            .unwrap();
        view.upsert(&canonical("reply", Some("root"), "2026-01-01T00:02:00Z", 1))
            .unwrap();
        view.upsert(&canonical("later", None, "2026-01-01T00:03:00Z", 1))
            .unwrap();
        view.tombstone(&RoomId::new("room"), &Id::new("root"), None)
            .unwrap();
        // Deleted root with a reply is listed as its body-purged tombstone.
        let ids: Vec<_> = cache
            .messages_in_room(&room)
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(ids, ["root", "reply", "later"]);
        assert!(cache
            .nth_message_in_room(0, "room")
            .unwrap()
            .presentation()
            .body
            .text
            .is_none());
        assert_eq!(cache.nb_messages_in_room(&room), 3);
        assert_eq!(
            cache.index_of_message_in_room(&"later".into(), &room),
            Some(2)
        );
        assert!(cache.message_exists_in_room(&"reply".into(), &room));
        assert_eq!(cache.canonical_rooms(), ["room".to_owned()]);
        // Never mixes: legacy adds and local deletes are refused for it.
        assert!(cache
            .add_message(&message("legacy", None, "2026-01-01T00:04:00Z"))
            .is_err());
        assert!(cache.delete_message(&"later".into(), &room).is_err());
        assert_eq!(cache.nb_messages_in_room(&room), 3);
        assert_eq!(cache.wipe_messages_in_room("room").unwrap(), 3);
        assert!(cache.room_is_empty(&room));
        assert!(cache.canonical_view(&room).is_some());
        // A legacy room cannot become canonical.
        let room: String = "room".into();
        let mut legacy = Cache::default();
        legacy
            .add_message(&message("a", None, "2026-01-01T00:00:00Z"))
            .unwrap();
        assert!(legacy.canonical_view_mut(&room).is_none());
        assert!(legacy.canonical_rooms().is_empty());
    }

    fn row_labels(cache: &Cache, room: &room::RoomId) -> Vec<String> {
        cache
            .rows_in_room(room)
            .map(|row| match row {
                ViewRow::Message(m) => m.id.as_str().to_owned(),
                ViewRow::DeletedRoot(m) => format!("deleted:{}", m.id.as_str()),
                ViewRow::MissingRoot(id) => format!("missing:{}", id.as_str()),
            })
            .collect()
    }

    #[test]
    fn missing_root_placeholder_is_a_row_but_not_a_message_and_is_replaced() {
        let mut cache = Cache::default();
        let room: String = "room".into();
        let view = cache.canonical_view_mut(&room).unwrap();
        view.upsert(&canonical("other", None, "2026-01-01T00:00:00Z", 1))
            .unwrap();
        view.upsert(&canonical("reply", Some("root"), "2026-01-01T00:05:00Z", 1))
            .unwrap();
        assert_eq!(
            row_labels(&cache, &room),
            ["other", "missing:root", "reply"]
        );
        // Selection indexes rows, so counts include the placeholder ...
        assert_eq!(cache.nb_messages_in_room(&room), 3);
        assert!(!cache.room_is_empty(&room));
        // ... but it is not a message: nothing to act on, never "exists".
        assert!(cache.nth_message_in_room(1, "room").is_err());
        assert_eq!(
            cache.nth_message_in_room(2, "room").unwrap().id.as_str(),
            "reply"
        );
        assert!(!cache.message_exists_in_room(&"root".into(), &room));
        assert_eq!(
            cache.index_of_message_in_room(&"reply".into(), &room),
            Some(2)
        );
        assert_eq!(
            cache
                .messages_in_room(&room)
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["other", "reply"]
        );
        // The root arrives: the placeholder is replaced in place.
        let view = cache.canonical_view_mut(&room).unwrap();
        view.upsert(&canonical("root", None, "2026-01-01T00:01:00Z", 1))
            .unwrap();
        assert_eq!(row_labels(&cache, &room), ["other", "root", "reply"]);
        assert_eq!(
            cache.nth_message_in_room(1, "room").unwrap().id.as_str(),
            "root"
        );
        assert!(cache.message_exists_in_room(&"root".into(), &room));
        // A deleted root is a tombstone row, distinct from the placeholder.
        let view = cache.canonical_view_mut(&room).unwrap();
        view.tombstone(&RoomId::new("room"), &Id::new("root"), None)
            .unwrap();
        assert_eq!(
            row_labels(&cache, &room),
            ["other", "deleted:root", "reply"]
        );
        assert!(cache.nth_message_in_room(1, "room").is_ok());
    }

    #[test]
    fn legacy_rows_are_messages_only() {
        let mut cache = Cache::default();
        cache
            .add_message(&message("reply", Some("root"), "2026-01-01T00:01:00Z"))
            .unwrap();
        let room: String = "room".into();
        assert!(cache
            .rows_in_room(&room)
            .all(|row| matches!(row, ViewRow::Message(_))));
        assert_eq!(
            cache.rows_in_room(&room).count(),
            cache.nb_messages_in_room(&room)
        );
    }
}
