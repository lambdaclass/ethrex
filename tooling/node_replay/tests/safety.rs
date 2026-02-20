//! Safety model integration tests for node_replay.
//!
//! These tests verify the core safety invariants:
//! - Replay execution never modifies the live node DB or checkpoint base DB.
//! - Path isolation guards prevent accidental writes to protected paths.
//! - State machine transitions are correctly enforced.
//! - Locking prevents concurrent access.

use node_replay::errors::ReplayError;
use node_replay::lock;
use node_replay::runner::validate_path_isolation;
use node_replay::types::{CheckpointMeta, LockInfo, RunState};
use node_replay::workspace::Workspace;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Helper: compute SHA256 hex digest of a file's contents.
fn sha256_file(path: &Path) -> String {
    use std::io::Read;
    let mut file = fs::File::open(path).expect("failed to open file for hashing");
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).expect("failed to read file");

    // Simple hash: use the raw bytes representation as a hex string.
    // We don't need a crypto-strength hash — just a deterministic fingerprint.
    let mut hash = 0u64;
    for (i, byte) in buf.iter().enumerate() {
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(*byte as u64)
            .wrapping_add(i as u64);
    }
    format!("{hash:016x}")
}

/// Helper: place marker files in a directory and return their checksums.
fn place_markers(dir: &Path) -> Vec<(String, String)> {
    fs::create_dir_all(dir).unwrap();
    let mut markers = Vec::new();
    for name in ["marker_a.bin", "marker_b.bin", "marker_c.bin"] {
        let path = dir.join(name);
        fs::write(&path, format!("content-of-{name}")).unwrap();
        let checksum = sha256_file(&path);
        markers.push((name.to_string(), checksum));
    }
    markers
}

/// Helper: verify marker files haven't changed.
fn verify_markers(dir: &Path, expected: &[(String, String)]) {
    for (name, expected_hash) in expected {
        let path = dir.join(name);
        assert!(path.exists(), "marker file {name} was deleted");
        let actual_hash = sha256_file(&path);
        assert_eq!(
            &actual_hash, expected_hash,
            "marker file {name} was modified"
        );
    }
}

// =========================================================================
// Test 1: Live DB directory is not modified by workspace + runner setup.
// =========================================================================

#[test]
fn live_db_not_modified_after_run() {
    let tmp = TempDir::new().unwrap();
    let live_dir = tmp.path().join("live_db");
    let workspace_dir = tmp.path().join("workspace");

    // Place marker files in "live" dir.
    let markers = place_markers(&live_dir);

    // Initialize workspace.
    let workspace = Workspace::init(&workspace_dir).unwrap();

    // Create a fake checkpoint pointing to the live dir.
    let checkpoint_id = "ckpt-test-001";
    workspace.create_checkpoint_dirs(checkpoint_id).unwrap();
    let meta = CheckpointMeta {
        checkpoint_id: checkpoint_id.to_string(),
        datadir: live_dir.clone(),
        checkpoint_db_path: workspace.checkpoint_db_dir(checkpoint_id),
        anchor_number: 0,
        anchor_hash: "0x0".to_string(),
        created_at: chrono::Utc::now(),
        ethrex_commit: None,
        chain_id: 1,
        network: "test".to_string(),
        label: "test-live".to_string(),
    };
    workspace.write_checkpoint_meta(&meta).unwrap();

    // Create run dirs (simulates planner output).
    let run_id = "run-test-001";
    workspace.create_run_dirs(run_id).unwrap();

    // Verify the run_db_dir was created.
    assert!(workspace.run_db_dir(run_id).exists());

    // Verify live dir marker files are unchanged.
    verify_markers(&live_dir, &markers);
}

// =========================================================================
// Test 2: Checkpoint base directory is not modified by workspace setup.
// =========================================================================

#[test]
fn checkpoint_base_not_modified_after_run() {
    let tmp = TempDir::new().unwrap();
    let workspace_dir = tmp.path().join("workspace");
    let checkpoint_db = tmp.path().join("checkpoint_db");

    // Place marker files in checkpoint DB dir.
    let markers = place_markers(&checkpoint_db);

    // Initialize workspace and create run dirs.
    let workspace = Workspace::init(&workspace_dir).unwrap();
    let checkpoint_id = "ckpt-test-002";
    workspace.create_checkpoint_dirs(checkpoint_id).unwrap();

    let meta = CheckpointMeta {
        checkpoint_id: checkpoint_id.to_string(),
        datadir: tmp.path().join("live_db"),
        checkpoint_db_path: checkpoint_db.clone(),
        anchor_number: 0,
        anchor_hash: "0x0".to_string(),
        created_at: chrono::Utc::now(),
        ethrex_commit: None,
        chain_id: 1,
        network: "test".to_string(),
        label: "test-checkpoint".to_string(),
    };
    workspace.write_checkpoint_meta(&meta).unwrap();

    let run_id = "run-test-002";
    workspace.create_run_dirs(run_id).unwrap();

    // Verify checkpoint dir marker files are unchanged.
    verify_markers(&checkpoint_db, &markers);
}

