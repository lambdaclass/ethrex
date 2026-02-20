//! Replay execution engine.

use crate::errors::ReplayError;
use crate::lock;
use crate::types::{EventName, ReplayEvent, ReplayMode, RunManifest, RunState, RunSummary};
use crate::workspace::Workspace;
use chrono::Utc;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::BlockHash;
use ethrex_storage::{EngineType, Store};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const LOCK_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Validate that the run DB path does not collide with the live or checkpoint DB paths.
/// All three paths are canonicalized before comparison.
pub fn validate_path_isolation(
    run_db: &Path,
    live_db: &Path,
    checkpoint_db: &Path,
) -> Result<(), ReplayError> {
    let run_canonical = run_db
        .canonicalize()
        .unwrap_or_else(|_| run_db.to_path_buf());
    let live_canonical = live_db
        .canonicalize()
        .unwrap_or_else(|_| live_db.to_path_buf());
    let checkpoint_canonical = checkpoint_db
        .canonicalize()
        .unwrap_or_else(|_| checkpoint_db.to_path_buf());

    if run_canonical == live_canonical {
        return Err(ReplayError::PathConflict {
            reason: format!(
                "run DB path '{}' resolves to the same location as the live DB '{}'",
                run_db.display(),
                live_db.display()
            ),
        });
    }

    if run_canonical == checkpoint_canonical {
        return Err(ReplayError::PathConflict {
            reason: format!(
                "run DB path '{}' resolves to the same location as the checkpoint DB '{}'",
                run_db.display(),
                checkpoint_db.display()
            ),
        });
    }

    Ok(())
}

/// Background heartbeat that keeps the run lock fresh while execution is in progress.
struct LockHeartbeat {
    run_id: String,
    stop: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl LockHeartbeat {
    fn start(lock_path: PathBuf, run_id: String) -> Result<Self, ReplayError> {
        // Validate ownership and refresh once before starting the periodic heartbeat.
        lock::refresh_lock(&lock_path, &run_id)?;

        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        let stop_thread = Arc::clone(&stop);
        let failure_thread = Arc::clone(&failure);
        let lock_path_thread = lock_path.clone();
        let run_id_thread = run_id.clone();

        let handle = std::thread::Builder::new()
            .name(format!("node-replay-heartbeat-{run_id_thread}"))
            .spawn(move || {
                while !stop_thread.load(Ordering::Acquire) {
                    std::thread::park_timeout(LOCK_HEARTBEAT_INTERVAL);
                    if stop_thread.load(Ordering::Acquire) {
                        break;
                    }
                    if let Err(err) = lock::refresh_lock(&lock_path_thread, &run_id_thread) {
                        match failure_thread.lock() {
                            Ok(mut slot) if slot.is_none() => {
                                *slot = Some(err.to_string());
                            }
                            _ => {}
                        }
                        break;
                    }
                }
            })
            .map_err(|e| {
                ReplayError::Internal(format!("failed to start lock heartbeat thread: {e}"))
            })?;

        Ok(Self {
            run_id,
            stop,
            failure,
            handle: Some(handle),
        })
    }

    fn check(&self) -> Result<(), ReplayError> {
        let failure = self
            .failure
            .lock()
            .map_err(|_| ReplayError::Internal("lock heartbeat mutex poisoned".to_string()))?;
        if let Some(msg) = failure.as_ref() {
            return Err(ReplayError::Internal(format!(
                "lock heartbeat failed for run '{}': {msg}",
                self.run_id
            )));
        }
        Ok(())
    }

    fn stop(mut self) -> Result<(), ReplayError> {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            handle
                .join()
                .map_err(|_| ReplayError::Internal("lock heartbeat thread panicked".to_string()))?;
        }
        self.check()
    }
}

