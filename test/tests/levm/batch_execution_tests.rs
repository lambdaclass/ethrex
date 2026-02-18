//! Tests for batch execution correctness.
//!
//! These tests verify that executing blocks in a batch (shared GeneralizedDatabase
//! across multiple blocks, single get_state_transitions() call at the end) produces
//! the same results as executing blocks one by one.
//!
//! Key scenarios tested:
//! - Account created and destroyed within the same batch should NOT produce
//!   a spurious trie leaf (regression test for #6219).

use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountInfo, AccountState, ChainConfig, Code, CodeMetadata},
};
use ethrex_levm::{
    account::{AccountStatus, LevmAccount},
    db::{Database, gen_db::GeneralizedDatabase},
    errors::DatabaseError,
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Test Database ====================

/// Minimal in-memory database that returns empty/default for everything.
struct EmptyDatabase;

impl Database for EmptyDatabase {
    fn get_account_state(&self, _address: Address) -> Result<AccountState, DatabaseError> {
        Ok(AccountState::default())
    }

    fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
        Ok(U256::zero())
    }

    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig::default())
    }

    fn get_account_code(&self, _code_hash: H256) -> Result<Code, DatabaseError> {
        Ok(Code::default())
    }

    fn get_code_metadata(&self, _code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        Ok(CodeMetadata { length: 0 })
    }
}

// ==================== Tests ====================

/// Regression test for issue #6219: `get_state_transitions()` emitted
/// `removed_storage=true` for any `DestroyedModified` account, even if
/// the account was created within the batch and never had storage in the trie.
///
/// Scenario:
///   - Block N (within batch): Contract C is CREATEd at address X with storage.
///   - Block N+M (within batch): C is SELFDESTRUCT'd (pre-Cancun).
///   - Block N+M (same block): Another contract with 0 balance SELFDESTRUCTs
///     with X as beneficiary, calling `increase_account_balance(X, 0)` which
///     triggers `get_account_mut(X)` → mark_modified() → DestroyedModified.
///   - X is now empty but DestroyedModified.
///   - At end of batch: `get_state_transitions()` is called.
///
/// Expected: No AccountUpdate should be emitted for X (empty initial + empty final).
/// Bug: `removed_storage=true` causes a spurious AccountUpdate, which downstream
/// in `apply_account_updates_from_trie_batch` inserts a default AccountState leaf
/// into the state trie, corrupting the state root.
#[test]
fn batch_no_spurious_update_for_account_created_and_destroyed_within_batch() {
    let address_x = Address::from_low_u64_be(0xDEAD);

    let mut db = GeneralizedDatabase::new(Arc::new(EmptyDatabase));

    // Simulate the initial state: X was first loaded from DB when accessed
    // during block N's CREATE. X didn't exist in DB → empty account, has_storage=false.
    db.initial_accounts_state.insert(
        address_x,
        LevmAccount {
            info: AccountInfo::default(),
            storage: FxHashMap::default(),
            has_storage: false,
            status: AccountStatus::Unmodified,
        },
    );

    // Simulate the final state after all blocks in the batch executed:
    // X was CREATEd (Modified), then SELFDESTRUCT'd (Destroyed), then touched
    // via another SELFDESTRUCT's increase_account_balance(X, 0) → DestroyedModified.
    // X is empty (default info, no storage).
    db.current_accounts_state.insert(
        address_x,
        LevmAccount {
            info: AccountInfo::default(),
            storage: FxHashMap::default(),
            has_storage: false,
            status: AccountStatus::DestroyedModified,
        },
    );

    let updates = db.get_state_transitions().unwrap();

    // ASSERTION: No AccountUpdate should be emitted for X.
    // X was empty before the batch (didn't exist in DB) and is empty after
    // (destroyed). There is nothing to update in the trie.
    //
    // BUG (current code): removed_storage=true is set unconditionally for
    // DestroyedModified, causing this assertion to fail. The spurious update
    // with {removed: false, info: None, removed_storage: true} causes
    // apply_account_updates_from_trie_batch to insert a default AccountState
    // leaf into the trie, corrupting the state root.
    let spurious_update = updates.iter().find(|u| u.address == address_x);
    assert!(
        spurious_update.is_none(),
        "No AccountUpdate should be emitted for an account created and destroyed within the batch. \
         Got: removed_storage={}, removed={}, info={:?}",
        spurious_update.map(|u| u.removed_storage).unwrap_or(false),
        spurious_update.map(|u| u.removed).unwrap_or(false),
        spurious_update.and_then(|u| u.info.as_ref()),
    );
}

