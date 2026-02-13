//! Offline profiling module for snap sync
//!
//! Replays the compute-only phases (InsertAccounts, InsertStorages) from
//! previously captured RLP snapshot files, enabling fast iteration on
//! performance optimisation without network I/O.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ethrex_common::constants::EMPTY_TRIE_HASH;
use ethrex_common::types::AccountState;
use ethrex_common::{H256, U256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::SyncError;
use super::snap_sync::compute_storage_roots;
use crate::utils::AccountsWithStorage;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct SnapProfileManifest {
    pub version: u32,
    pub chain_id: u64,
    pub rocksdb_enabled: bool,
    pub pivot: PivotInfo,
    pub post_accounts_insert_state_root: H256,
    pub paths: DatasetPaths,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PivotInfo {
    pub number: u64,
    pub hash: H256,
    pub state_root: H256,
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DatasetPaths {
    pub account_state_snapshots_dir: String,
    pub account_storages_snapshots_dir: String,
}

#[derive(Debug)]
pub struct SnapProfileResult {
    pub insert_accounts_duration: Duration,
    pub insert_storages_duration: Duration,
    pub total_duration: Duration,
    pub computed_state_root: H256,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load and validate a snap profile manifest from `{dataset_root}/manifest.json`.
pub fn load_manifest(dataset_root: &Path) -> Result<SnapProfileManifest, SyncError> {
    let manifest_path = dataset_root.join("manifest.json");
    let contents = std::fs::read_to_string(&manifest_path)
        .map_err(|e| SyncError::ProfileError(format!("Failed to read manifest: {e}")))?;

    let manifest: SnapProfileManifest = serde_json::from_str(&contents)
        .map_err(|e| SyncError::ProfileError(format!("Failed to parse manifest: {e}")))?;

    // Version check
    if manifest.version != 1 {
        return Err(SyncError::ProfileError(format!(
            "Unsupported manifest version: {} (expected 1)",
            manifest.version
        )));
    }

    // Only in-memory backend is supported for profiling
    if manifest.rocksdb_enabled {
        return Err(SyncError::ProfileError(
            "rocksdb_enabled=true is not supported for offline profiling".to_string(),
        ));
    }

    // Validate that snapshot directories exist and are non-empty
    let acc_dir = dataset_root.join(&manifest.paths.account_state_snapshots_dir);
    validate_non_empty_dir(&acc_dir, "account_state_snapshots")?;

    let storage_dir = dataset_root.join(&manifest.paths.account_storages_snapshots_dir);
    validate_non_empty_dir(&storage_dir, "account_storages_snapshots")?;

    Ok(manifest)
}

/// Replay InsertAccounts + InsertStorages from a captured dataset and return
/// timing results.
pub async fn run_once(dataset_root: &Path) -> Result<SnapProfileResult, SyncError> {
    let total_start = Instant::now();

    let manifest = load_manifest(dataset_root)?;

    // Fresh in-memory store
    let store = Store::new(".", EngineType::InMemory)
        .map_err(|e| SyncError::ProfileError(format!("Failed to create in-memory store: {e}")))?;

    let acc_dir = dataset_root.join(&manifest.paths.account_state_snapshots_dir);
    let storage_dir = dataset_root.join(&manifest.paths.account_storages_snapshots_dir);

    // ── InsertAccounts phase ──────────────────────────────────────────────
    let accounts_start = Instant::now();
    let computed_state_root = insert_accounts_phase(&store, &acc_dir).await?;
    let insert_accounts_duration = accounts_start.elapsed();

    info!(
        "InsertAccounts done in {:.2}s — state root: {computed_state_root:?}",
        insert_accounts_duration.as_secs_f64()
    );

    // ── InsertStorages phase ─────────────────────────────────────────────
    let storages_start = Instant::now();
    insert_storages_phase(&store, &storage_dir).await?;
    let insert_storages_duration = storages_start.elapsed();

    info!(
        "InsertStorages done in {:.2}s",
        insert_storages_duration.as_secs_f64()
    );

    Ok(SnapProfileResult {
        insert_accounts_duration,
        insert_storages_duration,
        total_duration: total_start.elapsed(),
        computed_state_root,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn validate_non_empty_dir(path: &Path, label: &str) -> Result<(), SyncError> {
    let entries = std::fs::read_dir(path).map_err(|e| {
        SyncError::ProfileError(format!("{label} directory {path:?} cannot be read: {e}"))
    })?;
    if entries.peekable().peek().is_none() {
        return Err(SyncError::ProfileError(format!(
            "{label} directory {path:?} is empty"
        )));
    }
    Ok(())
}

/// Mirrors the non-rocksdb `insert_accounts` in `snap_sync.rs`, without
/// storage_accounts tracking, code_hash_collector, or remove_dir_all.
async fn insert_accounts_phase(store: &Store, acc_dir: &Path) -> Result<H256, SyncError> {
    let mut computed_state_root = *EMPTY_TRIE_HASH;

    for entry in
        std::fs::read_dir(acc_dir).map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
    {
        let entry =
            entry.map_err(|err| SyncError::SnapshotReadError(acc_dir.to_path_buf(), err))?;
        info!("Reading account file from entry {entry:?}");

        let snapshot_path = entry.path();
        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;

        let account_states_snapshot: Vec<(H256, AccountState)> =
            RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        info!("Inserting accounts into the state trie");

        let store_clone = store.clone();
        let current_state_root: Result<H256, SyncError> =
            tokio::task::spawn_blocking(move || -> Result<H256, SyncError> {
                let mut trie = store_clone.open_direct_state_trie(computed_state_root)?;

                for (account_hash, account) in account_states_snapshot {
                    trie.insert(account_hash.0.to_vec(), account.encode_to_vec())?;
                }
                let current_state_root = trie.hash()?;
                Ok(current_state_root)
            })
            .await?;

        computed_state_root = current_state_root?;
    }

    info!("computed_state_root {computed_state_root}");
    Ok(computed_state_root)
}

/// Mirrors the non-rocksdb `insert_storages` in `snap_sync.rs`, without
/// remove_dir_all.
async fn insert_storages_phase(store: &Store, storage_dir: &Path) -> Result<(), SyncError> {
    for entry in std::fs::read_dir(storage_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?
    {
        let entry = entry
            .map_err(|err| SyncError::SnapshotReadError(storage_dir.to_path_buf(), err))?;
        info!("Reading account storage file from entry {entry:?}");

        let snapshot_path = entry.path();
        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;

        #[expect(clippy::type_complexity)]
        let account_storages_snapshot: Vec<AccountsWithStorage> =
            RLPDecode::decode(&snapshot_contents)
                .map(|all_accounts: Vec<(Vec<H256>, Vec<(H256, U256)>)>| {
                    all_accounts
                        .into_iter()
                        .map(|(accounts, storages)| AccountsWithStorage { accounts, storages })
                        .collect()
                })
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        let store_clone = store.clone();
        info!("Starting compute of account_storages_snapshot");

        let storage_trie_node_changes = tokio::task::spawn_blocking(move || {
            let store: Store = store_clone;

            account_storages_snapshot
                .into_par_iter()
                .flat_map(|account_storages| {
                    let storages: Arc<[_]> = account_storages.storages.into();
                    account_storages
                        .accounts
                        .into_par_iter()
                        .map(move |account| (account, storages.clone()))
                })
                .map(|(account, storages)| compute_storage_roots(store.clone(), account, &storages))
                .collect::<Result<Vec<_>, SyncError>>()
        })
        .await??;

        info!("Writing to db");
        store
            .write_storage_trie_nodes_batch(storage_trie_node_changes)
            .await?;
    }

    Ok(())
}