/// Execute a planned replay run using a per-run DB copy.
///
/// Safety model:
/// - The checkpoint base DB is opened briefly to create a hard-linked copy, then dropped.
/// - All execution writes go to the per-run DB at `runs/<run_id>/db/`.
/// - The live DB is opened read-only to fetch block data.
///
/// # Lock ownership
///
/// `lock_held` indicates whether the caller already acquired the run lock (and
/// transitioned state to Running). This must be `true` only when called from
/// `commands::resume_run`. When `false`, this function acquires the lock itself
/// and rejects `Running` state with `RunAlreadyRunning`.
///
/// The lock is released on all exit paths (success and error).
pub async fn execute_run(
    workspace: &Workspace,
    manifest: &RunManifest,
    _mode: &ReplayMode,
    lock_held: bool,
) -> Result<RunSummary, ReplayError> {
    let run_id = &manifest.run_id;

    // 1. Read current status and validate state.
    let mut status = workspace.read_run_status(run_id)?;

    // Idempotent: already completed.
    if status.state == RunState::Completed {
        return workspace.read_run_summary(run_id);
    }

    if !lock_held {
        // Run command: only accept Planned state. Reject states that
        // require the Resume command per plan contract.
        match status.state {
            RunState::Running => {
                return Err(ReplayError::RunAlreadyRunning(run_id.clone()));
            }
            RunState::Paused | RunState::Failed => {
                return Err(ReplayError::InvalidArgument(format!(
                    "run '{run_id}' is in {:?} state; use the resume command instead",
                    status.state
                )));
            }
            _ => {}
        }
    }

    let lock_path = workspace.run_lock_path(run_id);

    if !lock_held {
        // Validate the transition (Planned -> Running).
        status.state.transition_to(&RunState::Running)?;

        // Acquire the run lock.
        lock::acquire_lock(&lock_path, run_id)?;
    }

    // From this point, we own the lock. ALL fallible operations (including
    // status write and event emit) go through execute_run_inner so the
    // lock is released on every exit path.
    let heartbeat = match LockHeartbeat::start(lock_path.clone(), run_id.clone()) {
        Ok(heartbeat) => heartbeat,
        Err(e) => {
            let _ = lock::release_lock_if_owned(&lock_path, run_id);
            return Err(e);
        }
    };

    let mut result =
        execute_run_inner(workspace, manifest, &mut status, !lock_held, &heartbeat).await;
    let heartbeat_result = heartbeat.stop();
    if let (true, Err(e)) = (result.is_ok(), heartbeat_result) {
        result = Err(e);
    }

    match &result {
        Ok(_) => {
            lock::release_lock_if_owned(&lock_path, run_id)?;
        }
        Err(_) => {
            // Best-effort release — don't mask the original error.
            let _ = lock::release_lock_if_owned(&lock_path, run_id);
        }
    }

    result
}

/// Check if a cancel flag has been placed by `cancel_run`. If the flag exists,
/// write Canceled to status.json (the executor is the sole writer while it
/// holds the lock), remove the flag, and return `Ok(true)`. Returns `Ok(false)`
/// if no cancellation was requested.
fn check_cancel_flag(
    workspace: &Workspace,
    run_id: &str,
    status: &mut crate::types::RunStatus,
) -> Result<bool, ReplayError> {
    let flag_path = workspace.cancel_flag_path(run_id);
    if !flag_path.exists() {
        return Ok(false);
    }
    // Flag exists — write Canceled state and clean up the flag.
    status.state = RunState::Canceled;
    status.updated_at = Utc::now();
    workspace.write_run_status(status)?;
    workspace.append_event(&ReplayEvent::new(
        run_id.to_string(),
        EventName::RunFailed,
        status.last_completed_block,
        status.last_completed_hash.clone(),
    ))?;
    // Remove flag after status is written.
    let _ = std::fs::remove_file(&flag_path);
    Ok(true)
}

