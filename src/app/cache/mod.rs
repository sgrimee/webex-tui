//! Frontend-neutral message cache; legacy Webex values are projected in `bridge`.
use crate::frontend::data::{MessageSnapshot, PersonSnapshot};
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
#[derive(Default)]
pub(crate) struct Cache {
    pub(crate) rooms: Rooms,
    rooms_content: HashMap<RoomId, RoomContent>,
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
        self.rooms_content
            .entry(room_id.clone())
            .or_default()
            .add(msg)?;
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
            .map(|c| Box::new(c.messages()) as Box<dyn Iterator<Item = &MessageSnapshot>>)
            .unwrap_or_else(|| Box::new(std::iter::empty()))
    }
    pub(crate) fn room_is_empty(&self, id: &RoomId) -> bool {
        self.rooms_content.get(id).is_none_or(RoomContent::is_empty)
    }
    pub(crate) fn nb_messages_in_room(&self, id: &RoomId) -> usize {
        self.rooms_content.get(id).map_or(0, RoomContent::len)
    }
    #[allow(dead_code)]
    pub(crate) fn delete_message(&mut self, id: &MessageId, room: &RoomId) -> Result<()> {
        if let Some(content) = self.rooms_content.get_mut(room) {
            content.delete_message(id)?;
        }
        Ok(())
    }
    pub(crate) fn nth_message_in_room(&self, index: usize, room: &str) -> Result<&MessageSnapshot> {
        self.rooms_content
            .get(room)
            .ok_or(eyre!("Room {} not found", index))?
            .nth_message(index)
    }
    pub(crate) fn index_of_message_in_room(&self, id: &MessageId, room: &RoomId) -> Option<usize> {
        self.rooms_content
            .get(room)
            .and_then(|c| c.index_of_message(id))
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
        self.rooms_content
            .get_mut(id)
            .map(RoomContent::wipe)
            .ok_or(eyre!("Could not wipe messages, room {} not found", id))
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
}
