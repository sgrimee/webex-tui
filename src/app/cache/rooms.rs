use std::collections::HashSet;

use super::room::{Room, RoomId};
use super::room_list_filter::RoomsListFilter;

use chrono::Duration;
use log::*;
#[derive(Default, Debug)]
pub(crate) struct Rooms {
    /// RoomInfo sorted by last activity.
    sorted_rooms: Vec<Room>,
    /// Set of rooms for which we requested room info.
    requested_rooms: HashSet<RoomId>,
}

impl Rooms {
    /// Returns a mutable reference to the room for given id, if found.
    pub(crate) fn room_with_id_mut(&mut self, id: &RoomId) -> Option<&mut Room> {
        self.sorted_rooms.iter_mut().find(|room| room.id == *id)
    }

    /// Returns a reference to the room for given id, if found.
    pub(crate) fn room_with_id(&self, id: &RoomId) -> Option<&Room> {
        self.sorted_rooms.iter().find(|room| room.id == *id)
    }

    /// Adds a `RoomId` to the set of requested room
    pub(crate) fn add_requested(&mut self, room_id: RoomId) {
        self.requested_rooms.insert(room_id);
    }

    /// Adds or updates a `Room` in the store. If the room already exists, it is updated.
    /// The list is kept in order of last_activity.
    pub(crate) fn update_with_room(&mut self, room: &Room) {
        let mut room = room.clone();
        // If the room is already in the list
        if let Some(index) = self.sorted_rooms.iter().position(|r| r.id == room.id) {
            // Conserve the unread attribute
            room.unread = self.sorted_rooms[index].unread;
            // Remove the room from the store
            self.sorted_rooms.remove(index);
        }
        // The room is being added, remove it from the requested list
        self.requested_rooms.remove(&room.id);
        // Add it (back) at the correct position
        self.add_room_sorted(room);
    }

    /// Adds a room to the list of rooms, keeping the list sorted by last activity.
    /// It is an error to use this if the room already exists.
    fn add_room_sorted(&mut self, room: Room) {
        let pos = self
            .sorted_rooms
            .partition_point(|r| r.last_activity > room.last_activity);
        self.sorted_rooms.insert(pos, room);
    }

    /// Adjusts the position of the room in the list based in timestamp
    pub(crate) fn reposition_room(&mut self, room_id: &str) {
        if let Some(index) = self.sorted_rooms.iter().position(|r| r.id == room_id) {
            let room = self.sorted_rooms.remove(index);
            self.add_room_sorted(room);
        }
    }

    /// Returns whether the room is already present, or if it has already been requested.
    pub(crate) fn room_exists_or_requested(&self, id: &RoomId) -> bool {
        self.sorted_rooms.iter().any(|room| room.id == *id) || self.requested_rooms.contains(id)
    }

    /// Mark a room as unread.
    pub(crate) fn mark_unread(&mut self, id: &RoomId) {
        debug!("Marking room {id} unread");
        for room in &mut self.sorted_rooms {
            if room.id == *id {
                room.unread = true;
                break;
            }
        }
    }

    /// Mark a room as read.
    pub(crate) fn mark_read(&mut self, id: &RoomId) {
        debug!("Marking room {id} read");
        for room in &mut self.sorted_rooms {
            if room.id == *id {
                room.unread = false;
                break;
            }
        }
    }

