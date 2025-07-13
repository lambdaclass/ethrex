use std::fmt::Display;

#[derive(Debug, Clone, Default)]
pub enum TabsState {
    #[default]
    Overview = 0,
    Logs = 1,
    Contacts = 2,
}

impl TabsState {
    pub fn next(&mut self) {
        match self {
            TabsState::Overview => *self = TabsState::Logs,
            TabsState::Logs => *self = TabsState::Contacts,
            TabsState::Contacts => *self = TabsState::Overview,
        }
    }

    pub fn previous(&mut self) {
        match self {
            TabsState::Overview => *self = TabsState::Contacts,
            TabsState::Logs => *self = TabsState::Overview,
            TabsState::Contacts => *self = TabsState::Overview,
        }
    }
}

impl Display for TabsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TabsState::Overview => write!(f, "Overview"),
            TabsState::Logs => write!(f, "Logs"),
            TabsState::Contacts => write!(f, "Contacts"),
        }
    }
}

impl From<TabsState> for Option<usize> {
    fn from(state: TabsState) -> Self {
        match state {
            TabsState::Overview => Some(0),
            TabsState::Logs => Some(1),
            TabsState::Contacts => Some(2),
        }
    }
}
