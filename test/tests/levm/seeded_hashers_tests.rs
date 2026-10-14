//! Regression tests for the hashers behind the maps whose keys are chosen by contract
//! code or by the block producer.
//!
//! rustc-hash 2.x hashes a byte string of 17 to 32 bytes as
//! `multiply_mix(SEED2 ^ word(bytes[len - 16..len - 8]), ..) ^ len`, where `SEED2` is a
//! public constant and `multiply_mix(0, _) == 0`. Keys whose word at `len - 16..len - 8`
//! equals `SEED2` therefore all hash to `len`, whatever their other bytes hold. A `U256`
//! hashes as its 32 little-endian bytes, and a tuple ending in such a key inherits the
//! collision. Every test first checks that its keys still collapse under FxHash, so the
//! key shape stays an attack, and then that the map under test hashes them apart.

use std::{
    collections::HashSet,
    hash::{BuildHasher, Hash},
    sync::Arc,
};

use bytes::Bytes;
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
    db::gen_db::{CacheDB, GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    tracing::LevmCallTracer,
    vm::{AccessedAddresses, AccessedStorageSlots, VM, VMType},
};
use rustc_hash::{FxBuildHasher, FxHashMap};

use super::test_db::TestDatabase;

/// `SEED2` in rustc-hash 2.x: the word XORed into the key right before the final
/// multiply, for keys of 17 to 32 bytes.
const ZEROING_WORD: u64 = 0x13198a2e03707344;
/// Keys per test. Among this many keys a seeded 64-bit hasher collides with
/// probability well under 1e-12, so every key must get its own hash.
const KEYS: u64 = 4096;

/// 32-byte keys whose bytes `16..24` are `ZEROING_WORD`; the first word counts.
fn h256_keys() -> Vec<H256> {
    (0..KEYS)
        .map(|i| H256::from(U256([i, 0, ZEROING_WORD, 0]).to_little_endian()))
        .collect()
}

/// 20-byte keys whose bytes `4..12` are `ZEROING_WORD`; the last word counts.
fn address_keys() -> Vec<Address> {
    (0..KEYS)
        .map(|i| {
            let mut bytes = [0u8; 20];
            for (byte, value) in bytes.iter_mut().skip(4).zip(ZEROING_WORD.to_le_bytes()) {
                *byte = value;
            }
            for (byte, value) in bytes.iter_mut().skip(12).zip(i.to_le_bytes()) {
                *byte = value;
            }
            Address::from(bytes)
        })
        .collect()
}

fn calldata_keys() -> Vec<Bytes> {
    h256_keys()
        .into_iter()
        .map(|key| Bytes::copy_from_slice(key.as_bytes()))
        .collect()
}

fn slot_keys() -> Vec<(Address, H256)> {
    let address = Address::from_low_u64_be(0x7777);
    h256_keys().into_iter().map(|key| (address, key)).collect()
}

fn u256_slot_keys() -> Vec<(Address, U256)> {
    let address = Address::from_low_u64_be(0x7777);
    (0..KEYS)
        .map(|i| (address, U256([i, 0, ZEROING_WORD, 0])))
        .collect()
}

fn distinct_hashes<K: Hash>(hasher: &impl BuildHasher, keys: &[K]) -> usize {
    keys.iter()
        .map(|key| hasher.hash_one(key))
        .collect::<HashSet<_>>()
        .len()
}

/// Checks that `keys` share one FxHash, so they are the attack shape, and that
/// `hasher` gives each of them its own hash.
#[track_caller]
fn assert_hashed_apart<K: Hash>(hasher: &impl BuildHasher, keys: &[K]) {
    assert_eq!(
        distinct_hashes(&FxBuildHasher, keys),
        1,
        "keys no longer collide under FxHash: rustc-hash changed its constants, \
         update the key shape rather than the hasher under test"
    );
    assert_eq!(distinct_hashes(hasher, keys), keys.len());
}

#[test]
fn levm_account_storage_hashes_colliding_slots_apart() {
    assert_hashed_apart(LevmAccount::default().storage.hasher(), &h256_keys());
}

#[test]
fn cache_db_hashes_colliding_addresses_apart() {
    assert_hashed_apart(CacheDB::default().hasher(), &address_keys());
}

#[test]
fn call_frame_backup_hashes_colliding_keys_apart() {
    let mut backup = CallFrameBackup::default();
    assert_hashed_apart(backup.original_accounts_info.hasher(), &address_keys());
    assert_hashed_apart(
        backup.original_account_storage_slots.hasher(),
        &address_keys(),
    );
    let slots = backup
        .original_account_storage_slots
        .entry(Address::zero())
        .or_default();
    assert_hashed_apart(slots.hasher(), &h256_keys());
}

#[test]
fn accessed_addresses_and_slots_hash_colliding_keys_apart() {
    assert_hashed_apart(AccessedAddresses::default().hasher(), &address_keys());
    let mut slots = AccessedStorageSlots::default();
    assert_hashed_apart(slots.hasher(), &address_keys());
    assert_hashed_apart(
        slots.entry(Address::zero()).or_default().hasher(),
        &h256_keys(),
    );
}

#[test]
fn storage_original_values_hash_colliding_keys_apart() {
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

    assert_hashed_apart(vm.storage_original_values.hasher(), &address_keys());
    let slots = vm
        .storage_original_values
        .entry(Address::zero())
        .or_default();
    assert_hashed_apart(slots.hasher(), &h256_keys());
}

#[test]
fn account_update_added_storage_hashes_colliding_slots_apart() {
    assert_hashed_apart(
        AccountUpdate::default().added_storage.hasher(),
        &h256_keys(),
    );
}

#[test]
fn bal_synthesis_added_storage_hashes_colliding_slots_apart() {
    assert_hashed_apart(
        BalSynthesisItem::default().added_storage.hasher(),
        &h256_keys(),
    );
}

#[test]
fn bal_validation_index_hashes_colliding_keys_apart() {
    let mut changes = AccountChanges::new(Address::zero());
    changes.storage_changes.push(SlotChange::new(U256::zero()));
    let index = BlockAccessList::from_accounts(vec![changes]).build_validation_index();
    assert_hashed_apart(index.addr_to_idx.hasher(), &address_keys());
    let slots = index
        .slot_idx_by_account
        .first()
        .expect("one account in the BAL");
    assert_hashed_apart(slots.hasher(), &h256_keys());
}

#[test]
fn bal_recorder_tx_initial_storage_hashes_colliding_slots_apart() {
    let tx_initial = BlockAccessListRecorder::new().take_tx_initial_storage();
    assert_hashed_apart(tx_initial.hasher(), &u256_slot_keys());
}

#[test]
fn calldata_and_slot_tuples_collide_under_fxhash() {
    // The precompile result cache (calldata keys) and the warming caches (address, slot
    // tuples) hold their maps privately; their unit tests use these same key shapes.
    assert_eq!(distinct_hashes(&FxBuildHasher, &calldata_keys()), 1);
    assert_eq!(distinct_hashes(&FxBuildHasher, &slot_keys()), 1);
}
