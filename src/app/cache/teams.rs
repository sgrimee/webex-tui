use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use crate::frontend::data::TeamSnapshot;

pub(crate) type TeamId = String;

/// Frontend-neutral team cache, filled by the bridge glue (team names).
#[derive(Default)]
pub(crate) struct Teams {
    teams_by_id: HashMap<TeamId, TeamSnapshot>,
    requested_teams: HashSet<TeamId>,
    /// Teams the daemon could not return (e.g. 404 for a team this account
    /// cannot see), until when not to ask again. Kept across snapshot
    /// installs: each miss costs the daemon a Webex REST call.
    unavailable: HashMap<TeamId, Instant>,
}

/// How long a team the daemon reported missing or inaccessible is not asked
/// again (`not_found`, `forbidden`).
pub(crate) const TEAM_MISSING_TTL: Duration = Duration::from_secs(60 * 60);
/// The same for a Webex-side failure (`webex_unavailable`): retried sooner.
pub(crate) const TEAM_REMOTE_FAILURE_TTL: Duration = Duration::from_secs(5 * 60);

impl Teams {
    pub(crate) fn add(&mut self, team: TeamSnapshot) {
        let id = team.id.as_str().to_owned();
        self.requested_teams.remove(&id);
        self.unavailable.remove(&id);
        self.teams_by_id.insert(id, team);
    }

    pub(crate) fn team_with_id(&self, id: &TeamId) -> Option<&TeamSnapshot> {
        self.teams_by_id.get(id)
    }

    pub(crate) fn add_requested(&mut self, team_id: TeamId) {
        self.requested_teams.insert(team_id);
    }

    /// Known, being requested, or recently reported unavailable: in all
    /// three cases no new lookup is needed.
    pub(crate) fn exists_or_requested(&self, id: &TeamId) -> bool {
        self.exists_or_requested_at(id, Instant::now())
    }

    pub(crate) fn exists_or_requested_at(&self, id: &TeamId, now: Instant) -> bool {
        self.teams_by_id.contains_key(id)
            || self.requested_teams.contains(id)
            || self.unavailable.get(id).is_some_and(|until| now < *until)
    }

    /// The daemon answered that it cannot return this team: do not ask again
    /// for `ttl` (the room keeps its own title meanwhile).
    pub(crate) fn mark_unavailable(&mut self, id: &TeamId, now: Instant, ttl: Duration) {
        self.requested_teams.remove(id);
        self.unavailable.insert(id.clone(), now + ttl);
    }

    /// Allow a fresh request after a transient failure.
    pub(crate) fn forget_requested(&mut self, id: &TeamId) {
        self.requested_teams.remove(id);
    }

    /// Forget every outstanding request (new session/snapshot). Teams
    /// reported unavailable stay so until their expiry.
    pub(crate) fn clear_requested(&mut self) {
        self.requested_teams.clear();
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

    #[test]
    fn an_unavailable_team_is_not_asked_again_until_its_expiry_even_after_a_snapshot() {
        let mut teams = Teams::default();
        let now = Instant::now();
        let id: TeamId = "team-x".into();
        teams.add_requested(id.clone());
        teams.mark_unavailable(&id, now, Duration::from_secs(60));
        teams.clear_requested();
        assert!(teams.exists_or_requested_at(&id, now + Duration::from_secs(59)));
        assert!(!teams.exists_or_requested_at(&id, now + Duration::from_secs(60)));
        assert!(teams.team_with_id(&id).is_none());
        // A later successful lookup replaces the marker.
        teams.add(TeamSnapshot {
            id: FrontendTeamId::new("team-x"),
            name: Some("X".into()),
        });
        assert!(teams.exists_or_requested_at(&id, now + Duration::from_secs(3600)));
    }
}
