//! Offline profiling module for snap sync
//!
//! Replays the compute-only phases (InsertAccounts, InsertStorages) from
//! previously captured RLP snapshot files, enabling fast iteration on
//! performance optimisation without network I/O.

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ethrex_common::constants::EMPTY_TRIE_HASH;
use ethrex_common::types::AccountState;
use ethrex_common::{H256, U256};
use ethrex_p2p::sync::profile::load_manifest;
use ethrex_p2p::sync::compute_storage_roots;
use ethrex_p2p::sync::SyncError;
use ethrex_p2p::utils::AccountsWithStorage;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use tracing::info;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum ProfileBackend {
    InMemory,
    #[cfg(feature = "rocksdb")]
    RocksDb,
}

impl fmt::Display for ProfileBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfileBackend::InMemory => write!(f, "inmemory"),
            #[cfg(feature = "rocksdb")]
            ProfileBackend::RocksDb => write!(f, "rocksdb"),
        }
    }
}

#[derive(Debug)]
pub struct SnapProfileResult {
    pub insert_accounts_duration: Duration,
    pub insert_storages_duration: Duration,
    pub total_duration: Duration,
    pub computed_state_root: H256,
    pub backend: ProfileBackend,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Replay InsertAccounts + InsertStorages from a captured dataset using
/// an in-memory Store backend. Compatibility wrapper for `run_once_with_opts`.
pub async fn run_once(dataset_root: &Path) -> Result<SnapProfileResult, SyncError> {
    run_once_with_opts(dataset_root, ProfileBackend::InMemory, Path::new(".")).await
}

/// Replay InsertAccounts + InsertStorages from a captured dataset using the
/// specified backend and working directory.
///
/// - `ProfileBackend::InMemory` — all state lives in RAM. Fast but requires
///   enough memory to hold the entire state trie. `db_dir` is ignored.
/// - `ProfileBackend::RocksDb` — state is written to disk via RocksDB.
///   Uses `db_dir` for the database files. Suitable for large datasets.
pub async fn run_once_with_opts(
    dataset_root: &Path,
    backend: ProfileBackend,
    db_dir: &Path,
) -> Result<SnapProfileResult, SyncError> {
    let total_start = Instant::now();

    let manifest = load_manifest(dataset_root)?;

    let engine_type = match backend {
        ProfileBackend::InMemory => EngineType::InMemory,
        #[cfg(feature = "rocksdb")]
        ProfileBackend::RocksDb => EngineType::RocksDB,
    };

    let store = Store::new(db_dir.to_str().unwrap_or("."), engine_type)
        .map_err(|e| SyncError::ProfileError(format!("Failed to create store: {e}")))?;

    info!("Store created with backend: {backend}");

    let acc_dir = dataset_root.join(&manifest.paths.account_state_snapshots_dir);
    let storage_dir = dataset_root.join(&manifest.paths.account_storages_snapshots_dir);

    // -- InsertAccounts phase ------------------------------------------------
    let accounts_start = Instant::now();
    let computed_state_root = insert_accounts_phase(&store, &acc_dir).await?;
    let insert_accounts_duration = accounts_start.elapsed();

    info!(
        "InsertAccounts done in {:.2}s — state root: {computed_state_root:?}",
        insert_accounts_duration.as_secs_f64()
    );

    // -- InsertStorages phase ------------------------------------------------
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
        backend,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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
