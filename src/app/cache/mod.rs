// app/cache/mod.rs

//! A caching store for Webex messages and context.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use color_eyre::{eyre::eyre, Result};
use webex::{Message, Person};

pub(crate) mod msg_thread;
pub(crate) mod persons;
pub(crate) mod room;
pub(crate) mod room_and_team_title;
pub(crate) mod room_content;
pub(crate) mod room_list_filter;
pub(crate) mod rooms;
pub(crate) mod teams;

use self::room_and_team_title::RoomAndTeamTitle;
use self::room_content::RoomContent;
use persons::Persons;
use room::RoomId;
use rooms::Rooms;
use teams::Teams;

pub(crate) type MessageId = String;

/// `TeamsStore` maintains a local cache of room information,
/// messages in some of those rooms, and other state information
/// directly related to them.
///
/// Currently there is no garbage collection and the cache only grows.
/// This is usually acceptable for a daily session.
#[derive(Default, Debug)]
pub(crate) struct Cache {
    pub(crate) rooms: Rooms,
    rooms_content: HashMap<RoomId, RoomContent>,
    pub(crate) teams: Teams,
    pub(crate) me: Option<webex::Person>,
    pub(crate) persons: Persons,
}

impl Cache {
    /// Sets the user of the app, used to filter its own messages.
    pub(crate) fn set_me(&mut self, me: &Person) {
        self.me = Some(me.clone());
    }

    /// Returns true if me is not None, person_id is not None and person_id equals me.
    /// Returns false if they are different or either is None.
    pub(crate) fn is_me(&self, person_id: &Option<String>) -> bool {
        match (&self.me, person_id) {
            (Some(me), Some(id)) => me.id.eq(id),
            _ => false,
        }
    }

    /// Adds a message to the store, respecting the thread order.
    pub(crate) fn add_message(&mut self, msg: &Message) -> Result<()> {
        let room_id = msg.room_id.clone().ok_or(eyre!("message has no room id"))?;
        let content = self.rooms_content.entry(room_id.clone()).or_default();
        content.add(msg)?;
        // Update the room last activity if the room is already present.
        // If not, it will come later with the correct last activity.
        if let Some(room) = self.rooms.room_with_id_mut(&room_id) {
            // get message timestamp from last update, if not use its creatiom time, if not error
            let time_str = msg
                .updated
                .clone()
                .or_else(|| msg.created.clone())
                .ok_or(eyre!("message has no timestamp"))?;
            // convert it to DateTime<Utc> and use it to update the room last activity
            let timestamp = DateTime::parse_from_rfc3339(&time_str)?.with_timezone(&Utc);
            room.update_last_activity(timestamp);
            // Re-position the room in the list with the new timestamp
            self.rooms.reposition_room(&room_id);
        }
        Ok(())
    }

    /// Returns an iterator with all pre-loaded messages in the room, in display order.
    pub(crate) fn messages_in_room<'a>(
        &'a self,
        id: &RoomId,
    ) -> Box<dyn Iterator<Item = &'a Message> + 'a> {
        match self.rooms_content.get(id) {
            Some(content) => Box::new(content.messages()),
            None => Box::new(::std::iter::empty()),
        }
    }

    /// Returns whether there are any messages in the room.
    pub(crate) fn room_is_empty(&self, id: &RoomId) -> bool {
        match self.rooms_content.get(id) {
            Some(content) => content.is_empty(),
            None => true,
        }
    }

    /// Returns the number of messages in the room.
    /// More efficient than `messages_in_room` if only the count is needed.
    pub(crate) fn nb_messages_in_room(&self, id: &RoomId) -> usize {
        match self.rooms_content.get(id) {
            Some(content) => content.len(),
            None => 0,
        }
    }

    /// Deletes message with `msg_id` in `room_id` if it exists.
    pub(crate) fn delete_message(&mut self, msg_id: &MessageId, room_id: &RoomId) -> Result<()> {
        if let Some(content) = self.rooms_content.get_mut(room_id) {
            content.delete_message(msg_id)?;
        }
        Ok(())
    }

    pub(crate) fn nth_message_in_room(&self, index: usize, room_id: &str) -> Result<&Message> {
        self.rooms_content
            .get(room_id)
            .ok_or(eyre!("Room {} not found", index))?
            .nth_message(index)
    }

    /// Returns the index of the message with given id in the room.
    /// If the message is not found, returns None.
    pub(crate) fn index_of_message_in_room(
        &self,
        msg_id: &MessageId,
        room_id: &RoomId,
    ) -> Option<usize> {
        self.rooms_content
            .get(room_id)
            .and_then(|content| content.index_of_message(msg_id))
    }

    /// Returns true if the message with given id is in the room, false otherwise.
    pub(crate) fn message_exists_in_room(&self, msg_id: &MessageId, room_id: &RoomId) -> bool {
        self.index_of_message_in_room(msg_id, room_id).is_some()
    }

    /// Returns a tuple with the title of the room and the team name if any.
    ///
    /// If the room is not found, returns an error.
    /// If the room has no title, returns "No room title" and no team name.
    /// If the room title is the same as the team name, returns "General" and the team name.
    /// If the room title is different from the team name, returns the room title and the team name.
    /// If the room has no team, returns the room title and no team name.
    pub(crate) fn room_and_team_title(&self, room_id: &RoomId) -> Result<RoomAndTeamTitle> {
        let room = self
            .rooms
            .room_with_id(room_id)
            .ok_or(eyre!("Room not found"))?;
        let room_title = room.title.clone().unwrap_or(String::from("No room title"));
        let team_name = room.team_id.clone().and_then(|team_id| {
            self.teams
                .team_with_id(&team_id)
                .and_then(|team| team.name.clone())
        });
        let (room_title, team_name) = match team_name {
            None => (room_title.to_string(), None),
            Some(team_name) if team_name == room_title => ("General".to_string(), Some(team_name)),
            Some(team_name) => (room_title.to_string(), Some(team_name)),
        };
        Ok(RoomAndTeamTitle {
            room_title,
            team_name,
        })
    }

    /// Removes all messages in the room.
    /// Returns the number of messages removed.
    pub(crate) fn wipe_messages_in_room(&mut self, room_id: &str) -> Result<usize> {
        match self.rooms_content.get_mut(room_id) {
            Some(content) => Ok(content.wipe()),
            None => Err(eyre!("Could not wipe messages, room {} not found", room_id)),
        }
    }

    /// Removes a room completely.
    pub(crate) fn remove_room(&mut self, room_id: &RoomId) {
        self.rooms_content.remove(room_id);
        self.rooms.remove_room(room_id);
    }
}

