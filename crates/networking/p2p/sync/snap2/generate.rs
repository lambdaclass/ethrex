//! Rebuilding the tries from the snap/2 flat state.
//!
//! From devp2p `caps/snap.md`, "Synchronization algorithm":
//!
//! > Once the flat state is consistent with the latest pivot, reconstruct all
//! > tries locally and verify the resulting root against the last header.
//!
//! This is the only place a snap/2 sync checks its work. snap/1 gets the check
//! for free because healing walks down from the pivot's root and cannot finish
//! unless the trie reaches it; snap/2 removes healing, so the root comparison
//! here is what stands between a corrupt download and a corrupt chain.
//!
//! Each account is written with the root its storage trie actually hashes to,
//! not the one served with it. The served roots come from whichever pivot
//! answered that account's range and disagree with the slots on disk as soon as
//! the pivot moves.

use ethrex_common::{
    H256,
    constants::EMPTY_TRIE_HASH,
    types::{AccountState, BlockHeader},
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{Store, store::StorageUpdates};
use ethrex_trie::trie_sorted::trie_from_sorted_accounts_wrap;
use tracing::{debug, info};

use crate::sync::{SyncError, snap2::FlatState};

/// Rebuild every trie from `flat` and check the resulting state root against
/// `pivot`.
///
/// A mismatch means the flat state does not describe the pivot: a peer served
/// bad data, an access list was applied out of order or against the wrong fork,
/// or a diff was applied to a key the download had not reached. None of those
/// are attributable here, so the caller has to discard and resync.
pub async fn reconstruct_and_verify(
    store: &Store,
    flat: &FlatState,
    pivot: &BlockHeader,
) -> Result<(), SyncError> {
    let storage_roots = build_storage_tries(store, flat).await?;
    let state_root = build_state_trie(store, flat, &storage_roots)?;

    if state_root != pivot.state_root {
        return Err(SyncError::StateRootMismatch(pivot.state_root, state_root));
    }
    info!(
        block = pivot.number,
        root = %state_root,
        "snap/2 reconstruction matches the pivot state root"
    );
    Ok(())
}

/// A contract with at least this many slots is built with the bulk sorted-trie
/// builder rather than slot by slot.
///
/// The builder constructs bottom-up from sorted leaves instead of descending
/// from the root once per slot, but it spawns its own writer pool per call. For
/// a contract with a handful of slots that pool costs more than the descents it
/// saves, and most contracts have a handful of slots.
const BULK_STORAGE_TRIE_SLOTS: usize = 1024;

/// Storage trie nodes buffered before a write transaction is opened.
///
/// Writing each contract as it is built opens one transaction per contract,
/// which on a full state is the dominant cost of this pass.
const STORAGE_WRITE_BATCH_NODES: usize = 20_000;

/// Build one storage trie per account holding slots, returning what each hashes
/// to.
///
/// Single-threaded across accounts, unlike the snap/1 storage insert which fans
/// out across a thread pool.
async fn build_storage_tries(
    store: &Store,
    flat: &FlatState,
) -> Result<std::collections::BTreeMap<H256, H256>, SyncError> {
    let mut storage_roots = std::collections::BTreeMap::new();
    let mut pending: StorageUpdates = Vec::new();
    let mut pending_nodes = 0usize;
    let mut contracts = 0u64;

    // The account stream names every account; only those with slots get a trie.
    for entry in flat.iter_accounts() {
        let (account_hash, _) = entry?;
        let mut slots = flat.iter_slots(account_hash);

        // Take enough slots to tell the two builders apart without materializing
        // a large contract, which is exactly the case the bulk path exists for.
        let head: Vec<(H256, Vec<u8>)> = slots.by_ref().take(BULK_STORAGE_TRIE_SLOTS).collect();
        if head.is_empty() {
            continue;
        }

        let trie = store.open_direct_storage_trie(account_hash, *EMPTY_TRIE_HASH)?;
        let storage_root = if head.len() < BULK_STORAGE_TRIE_SLOTS {
            let mut trie = trie;
            for (slot_hash, value) in head {
                trie.insert(slot_hash.0.to_vec(), value)?;
            }
            let (storage_root, changes) = trie.collect_changes_since_last_hash(&NativeCrypto);
            pending_nodes += changes.len();
            pending.push((account_hash, changes));
            if pending_nodes >= STORAGE_WRITE_BATCH_NODES {
                store
                    .write_storage_trie_nodes_batch(std::mem::take(&mut pending))
                    .await?;
                pending_nodes = 0;
            }
            storage_root
        } else {
            // The bulk builder writes through the trie's own db, so these nodes
            // never join the batch above.
            let mut sorted = head.into_iter().chain(slots);
            trie_from_sorted_accounts_wrap(trie.db(), &mut sorted)
                .map_err(SyncError::TrieGenerationError)?
        };

        storage_roots.insert(account_hash, storage_root);
        contracts += 1;
    }

    if !pending.is_empty() {
        store.write_storage_trie_nodes_batch(pending).await?;
    }

    debug!("snap/2 reconstruction built {contracts} storage tries");
    Ok(storage_roots)
}

/// Build the account trie, writing each contract's reconstructed storage root
/// in place of the served one.
fn build_state_trie(
    store: &Store,
    flat: &FlatState,
    storage_roots: &std::collections::BTreeMap<H256, H256>,
) -> Result<H256, SyncError> {
    let trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
    let mut failure: Option<SyncError> = None;

    // `trie_from_sorted_accounts_wrap` consumes an infallible stream, so a
    // decode or iteration error is parked here and surfaced afterwards. The
    // account it failed on is skipped, which changes the root, so the error
    // must be checked before the root is trusted.
    let mut accounts = flat.iter_accounts().filter_map(|entry| {
        let (account_hash, encoded) = match entry {
            Ok(entry) => entry,
            Err(err) => {
                failure.get_or_insert(err);
                return None;
            }
        };
        let Some(root) = storage_roots.get(&account_hash) else {
            return Some((account_hash, encoded));
        };
        match AccountState::decode(&encoded) {
            Ok(mut account) => {
                account.storage_root = *root;
                Some((account_hash, account.encode_to_vec()))
            }
            Err(err) => {
                failure.get_or_insert(SyncError::Rlp(err));
                None
            }
        }
    });

    let state_root = trie_from_sorted_accounts_wrap(trie.db(), &mut accounts)
        .map_err(SyncError::TrieGenerationError)?;
    drop(accounts);

    match failure {
        Some(err) => Err(err),
        None => Ok(state_root),
    }
}