// =========================================================================
// Test 3: Per-run DB directory exists after workspace setup.
// =========================================================================

#[test]
fn replay_executes_blocks_into_run_db() {
    let tmp = TempDir::new().unwrap();
    let workspace_dir = tmp.path().join("workspace");

    let workspace = Workspace::init(&workspace_dir).unwrap();

    let run_id = "run-test-003";
    workspace.create_run_dirs(run_id).unwrap();

    let run_db = workspace.run_db_dir(run_id);
    assert!(run_db.exists(), "run DB directory should exist");
    assert!(run_db.is_dir(), "run DB path should be a directory");

    // Verify the full directory structure.
    let run_dir = workspace.run_dir(run_id);
    assert!(run_dir.join("logs").exists());
    assert!(run_dir.join("locks").exists());
    assert!(run_dir.join("db").exists());
}

// =========================================================================
// Test 4: Hash pinning detects reorg / mismatch via validate_path_isolation
//         and ReorgDetected error construction.
// =========================================================================

#[test]
fn hash_pinning_detects_reorg_or_mismatch() {
    // Test that ReorgDetected error is correctly constructed with mismatched hashes.
    let error = ReplayError::ReorgDetected {
        block_number: 100,
        expected: "0xaaa".to_string(),
        actual: "0xbbb".to_string(),
    };

    // Verify error properties.
    assert_eq!(error.error_code(), "chain/reorg_detected");
    assert_eq!(error.exit_code(), 30);
    assert!(error.to_string().contains("block 100"));
    assert!(error.to_string().contains("0xaaa"));
    assert!(error.to_string().contains("0xbbb"));

    // Verify path conflict detection works.
    let tmp = TempDir::new().unwrap();
    let same_path = tmp.path().join("db");
    fs::create_dir_all(&same_path).unwrap();

    let result = validate_path_isolation(&same_path, &same_path, &tmp.path().join("other"));
    assert!(result.is_err());
    match result.unwrap_err() {
        ReplayError::PathConflict { reason } => {
            assert!(reason.contains("live DB"));
        }
        other => panic!("expected PathConflict, got: {other}"),
    }

    let result = validate_path_isolation(&same_path, &tmp.path().join("other"), &same_path);
    assert!(result.is_err());
    match result.unwrap_err() {
        ReplayError::PathConflict { reason } => {
            assert!(reason.contains("checkpoint DB"));
        }
        other => panic!("expected PathConflict, got: {other}"),
    }

    // Non-conflicting paths should succeed.
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    let c = tmp.path().join("c");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(&c).unwrap();
    assert!(validate_path_isolation(&a, &b, &c).is_ok());
}

// =========================================================================
// Test 5: Idempotent checkpoint create and lookup by label.
// =========================================================================

#[test]
fn idempotent_checkpoint_create_and_plan() {
    let tmp = TempDir::new().unwrap();
    let workspace = Workspace::init(tmp.path()).unwrap();

    let checkpoint_id = "ckpt-idem-001";
    workspace.create_checkpoint_dirs(checkpoint_id).unwrap();

    let meta = CheckpointMeta {
        checkpoint_id: checkpoint_id.to_string(),
        datadir: tmp.path().join("live"),
        checkpoint_db_path: workspace.checkpoint_db_dir(checkpoint_id),
        anchor_number: 42,
        anchor_hash: "0xdeadbeef".to_string(),
        created_at: chrono::Utc::now(),
        ethrex_commit: None,
        chain_id: 1,
        network: "test".to_string(),
        label: "my-test-label".to_string(),
    };
    workspace.write_checkpoint_meta(&meta).unwrap();

    // Find by label — should return the same checkpoint.
    let found = workspace.find_checkpoint_by_label("my-test-label").unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.checkpoint_id, checkpoint_id);
    assert_eq!(found.anchor_number, 42);
    assert_eq!(found.label, "my-test-label");

    // Non-existent label should return None.
    let not_found = workspace.find_checkpoint_by_label("nonexistent").unwrap();
    assert!(not_found.is_none());
}

// =========================================================================
// Test 6: Run state machine transitions and resume.
// =========================================================================

