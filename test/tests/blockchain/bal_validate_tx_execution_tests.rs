//! Unit tests calling `LEVM::validate_tx_execution` directly (bypassing the
//! full block-import pipeline exercised in `bal_content_validation_tests.rs`).
//!
//! These bind the same "no-op BAL entry" (Phase 1, PART A) and "missing
//! storage-write omission" (Phase 2, PART B) checks, but as pure unit tests
//! against the validation function itself: construct a post-execution
//! `current_state` plus a hand-built `BlockAccessList`, call
//! `validate_tx_execution`, and assert on the `Result` directly instead of
//! round-tripping through a real block/store/parallel-execution pipeline.
//!
//! `bal_idx = 1` / `seed_idx = 0` throughout (tx 0 of the block): every BAL
//! change list below uses `block_access_index = 1`, which is `> seed_idx`, so
//! `validate_tx_execution`'s `seeded_*` helpers always fall through to
//! `system_seed` (never touch `store`) — `store` only needs to exist to
//! satisfy the signature.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountInfo, ChainConfig, Code, CodeMetadata,
        block_access_list::{
            AccountChanges, BalanceChange, BlockAccessList, CodeChange, NonceChange, SlotChange,
            StorageChange,
        },
    },
    utils::u256_to_h256,
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    account::{AccountStatus, LevmAccount},
    db::{Database, gen_db::CacheDB},
    errors::DatabaseError,
};
use ethrex_vm::backends::levm::{BalValidationError, LEVM};
use rustc_hash::FxHashMap;

/// Minimal `Database` stub. Every scenario below seeds pre-tx values directly
/// via `system_seed`, so `validate_tx_execution`'s `store` fallback is never
/// actually reached; this only exists to satisfy the `&Arc<dyn Database>`
/// parameter.
struct UnusedStore;

impl Database for UnusedStore {
    fn get_account_state(
        &self,
        _address: Address,
    ) -> Result<ethrex_common::types::AccountState, DatabaseError> {
        Err(DatabaseError::Custom(
            "UnusedStore: no test scenario should reach the store fallback".into(),
        ))
    }
    fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
        Err(DatabaseError::Custom(
            "UnusedStore: no test scenario should reach the store fallback".into(),
        ))
    }
    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Err(DatabaseError::Custom("UnusedStore: not implemented".into()))
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Err(DatabaseError::Custom("UnusedStore: not implemented".into()))
    }
    fn get_account_code(&self, _code_hash: H256) -> Result<Code, DatabaseError> {
        Err(DatabaseError::Custom(
            "UnusedStore: no test scenario should reach the store fallback".into(),
        ))
    }
    fn get_code_metadata(&self, _code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        Err(DatabaseError::Custom("UnusedStore: not implemented".into()))
    }
}

fn unused_store() -> Arc<dyn Database> {
    Arc::new(UnusedStore)
}

fn addr(byte: u8) -> Address {
    let mut a = Address::zero();
    a.0[19] = byte;
    a
}

fn account_with(balance: U256, nonce: u64, code_hash: H256, status: AccountStatus) -> LevmAccount {
    LevmAccount {
        info: AccountInfo {
            code_hash,
            balance,
            nonce,
        },
        storage: FxHashMap::default(),
        has_storage: false,
        status,
        exists: true,
    }
}

/// Asserts `result` is `Err(BalValidationError::Mismatch(_))` whose message
/// contains `needle`.
fn assert_mismatch(result: &Result<(), BalValidationError>, needle: &str) {
    match result {
        Err(BalValidationError::Mismatch(msg)) => {
            assert!(
                msg.contains(needle),
                "expected mismatch message to contain {needle:?}, got: {msg}"
            );
        }
        other => panic!("expected Err(BalValidationError::Mismatch(_)), got: {other:?}"),
    }
}

#[test]
fn noop_balance_change_rejected() {
    let address = addr(1);
    let pre_balance = U256::from(100);

    let mut system_seed: CacheDB = FxHashMap::default();
    system_seed.insert(
        address,
        account_with(
            pre_balance,
            0,
            *ethrex_common::constants::EMPTY_KECCAK_HASH,
            AccountStatus::Unmodified,
        ),
    );

    // Post-execution state == pre-state: the "change" is a no-op.
    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();
    current_state.insert(
        address,
        account_with(
            pre_balance,
            0,
            *ethrex_common::constants::EMPTY_KECCAK_HASH,
            AccountStatus::Modified,
        ),
    );

    let bal = BlockAccessList::from_accounts(vec![
        AccountChanges::new(address).with_balance_changes(vec![BalanceChange::new(1, pre_balance)]),
    ]);
    let index = bal.build_validation_index();
    let codes: FxHashMap<H256, Code> = FxHashMap::default();
    let system_seed_map: CacheDB = system_seed;

    let result = LEVM::validate_tx_execution(
        1,
        0,
        &current_state,
        &codes,
        &bal,
        &index,
        &system_seed_map,
        &unused_store(),
    );
    assert_mismatch(&result, "no-op BAL balance change");
}

