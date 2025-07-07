use std::fmt::Display;

#[derive(Debug, Clone, Default)]
pub enum TabsState {
    #[default]
    Overview = 0,
    Logs = 1,
}

impl TabsState {
    pub fn next(&mut self) {
        match self {
            TabsState::Overview => *self = TabsState::Logs,
            TabsState::Logs => *self = TabsState::Overview,
        }
    }

    pub fn previous(&mut self) {
        match self {
            TabsState::Overview => *self = TabsState::Logs,
            TabsState::Logs => *self = TabsState::Overview,
        }
    }
}

impl Display for TabsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TabsState::Overview => write!(f, "Overview"),
            TabsState::Logs => write!(f, "Logs"),
        }
    }
}

impl From<TabsState> for Option<usize> {
    fn from(state: TabsState) -> Self {
        match state {
            TabsState::Overview => Some(0),
            TabsState::Logs => Some(1),
        }
    }
}
