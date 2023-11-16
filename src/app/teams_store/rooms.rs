use super::room_list_filter::RoomsListFilter;
use super::room_list_order::RoomsListOrder;
use super::RoomId;
use chrono::{DateTime, Duration, Utc};
// use color_eyre::{eyre::eyre, Result};
use log::*;
use std::collections::{HashMap, HashSet};
use webex::Room;

#[derive(Default, Debug)]
pub struct Rooms {
    /// The rooms in the store, indexed by room id.
    rooms_by_id: HashMap<RoomId, Room>,
    /// The rooms that have unread messages.
    unread_rooms: HashSet<RoomId>,
}

impl Rooms {
    /// Returns the room for given id, if found.
    pub fn room_with_id(&self, id: &RoomId) -> Option<&Room> {
        self.rooms_by_id.get(id)
    }

    /// Adds or update a room to the store.
    pub fn update_room(&mut self, room: Room) {
        self.rooms_by_id.insert(room.id.to_owned(), room);
    }

    /// Mark a room as unread.
    pub fn mark_unread(&mut self, id: &RoomId) {
        debug!("Marking room {} unread", id);
        self.unread_rooms.insert(id.clone());
    }

    /// Mark a room as read.
    pub fn mark_read(&mut self, id: &RoomId) {
        debug!("Marking room {} read", id);
        self.unread_rooms.remove(id);
    }

    /// Returns whether the room has unread messages.
    pub fn room_has_unread(&self, id: &RoomId) -> bool {
        self.unread_rooms.contains(id)
    }

    /// Returns whether the room has seen any activity in the past specified period.
    /// Panics if room is not known.
    fn room_has_activity_since(&self, duration: Duration, id: &RoomId) -> bool {
        let room = self.rooms_by_id.get(id).unwrap();
        let last_activity = DateTime::parse_from_rfc3339(&room.last_activity).unwrap();
        let now = Utc::now();
        last_activity > (now - duration)
    }

    /// Returns whether a room is a 1-1 chat
    // panics if room is not known
    fn room_is_direct(&self, id: &RoomId) -> bool {
        let room = self.rooms_by_id.get(id).unwrap();
        room.room_type == "direct"
    }

    /// Returns whether a room is a space.
    /// panics if room is not known.
    fn room_is_space(&self, id: &RoomId) -> bool {
        let room = self.rooms_by_id.get(id).unwrap();
        room.room_type == "group"
    }

    /// Returns an iterator to rooms with the given filter.
    #[allow(clippy::needless_lifetimes)]
    pub fn rooms_filtered_by<'a>(
        &'a self,
        filter: &'a RoomsListFilter,
        _order: &'a RoomsListOrder,
    ) -> impl Iterator<Item = &'a Room> {
        self.rooms_by_id.values().filter(move |room| match filter {
            RoomsListFilter::All => true,
            RoomsListFilter::Direct => self.room_is_direct(&room.id),
            RoomsListFilter::Recent => self.room_has_activity_since(Duration::hours(24), &room.id),
            RoomsListFilter::Spaces => self.room_is_space(&room.id),
            RoomsListFilter::Unread => self.room_has_unread(&room.id),
        })
    }
}