#[test]
fn noop_nonce_change_rejected() {
    let address = addr(2);
    let pre_nonce = 5u64;

    let mut system_seed: CacheDB = FxHashMap::default();
    system_seed.insert(
        address,
        account_with(
            U256::zero(),
            pre_nonce,
            *ethrex_common::constants::EMPTY_KECCAK_HASH,
            AccountStatus::Unmodified,
        ),
    );

    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();
    current_state.insert(
        address,
        account_with(
            U256::zero(),
            pre_nonce,
            *ethrex_common::constants::EMPTY_KECCAK_HASH,
            AccountStatus::Modified,
        ),
    );

    let bal = BlockAccessList::from_accounts(vec![
        AccountChanges::new(address).with_nonce_changes(vec![NonceChange::new(1, pre_nonce)]),
    ]);
    let index = bal.build_validation_index();
    let codes: FxHashMap<H256, Code> = FxHashMap::default();

    let result = LEVM::validate_tx_execution(
        1,
        0,
        &current_state,
        &codes,
        &bal,
        &index,
        &system_seed,
        &unused_store(),
    );
    assert_mismatch(&result, "no-op BAL nonce change");
}

#[test]
fn noop_code_change_rejected() {
    let address = addr(3);
    let code_bytes = Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]);
    let code = Code::from_bytecode(code_bytes.clone(), &NativeCrypto);
    let code_hash = code.hash;

    let mut system_seed: CacheDB = FxHashMap::default();
    system_seed.insert(
        address,
        account_with(U256::zero(), 0, code_hash, AccountStatus::Unmodified),
    );

    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();
    current_state.insert(
        address,
        account_with(U256::zero(), 0, code_hash, AccountStatus::Modified),
    );

    let bal = BlockAccessList::from_accounts(vec![
        AccountChanges::new(address).with_code_changes(vec![CodeChange::new(1, code_bytes)]),
    ]);
    let index = bal.build_validation_index();
    let mut codes: FxHashMap<H256, Code> = FxHashMap::default();
    codes.insert(code_hash, code);

    let result = LEVM::validate_tx_execution(
        1,
        0,
        &current_state,
        &codes,
        &bal,
        &index,
        &system_seed,
        &unused_store(),
    );
    assert_mismatch(&result, "no-op BAL code change");
}

#[test]
fn noop_storage_change_rejected() {
    let address = addr(4);
    let slot = U256::from(7);
    let key = u256_to_h256(slot);
    let pre_value = U256::from(42);

    let mut system_seed: CacheDB = FxHashMap::default();
    let mut seed_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Unmodified,
    );
    seed_account.storage.insert(key, pre_value);
    system_seed.insert(address, seed_account);

    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();
    let mut current_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Modified,
    );
    current_account.storage.insert(key, pre_value);
    current_state.insert(address, current_account);

    let bal =
        BlockAccessList::from_accounts(vec![AccountChanges::new(address).with_storage_changes(
            vec![SlotChange::with_changes(
                slot,
                vec![StorageChange::new(1, pre_value)],
            )],
        )]);
    let index = bal.build_validation_index();
    let codes: FxHashMap<H256, Code> = FxHashMap::default();

    let result = LEVM::validate_tx_execution(
        1,
        0,
        &current_state,
        &codes,
        &bal,
        &index,
        &system_seed,
        &unused_store(),
    );
    assert_mismatch(&result, "no-op BAL storage change");
}

/// Positive control: a genuine (non-no-op) balance change is accepted.
#[test]
fn genuine_balance_change_accepted() {
    let address = addr(5);
    let pre_balance = U256::from(100);
    let post_balance = U256::from(80);

    let mut system_seed: CacheDB = FxHashMap::default();
    system_seed.insert(
        address,
        account_with(
            pre_balance,
            0,
            *ethrex_common::constants::EMPTY_KECCAK_HASH,
            AccountStatus::Unmodified,
        ),
    );

    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();
    current_state.insert(
        address,
        account_with(
            post_balance,
            0,
            *ethrex_common::constants::EMPTY_KECCAK_HASH,
            AccountStatus::Modified,
        ),
    );

    let bal = BlockAccessList::from_accounts(vec![
        AccountChanges::new(address)
            .with_balance_changes(vec![BalanceChange::new(1, post_balance)]),
    ]);
    let index = bal.build_validation_index();
    let codes: FxHashMap<H256, Code> = FxHashMap::default();

    let result = LEVM::validate_tx_execution(
        1,
        0,
        &current_state,
        &codes,
        &bal,
        &index,
        &system_seed,
        &unused_store(),
    );
    assert!(result.is_ok(), "expected Ok(()), got: {result:?}");
}

