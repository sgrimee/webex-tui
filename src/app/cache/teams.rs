use std::collections::{HashMap, HashSet};

use crate::frontend::data::TeamSnapshot;

pub(crate) type TeamId = String;

/// Frontend-neutral team cache; legacy transport values are projected in `bridge`.
#[derive(Default)]
pub(crate) struct Teams {
    teams_by_id: HashMap<TeamId, TeamSnapshot>,
    requested_teams: HashSet<TeamId>,
}

impl Teams {
    pub(crate) fn add(&mut self, team: TeamSnapshot) {
        let id = team.id.as_str().to_owned();
        self.requested_teams.remove(&id);
        self.teams_by_id.insert(id, team);
    }

    pub(crate) fn team_with_id(&self, id: &TeamId) -> Option<&TeamSnapshot> {
        self.teams_by_id.get(id)
    }

    pub(crate) fn add_requested(&mut self, team_id: TeamId) {
        self.requested_teams.insert(team_id);
    }

    pub(crate) fn exists_or_requested(&self, id: &TeamId) -> bool {
        self.teams_by_id.contains_key(id) || self.requested_teams.contains(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::data::TeamId as FrontendTeamId;

    #[test]
    fn stores_neutral_team_snapshot_and_requested_marker() {
        let mut teams = Teams::default();
        teams.add(TeamSnapshot {
            id: FrontendTeamId::new("team-a"),
            name: Some("Team A".into()),
        });
        assert_eq!(
            teams
                .team_with_id(&"team-a".into())
                .unwrap()
                .name
                .as_deref(),
            Some("Team A")
        );
        teams.add_requested("team-b".into());
        assert!(teams.exists_or_requested(&"team-a".into()));
        assert!(teams.exists_or_requested(&"team-b".into()));
    }
}
