use std::collections::HashSet;

use super::room::{Room, RoomId};
use super::room_list_filter::RoomsListFilter;

use chrono::Duration;
use log::*;
use webex::Room as WebexRoom;
#[derive(Default, Debug)]
pub struct Rooms {
    /// RoomInfo sorted by last activity.
    sorted_rooms: Vec<Room>,
    /// Set of rooms for which we requested room info.
    requested_rooms: HashSet<RoomId>,
}

impl Rooms {
    /// Returns a mutable reference to the room for given id, if found.
    pub fn room_with_id_mut(&mut self, id: &RoomId) -> Option<&mut Room> {
        self.sorted_rooms.iter_mut().find(|room| room.id() == id)
    }

    /// Returns a reference to the room for given id, if found.
    pub fn room_with_id(&self, id: &RoomId) -> Option<&Room> {
        self.sorted_rooms.iter().find(|room| room.id() == id)
    }

    /// Adds a `RoomId` to the set of requested room
    pub fn add_requested_room(&mut self, room_id: RoomId) {
        self.requested_rooms.insert(room_id);
    }

    /// Adds or updates a `WebexRoom` in the store. If the room already exists, it is updated.
    /// The list is kept in order of last_activity.
    pub fn update_with_webex_room(&mut self, webex_room: WebexRoom) {
        let room_id = webex_room.id.clone();
        let mut room: Room = webex_room.into();
        // If the room is already in the list, remove it.
        if let Some(index) = self.sorted_rooms.iter().position(|r| r.id() == room.id()) {
            // Conserve the unread attribute, replace the rest
            room.set_unread(self.sorted_rooms[index].unread());
            self.sorted_rooms[index] = room;
        } else {
            // Add the room at the right position
            let pos = self
                .sorted_rooms
                .partition_point(|r| r.last_activity() > room.last_activity());
            self.sorted_rooms.insert(pos, room);
        }
        // The room has been received, remove it from the requested list
        self.requested_rooms.remove(&room_id);
    }

    /// Returns whether the room is already present, or if it has already been requested.
    pub fn room_exists_or_requested(&self, id: &RoomId) -> bool {
        self.sorted_rooms.iter().any(|room| room.id() == id) || self.requested_rooms.contains(id)
    }

    /// Mark a room as unread.
    pub fn mark_unread(&mut self, id: &RoomId) {
        debug!("Marking room {} unread", id);
        for room in &mut self.sorted_rooms {
            if room.id() == id {
                room.set_unread(true);
                break;
            }
        }
    }

    /// Mark a room as read.
    pub fn mark_read(&mut self, id: &RoomId) {
        debug!("Marking room {} read", id);
        for room in &mut self.sorted_rooms {
            if room.id() == id {
                room.set_unread(false);
                break;
            }
        }
    }

    /// Returns an iterator to rooms with the given filter.
    #[allow(clippy::needless_lifetimes)]
    pub fn rooms_filtered_by<'a>(
        &'a self,
        filter: &'a RoomsListFilter,
    ) -> impl Iterator<Item = &'a Room> {
        self.sorted_rooms.iter().filter(move |room| match filter {
            RoomsListFilter::All => true,
            RoomsListFilter::Direct => room.is_direct(),
            RoomsListFilter::Recent => room.has_activity_since(Duration::hours(24)),
            RoomsListFilter::Spaces => room.is_space(),
            RoomsListFilter::Unread => room.unread(),
        })
    }
}
