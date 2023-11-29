use std::collections::{HashMap, HashSet};

use webex::Team;

pub type TeamId = String;

/// Cache for `Team` objects
#[derive(Default, Debug)]
pub(crate) struct Teams {
    teams_by_id: HashMap<TeamId, Team>,
    requested_teams: HashSet<TeamId>,
}

impl Teams {
    /// Add a `Team` to the cache.
    pub(crate) fn add(&mut self, team: Team) {
        self.requested_teams.remove(&team.id);
        self.teams_by_id.insert(team.id.clone(), team);
    }

    /// Returns a reference to the team for given id, if found.
    pub(crate) fn team_with_id(&self, id: &TeamId) -> Option<&Team> {
        self.teams_by_id.get(id)
    }

    /// Adds a `TeamId` to the set of requested teams
    pub(crate) fn add_requested(&mut self, team_id: TeamId) {
        self.requested_teams.insert(team_id);
    }

    /// Returns whether the team is already present, or if it has already been requested.
    pub(crate) fn exists_or_requested(&self, id: &TeamId) -> bool {
        self.teams_by_id.contains_key(id) || self.requested_teams.contains(id)
    }
}
