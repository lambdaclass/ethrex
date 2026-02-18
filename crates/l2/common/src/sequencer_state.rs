use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct SequencerState(Arc<Mutex<SequencerStatus>>);

impl SequencerState {
    pub fn status(&self) -> SequencerStatus {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn new_status(&self, status: SequencerStatus) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = status;
    }
}

impl From<SequencerStatus> for SequencerState {
    fn from(status: SequencerStatus) -> Self {
        Self(Arc::new(Mutex::new(status)))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, Copy)]
pub enum SequencerStatus {
    Sequencing,
    #[default]
    Syncing,
    Following,
}

impl std::fmt::Display for SequencerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SequencerStatus::Sequencing => write!(f, "Sequencing"),
            SequencerStatus::Syncing => write!(f, "Syncing"),
            SequencerStatus::Following => write!(f, "Following"),
        }
    }
}
