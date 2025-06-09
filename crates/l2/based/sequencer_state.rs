use std::sync::Arc;

use tokio::sync::Mutex;

pub type SequencerState = Arc<Mutex<SequencerStatus>>;

#[derive(Debug, Default, Clone)]
pub enum SequencerStatus {
    Sequencing,
    #[default]
    Following,
}

impl std::fmt::Display for SequencerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SequencerStatus::Sequencing => write!(f, "Sequencing"),
            SequencerStatus::Following => write!(f, "Following"),
        }
    }
}