/// Same as above but for get_state_transitions_tx() (used in the pipeline path).
#[test]
fn batch_tx_no_spurious_update_for_account_created_and_destroyed_within_batch() {
    let address_x = Address::from_low_u64_be(0xDEAD);

    let mut db = GeneralizedDatabase::new(Arc::new(EmptyDatabase));

    db.initial_accounts_state.insert(
        address_x,
        LevmAccount {
            info: AccountInfo::default(),
            storage: FxHashMap::default(),
            has_storage: false,
            status: AccountStatus::Unmodified,
        },
    );

    db.current_accounts_state.insert(
        address_x,
        LevmAccount {
            info: AccountInfo::default(),
            storage: FxHashMap::default(),
            has_storage: false,
            status: AccountStatus::DestroyedModified,
        },
    );

    let updates = db.get_state_transitions_tx().unwrap();

    let spurious_update = updates.iter().find(|u| u.address == address_x);
    assert!(
        spurious_update.is_none(),
        "No AccountUpdate should be emitted for an account created and destroyed within the batch (tx path). \
         Got: removed_storage={}, removed={}, info={:?}",
        spurious_update.map(|u| u.removed_storage).unwrap_or(false),
        spurious_update.map(|u| u.removed).unwrap_or(false),
        spurious_update.and_then(|u| u.info.as_ref()),
    );
}

/// Verify that removed_storage IS correctly set when the account actually
/// had storage in the trie (i.e. existed before the batch started).
///
/// This tests that the fix doesn't break the legitimate case: an account
/// that existed in the trie with storage, gets SELFDESTRUCT'd, and then
/// gets modified (e.g., receives ETH). Its storage should be cleared.
#[test]
fn batch_removed_storage_set_when_account_had_trie_storage() {
    let address_x = Address::from_low_u64_be(0xBEEF);

    let mut db = GeneralizedDatabase::new(Arc::new(EmptyDatabase));

    // Account existed in the trie before the batch with storage.
    db.initial_accounts_state.insert(
        address_x,
        LevmAccount {
            info: AccountInfo {
                nonce: 1,
                balance: U256::from(1000),
                code_hash: *EMPTY_KECCACK_HASH,
            },
            storage: FxHashMap::default(),
            has_storage: true, // KEY: account had storage in the trie
            status: AccountStatus::Unmodified,
        },
    );

    // After SELFDESTRUCT + receiving some ETH → DestroyedModified with balance.
    db.current_accounts_state.insert(
        address_x,
        LevmAccount {
            info: AccountInfo {
                nonce: 0,
                balance: U256::from(42),
                code_hash: *EMPTY_KECCACK_HASH,
            },
            storage: FxHashMap::default(),
            has_storage: false,
            status: AccountStatus::DestroyedModified,
        },
    );

    let updates = db.get_state_transitions().unwrap();

    let update = updates
        .iter()
        .find(|u| u.address == address_x)
        .expect("AccountUpdate should be emitted for account that existed in trie");

    // removed_storage should be true because the account DID have storage in the trie.
    assert!(
        update.removed_storage,
        "removed_storage should be true when account had storage in the trie"
    );
    // Account should not be removed (it has balance).
    assert!(
        !update.removed,
        "Account should not be removed (has balance)"
    );
    // Info should be updated (balance changed).
    assert!(update.info.is_some(), "Info should be updated");
}

/// Test that a DestroyedModified account with NO prior trie storage but with
/// balance changes still gets an info update (but NOT removed_storage).
///
/// Scenario: Account didn't exist before batch, was created, destroyed, then
/// received some ETH. It should appear in the trie with the new balance, but
/// removed_storage should be false (nothing to remove from trie).
#[test]
fn batch_destroyed_modified_with_balance_but_no_prior_storage() {
    let address_x = Address::from_low_u64_be(0xCAFE);

    let mut db = GeneralizedDatabase::new(Arc::new(EmptyDatabase));

    // Account didn't exist before the batch.
    db.initial_accounts_state.insert(
        address_x,
        LevmAccount {
            info: AccountInfo::default(),
            storage: FxHashMap::default(),
            has_storage: false,
            status: AccountStatus::Unmodified,
        },
    );

    // Created in batch, then destroyed, then received 1 ETH.
    // DestroyedModified with balance > 0.
    db.current_accounts_state.insert(
        address_x,
        LevmAccount {
            info: AccountInfo {
                nonce: 0,
                balance: U256::from(1_000_000_000_000_000_000u64), // 1 ETH
                code_hash: *EMPTY_KECCACK_HASH,
            },
            storage: FxHashMap::default(),
            has_storage: false,
            status: AccountStatus::DestroyedModified,
        },
    );

    let updates = db.get_state_transitions().unwrap();

    let update = updates
        .iter()
        .find(|u| u.address == address_x)
        .expect("AccountUpdate should be emitted (balance changed from 0)");

    // Info should be updated (balance went from 0 to 1 ETH).
    assert!(update.info.is_some(), "Info should reflect the new balance");
    // removed_storage should be false — there was never storage in the trie.
    assert!(
        !update.removed_storage,
        "removed_storage should be false when account never had storage in the trie"
    );
}
