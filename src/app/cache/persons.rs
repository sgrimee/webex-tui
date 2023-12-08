use std::collections::{HashMap, HashSet};
use webex::types::Person as WebexPerson;

pub type PersonId = String;

/// Wrapper around a webex Person
#[derive(Debug, Default)]
pub struct Persons {
    pub by_id: HashMap<PersonId, WebexPerson>,
    pub requested: HashSet<PersonId>,
}

impl Persons {
    pub fn get(&self, id: &str) -> Option<&WebexPerson> {
        self.by_id.get(id)
    }

    pub fn insert(&mut self, person: WebexPerson) {
        self.by_id.insert(person.id.clone(), person);
    }

    pub fn add_requested(&mut self, id: &str) {
        self.requested.insert(id.to_string());
    }

    pub fn exists_or_requested(&self, id: &str) -> bool {
        self.by_id.contains_key(id) || self.requested.contains(id)
    }
}