#[cfg(test)]
mod tests {
    use crate::app::cache::room::Room;

    use super::*;

    fn make_message(id: &str, room_id: &str, parent_id: Option<&str>) -> Message {
        Message {
            id: Some(id.to_string()),
            parent_id: parent_id.map(|x| x.to_string()),
            room_id: Some(room_id.to_string()),
            created: Some(chrono::Utc::now().to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn should_add_message_with_unknown_room() {
        let mut store = Cache::default();
        let room_id = "some_new_room_id";
        let message1 = make_message("message1", room_id, None);
        store.add_message(&message1).unwrap();
        assert_eq!(store.rooms_content[room_id].len(), 1);
    }

    #[test]
    fn should_add_message_with_known_room() {
        let mut store = Cache::default();
        let room_id = "some_new_room_id";
        let message1 = make_message("message1", room_id, None);
        // add the message once to the empty store
        store.add_message(&message1).unwrap();
        // add another message to the same room
        let message2 = make_message("message2", room_id, None);
        store.add_message(&message2).unwrap();
        assert_eq!(store.rooms_content[room_id].len(), 2);
    }

    #[test]
    fn should_sort_messages_by_thread() {
        let mut store = Cache::default();
        let room_id: RoomId = "some_new_room_id".into();
        store
            .add_message(&make_message("message1", &room_id, None))
            .unwrap();
        store
            .add_message(&make_message("message2", &room_id, None))
            .unwrap();
        store
            .add_message(&make_message("child_of_1", &room_id, Some("message1")))
            .unwrap();
        let expected = [
            "message1".to_string(),
            "child_of_1".to_string(),
            "message2".to_string(),
        ];
        for (i, msg) in store.messages_in_room(&room_id).enumerate() {
            assert_eq!(&expected[i], msg.id.as_ref().unwrap());
        }
    }

    #[test]
    fn test_room_and_team_title() {
        const TEAM_ID: &str = "some_new_team_id";
        const TEAM_NAME: &str = "Team name";
        let mut store = Cache::default();
        let team = webex::Team {
            id: TEAM_ID.into(),
            name: Some(TEAM_NAME.into()),
            created: "2020-01-01T00:00:00.000Z".to_string(),
            description: None,
        };
        store.teams.add(team);

        // Non-General room in a team
        store.rooms.update_with_room(&Room {
            id: String::from("room1"),
            title: Some(String::from("room1 title")),
            team_id: Some(TEAM_ID.into()),
            ..Default::default()
        });
        assert_eq!(
            store.room_and_team_title(&String::from("room1")).unwrap(),
            RoomAndTeamTitle {
                room_title: String::from("room1 title"),
                team_name: Some(String::from("Team name")),
            }
        );

        // General room in a team
        store.rooms.update_with_room(&Room {
            id: String::from("room2"),
            title: Some(TEAM_NAME.into()),
            team_id: Some(TEAM_ID.into()),
            ..Default::default()
        });
        assert_eq!(
            store.room_and_team_title(&String::from("room2")).unwrap(),
            RoomAndTeamTitle {
                room_title: String::from("General"),
                team_name: Some(String::from("Team name")),
            }
        );

        // Room with no team
        store.rooms.update_with_room(&Room {
            id: String::from("room3"),
            title: Some(String::from("room3 title")),
            team_id: None,
            ..Default::default()
        });
        assert_eq!(
            store.room_and_team_title(&String::from("room3")).unwrap(),
            RoomAndTeamTitle {
                room_title: String::from("room3 title"),
                ..Default::default()
            }
        );
    }

    #[test]
    fn test_delete_message() {
        let mut store = Cache::default();
        let room_id = String::from("some_new_room_id") as RoomId;
        let message1 = make_message("message1", &room_id, None);
        store.add_message(&message1).unwrap();
        let message2 = make_message("message2", &room_id, None);
        store.add_message(&message2).unwrap();
        let message3 = make_message("message3", &room_id, None);
        store.add_message(&message3).unwrap();
        assert_eq!(store.rooms_content[&room_id].len(), 3);
        store
            .delete_message(&"message2".to_string(), &room_id)
            .unwrap();
        assert_eq!(store.rooms_content[&room_id].len(), 2);
        assert_eq!(
            store.rooms_content[&room_id]
                .messages()
                .next()
                .unwrap()
                .id
                .as_ref()
                .unwrap(),
            "message1"
        );
        assert_eq!(
            store.rooms_content[&room_id]
                .messages()
                .nth(1)
                .unwrap()
                .id
                .as_ref()
                .unwrap(),
            "message3"
        );
        assert_eq!(
            store.index_of_message_in_room(&String::from("message3") as &MessageId, &room_id),
            Some(1)
        )
    }

    // is_me
    #[test]
    fn test_is_me() {
        let mut store = Cache::default();
        let me = Person {
            id: "me".to_string(),
            ..Default::default()
        };
        store.set_me(&me);
        assert!(store.is_me(&Some("me".to_string())));
        assert!(!store.is_me(&Some("not me".to_string())));
        assert!(!store.is_me(&None));
    }

    // message_exists_in_room
    #[test]
    fn test_message_exists_in_room() {
        let mut store = Cache::default();
        let room_id = String::from("some_new_room_id") as RoomId;
        let message1 = make_message("message1", &room_id, None);
        store.add_message(&message1).unwrap();
        let message2 = make_message("message2", &room_id, None);
        store.add_message(&message2).unwrap();
        let message3 = make_message("message3", &room_id, None);
        store.add_message(&message3).unwrap();
        assert!(store.message_exists_in_room(&"message2".to_string(), &room_id));
        assert!(!store.message_exists_in_room(&"message4".to_string(), &room_id));
    }

    // messages_in_room
    #[test]
    fn test_messages_in_room() {
        let mut store = Cache::default();
        let room_id = String::from("some_new_room_id") as RoomId;
        let message1 = make_message("message1", &room_id, None);
        store.add_message(&message1).unwrap();
        let message2 = make_message("message2", &room_id, None);
        store.add_message(&message2).unwrap();
        let message3 = make_message("message3", &room_id, None);
        store.add_message(&message3).unwrap();
        let mut messages = store.messages_in_room(&room_id);
        assert_eq!(messages.next().unwrap().id.as_ref().unwrap(), "message1");
        assert_eq!(messages.next().unwrap().id.as_ref().unwrap(), "message2");
        assert_eq!(messages.next().unwrap().id.as_ref().unwrap(), "message3");
    }

    // nb_messages_in_room
    #[test]
    fn test_nb_messages_in_room() {
        let mut store = Cache::default();
        let room_id = String::from("some_new_room_id") as RoomId;
        let message1 = make_message("message1", &room_id, None);
        store.add_message(&message1).unwrap();
        let message2 = make_message("message2", &room_id, None);
        store.add_message(&message2).unwrap();
        let message3 = make_message("message3", &room_id, None);
        store.add_message(&message3).unwrap();
        assert_eq!(store.nb_messages_in_room(&room_id), 3);
    }

    // nth_message_in_room
    #[test]
    fn test_nth_message_in_room() {
        let mut store = Cache::default();
        let room_id = String::from("some_new_room_id") as RoomId;
        let message1 = make_message("message1", &room_id, None);
        store.add_message(&message1).unwrap();
        let message2 = make_message("message2", &room_id, None);
        store.add_message(&message2).unwrap();
        let message3 = make_message("message3", &room_id, None);
        store.add_message(&message3).unwrap();
        assert_eq!(
            store
                .nth_message_in_room(1, &room_id)
                .unwrap()
                .id
                .as_ref()
                .unwrap(),
            "message2"
        );
    }

    // room_is_empty
    #[test]
    fn test_room_is_empty() {
        let mut store = Cache::default();
        let room_id = String::from("some_new_room_id") as RoomId;
        assert!(store.room_is_empty(&room_id));
        let message1 = make_message("message1", &room_id, None);
        store.add_message(&message1).unwrap();
        assert!(!store.room_is_empty(&room_id));
    }
}
