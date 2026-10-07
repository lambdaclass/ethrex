//! Patching the snap/2 flat state with a block's access list.
//!
//! From devp2p `caps/snap.md`, "Synchronization algorithm":
//!
//! > BAL application: synchronization continuously fetches BALs of all blocks
//! > starting at the initial pivot block, and applies their state diff to the
//! > state. By doing this, the final state is made consistent with all state
//! > modifications performed since the sync started.
//!
//! Only the post-block value of each entry matters, so the last change
//! recorded for a key wins and the intermediate ones are ignored.
//!
//! Each account's `storage_root` is left as served. It is wrong in general —
//! that is the whole reason the flat state needs patching — and it is
//! recomputed from the reconstructed storage tries, so writing anything here
//! would be discarded.
//!
//! # Correctness premise
//!
//! Carried from go-ethereum's `eth/protocols/snap/bal_apply.go`: correctness
//! rests on the access list enumerating every storage change as an individual
//! slot write. This holds post [EIP-6780]: pre-existing contracts can no longer
//! be destructed, so storage only changes via SSTOREs, all of which are
//! recorded. Networks with the legacy SELFDESTRUCT break this premise:
//! wholesale storage wipes carry no per-slot writes, leaving already-downloaded
//! slots stale.
//!
//! [EIP-6780]: https://eips.ethereum.org/EIPS/eip-6780

use ethrex_common::{
    H256,
    constants::EMPTY_KECCAK_HASH,
    types::{Code, block_access_list::BlockAccessList},
    utils::keccak,
};
use ethrex_storage::{Store, hash_address, hash_key};

use crate::sync::{
    SyncError,
    bal_healing::store_code_sync,
    snap2::{DownloadCursor, FlatState},
};

/// What one access list changed in the flat state. Entries the download has not
/// reached are counted as skipped rather than written.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlatApplyStats {
    pub accounts_written: u64,
    pub accounts_deleted: u64,
    pub slots_written: u64,
    pub slots_deleted: u64,
    pub accounts_skipped: u64,
    pub slots_skipped: u64,
}

/// Apply one block's access list to the flat state.
///
/// A key the download has not reached yet is skipped: the range response that
/// eventually covers it is served at a later root and already carries this
/// change, so patching it now would be undone, and patching it after the
/// response lands would apply it twice.
pub fn apply_bal_flat(
    flat: &FlatState,
    cursor: &DownloadCursor,
    store: &Store,
    bal: &BlockAccessList,
) -> Result<FlatApplyStats, SyncError> {
    let mut stats = FlatApplyStats::default();

    for changes in bal.accounts() {
        let account_hash = H256::from_slice(&hash_address(&changes.address));

        for slot in &changes.storage_changes {
            let Some(last) = slot.slot_changes.last() else {
                continue;
            };
            let slot_hash = H256::from_slice(&hash_key(&H256::from(slot.slot.to_big_endian())));
            if !cursor.is_storage_fetched(account_hash, slot_hash) {
                stats.slots_skipped += 1;
                continue;
            }
            if last.post_value.is_zero() {
                flat.delete_slot(account_hash, slot_hash)?;
                stats.slots_deleted += 1;
            } else {
                flat.put_slot(account_hash, slot_hash, last.post_value)?;
                stats.slots_written += 1;
            }
        }

        // `storage_reads` record slots the block read without changing, so they
        // carry no post-value and need no patch.

        if !cursor.is_account_fetched(account_hash) {
            stats.accounts_skipped += 1;
            continue;
        }

        let existing = flat.get_account(account_hash)?;
        let is_new = existing.is_none();
        let mut account = existing.unwrap_or_default();

        if let Some(last) = changes.balance_changes.last() {
            account.balance = last.post_balance;
        }
        if let Some(last) = changes.nonce_changes.last() {
            account.nonce = last.post_nonce;
        }
        if let Some(last) = changes.code_changes.last() {
            if last.new_code.is_empty() {
                // Code removal, or an EIP-7702 delegation being cleared.
                account.code_hash = *EMPTY_KECCAK_HASH;
            } else {
                let code_hash = keccak(&last.new_code);
                store_code_sync(
                    store,
                    Code::from_bytecode_unchecked(last.new_code.clone(), code_hash),
                )?;
                account.code_hash = code_hash;
            }
        }

        // EIP-161 emptiness, which the storage root is not part of: an account
        // with slots but no balance, nonce or code does not exist.
        let is_empty = account.balance.is_zero()
            && account.nonce == 0
            && account.code_hash == *EMPTY_KECCAK_HASH;

        match (is_empty, is_new) {
            // Created and drained within the block, or a net-zero change over
            // it. There was no leaf before and there must be none after.
            (true, true) => {}
            // An account that existed and was fully drained. Removing it keeps
            // the trie build from picking up an empty leaf.
            (true, false) => {
                flat.delete_account(account_hash)?;
                stats.accounts_deleted += 1;
            }
            _ => {
                flat.put_account(account_hash, &account)?;
                stats.accounts_written += 1;
            }
        }
    }

    Ok(stats)
}
