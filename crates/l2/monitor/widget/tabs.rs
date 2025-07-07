use std::fmt::Display;

#[derive(Debug, Clone, Default)]
pub enum TabsSate {
    #[default]
    Overview = 0,
    Logs = 1,
}

impl TabsSate {
    pub fn next(&mut self) {
        match self {
            TabsSate::Overview => *self = TabsSate::Logs,
            TabsSate::Logs => *self = TabsSate::Overview,
        }
    }

    pub fn previous(&mut self) {
        match self {
            TabsSate::Overview => *self = TabsSate::Logs,
            TabsSate::Logs => *self = TabsSate::Overview,
        }
    }
}

impl Display for TabsSate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TabsSate::Overview => write!(f, "Overview"),
            TabsSate::Logs => write!(f, "Logs"),
        }
    }
}

impl From<TabsSate> for Option<usize> {
    fn from(state: TabsSate) -> Self {
        match state {
            TabsSate::Overview => Some(0),
            TabsSate::Logs => Some(1),
        }
    }
}