    /// Returns an iterator to rooms with the given filter.
    #[allow(clippy::needless_lifetimes)]
    pub(crate) fn rooms_filtered_by<'a>(
        &'a self,
        filter: &'a RoomsListFilter,
    ) -> impl Iterator<Item = &'a Room> {
        self.sorted_rooms.iter().filter(move |room| match filter {
            RoomsListFilter::All => true,
            RoomsListFilter::Direct => room.is_direct(),
            RoomsListFilter::Recent => room.has_activity_since(Duration::hours(24)),
            RoomsListFilter::Spaces => room.is_space(),
            RoomsListFilter::Unread => room.unread,
            RoomsListFilter::InactiveSpaces => {
                !room.is_direct()
                    && !room.has_activity_since(Duration::days(365))
                    && !room.is_moderated()
            }
        })
    }

    /// Remove a room completely
    pub(crate) fn remove_room(&mut self, id: &RoomId) {
        debug!("Removing room {id}");
        self.sorted_rooms.retain(|room| room.id != *id);
        self.requested_rooms.remove(id);
    }

    /// Returns a reference to the sorted rooms list
    #[allow(dead_code)]
    pub(crate) fn sorted_rooms(&self) -> &Vec<Room> {
        &self.sorted_rooms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::cache::room::RoomId;
    use chrono::{TimeZone, Utc};

    #[test]
    fn test_update_with_room() {
        let mut rooms = Rooms::default();
        let room1 = Room {
            id: String::from("1") as RoomId,
            title: Some(String::from("Room 1")),
            last_activity: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 1).unwrap(),
            ..Default::default()
        };
        let room2 = Room {
            id: String::from("2") as RoomId,
            title: Some(String::from("Room 2")),
            last_activity: Utc.with_ymd_and_hms(2020, 1, 2, 0, 0, 1).unwrap(),
            ..Default::default()
        };
        let room3 = Room {
            id: String::from("3") as RoomId,
            title: Some(String::from("Room 3")),
            last_activity: Utc.with_ymd_and_hms(2020, 1, 3, 0, 0, 1).unwrap(),
            ..Default::default()
        };
        rooms.update_with_room(&room1);
        rooms.update_with_room(&room2);
        rooms.update_with_room(&room3);
        assert_eq!(rooms.sorted_rooms[0], room3);
        assert_eq!(rooms.sorted_rooms[1], room2);
        assert_eq!(rooms.sorted_rooms[2], room1);
        let room2_updated = Room {
            id: String::from("2") as RoomId,
            title: Some(String::from("Room 2")),
            last_activity: Utc.with_ymd_and_hms(2020, 1, 4, 0, 0, 1).unwrap(),
            ..Default::default()
        };
        rooms.update_with_room(&room2_updated);
        assert_eq!(rooms.sorted_rooms[0], room2_updated);
        assert_eq!(rooms.sorted_rooms[1], room3);
        assert_eq!(rooms.sorted_rooms[2], room1);
    }

    #[test]
    fn test_add_room_sorted() {
        let mut rooms = Rooms::default();
        let room1 = Room {
            id: String::from("1") as RoomId,
            title: Some(String::from("Room 1")),
            last_activity: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 1).unwrap(),
            ..Default::default()
        };
        let room2 = Room {
            id: String::from("2") as RoomId,
            title: Some(String::from("Room 2")),
            last_activity: Utc.with_ymd_and_hms(2020, 1, 2, 0, 0, 1).unwrap(),
            ..Default::default()
        };
        let room3 = Room {
            id: String::from("3") as RoomId,
            title: Some(String::from("Room 3")),
            last_activity: Utc.with_ymd_and_hms(2020, 1, 3, 0, 0, 1).unwrap(),
            ..Default::default()
        };
        rooms.add_room_sorted(room1);
        rooms.add_room_sorted(room2);
        rooms.add_room_sorted(room3);
        assert_eq!(rooms.sorted_rooms[0].id, "3");
        assert_eq!(rooms.sorted_rooms[1].id, "2");
        assert_eq!(rooms.sorted_rooms[2].id, "1");
    }

    #[test]
    fn test_room_exists_or_requested() {
        let mut rooms = Rooms::default();
        let room1 = Room {
            id: String::from("1") as RoomId,
            ..Default::default()
        };
        rooms.update_with_room(&room1);
        rooms.add_requested(String::from("2") as RoomId);
        assert!(rooms.room_exists_or_requested(&String::from("1") as &RoomId));
        assert!(rooms.room_exists_or_requested(&String::from("2") as &RoomId));
        assert!(!rooms.room_exists_or_requested(&String::from("3") as &RoomId));
    }

    #[test]
    fn test_remove_room() {
        let mut rooms = Rooms::default();
        let room1 = Room {
            id: String::from("1") as RoomId,
            ..Default::default()
        };
        rooms.update_with_room(&room1);
        rooms.add_requested(String::from("2") as RoomId);
        assert_eq!(rooms.sorted_rooms.len(), 1);
        assert_eq!(rooms.requested_rooms.len(), 1);
        rooms.remove_room(&String::from("1") as &RoomId);
        assert_eq!(rooms.sorted_rooms.len(), 0);
        assert_eq!(rooms.requested_rooms.len(), 1);
        rooms.remove_room(&String::from("2") as &RoomId);
        assert_eq!(rooms.sorted_rooms.len(), 0);
        assert_eq!(rooms.requested_rooms.len(), 0);
    }

    #[test]
    fn test_inactive_spaces_filter_excludes_moderated() {
        let mut rooms = Rooms::default();

        // Create an old, inactive, non-moderated space (should be included)
        let inactive_space = Room {
            id: String::from("inactive") as RoomId,
            title: Some(String::from("Old Inactive Space")),
            room_type: String::from("group"),
            is_locked: false,
            last_activity: Utc::now() - Duration::days(400),
            ..Default::default()
        };

        // Create an old, inactive, moderated space (should be excluded)
        let moderated_space = Room {
            id: String::from("moderated") as RoomId,
            title: Some(String::from("Moderated Space")),
            room_type: String::from("group"),
            is_locked: true,
            last_activity: Utc::now() - Duration::days(400),
            ..Default::default()
        };

        // Create a direct message (should be excluded)
        let direct_message = Room {
            id: String::from("direct") as RoomId,
            title: Some(String::from("Direct Chat")),
            room_type: String::from("direct"),
            is_locked: false,
            last_activity: Utc::now() - Duration::days(400),
            ..Default::default()
        };

        // Create a recent space (should be excluded)
        let recent_space = Room {
            id: String::from("recent") as RoomId,
            title: Some(String::from("Recent Space")),
            room_type: String::from("group"),
            is_locked: false,
            last_activity: Utc::now() - Duration::days(10),
            ..Default::default()
        };

        rooms.update_with_room(&inactive_space);
        rooms.update_with_room(&moderated_space);
        rooms.update_with_room(&direct_message);
        rooms.update_with_room(&recent_space);

        let filtered: Vec<&Room> = rooms
            .rooms_filtered_by(&RoomsListFilter::InactiveSpaces)
            .collect();

        // Only the inactive, non-moderated space should be included
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "inactive");
    }
}
