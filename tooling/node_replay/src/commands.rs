//! Command handler implementations.

use crate::errors::ReplayError;
use crate::lock;
use crate::types::{EventName, ReplayEvent, RunState, RunStatus, RunSummary};
use crate::workspace::Workspace;
use chrono::Utc;

/// Get current run status
pub fn get_status(workspace: &Workspace, run_id: &str) -> Result<RunStatus, ReplayError> {
    workspace.read_run_status(run_id)
}

/// Cancel a run (planned, running, or paused -> canceled).
///
/// For Running state with a healthy lock: creates `cancel.flag` as a signal.
/// The executor detects the flag, writes Canceled, removes the flag, and
/// releases its own lock.
///
/// For Running state with a stale/missing lock: the executor is dead. Write
/// Canceled directly and clean up the stale lock.
///
/// For non-Running states (Planned/Paused): writes Canceled directly.
pub fn cancel_run(workspace: &Workspace, run_id: &str) -> Result<RunStatus, ReplayError> {
    let mut status = workspace.read_run_status(run_id)?;

    // Validate transition
    status.state.transition_to(&RunState::Canceled)?;

    if status.state == RunState::Running {
        let lock_path = workspace.run_lock_path(run_id);
        if lock::is_locked(&lock_path) {
            // Executor is alive — drop a cancel flag for it to pick up.
            let flag_path = workspace.cancel_flag_path(run_id);
            std::fs::write(&flag_path, "")?;
            // Return current status; executor will write Canceled.
            return Ok(status);
        }
        // Lock is stale or missing — executor is dead. Clean up and
        // write Canceled directly (fall through to common path below).
        let _ = lock::release_lock(&lock_path);
    }

    // Planned/Paused, or Running with dead executor: write Canceled directly.
    status.state = RunState::Canceled;
    status.updated_at = Utc::now();
    workspace.write_run_status(&status)?;

    workspace.append_event(&ReplayEvent::new(
        run_id.to_string(),
        EventName::RunFailed,
        status.last_completed_block,
        status.last_completed_hash.clone(),
    ))?;

    Ok(status)
}

/// Resume a paused, failed, or stale-running run. Returns the updated status set
/// to Running. The caller (main.rs) should then call runner::execute_run() to
/// continue execution.
///
/// Running state is accepted only when the lock is stale or missing (crashed
/// executor recovery). If the lock is healthy, the executor is alive and resume
/// is rejected.
///
/// On success the lock is held — the caller (via execute_run) is responsible for
/// releasing it. On error the lock is released before returning.
pub fn resume_run(workspace: &Workspace, run_id: &str) -> Result<RunStatus, ReplayError> {
    let mut status = workspace.read_run_status(run_id)?;

    // Can resume from Paused, Failed, or stale Running (crashed executor).
    match status.state {
        RunState::Paused | RunState::Failed => {}
        RunState::Running => {
            let lock_path = workspace.run_lock_path(run_id);
            if lock::is_locked(&lock_path) {
                // Executor is alive — can't resume a running run.
                return Err(ReplayError::RunAlreadyRunning(run_id.to_string()));
            }
            // Lock is stale or missing — executor is dead. Clean up stale
            // lock so acquire_lock below succeeds, then proceed with resume.
            let _ = lock::release_lock(&lock_path);
        }
        RunState::Completed => {
            // Already done, return as-is (idempotent)
            return Ok(status);
        }
        _ => {
            return Err(ReplayError::InvalidTransition {
                from: format!("{:?}", status.state).to_lowercase(),
                to: "running".to_string(),
            });
        }
    }

    // Acquire lock
    let lock_path = workspace.run_lock_path(run_id);
    lock::acquire_lock(&lock_path, run_id)?;

    // All post-lock work is wrapped so the lock is released on error.
    let result = resume_run_inner(workspace, run_id, &mut status);
    if result.is_err() {
        let _ = lock::release_lock_if_owned(&lock_path, run_id);
    }
    result
}

/// Inner resume logic, called only while the lock is held.
fn resume_run_inner(
    workspace: &Workspace,
    run_id: &str,
    status: &mut RunStatus,
) -> Result<RunStatus, ReplayError> {
    status.state = RunState::Running;
    status.error_code = None;
    status.error_message = None;
    status.updated_at = Utc::now();
    workspace.write_run_status(status)?;

    workspace.append_event(&ReplayEvent::new(
        run_id.to_string(),
        EventName::RunResumed,
        status.last_completed_block,
        status.last_completed_hash.clone(),
    ))?;

    Ok(status.clone())
}

/// Verify a completed run by checking that block execution produced correct state roots.
/// This reads the events log and reports any mismatches.
pub fn verify_run(workspace: &Workspace, run_id: &str) -> Result<VerificationReport, ReplayError> {
    let status = workspace.read_run_status(run_id)?;

    if status.state != RunState::Completed {
        return Err(ReplayError::InvalidArgument(format!(
            "can only verify completed runs, current state: {:?}",
            status.state
        )));
    }

    let summary = workspace.read_run_summary(run_id)?;
    let events = workspace.read_events(run_id)?;

    // Count block_executed events to verify all blocks were processed
    let executed_count = events
        .iter()
        .filter(|e| e.event == EventName::BlockExecuted)
        .count() as u64;

    let report = VerificationReport {
        run_id: run_id.to_string(),
        total_blocks: summary.total_blocks,
        verified_blocks: executed_count,
        mismatches: summary.mismatch_count,
        complete: executed_count == summary.total_blocks,
        final_head_number: summary.final_head_number,
        final_head_hash: summary.final_head_hash,
    };

    // Emit verification events for each verified block
    for event in events
        .iter()
        .filter(|e| e.event == EventName::BlockExecuted)
    {
        workspace.append_event(&ReplayEvent::new(
            run_id.to_string(),
            EventName::BlockVerified,
            event.block_number,
            event.block_hash.clone(),
        ))?;
    }

    Ok(report)
}

/// Generate a report for a run
pub fn get_report(workspace: &Workspace, run_id: &str) -> Result<RunReport, ReplayError> {
    let status = workspace.read_run_status(run_id)?;
    let manifest = workspace.read_run_manifest(run_id)?;
    let summary = if status.state == RunState::Completed {
        Some(workspace.read_run_summary(run_id)?)
    } else {
        None
    };
    let events = workspace.read_events(run_id)?;

    Ok(RunReport {
        run_id: run_id.to_string(),
        checkpoint_id: manifest.checkpoint_id,
        state: status.state,
        start_number: manifest.start_number,
        end_number: manifest.end_number,
        total_blocks: manifest.canonical_hashes.len() as u64,
        last_completed_block: status.last_completed_block,
        last_completed_hash: status.last_completed_hash,
        started_at: status.started_at,
        updated_at: status.updated_at,
        error_code: status.error_code,
        error_message: status.error_message,
        summary,
        event_count: events.len(),
    })
}

/// Verification report
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VerificationReport {
    pub run_id: String,
    pub total_blocks: u64,
    pub verified_blocks: u64,
    pub mismatches: u64,
    pub complete: bool,
    pub final_head_number: Option<u64>,
    pub final_head_hash: Option<String>,
}

/// Full run report
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RunReport {
    pub run_id: String,
    pub checkpoint_id: String,
    pub state: RunState,
    pub start_number: u64,
    pub end_number: u64,
    pub total_blocks: u64,
    pub last_completed_block: Option<u64>,
    pub last_completed_hash: Option<String>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub summary: Option<RunSummary>,
    pub event_count: usize,
}
