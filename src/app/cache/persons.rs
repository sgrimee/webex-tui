use std::collections::{HashMap, HashSet};

use crate::frontend::data::PersonSnapshot;

pub(crate) type PersonId = String;

/// Frontend-neutral person cache, filled by the bridge glue (senders).
#[derive(Default)]
pub(crate) struct Persons {
    by_id: HashMap<PersonId, PersonSnapshot>,
    requested: HashSet<PersonId>,
}

impl Persons {
    pub(crate) fn get(&self, id: &str) -> Option<&PersonSnapshot> {
        self.by_id.get(id)
    }

    pub(crate) fn insert(&mut self, person: PersonSnapshot) {
        let id = person.id.as_str().to_owned();
        self.requested.remove(&id);
        self.by_id.insert(id, person);
    }

    pub(crate) fn add_requested(&mut self, id: &str) {
        self.requested.insert(id.to_owned());
    }

    pub(crate) fn exists_or_requested(&self, id: &str) -> bool {
        self.by_id.contains_key(id) || self.requested.contains(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::data::PersonId as FrontendPersonId;

    #[test]
    fn stores_neutral_person_snapshot_and_requested_marker() {
        let mut persons = Persons::default();
        persons.insert(PersonSnapshot {
            id: FrontendPersonId::new("person-a"),
            display_name: Some("Person A".into()),
            emails: None,
        });
        assert_eq!(
            persons.get("person-a").unwrap().display_name.as_deref(),
            Some("Person A")
        );
        persons.add_requested("person-b");
        assert!(persons.exists_or_requested("person-a"));
        assert!(persons.exists_or_requested("person-b"));
    }
}