/// A storage slot execution actually wrote (differs from its pre-tx value),
/// declared in neither the BAL account's `storage_changes` nor
/// `storage_reads`, must be rejected.
#[test]
fn missing_storage_write_rejected() {
    let address = addr(6);
    let slot = U256::from(3);
    let key = u256_to_h256(slot);
    let pre_value = U256::zero();
    let post_value = U256::from(9);

    let mut system_seed: CacheDB = FxHashMap::default();
    let mut seed_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Unmodified,
    );
    seed_account.storage.insert(key, pre_value);
    system_seed.insert(address, seed_account);

    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();
    let mut current_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Modified,
    );
    current_account.storage.insert(key, post_value);
    current_state.insert(address, current_account);

    // The account appears in the BAL (so it's not flagged as wholly "absent
    // from BAL"), but declares nothing at all about this slot: neither a
    // storage_changes entry nor a storage_reads entry.
    let bal = BlockAccessList::from_accounts(vec![AccountChanges::new(address)]);
    let index = bal.build_validation_index();
    let codes: FxHashMap<H256, Code> = FxHashMap::default();

    let result = LEVM::validate_tx_execution(
        1,
        0,
        &current_state,
        &codes,
        &bal,
        &index,
        &system_seed,
        &unused_store(),
    );
    assert_mismatch(&result, "absent from BAL");
}

/// Positive control: a slot declared solely via `storage_reads` (genuinely
/// read, value unchanged from its pre-tx value) must be accepted.
#[test]
fn read_only_slot_not_in_changes_accepted() {
    let address = addr(7);
    let slot = U256::from(11);
    let key = u256_to_h256(slot);
    let value = U256::from(77);

    let mut system_seed: CacheDB = FxHashMap::default();
    let mut seed_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Unmodified,
    );
    seed_account.storage.insert(key, value);
    system_seed.insert(address, seed_account);

    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();
    let mut current_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Modified,
    );
    current_account.storage.insert(key, value);
    current_state.insert(address, current_account);

    let bal = BlockAccessList::from_accounts(vec![
        AccountChanges::new(address).with_storage_reads(vec![slot]),
    ]);
    let index = bal.build_validation_index();
    let codes: FxHashMap<H256, Code> = FxHashMap::default();

    let result = LEVM::validate_tx_execution(
        1,
        0,
        &current_state,
        &codes,
        &bal,
        &index,
        &system_seed,
        &unused_store(),
    );
    assert!(result.is_ok(), "expected Ok(()), got: {result:?}");
}

/// A slot the BAL declares a change for only at a LATER tx index (so this tx
/// has no exact change and `seeded_pos == 0`) that execution nonetheless wrote
/// at THIS tx must be rejected: the write's BAL entry at this index was omitted.
/// Binds the seeded_pos==0 fix — the pre-fix code skipped this check entirely.
#[test]
fn storage_change_omitted_at_this_index_rejected() {
    let address = addr(8);
    let slot = U256::from(5);
    let key = u256_to_h256(slot);
    let pre_value = U256::zero();
    let post_value = U256::from(9);

    let mut system_seed: CacheDB = FxHashMap::default();
    let mut seed_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Unmodified,
    );
    seed_account.storage.insert(key, pre_value);
    system_seed.insert(address, seed_account);

    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();
    let mut current_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Modified,
    );
    current_account.storage.insert(key, post_value);
    current_state.insert(address, current_account);

    // BAL declares the slot's only change at index 2 (a later tx), leaving this
    // tx's write (index 1) unrecorded.
    let bal =
        BlockAccessList::from_accounts(vec![AccountChanges::new(address).with_storage_changes(
            vec![SlotChange::with_changes(
                slot,
                vec![StorageChange::new(2, post_value)],
            )],
        )]);
    let index = bal.build_validation_index();
    let codes: FxHashMap<H256, Code> = FxHashMap::default();

    let result = LEVM::validate_tx_execution(
        1,
        0,
        &current_state,
        &codes,
        &bal,
        &index,
        &system_seed,
        &unused_store(),
    );
    assert_mismatch(&result, "has no change at index");
}

/// Positive control for the seeded_pos==0 fix: the same later-index BAL change,
/// but this tx only READ the slot (value == start-of-block), must be accepted —
/// the fix must not reject a slot that did not actually change at this tx.
#[test]
fn storage_read_at_this_index_with_later_change_accepted() {
    let address = addr(9);
    let slot = U256::from(5);
    let key = u256_to_h256(slot);
    let value = U256::from(42);

    let mut system_seed: CacheDB = FxHashMap::default();
    let mut seed_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Unmodified,
    );
    seed_account.storage.insert(key, value);
    system_seed.insert(address, seed_account);

    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();
    let mut current_account = account_with(
        U256::zero(),
        0,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        AccountStatus::Modified,
    );
    // Value unchanged from start-of-block: the slot was read, not written, at this tx.
    current_account.storage.insert(key, value);
    current_state.insert(address, current_account);

    let bal =
        BlockAccessList::from_accounts(vec![AccountChanges::new(address).with_storage_changes(
            vec![SlotChange::with_changes(
                slot,
                vec![StorageChange::new(2, U256::from(99))],
            )],
        )]);
    let index = bal.build_validation_index();
    let codes: FxHashMap<H256, Code> = FxHashMap::default();

    let result = LEVM::validate_tx_execution(
        1,
        0,
        &current_state,
        &codes,
        &bal,
        &index,
        &system_seed,
        &unused_store(),
    );
    assert!(result.is_ok(), "expected Ok(()), got: {result:?}");
}
