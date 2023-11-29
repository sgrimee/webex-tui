use std::fmt::{Display, Formatter, Result};

/// The title of a room and its team if any.
#[derive(Debug, PartialEq, Clone)]
pub struct RoomAndTeamTitle {
    pub room_title: String,
    pub team_name: Option<String>,
}

impl Display for RoomAndTeamTitle {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match &self.team_name {
            None => write!(f, "{}", self.room_title),
            Some(team_name) => write!(f, "{} ({})", self.room_title, team_name),
        }
    }
}

impl Default for RoomAndTeamTitle {
    fn default() -> Self {
        Self {
            room_title: String::from("No room title"),
            team_name: None,
        }
    }
}
