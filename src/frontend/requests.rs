//! Request/result correlation for a selected room's message-page presentation.
//!
//! These pure types do not dispatch work or change backend/cache state. A late
//! result may still update its own room's cache; `is_current` only gates changes
//! to the current view (selection/loading). Writes must not use this latest-read
//! policy: each write needs its own outcome, regardless of later selection.

/// Context captured before a selected-room page read is dispatched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoomRequestContext {
    pub(crate) room_id: String,
    pub(crate) view_generation: u64,
    pub(crate) request_id: u64,
}

/// A typed worker result returns the exact context supplied with the request.
#[cfg(test)]
pub(crate) struct RoomResult<T, E> {
    pub(crate) context: RoomRequestContext,
    pub(crate) outcome: Result<T, E>,
}

/// Counter exhaustion fails closed instead of wrapping and accepting stale work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CounterExhausted;

impl std::fmt::Display for CounterExhausted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Frontend request counter exhausted")
    }
}

impl std::error::Error for CounterExhausted {}

/// Latest-read context for the selected room. Not an application-wide tracker:
/// independent read targets should have their own tracker/request identity.
#[derive(Default)]
#[cfg(test)]
pub(crate) struct RoomViewRequests {
    active_room: Option<String>,
    view_generation: u64,
    request_sequence: u64,
    latest_request: Option<u64>,
}

#[cfg(test)]
impl RoomViewRequests {
    /// Change focus, invalidating outstanding results even for A → B → A.
    /// Selecting the already-active room leaves its outstanding read intact.
    pub(crate) fn select_room(&mut self, room_id: Option<String>) -> Result<(), CounterExhausted> {
        if self.active_room != room_id {
            let generation = self
                .view_generation
                .checked_add(1)
                .ok_or(CounterExhausted)?;
            self.active_room = room_id;
            self.view_generation = generation;
            self.latest_request = None;
        }
        Ok(())
    }

    /// Start a new selected-room read. With no room selected, no work is created.
    pub(crate) fn start_read(&mut self) -> Result<Option<RoomRequestContext>, CounterExhausted> {
        let Some(room_id) = &self.active_room else {
            return Ok(None);
        };
        let request_id = self
            .request_sequence
            .checked_add(1)
            .ok_or(CounterExhausted)?;
        self.request_sequence = request_id;
        self.latest_request = Some(request_id);
        Ok(Some(RoomRequestContext {
            room_id: room_id.clone(),
            view_generation: self.view_generation,
            request_id,
        }))
    }

    /// Whether this read result may affect current selection/loading. Results
    /// for a previous room, focus generation or superseded read are stale here.
    pub(crate) fn is_current(&self, context: &RoomRequestContext) -> bool {
        self.active_room.as_deref() == Some(context.room_id.as_str())
            && self.view_generation == context.view_generation
            && self.latest_request == Some(context.request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected_room() -> RoomViewRequests {
        let mut requests = RoomViewRequests::default();
        requests.select_room(Some("room-a".into())).unwrap();
        requests
    }

    #[test]
    fn no_selection_creates_no_request() {
        let mut requests = RoomViewRequests::default();
        assert_eq!(requests.start_read().unwrap(), None);
        assert_eq!(requests.request_sequence, 0);
    }

    #[test]
    fn typed_result_retains_context_and_outcome() {
        let mut requests = selected_room();
        let result = RoomResult::<_, &str> {
            context: requests.start_read().unwrap().unwrap(),
            outcome: Ok(vec!["message-a"]),
        };
        assert!(requests.is_current(&result.context));
        assert_eq!(result.outcome.unwrap(), ["message-a"]);
    }

    #[test]
    fn older_read_cannot_clear_newer_read_loading() {
        let mut requests = selected_room();
        let old = requests.start_read().unwrap().unwrap();
        let new = requests.start_read().unwrap().unwrap();
        assert!(new.request_id > old.request_id);
        assert!(!requests.is_current(&old));
        assert!(requests.is_current(&new));
    }

    #[test]
    fn room_a_result_cannot_change_room_b_view() {
        let mut requests = selected_room();
        let room_a = requests.start_read().unwrap().unwrap();
        requests.select_room(Some("room-b".into())).unwrap();
        let room_b = requests.start_read().unwrap().unwrap();
        assert!(!requests.is_current(&room_a));
        assert!(requests.is_current(&room_b));
    }

    #[test]
    fn returning_to_room_a_does_not_accept_its_old_result() {
        let mut requests = selected_room();
        let old = requests.start_read().unwrap().unwrap();
        requests.select_room(Some("room-b".into())).unwrap();
        requests.select_room(Some("room-a".into())).unwrap();
        let new = requests.start_read().unwrap().unwrap();
        assert_ne!(old.view_generation, new.view_generation);
        assert!(!requests.is_current(&old));
        assert!(requests.is_current(&new));
    }

    #[test]
    fn same_room_selection_preserves_in_flight_read() {
        let mut requests = selected_room();
        let context = requests.start_read().unwrap().unwrap();
        requests.select_room(Some("room-a".into())).unwrap();
        assert!(requests.is_current(&context));
    }

    #[test]
    fn clearing_selection_invalidates_even_failed_results() {
        let mut requests = selected_room();
        let result = RoomResult::<Vec<String>, _> {
            context: requests.start_read().unwrap().unwrap(),
            outcome: Err("offline"),
        };
        requests.select_room(None).unwrap();
        assert!(!requests.is_current(&result.context));
        assert_eq!(result.outcome, Err("offline"));
    }

    #[test]
    fn request_counter_cannot_wrap() {
        let mut requests = selected_room();
        requests.request_sequence = u64::MAX;
        assert_eq!(requests.start_read(), Err(CounterExhausted));
        assert_eq!(requests.request_sequence, u64::MAX);
        assert_eq!(requests.latest_request, None);
    }

    #[test]
    fn generation_counter_cannot_wrap_or_change_selection_on_error() {
        let mut requests = selected_room();
        let context = requests.start_read().unwrap().unwrap();
        requests.view_generation = u64::MAX;
        assert_eq!(requests.select_room(None), Err(CounterExhausted));
        assert_eq!(requests.active_room.as_deref(), Some("room-a"));
        assert_eq!(requests.view_generation, u64::MAX);
        assert_eq!(requests.latest_request, Some(context.request_id));
    }
}
