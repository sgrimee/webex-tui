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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_persons() {
        let mut persons = Persons::default();
        let person = WebexPerson {
            id: "id".to_string(),
            ..Default::default()
        };
        persons.insert(person.clone());
        assert_eq!(persons.get("id"), Some(&person));
        assert_eq!(persons.get("id2"), None);
        assert!(persons.exists_or_requested("id"));
        assert!(!persons.exists_or_requested("id2"));
        persons.add_requested("id2");
        assert!(persons.exists_or_requested("id2"));
    }
}
