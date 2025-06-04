#[derive(Debug, Default, Clone)]
pub enum SequencerState {
    Sequencing,
    #[default]
    Following,
}

impl std::fmt::Display for SequencerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SequencerState::Sequencing => write!(f, "Sequencing"),
            SequencerState::Following => write!(f, "Following"),
        }
    }
}
