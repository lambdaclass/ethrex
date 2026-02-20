//! Replay planning: selecting blocks and producing run manifests.

use crate::errors::ReplayError;
use crate::types::{EventName, Finality, ReplayEvent, ReplayMode, RunManifest, RunStatus};
use crate::workspace::Workspace;
use chrono::Utc;
use ethrex_common::types::BlockHash;
use ethrex_storage::{EngineType, Store};
use std::path::Path;
use uuid::Uuid;

/// Plan a replay run: resolve canonical hashes for the given block range.
///
/// Opens the live node datadir to read canonical block hashes, verifies parent
/// hash continuity for reorg detection, then writes a pinned `RunManifest`.
pub async fn plan_run(
    workspace: &Workspace,
    checkpoint_id: &str,
    blocks: u64,
    _finality: &Finality,
    datadir: &Path,
) -> Result<RunManifest, ReplayError> {
    // 1. Read checkpoint metadata to know the anchor point.
    let checkpoint = workspace.read_checkpoint_meta(checkpoint_id)?;
    let anchor_number = checkpoint.anchor_number;
    let start_number = anchor_number + 1;
    let end_number = anchor_number + blocks;

    // 2. Open the LIVE store (not the checkpoint DB) to read canonical hashes.
    let store = open_live_store(datadir).await?;

    // 3. Resolve and verify canonical hashes for [start_number, end_number].
    let mut canonical_hashes: Vec<String> = Vec::with_capacity(blocks as usize);

    for block_num in start_number..=end_number {
        let hash = store
            .get_canonical_block_hash_sync(block_num)
            .map_err(|e| {
                ReplayError::Internal(format!("failed to get hash for block {block_num}: {e}"))
            })?
            .ok_or_else(|| {
                ReplayError::InvalidArgument(format!(
                    "block {block_num} not found in canonical chain \
                     (chain may not have reached this point yet)"
                ))
            })?;

        // Determine expected parent hash.
        let expected_parent = if block_num == start_number {
            checkpoint.anchor_hash.clone()
        } else {
            canonical_hashes
                .last()
                .cloned()
                .expect("canonical_hashes is non-empty for block_num > start_number")
        };

        verify_parent_hash(&store, block_num, hash, &expected_parent)?;

        canonical_hashes.push(format!("{hash:#x}"));
    }

    let expected_head_hash = canonical_hashes
        .last()
        .cloned()
        .ok_or_else(|| ReplayError::InvalidArgument("no blocks to plan".to_string()))?;

    // 4. Generate run ID and build manifest.
    let run_id = Uuid::new_v4().to_string();
    let manifest = RunManifest {
        run_id: run_id.clone(),
        checkpoint_id: checkpoint_id.to_string(),
        mode: ReplayMode::Isolated,
        start_number,
        end_number,
        canonical_hashes,
        expected_head_hash,
        created_at: Utc::now(),
        idempotency_key: None,
    };

    // 5. Write manifest and initial planned status.
    workspace.create_run_dirs(&run_id)?;
    workspace.write_run_manifest(&manifest)?;
    workspace.write_run_status(&RunStatus::new_planned(run_id.clone()))?;

    // 6. Emit run_planned event.
    workspace.append_event(&ReplayEvent::new(run_id, EventName::RunPlanned, None, None))?;

    Ok(manifest)
}

async fn open_live_store(datadir: &Path) -> Result<Store, ReplayError> {
    let store = Store::new_read_only(datadir, EngineType::RocksDB)
        .map_err(|e| ReplayError::InvalidArgument(format!("failed to open live store: {e}")))?;
    store
        .load_initial_state()
        .await
        .map_err(|e| ReplayError::Internal(format!("failed to load live store state: {e}")))?;
    Ok(store)
}

/// Verify that a block's parent_hash matches the expected value (reorg detection).
pub fn verify_parent_hash(
    store: &Store,
    block_number: u64,
    block_hash: BlockHash,
    expected_parent: &str,
) -> Result<(), ReplayError> {
    let header = store
        .get_block_header_by_hash(block_hash)
        .map_err(|e| {
            ReplayError::Internal(format!(
                "failed to get header for block {block_number}: {e}"
            ))
        })?
        .ok_or_else(|| {
            ReplayError::Internal(format!("header not found for block {block_number}"))
        })?;

    let actual_parent = format!("{:#x}", header.parent_hash);
    if actual_parent != expected_parent {
        return Err(ReplayError::ReorgDetected {
            block_number,
            expected: expected_parent.to_string(),
            actual: actual_parent,
        });
    }
    Ok(())
}
