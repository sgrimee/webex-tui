#[derive(Clone)]
pub enum AppState {
    Init,
    Initialized { editing_mode: bool },
}

impl AppState {
    pub fn initialized() -> Self {
        let editing_mode = false;

        Self::Initialized { editing_mode }
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
        return false;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::Init
    }
}
