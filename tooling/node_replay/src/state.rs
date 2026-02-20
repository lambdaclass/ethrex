//! Run state persistence and tracking.

use crate::errors::ReplayError;
use crate::types::RunState;

impl RunState {
    /// Validate and perform a state transition. Returns error for invalid transitions.
    pub fn transition_to(&self, target: &RunState) -> Result<RunState, ReplayError> {
        if self.can_transition_to(target) {
            Ok(target.clone())
        } else {
            Err(ReplayError::InvalidTransition {
                from: format!("{:?}", self).to_lowercase(),
                to: format!("{:?}", target).to_lowercase(),
            })
        }
    }

    /// Check if transition from self to target is valid
    pub fn can_transition_to(&self, target: &RunState) -> bool {
        matches!(
            (self, target),
            // Valid transitions per plan:
            (RunState::Planned, RunState::Running)
                | (RunState::Running, RunState::Paused)
                | (RunState::Paused, RunState::Running)
                | (RunState::Failed, RunState::Running)
                | (RunState::Running, RunState::Completed)
                | (RunState::Running, RunState::Failed)
                // Any non-terminal to canceled
                | (RunState::Planned, RunState::Canceled)
                | (RunState::Running, RunState::Canceled)
                | (RunState::Paused, RunState::Canceled)
        )
    }
}
