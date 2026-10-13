//! Frontend-neutral presentation cache fed by the bridge glue.
//!
//! Each loaded room holds a canonical `RoomMessageView` (created when the room
//! is first opened); rooms, teams, persons and the account are snapshots.
use crate::frontend::{
    data::{MessageSnapshot, PersonSnapshot},
    message_view::{RoomMessageView, ViewRow},
};
use color_eyre::{eyre::eyre, Result};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use std::collections::HashMap;
pub(crate) mod persons;
pub(crate) mod room;
pub(crate) mod room_and_team_title;
pub(crate) mod room_list_filter;
pub(crate) mod rooms;
pub(crate) mod teams;
use self::room_and_team_title::RoomAndTeamTitle;
use persons::Persons;
use room::RoomId;
use rooms::Rooms;
use teams::Teams;

/// Displayed messages in order. Deleted-root placeholders are included as
/// their (body-purged) tombstone; missing-root placeholders have no snapshot
/// and are not listed here, so this is not row-indexed.
fn view_messages(view: &RoomMessageView) -> impl Iterator<Item = &MessageSnapshot> + '_ {
    view.rows().filter_map(|row| match row {
        ViewRow::Message(message) | ViewRow::DeletedRoot(message) => Some(message),
        ViewRow::MissingRoot(_) => None,
    })
}

#[derive(Default)]
pub(crate) struct Cache {
    pub(crate) rooms: Rooms,
    rooms_content: HashMap<RoomId, RoomMessageView>,
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
    pub(crate) fn messages_in_room(
        &self,
        id: &RoomId,
    ) -> Box<dyn Iterator<Item = &MessageSnapshot> + '_> {
        match self.rooms_content.get(id) {
            Some(view) => Box::new(view_messages(view)),
            None => Box::new(std::iter::empty()),
        }
    }
    /// Displayed rows, in the order the selection index addresses: messages,
    /// a deleted-root tombstone row and a missing-root placeholder row (a
    /// reply whose root is not loaded or unknown, replaced when it arrives).
    pub(crate) fn rows_in_room(&self, id: &RoomId) -> Box<dyn Iterator<Item = ViewRow<'_>> + '_> {
        match self.rooms_content.get(id) {
            Some(view) => Box::new(view.rows()),
            None => Box::new(std::iter::empty()),
        }
    }
    pub(crate) fn nb_messages_in_room(&self, id: &RoomId) -> usize {
        self.rooms_content.get(id).map_or(0, RoomMessageView::len)
    }
    /// Message at a row index. A missing-root placeholder has no message to
    /// act on (reply, edit, copy, delete), so selecting it is an error.
    pub(crate) fn nth_message_in_room(&self, index: usize, room: &str) -> Result<&MessageSnapshot> {
        let view = self
            .rooms_content
            .get(room)
            .ok_or(eyre!("Room {} not found", room))?;
        match view.row(index) {
            Some(ViewRow::Message(message) | ViewRow::DeletedRoot(message)) => Ok(message),
            Some(ViewRow::MissingRoot(_)) => Err(eyre!(
                "Row {} is a placeholder for a thread start that is not loaded",
                index
            )),
            None => Err(eyre!("Message {} not found in room", index)),
        }
    }

    /// The room's view, created on first use (opening the room).
    pub(crate) fn canonical_view_mut(&mut self, id: &RoomId) -> &mut RoomMessageView {
        self.rooms_content.entry(id.clone()).or_insert_with(|| {
            RoomMessageView::new(crate::frontend::data::RoomId::new(id.as_str()))
        })
    }

    /// The room's view, if it was ever opened.
    pub(crate) fn canonical_view(&self, id: &RoomId) -> Option<&RoomMessageView> {
        self.rooms_content.get(id)
    }

    /// The room's view only if it already exists (never creates one).
    pub(crate) fn loaded_view_mut(&mut self, id: &RoomId) -> Option<&mut RoomMessageView> {
        self.rooms_content.get_mut(id)
    }

    /// Whether `root` is known deleted in this room's view (a stored
    /// tombstone or an ID-only delete fence). Unknown roots are not.
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

    /// Rooms whose view has been created (loaded or requested).
    pub(crate) fn canonical_rooms(&self) -> Vec<RoomId> {
        let mut rooms: Vec<RoomId> = self.rooms_content.keys().cloned().collect();
        rooms.sort();
        rooms
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

    fn canonical(id: &str, parent: Option<&str>, created: &str, revision: u64) -> MessageSnapshot {
        MessageSnapshot {
            id: Id::new(id),
            room_id: RoomId::new("room"),
            revision: Some(revision),
            parent_id: parent.map(Id::new),
            sender_id: None,
            sender_email: None,
            created_at: Some(created.into()),
            updated_at: None,
            status: MessageStatus::Active,
            body: Some(OwnedMessageBody::Forms {
                text: Some(id.into()),
                markdown: None,
                html: None,
            }),
            attachment_count: None,
        }
    }

    #[test]
    fn count_unknown_room_is_zero() {
        assert_eq!(Cache::default().nb_messages_in_room(&"missing".into()), 0);
        assert!(Cache::default().canonical_view(&"missing".into()).is_none());
    }

    #[test]
    fn rooms_serve_the_read_api_in_display_order() {
        let mut cache = Cache::default();
        let room: String = "room".into();
        let view = cache.canonical_view_mut(&room);
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
        assert_eq!(cache.canonical_rooms(), ["room".to_owned()]);
        assert!(cache.loaded_view_mut(&"other".into()).is_none());
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
        let view = cache.canonical_view_mut(&room);
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
        // ... but it is not a message: nothing to act on.
        assert!(cache.nth_message_in_room(1, "room").is_err());
        assert_eq!(
            cache.nth_message_in_room(2, "room").unwrap().id.as_str(),
            "reply"
        );
        assert_eq!(
            cache
                .messages_in_room(&room)
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["other", "reply"]
        );
        // The root arrives: the placeholder is replaced in place.
        let view = cache.canonical_view_mut(&room);
        view.upsert(&canonical("root", None, "2026-01-01T00:01:00Z", 1))
            .unwrap();
        assert_eq!(row_labels(&cache, &room), ["other", "root", "reply"]);
        assert_eq!(
            cache.nth_message_in_room(1, "room").unwrap().id.as_str(),
            "root"
        );
        // A deleted root is a tombstone row, distinct from the placeholder.
        let view = cache.canonical_view_mut(&room);
        view.tombstone(&RoomId::new("room"), &Id::new("root"), None)
            .unwrap();
        assert_eq!(
            row_labels(&cache, &room),
            ["other", "deleted:root", "reply"]
        );
        assert!(cache.nth_message_in_room(1, "room").is_ok());
    }
}