/// Inner execution logic, called only while the lock is held.
/// The caller (`execute_run`) is responsible for lock release.
///
/// `emit_start` controls whether to write the initial Running status and
/// RunStarted event. When called from resume, `resume_run` already did this.
async fn execute_run_inner(
    workspace: &Workspace,
    manifest: &RunManifest,
    status: &mut crate::types::RunStatus,
    emit_start: bool,
    heartbeat: &LockHeartbeat,
) -> Result<RunSummary, ReplayError> {
    let run_id = &manifest.run_id;

    // Write initial Running status and emit start event (skipped on resume,
    // since resume_run already did this).
    if emit_start {
        status.state = RunState::Running;
        status.started_at = Some(Utc::now());
        status.updated_at = Utc::now();
        workspace.write_run_status(status)?;

        workspace.append_event(&ReplayEvent::new(
            run_id.clone(),
            EventName::RunStarted,
            None,
            None,
        ))?;
    }

    // 2. Read checkpoint metadata.
    let checkpoint_meta = workspace.read_checkpoint_meta(&manifest.checkpoint_id)?;

    // 3. Create per-run DB copy from the checkpoint base.
    let run_db_path = workspace.run_db_dir(run_id);

    // Only create the checkpoint copy if the run DB doesn't already exist
    // (it may already exist from a previous attempt when resuming).
    let run_db_has_data = run_db_path.exists()
        && std::fs::read_dir(&run_db_path)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);

    if !run_db_has_data {
        let checkpoint_store =
            Store::new_read_only(&checkpoint_meta.checkpoint_db_path, EngineType::RocksDB)
                .map_err(|e| {
                    ReplayError::CheckpointFailed(format!("failed to open checkpoint store: {e}"))
                })?;

        // Remove the empty db dir so RocksDB checkpoint can create it fresh.
        if run_db_path.exists() {
            std::fs::remove_dir(&run_db_path)?;
        }

        checkpoint_store
            .create_checkpoint(&run_db_path)
            .map_err(|e| {
                ReplayError::CheckpointFailed(format!("failed to create per-run DB copy: {e}"))
            })?;

        // Drop checkpoint store immediately — we no longer need it.
        drop(checkpoint_store);
    }

    // 4. Path guard: ensure run DB doesn't collide with live or checkpoint DB.
    validate_path_isolation(
        &run_db_path,
        &checkpoint_meta.datadir,
        &checkpoint_meta.checkpoint_db_path,
    )?;

    // 5. Open the per-run DB for execution (all writes go here).
    let mut run_store = Store::new(&run_db_path, EngineType::RocksDB)
        .map_err(|e| ReplayError::Internal(format!("failed to open per-run store: {e}")))?;
    run_store.load_chain_config().await.map_err(|e| {
        ReplayError::Internal(format!(
            "failed to load chain config from per-run store: {e}"
        ))
    })?;
    run_store.load_initial_state().await.map_err(|e| {
        ReplayError::Internal(format!(
            "failed to load initial state from per-run store: {e}"
        ))
    })?;
    let blockchain = Blockchain::default_with_store(run_store);

    // 6. Open the live store read-only to fetch block data (blocks after the
    //    anchor only exist in the live node's DB, not the checkpoint copy).
    let live_store =
        Store::new_read_only(&checkpoint_meta.datadir, EngineType::RocksDB).map_err(|e| {
            ReplayError::Internal(format!("failed to open live store for block data: {e}"))
        })?;
    live_store
        .load_initial_state()
        .await
        .map_err(|e| ReplayError::Internal(format!("failed to load live store state: {e}")))?;

    // 7. Determine resume point (skip blocks already processed in a previous run).
    let start_idx = match status.last_completed_block {
        Some(last) => ((last - manifest.start_number) + 1) as usize,
        None => 0,
    };

    // 8. Execute blocks sequentially.
    let run_start = Instant::now();
    let total_blocks = manifest.canonical_hashes.len() as u64;
    let mut executed_blocks = start_idx as u64;

    for (idx, hash_hex) in manifest.canonical_hashes.iter().enumerate().skip(start_idx) {
        let block_number = manifest.start_number + idx as u64;

        // Ensure the background heartbeat still owns and refreshes the lock.
        heartbeat.check()?;

        // Cooperative cancellation: check for cancel.flag placed by cancel_run.
        // check_cancel_flag writes Canceled to status.json and removes the flag
        // atomically (no TOCTOU — the flag file is only created by cancel_run,
        // and only consumed here by the executor that holds the lock).
        if check_cancel_flag(workspace, run_id, status)? {
            return Err(ReplayError::RunCanceled(run_id.clone()));
        }

        let block_hash = BlockHash::from_str(hash_hex)
            .map_err(|_| ReplayError::InvalidArgument(format!("invalid block hash: {hash_hex}")))?;

        workspace.append_event(&ReplayEvent::new(
            run_id.clone(),
            EventName::BlockStarted,
            Some(block_number),
            Some(hash_hex.clone()),
        ))?;

        // Fetch full block from the live store.
        let block = live_store
            .get_block_by_hash(block_hash)
            .await
            .map_err(|e| ReplayError::BlockFailed {
                block_number,
                reason: format!("failed to fetch block: {e}"),
            })?
            .ok_or_else(|| ReplayError::BlockFailed {
                block_number,
                reason: format!("block {hash_hex} not found in live store"),
            })?;

        // Execute block against the per-run DB state.
        if let Err(e) = blockchain.add_block_pipeline(block) {
            heartbeat.check()?;
            // Check cancellation before writing Failed.
            if check_cancel_flag(workspace, run_id, status)? {
                return Err(ReplayError::RunCanceled(run_id.clone()));
            }
            status.state = RunState::Failed;
            status.error_code = Some("execution/block_failed".to_string());
            status.error_message = Some(format!("block {block_number}: {e}"));
            status.updated_at = Utc::now();
            workspace.write_run_status(status)?;
            workspace.append_event(&ReplayEvent::new(
                run_id.clone(),
                EventName::RunFailed,
                Some(block_number),
                Some(hash_hex.clone()),
            ))?;
            return Err(ReplayError::BlockFailed {
                block_number,
                reason: e.to_string(),
            });
        }

        heartbeat.check()?;
        // Check cancellation before writing progress.
        if check_cancel_flag(workspace, run_id, status)? {
            return Err(ReplayError::RunCanceled(run_id.clone()));
        }

        workspace.append_event(&ReplayEvent::new(
            run_id.clone(),
            EventName::BlockExecuted,
            Some(block_number),
            Some(hash_hex.clone()),
        ))?;

        executed_blocks += 1;
        status.last_completed_block = Some(block_number);
        status.last_completed_hash = Some(hash_hex.clone());
        status.updated_at = Utc::now();
        workspace.write_run_status(status)?;
    }

    heartbeat.check()?;
    // 9. Final cancellation check before marking complete.
    if check_cancel_flag(workspace, run_id, status)? {
        return Err(ReplayError::RunCanceled(run_id.clone()));
    }

    // 10. Mark complete, write summary.
    let duration_ms = run_start.elapsed().as_millis() as u64;

    status.state = RunState::Completed;
    status.updated_at = Utc::now();
    workspace.write_run_status(status)?;

    let summary = RunSummary {
        run_id: run_id.clone(),
        state: RunState::Completed,
        total_blocks,
        executed_blocks,
        duration_ms,
        avg_block_ms: if executed_blocks > 0 {
            duration_ms as f64 / executed_blocks as f64
        } else {
            0.0
        },
        mismatch_count: 0,
        final_head_number: status.last_completed_block,
        final_head_hash: status.last_completed_hash.clone(),
    };
    workspace.write_run_summary(&summary)?;

    workspace.append_event(&ReplayEvent::new(
        run_id.clone(),
        EventName::RunCompleted,
        status.last_completed_block,
        status.last_completed_hash.clone(),
    ))?;

    Ok(summary)
}