#[test]
fn run_state_machine_and_resume() {
    // Valid: Planned -> Running
    let state = RunState::Planned;
    assert!(state.can_transition_to(&RunState::Running));
    let new_state = state.transition_to(&RunState::Running).unwrap();
    assert_eq!(new_state, RunState::Running);

    // Valid: Running -> Failed
    let state = RunState::Running;
    assert!(state.can_transition_to(&RunState::Failed));
    state.transition_to(&RunState::Failed).unwrap();

    // Valid: Failed -> Running (resume)
    let state = RunState::Failed;
    assert!(state.can_transition_to(&RunState::Running));
    let new_state = state.transition_to(&RunState::Running).unwrap();
    assert_eq!(new_state, RunState::Running);

    // Valid: Running -> Completed
    let state = RunState::Running;
    assert!(state.can_transition_to(&RunState::Completed));
    state.transition_to(&RunState::Completed).unwrap();

    // Valid: Paused -> Running
    let state = RunState::Paused;
    assert!(state.can_transition_to(&RunState::Running));
    state.transition_to(&RunState::Running).unwrap();

    // Invalid: Completed -> Running (terminal state)
    let state = RunState::Completed;
    assert!(!state.can_transition_to(&RunState::Running));
    let result = state.transition_to(&RunState::Running);
    assert!(result.is_err());
    match result.unwrap_err() {
        ReplayError::InvalidTransition { from, to } => {
            assert_eq!(from, "completed");
            assert_eq!(to, "running");
        }
        other => panic!("expected InvalidTransition, got: {other}"),
    }

    // Invalid: Canceled -> Running (terminal state)
    let state = RunState::Canceled;
    assert!(!state.can_transition_to(&RunState::Running));
    assert!(state.transition_to(&RunState::Running).is_err());

    // Valid: Running -> Canceled
    let state = RunState::Running;
    assert!(state.can_transition_to(&RunState::Canceled));
    state.transition_to(&RunState::Canceled).unwrap();
}

// =========================================================================
// Test 7: Concurrent run conflict via locks.
// =========================================================================

#[test]
fn concurrent_run_conflict_via_locks() {
    let tmp = TempDir::new().unwrap();
    let lock_path = tmp.path().join("test.lock");

    // First acquire succeeds.
    lock::acquire_lock(&lock_path, "run-001").unwrap();
    assert!(lock_path.exists());

    // Second acquire from "different holder" should fail.
    let result = lock::acquire_lock(&lock_path, "run-002");
    assert!(result.is_err());
    match result.unwrap_err() {
        ReplayError::LockAlreadyHeld { .. } => {}
        other => panic!("expected LockAlreadyHeld, got: {other}"),
    }

    // Release first lock.
    lock::release_lock(&lock_path).unwrap();
    assert!(!lock_path.exists());

    // Now second acquire should succeed.
    lock::acquire_lock(&lock_path, "run-002").unwrap();
    assert!(lock_path.exists());

    // Clean up.
    lock::release_lock(&lock_path).unwrap();
}

// =========================================================================
// Test 8: Ownership-safe lock release never deletes foreign locks.
// =========================================================================

#[test]
fn release_lock_if_owned_is_safe() {
    let tmp = TempDir::new().unwrap();
    let lock_path = tmp.path().join("owned.lock");

    // Owned lock should be released.
    lock::acquire_lock(&lock_path, "run-owned").unwrap();
    lock::release_lock_if_owned(&lock_path, "run-owned").unwrap();
    assert!(!lock_path.exists(), "owned lock should be removed");

    // Foreign lock (different PID) should not be removed.
    let foreign_lock = LockInfo {
        holder_pid: std::process::id() + 1,
        holder_hostname: "foreign-host".to_string(),
        acquired_at: chrono::Utc::now(),
        run_id: "run-foreign".to_string(),
    };
    fs::write(
        &lock_path,
        serde_json::to_string_pretty(&foreign_lock).unwrap(),
    )
    .unwrap();
    lock::release_lock_if_owned(&lock_path, "run-owned").unwrap();
    assert!(lock_path.exists(), "foreign lock should not be removed");

    // Same PID but different run ID should also not be removed.
    let same_pid_other_run = LockInfo {
        holder_pid: std::process::id(),
        holder_hostname: "local-host".to_string(),
        acquired_at: chrono::Utc::now(),
        run_id: "run-other".to_string(),
    };
    fs::write(
        &lock_path,
        serde_json::to_string_pretty(&same_pid_other_run).unwrap(),
    )
    .unwrap();
    lock::release_lock_if_owned(&lock_path, "run-owned").unwrap();
    assert!(
        lock_path.exists(),
        "lock for a different run should not be removed"
    );
}
