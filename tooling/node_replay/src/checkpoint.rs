//! Checkpoint creation and management.

use crate::errors::ReplayError;
use crate::types::CheckpointMeta;
use crate::workspace::Workspace;
use chrono::Utc;
use ethrex_common::types::BlockHash;
use ethrex_storage::{EngineType, Store};
use std::path::Path;
use uuid::Uuid;

const MAX_ANCHOR_BACKTRACK: u64 = 16_384;

/// Create a checkpoint from a live node's datadir.
///
/// If a checkpoint with the same label and datadir already exists, it is
/// returned immediately (idempotent).
pub async fn create_checkpoint(
    workspace: &Workspace,
    datadir: &Path,
    label: &str,
) -> Result<CheckpointMeta, ReplayError> {
    // 1. Idempotency: return existing checkpoint if label + datadir match.
    if let Some(existing) = workspace.find_checkpoint_by_label(label)?
        && existing.datadir == datadir
    {
        return Ok(existing);
    }

    // 2. Open the store at the live node's datadir.
    let store = open_store(datadir).await?;

    // 3. Get the latest persisted block number.
    let latest_number = store
        .get_latest_block_number()
        .await
        .map_err(|e| ReplayError::CheckpointFailed(format!("failed to get latest block: {e}")))?;

    // 4. Pick an executable anchor whose state root exists in the DB.
    let (anchor_number, anchor_hash) = select_executable_anchor(&store, latest_number)?;

    // 5. Generate a unique checkpoint ID and create directory structure.
    let checkpoint_id = Uuid::new_v4().to_string();
    workspace.create_checkpoint_dirs(&checkpoint_id)?;

    // 6. Create RocksDB checkpoint (hard-linked copy of the DB).
    let checkpoint_db_path = workspace.checkpoint_db_dir(&checkpoint_id);
    if checkpoint_db_path.exists() {
        std::fs::remove_dir_all(&checkpoint_db_path)?;
    }
    store
        .create_checkpoint(&checkpoint_db_path)
        .map_err(|e| ReplayError::CheckpointFailed(format!("RocksDB checkpoint failed: {e}")))?;

    // 7. Read chain config for chain_id.
    let chain_config = store.get_chain_config();

    // 8. Build and persist metadata.
    let meta = CheckpointMeta {
        checkpoint_id,
        datadir: datadir.to_path_buf(),
        checkpoint_db_path,
        anchor_number,
        anchor_hash: format!("{anchor_hash:#x}"),
        created_at: Utc::now(),
        ethrex_commit: option_env!("VERGEN_GIT_SHA").map(String::from),
        chain_id: chain_config.chain_id,
        network: "unknown".to_string(),
        label: label.to_string(),
    };
    workspace.write_checkpoint_meta(&meta)?;

    Ok(meta)
}

/// List all checkpoints in the workspace, ordered by creation time.
pub fn list_checkpoints(workspace: &Workspace) -> Result<Vec<CheckpointMeta>, ReplayError> {
    workspace.list_checkpoints()
}

/// Open a RocksDB-backed Store from a live node's datadir.
async fn open_store(datadir: &Path) -> Result<Store, ReplayError> {
    let mut store = Store::new_read_only(datadir, EngineType::RocksDB)
        .map_err(|e| ReplayError::CheckpointFailed(format!("failed to open store: {e}")))?;

    store
        .load_chain_config()
        .await
        .map_err(|e| ReplayError::CheckpointFailed(format!("failed to load chain config: {e}")))?;
    store
        .load_initial_state()
        .await
        .map_err(|e| ReplayError::CheckpointFailed(format!("failed to load initial state: {e}")))?;

    Ok(store)
}

fn select_executable_anchor(
    store: &Store,
    latest_number: u64,
) -> Result<(u64, BlockHash), ReplayError> {
    let mut candidate = latest_number;

    for _ in 0..=MAX_ANCHOR_BACKTRACK {
        let canonical_hash = match store
            .get_canonical_block_hash_sync(candidate)
            .map_err(|e| {
                ReplayError::CheckpointFailed(format!(
                    "failed to get canonical hash for block {candidate}: {e}"
                ))
            })? {
            Some(hash) => hash,
            None => {
                if candidate == 0 {
                    break;
                }
                candidate -= 1;
                continue;
            }
        };

        let header = match store.get_block_header(candidate).map_err(|e| {
            ReplayError::CheckpointFailed(format!(
                "failed to get canonical header for block {candidate}: {e}"
            ))
        })? {
            Some(header) => header,
            None => {
                if candidate == 0 {
                    break;
                }
                candidate -= 1;
                continue;
            }
        };

        let has_state_root = store.has_state_root(header.state_root).map_err(|e| {
            ReplayError::CheckpointFailed(format!(
                "failed to verify state root for block {candidate}: {e}"
            ))
        })?;
        if has_state_root {
            return Ok((candidate, canonical_hash));
        }

        if candidate == 0 {
            break;
        }
        candidate -= 1;
    }

    Err(ReplayError::CheckpointFailed(format!(
        "no executable anchor found within {MAX_ANCHOR_BACKTRACK} blocks from latest block {latest_number}"
    )))
}
