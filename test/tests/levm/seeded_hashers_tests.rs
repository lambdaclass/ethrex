//! Every map whose keys a block can choose hashes them with a seed of its own, so keys
//! crafted to share one bucket under an unseeded hasher such as FxHash land apart.
//! `ethrex_common::seeded_hash` tests how the seeded hasher spreads such keys; these tests
//! take each map as production code builds it and check that it is seeded at all.

use std::{hash::BuildHasher, sync::Arc};

use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountUpdate, Code, EIP1559Transaction, Fork, Transaction, TxKind,
        block_access_list::{
            AccountChanges, BalSynthesisItem, BlockAccessList, BlockAccessListRecorder, SlotChange,
        },
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    account::LevmAccount,
    call_frame::CallFrameBackup,
    db::gen_db::GeneralizedDatabase,
    environment::{EVMConfig, Environment},
    tracing::LevmCallTracer,
    vm::{AccessedAddresses, AccessedStorageSlots, VM, VMType},
};
use rustc_hash::FxHashMap;

use super::test_db::TestDatabase;

/// Fails if `state` hashes a key like a fresh instance of its type does. Every instance of
/// an unseeded hasher agrees on every key, so a key crafted for one bucket of one map lands
/// in that bucket in all of them.
#[track_caller]
fn assert_seeded<S: BuildHasher + Default>(state: &S) {
    let key = H256::repeat_byte(0x5a);
    assert_ne!(
        state.hash_one(key),
        S::default().hash_one(key),
        "this map hashes its keys without a seed of its own"
    );
}

#[test]
fn levm_account_storage_is_seeded() {
    assert_seeded(LevmAccount::default().storage.hasher());
}

#[test]
fn generalized_database_maps_are_seeded() {
    // Code is keyed by its hash, but deployed code can be ground into a bucket and called
    // again for 100 gas, with every call looking its code up by hash.
    let db = GeneralizedDatabase::new(Arc::new(TestDatabase::new()));
    assert_seeded(db.current_accounts_state.hasher());
    assert_seeded(db.initial_accounts_state.hasher());
    assert_seeded(db.codes.hasher());
    assert_seeded(db.code_metadata.hasher());
}

#[test]
fn call_frame_backup_is_seeded() {
    let mut backup = CallFrameBackup::default();
    assert_seeded(backup.original_accounts_info.hasher());
    assert_seeded(backup.original_account_storage_slots.hasher());
    let slots = backup
        .original_account_storage_slots
        .entry(Address::zero())
        .or_default();
    assert_seeded(slots.hasher());
}

#[test]
fn accessed_addresses_and_slots_are_seeded() {
    assert_seeded(AccessedAddresses::default().hasher());
    let mut slots = AccessedStorageSlots::default();
    assert_seeded(slots.hasher());
    assert_seeded(slots.entry(Address::zero()).or_default().hasher());
}

#[test]
fn storage_original_values_are_seeded() {
    let origin = Address::from_low_u64_be(0x1000);
    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    accounts.insert(
        origin,
        Account::new(
            U256::from(1u64) << 64,
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    let mut db =
        GeneralizedDatabase::new_with_account_state(Arc::new(TestDatabase::new()), accounts);
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        gas_limit: 1_000_000,
        to: TxKind::Call(Address::from_low_u64_be(0xBEEF)),
        ..Default::default()
    });
    let fork = Fork::Prague;
    let env = Environment {
        origin,
        gas_limit: 1_000_000,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        block_number: 1,
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        chain_id: U256::from(1),
        base_blob_fee_per_gas: U256::from(1),
        tx_max_fee_per_gas: Some(U256::zero()),
        block_gas_limit: 30_000_000,
        disable_balance_check: true,
        ..Default::default()
    };
    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new");

    assert_seeded(vm.storage_original_values.hasher());
    let slots = vm
        .storage_original_values
        .entry(Address::zero())
        .or_default();
    assert_seeded(slots.hasher());
}

#[test]
fn account_update_added_storage_is_seeded() {
    assert_seeded(AccountUpdate::default().added_storage.hasher());
}

#[test]
fn bal_synthesis_added_storage_is_seeded() {
    assert_seeded(BalSynthesisItem::default().added_storage.hasher());
}

#[test]
fn bal_validation_index_is_seeded() {
    let mut changes = AccountChanges::new(Address::zero());
    changes.storage_changes.push(SlotChange::new(U256::zero()));
    let index = BlockAccessList::from_accounts(vec![changes]).build_validation_index();
    assert_seeded(index.addr_to_idx.hasher());
    let slots = index
        .slot_idx_by_account
        .first()
        .expect("one account in the BAL");
    assert_seeded(slots.hasher());
}

#[test]
fn bal_recorder_tx_initial_storage_is_seeded() {
    assert_seeded(
        BlockAccessListRecorder::new()
            .take_tx_initial_storage()
            .hasher(),
    );
}
