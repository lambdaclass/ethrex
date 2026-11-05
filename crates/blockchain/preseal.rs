//! Pre-sealing a payload build: the state tree of the held build is kept up to
//! date while the build loop runs, so the seal only re-hashes what changed
//! since the last update instead of computing a full state root.
//!
//! The tree moves from one best build to the next by diff (`rebase_writes`):
//! builds of one slot share most of their state changes, so the difference
//! between two of them is much smaller than either. Every key the older build
//! touched and the newer one does not goes back to its parent value, and the
//! tree keeps those paths loaded for the rest of the slot.

use ethrex_common::{
    H256, U256,
    types::{AccountDiff, AccountUpdate, StateDiff},
    utils::keccak,
};
use rustc_hash::FxHashMap;

use crate::sharded_trie::AccountWrite;

/// What moving a tree from the build it reflects to a target build takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Rebase {
    /// At most one write per account; apply them in any order.
    Writes(Vec<AccountWrite>),
    /// The tree cannot be moved by writes: reopen it at the parent and apply
    /// the target in full. A diff destroyed an account that had storage
    /// (`removed_storage`), whose parent storage cannot be restored slot by
    /// slot, or `held` lacks a parent value that a revert needs.
    Reset,
}

/// The held diff lacks the parent value of a slot it wrote.
struct MissingParentValue;

/// Writes that turn a tree reflecting `held` into one reflecting `target`.
/// A field or slot missing from the target is at its parent value, which
/// `held` carries. `None` means the tree is at the parent state, so the
/// writes are the target's updates as they are, `removed_storage` included.
///
/// `target` holds at most one update per address, as `get_state_transitions`
/// emits them. Only accounts and slots whose value changes get a write: the
/// tree rewrites every account a write names, so a write that changes nothing
/// still costs a leaf.
pub fn rebase_writes<'a>(
    held: Option<&StateDiff>,
    target: impl IntoIterator<Item = &'a AccountUpdate>,
) -> Rebase {
    let Some(held) = held else {
        return Rebase::Writes(target.into_iter().map(AccountWrite::from_update).collect());
    };
    // The VM sets `removed_storage` only for an account that had storage, and
    // then reports zero for the parent value of every slot it did not read,
    // so neither side's slot values say what the tree must go back to.
    if held.values().any(|diff| diff.update.removed_storage) {
        return Rebase::Reset;
    }
    let mut targets: FxHashMap<_, &AccountUpdate> = FxHashMap::default();
    for update in target {
        if update.removed_storage {
            return Rebase::Reset;
        }
        let previous = targets.insert(update.address, update);
        debug_assert!(
            previous.is_none(),
            "two target updates for {}",
            update.address
        );
    }

    let mut writes = Vec::new();
    for (address, update) in &targets {
        let write = match held.get(address) {
            Some(diff) => rebase_account(diff, Some(update)),
            // The tree holds the parent state for an account `held` does not
            // touch, which is what the update is relative to.
            None => Ok(Some(AccountWrite::from_update(update))),
        };
        match write {
            Ok(write) => writes.extend(write),
            Err(MissingParentValue) => return Rebase::Reset,
        }
    }
    for (address, diff) in held {
        if targets.contains_key(address) {
            continue;
        }
        match rebase_account(diff, None) {
            Ok(write) => writes.extend(write),
            Err(MissingParentValue) => return Rebase::Reset,
        }
    }
    Rebase::Writes(writes)
}

/// The write that moves one account from `held` to `target`, or `None` if the
/// tree already holds what the target needs. A `target` of `None` means the
/// target leaves the account at its parent state.
fn rebase_account(
    held: &AccountDiff,
    target: Option<&AccountUpdate>,
) -> Result<Option<AccountWrite>, MissingParentValue> {
    let held_update = &held.update;
    let removed = target.is_some_and(|update| update.removed);
    let held_info = held_update.info.as_ref().unwrap_or(&held.parent_info);
    let target_info = target
        .and_then(|update| update.info.as_ref())
        .unwrap_or(&held.parent_info);
    // A removed account keeps its last written fields in the tree, hidden, and
    // gets them back when a later write makes it exist again. Writing the
    // fields whenever it comes back means the rebase never depends on them.
    let write_info = target_info != held_info || (held_update.removed && !removed);

    let mut storage: Vec<(H256, U256)> = Vec::new();
    if let Some(update) = target {
        for (slot, value) in &update.added_storage {
            // A slot `held` does not write is at its parent value in the
            // tree, which the VM only reports a slot for when it changed.
            if held_update.added_storage.get(slot) != Some(value) {
                storage.push((keccak(slot), *value));
            }
        }
    }
    for (slot, held_value) in &held_update.added_storage {
        if target.is_some_and(|update| update.added_storage.contains_key(slot)) {
            continue;
        }
        let parent_value = held.parent_storage.get(slot).ok_or(MissingParentValue)?;
        if parent_value != held_value {
            storage.push((keccak(slot), *parent_value));
        }
    }

    if !write_info && storage.is_empty() && removed == held_update.removed {
        return Ok(None);
    }
    let mut write = AccountWrite {
        hashed_address: keccak(held_update.address),
        // Every write sets whether the account exists, so each one carries
        // the target's flag, even when only slots change.
        removed,
        storage,
        ..AccountWrite::default()
    };
    if write_info {
        // In full, so nothing the tree kept from an earlier write survives.
        write.nonce = Some(target_info.nonce);
        write.balance = Some(target_info.balance);
        write.code_hash = Some(target_info.code_hash);
        // As `AccountWrite::from_update`: the target's new code goes with its
        // info. A revert needs none, since the parent's code is stored.
        write.code = target.and_then(|update| {
            let info = update.info.as_ref()?;
            update.code.clone().map(|code| (info.code_hash, code))
        });
    }
    Ok(Some(write))
}
