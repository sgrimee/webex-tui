use crate::teams::store::Store;

// #[derive(Clone)]
pub enum AppState {
    Init,
    Initialized {
        editing_mode: bool,
        teams_store: Store,
        active_room: Option<String>,
    },
}

impl AppState {
    pub fn initialized() -> Self {
        let editing_mode = false;
        let teams_store = Store::default();
        let active_room = None;
        Self::Initialized {
            editing_mode,
            teams_store,
            active_room,
        }
    }

    pub fn is_initialized(&self) -> bool {
        matches!(self, &Self::Initialized { .. })
    }

    pub fn set_editing(&mut self, value: bool) {
        if let Self::Initialized { editing_mode, .. } = self {
            *editing_mode = value;
        }
    }

    pub fn is_editing(&self) -> bool {
        if let Self::Initialized { editing_mode, .. } = self {
            return *editing_mode;
        }
        false
    }

    pub fn store(&mut self) -> Option<&mut Store> {
        if let Self::Initialized { teams_store, .. } = self {
            Some(teams_store)
        } else {
            None
        }
    }

    pub fn active_room(&self) -> Option<String> {
        if let Self::Initialized { active_room, .. } = self {
            active_room.clone()
        } else {
            None
        }
    }

    pub fn set_active_room(&mut self, room: &str) {
        if let Self::Initialized { active_room, .. } = self {
            *active_room = Some(room.to_string());
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::Init
    }
}
