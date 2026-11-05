//! `ShardedStateTrie` must give the same root as the serial
//! `Store::apply_account_updates_batch` for the same net state change.

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    sharded_trie::{AccountWrite, ShardedStateTrie},
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::{EMPTY_KECCAK_HASH, EMPTY_TRIE_HASH},
    seeded_hash::SeededHashMap,
    types::{
        AccountInfo, AccountUpdate, BlockHeader, Code, Genesis, GenesisAccount,
        block_access_list::BalSynthesisItem,
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{AccountUpdatesList, EngineType, Store};
use ethrex_trie::{Node, TrieNode};

use super::payload_build_loop_tests::{
    address_for, blockchain_for, payload_block, read_private_keys, setup_store, signed_transfer,
};

/// Deterministic generator, so a failure is reproducible from its seed.
pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(super) fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    pub(super) fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Generated account `i`. Indices at or above the generated count are
/// accounts the parent does not have.
pub(super) fn address(i: u64) -> Address {
    H160::from_low_u64_be(0xA000_0000 + i)
}

/// Storage slot `j`, as the VM keys it (unhashed).
pub(super) fn slot(j: u64) -> H256 {
    H256::from_low_u64_be(0x5107_0000 + j)
}

/// Every third generated account is a contract with storage in the parent.
fn has_storage(i: u64) -> bool {
    i.is_multiple_of(3)
}

fn fixture_genesis() -> Genesis {
    serde_json::from_slice(include_bytes!("../../../fixtures/genesis/l1.json"))
        .expect("fixture genesis parses")
}

fn generated_account(i: u64, slots: u64) -> GenesisAccount {
    let (code, storage) = if has_storage(i) {
        // Genesis keys storage by `U256`; it converts back with
        // `H256(key.to_big_endian())`, so slot `j` round-trips to `slot(j)`
        // and a later write to `slot(j)` overwrites it.
        let storage = (0..slots)
            .map(|j| (U256::from_big_endian(slot(j).as_bytes()), U256::from(j + 1)))
            .collect();
        (Bytes::from_static(&[0x60, 0x00, 0x00]), storage)
    } else {
        (Bytes::new(), BTreeMap::new())
    };
    GenesisAccount {
        code,
        storage,
        balance: U256::from(1_000_000 + i),
        nonce: i % 4,
    }
}

/// The fixture genesis plus `accounts` generated accounts, every third one
/// with `slots` storage slots.
fn genesis(accounts: u64, slots: u64) -> Genesis {
    let mut genesis = fixture_genesis();
    for i in 0..accounts {
        genesis
            .alloc
            .insert(address(i), generated_account(i, slots));
    }
    genesis
}

async fn store_with(genesis: Genesis) -> (Store, BlockHeader) {
    let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("genesis state");
    let header = store
        .get_block_header(0)
        .expect("read genesis header")
        .expect("genesis header");
    (store, header)
}

/// Builds a store whose genesis holds `accounts` generated accounts, every
/// third one with `slots` storage slots, and returns it with the genesis
/// header.
pub(super) async fn parent_state(accounts: u64, slots: u64) -> (Store, BlockHeader) {
    store_with(genesis(accounts, slots)).await
}

fn serial_root(store: &Store, parent: &BlockHeader, updates: &[AccountUpdate]) -> H256 {
    store
        .apply_account_updates_batch(parent.hash(), updates)
        .expect("serial apply")
        .expect("parent state exists")
        .state_trie_hash
}

fn info_with_code(rng: &mut Rng, code: Bytes) -> (AccountInfo, Option<Code>) {
    let info = AccountInfo {
        code_hash: keccak(&code),
        balance: U256::from(1 + rng.below(1 << 40)),
        // Never zero, so an info write never leaves an empty account.
        nonce: 1 + rng.below(100),
    };
    let code = (!code.is_empty()).then(|| Code::from_bytecode(code, &NativeCrypto));
    (info, code)
}

/// One random update for account `i`, which the parent has if `exists`.
fn random_update(rng: &mut Rng, i: u64, exists: bool, slots: u64) -> AccountUpdate {
    let address = address(i);
    if rng.chance(10) {
        return AccountUpdate::removed(address);
    }
    let mut update = AccountUpdate::new(address);
    let writes_slots = rng.chance(if exists { 50 } else { 30 });
    // A new account always gets info: without it the result would be the
    // empty account (or a nonce-0 holder of storage), which the VM never
    // emits without `removed`. An existing one keeps its non-empty parent
    // fields when it gets none.
    if !exists || !writes_slots || rng.chance(50) {
        let code = if writes_slots && !exists {
            Bytes::from(rng.next().to_be_bytes().to_vec())
        } else {
            Bytes::new()
        };
        let (info, code) = info_with_code(rng, code);
        update.info = Some(info);
        update.code = code;
    }
    if writes_slots {
        for _ in 0..1 + rng.below(12) {
            // Overwrite or delete a parent slot, or write a fresh one (a zero
            // write to a fresh slot deletes nothing, which must be harmless).
            let j = if exists && has_storage(i) && rng.chance(60) {
                rng.below(slots)
            } else {
                slots + rng.below(slots + 8)
            };
            let value = if rng.chance(25) {
                U256::zero()
            } else {
                U256::from(1 + rng.below(1 << 32))
            };
            update.added_storage.insert(slot(j), value);
        }
    }
    update
}

/// One random batch of `AccountUpdate`s over existing and new addresses:
/// info changes, new accounts, slot inserts, overwrites, deletions (zero) and
/// removals. Never produces an update whose result is an empty account that is
/// not `removed`: the VM never emits that.
fn random_updates(rng: &mut Rng, accounts: u64, slots: u64, n: usize) -> Vec<AccountUpdate> {
    let mut picked = BTreeSet::new();
    let mut updates = Vec::with_capacity(n);
    while updates.len() < n {
        let i = if rng.chance(25) {
            accounts + rng.below(accounts / 2 + 1)
        } else {
            rng.below(accounts)
        };
        if picked.insert(i) {
            updates.push(random_update(rng, i, i < accounts, slots));
        }
    }
    updates
}

/// Merges `updates` into the net change per address, as the serial path
/// would see one block's worth of them.
fn merge_into(net: &mut BTreeMap<Address, AccountUpdate>, updates: &[AccountUpdate]) {
    for update in updates {
        net.entry(update.address)
            .or_insert_with(|| AccountUpdate::new(update.address))
            .merge(update.clone());
    }
}

/// An address whose hashed form starts with `nibble`, so it lands in that
/// shard. `skip` picks a different one each time.
fn address_in_shard(nibble: u8, skip: usize) -> Address {
    (0u64..)
        .map(|k| H160::from_low_u64_be(0xB000_0000 + k))
        .filter(|a| keccak(a).as_bytes()[0] >> 4 == nibble)
        .nth(skip)
        .expect("an address exists for every nibble")
}

fn new_account(address: Address, nonce: u64) -> AccountUpdate {
    AccountUpdate {
        info: Some(AccountInfo {
            nonce,
            balance: U256::from(77),
            ..Default::default()
        }),
        ..AccountUpdate::new(address)
    }
}

#[tokio::test]
async fn one_batch_matches_serial() {
    // Neither side writes to the store, so one parent state serves every seed.
    let (store, parent) = parent_state(300, 40).await;
    for seed in 1..=20 {
        let mut rng = Rng(seed);
        let updates = random_updates(&mut rng, 300, 40, 120);
        let serial = serial_root(&store, &parent, &updates);
        let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
        tree.apply(updates.iter().map(AccountWrite::from_update));
        assert_eq!(tree.root().unwrap(), serial, "seed {seed}");
    }
}

#[tokio::test]
async fn repeated_batches_match_serial_of_net_change() {
    let (store, parent) = parent_state(300, 40).await;
    // Small batches run inline, large ones on threads; mix both.
    let sizes = [60, 3, 40, 1, 80];
    for seed in 1..=10 {
        let mut rng = Rng(seed);
        let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
        let mut net = BTreeMap::new();
        for (batch, n) in sizes.into_iter().enumerate() {
            let updates = random_updates(&mut rng, 300, 40, n);
            merge_into(&mut net, &updates);
            tree.apply(updates.iter().map(AccountWrite::from_update));
            let merged: Vec<AccountUpdate> = net.values().cloned().collect();
            assert_eq!(
                tree.root().unwrap(),
                serial_root(&store, &parent, &merged),
                "seed {seed} batch {batch}"
            );
        }
    }
}

/// A removal hides an account without discarding what was written to it, as
/// `AccountUpdate::merge` does: info written before the removal comes back
/// when a later write without info makes the account exist again.
#[tokio::test]
async fn info_written_before_a_removal_survives_it() {
    let (store, parent) = parent_state(30, 10).await;
    let target = address(3);
    let mut with_info = new_account(target, 42);
    with_info.added_storage.insert(slot(1), U256::from(9));
    let mut slots_only = AccountUpdate::new(target);
    slots_only.added_storage.insert(slot(2), U256::from(5));
    let batches = [
        vec![with_info],
        vec![AccountUpdate::removed(target)],
        vec![slots_only],
    ];

    let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
    let mut net = BTreeMap::new();
    for (batch, updates) in batches.iter().enumerate() {
        merge_into(&mut net, updates);
        tree.apply(updates.iter().map(AccountWrite::from_update));
        let merged: Vec<AccountUpdate> = net.values().cloned().collect();
        assert_eq!(
            tree.root().unwrap(),
            serial_root(&store, &parent, &merged),
            "batch {batch}"
        );
    }
}

#[tokio::test]
async fn removed_storage_matches_serial() {
    let (store, parent) = parent_state(30, 40).await;
    // Accounts 3 and 6 have 40 parent slots each.
    let mut reset = AccountUpdate::new(address(3));
    reset.removed_storage = true;
    for j in [1, 50, 51] {
        reset.added_storage.insert(slot(j), U256::from(7 + j));
    }
    let mut reset_without_slots = new_account(address(6), 9);
    reset_without_slots.removed_storage = true;
    let updates = vec![reset.clone(), reset_without_slots];

    let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
    tree.apply(updates.iter().map(AccountWrite::from_update));
    assert_eq!(
        tree.root().unwrap(),
        serial_root(&store, &parent, &updates),
        "one batch"
    );

    // A reset after earlier slot writes drops them too: the net change of
    // "write slots, then reset and write 3" is the reset alone.
    let mut earlier = AccountUpdate::new(address(3));
    for j in 0..20 {
        earlier.added_storage.insert(slot(j), U256::from(100 + j));
    }
    let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
    tree.apply([AccountWrite::from_update(&earlier)]);
    tree.root().unwrap();
    tree.apply([AccountWrite::from_update(&reset)]);
    assert_eq!(
        tree.root().unwrap(),
        serial_root(&store, &parent, &[reset]),
        "reset after a root"
    );
}

#[tokio::test]
async fn small_tree_with_root_leaf() {
    let mut genesis = fixture_genesis();
    genesis.alloc.clear();
    let first = address(0);
    genesis.alloc.insert(first, generated_account(0, 5));
    let (store, parent) = store_with(genesis).await;
    let root_node = store
        .open_state_trie(parent.state_root)
        .unwrap()
        .root_node()
        .unwrap()
        .expect("non-empty parent");
    assert!(matches!(*root_node, Node::Leaf(_)), "parent root is a leaf");

    let first_nibble = keccak(first).as_bytes()[0] >> 4;
    let second = address_in_shard((first_nibble + 1) % 16, 0);
    // Lands in the shard the first account leaves empty in step 3.
    let third = address_in_shard(first_nibble, 0);
    let steps = [
        // The root stays a leaf.
        vec![new_account(first, 5)],
        // The root becomes a branch.
        vec![new_account(second, 1)],
        // Back to a single leaf, and the first account's shard is empty.
        vec![AccountUpdate::removed(first)],
        // The emptied shard gets an account again.
        vec![new_account(third, 3)],
        vec![
            AccountUpdate::removed(second),
            AccountUpdate::removed(third),
        ],
    ];

    let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
    let mut net = BTreeMap::new();
    for (step, updates) in steps.iter().enumerate() {
        merge_into(&mut net, updates);
        tree.apply(updates.iter().map(AccountWrite::from_update));
        let merged: Vec<AccountUpdate> = net.values().cloned().collect();
        assert_eq!(
            tree.root().unwrap(),
            serial_root(&store, &parent, &merged),
            "step {step}"
        );
    }
    assert_eq!(tree.root().unwrap(), *EMPTY_TRIE_HASH);
}

#[tokio::test]
async fn emptying_every_account() {
    let genesis = genesis(100, 10);
    let addresses: Vec<Address> = genesis.alloc.keys().copied().collect();
    let (store, parent) = store_with(genesis).await;

    let removals: Vec<AccountUpdate> = addresses
        .iter()
        .map(|a| AccountUpdate::removed(*a))
        .collect();
    let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
    tree.apply(removals.iter().map(AccountWrite::from_update));
    assert_eq!(tree.root().unwrap(), *EMPTY_TRIE_HASH, "one batch");
    assert_eq!(serial_root(&store, &parent, &removals), *EMPTY_TRIE_HASH);

    // Half, then the rest, with a root in between.
    let (first, rest) = removals.split_at(removals.len() / 2);
    let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
    tree.apply(first.iter().map(AccountWrite::from_update));
    assert_eq!(
        tree.root().unwrap(),
        serial_root(&store, &parent, first),
        "first half"
    );
    tree.apply(rest.iter().map(AccountWrite::from_update));
    assert_eq!(tree.root().unwrap(), *EMPTY_TRIE_HASH, "two batches");
}

/// Generated index of the hot contract; outside the range `random_updates`
/// picks from.
pub(super) const HOT: u64 = 1_000;

/// `genesis(30, 10)` plus a contract at `address(HOT)` with `slots` slots.
pub(super) async fn hot_parent_state(slots: u64) -> (Store, BlockHeader) {
    let mut genesis = genesis(30, 10);
    let storage = (0..slots)
        .map(|j| (U256::from_big_endian(slot(j).as_bytes()), U256::from(j + 1)))
        .collect();
    genesis.alloc.insert(
        address(HOT),
        GenesisAccount {
            code: Bytes::from_static(&[0x60, 0x00, 0x00]),
            storage,
            balance: U256::one(),
            nonce: 1,
        },
    );
    store_with(genesis).await
}

/// Slot writes `(slot index, value)` to `address`; a zero value deletes.
fn slot_writes(address: Address, writes: impl IntoIterator<Item = (u64, u64)>) -> AccountUpdate {
    let mut update = AccountUpdate::new(address);
    for (j, value) in writes {
        update.added_storage.insert(slot(j), U256::from(value));
    }
    update
}

/// `n` random writes over slots `0..4000`, a tenth of them deletions.
fn random_slot_writes(rng: &mut Rng, address: Address, n: usize) -> AccountUpdate {
    let mut update = AccountUpdate::new(address);
    while update.added_storage.len() < n {
        let value = if rng.chance(10) {
            0
        } else {
            1 + rng.below(1 << 32)
        };
        update
            .added_storage
            .insert(slot(rng.below(4000)), U256::from(value));
    }
    update
}

#[tokio::test]
async fn hot_account_matches_serial() {
    let (store, parent) = hot_parent_state(3000).await;
    // 2500 writes: 1200 overwrites, 1000 inserts and 300 deletions.
    let hot = slot_writes(
        address(HOT),
        (0..1200)
            .map(|j| (j, 5_000 + j))
            .chain((3000..4000).map(|j| (j, j)))
            .chain((1200..1500).map(|j| (j, 0))),
    );
    // Other accounts in the same batch, so the normal storage bins run too.
    let mut updates = random_updates(&mut Rng(7), 30, 10, 20);
    updates.push(hot);

    let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
    tree.apply(updates.iter().map(AccountWrite::from_update));
    assert_eq!(tree.root().unwrap(), serial_root(&store, &parent, &updates));
    assert!(tree.is_sharded_storage(keccak(address(HOT))));
}

#[tokio::test]
async fn hot_account_cleared() {
    let (store, parent) = hot_parent_state(3000).await;
    let in_first_bucket = |j: &u64| keccak(slot(*j)).as_bytes()[0] >> 4 == 0;

    // Delete every slot outside the first bucket and leave that bucket
    // untouched, so the storage root collapses onto a child the tree never
    // loaded.
    let outside = slot_writes(
        address(HOT),
        (0..3000).filter(|j| !in_first_bucket(j)).map(|j| (j, 0)),
    );
    let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
    tree.apply([AccountWrite::from_update(&outside)]);
    assert_eq!(
        tree.root().unwrap(),
        serial_root(&store, &parent, std::slice::from_ref(&outside)),
        "one bucket left"
    );
    assert!(tree.is_sharded_storage(keccak(address(HOT))));

    let inside = slot_writes(
        address(HOT),
        (0..3000).filter(in_first_bucket).map(|j| (j, 0)),
    );
    tree.apply([AccountWrite::from_update(&inside)]);
    let mut every_slot = outside;
    every_slot.merge(inside);
    let cleared = serial_root(&store, &parent, &[every_slot]);
    assert_eq!(tree.root().unwrap(), cleared, "every slot deleted");
    // Deleting every slot leaves the empty storage root, as a reset does.
    let reset = AccountUpdate {
        removed_storage: true,
        ..AccountUpdate::new(address(HOT))
    };
    assert_eq!(cleared, serial_root(&store, &parent, &[reset]));
}

/// The storage kind is chosen when the trie first opens and kept: an
/// account first written with `HOT_STORAGE_SLOTS` or more slots stays split
/// through small batches and a reset, and one first written with fewer stays
/// whole when a large batch comes later.
#[tokio::test]
async fn hot_account_across_batches() {
    let (store, parent) = hot_parent_state(3000).await;
    let hot = address(HOT);
    // A contract with 10 parent slots.
    let warm = address(3);
    let mut rng = Rng(11);
    let batches = [
        // The hot trie opens split, the warm one whole.
        vec![
            random_slot_writes(&mut rng, hot, 2500),
            random_slot_writes(&mut rng, warm, 1000),
        ],
        vec![
            random_slot_writes(&mut rng, hot, 1000),
            random_slot_writes(&mut rng, warm, 2500),
        ],
        // Small enough to run inline.
        vec![random_slot_writes(&mut rng, hot, 5)],
        vec![random_slot_writes(&mut rng, hot, 1000)],
        vec![
            random_slot_writes(&mut rng, hot, 1000),
            random_slot_writes(&mut rng, warm, 1000),
        ],
    ];

    let mut tree = ShardedStateTrie::open(store.clone(), parent.state_root).unwrap();
    let mut net = BTreeMap::new();
    for (batch, updates) in batches.iter().enumerate() {
        merge_into(&mut net, updates);
        tree.apply(updates.iter().map(AccountWrite::from_update));
        let merged: Vec<AccountUpdate> = net.values().cloned().collect();
        assert_eq!(
            tree.root().unwrap(),
            serial_root(&store, &parent, &merged),
            "batch {batch}"
        );
        assert!(tree.is_sharded_storage(keccak(hot)), "batch {batch}");
        assert!(!tree.is_sharded_storage(keccak(warm)), "batch {batch}");
    }

    // A reset drops every earlier slot write. `AccountUpdate::merge` keeps
    // them, so the expected state is the reset alone plus the warm account's
    // net change.
    let mut reset = random_slot_writes(&mut rng, hot, 50);
    reset.removed_storage = true;
    tree.apply([AccountWrite::from_update(&reset)]);
    let expected = vec![reset, net[&warm].clone()];
    assert_eq!(
        tree.root().unwrap(),
        serial_root(&store, &parent, &expected),
        "reset"
    );
    assert!(tree.is_sharded_storage(keccak(hot)), "reset");
}

#[tokio::test]
async fn finish_after_root_is_an_error() {
    let (store, parent) = parent_state(30, 10).await;
    let updates = random_updates(&mut Rng(3), 30, 10, 10);
    let mut tree = ShardedStateTrie::open(store, parent.state_root).unwrap();
    tree.apply(updates.iter().map(AccountWrite::from_update));
    tree.root().unwrap();
    // The root memoized node hashes, so a node collection now would be
    // missing nodes; it must fail rather than return them.
    assert!(tree.finish().is_err());
}

/// An account update list in a form two runs can be compared in: every node
/// list sorted, since shards and workers emit them in any order. Code updates
/// are sorted too, as both sides push them in the input map's iteration order,
/// which nothing guarantees to match.
type Normalized = (
    H256,
    Vec<TrieNode>,
    Vec<(H256, Vec<TrieNode>)>,
    Vec<(H256, Code)>,
);

fn normalized(mut list: AccountUpdatesList) -> Normalized {
    list.state_updates.sort();
    for (_, nodes) in &mut list.storage_updates {
        nodes.sort();
    }
    list.storage_updates.sort();
    list.code_updates
        .sort_by_key(|(hash, code)| (*hash, code.hash));
    (
        list.state_trie_hash,
        list.state_updates,
        list.storage_updates,
        list.code_updates,
    )
}

/// Generated index of a contract with `BAL_HOT_SLOTS` parent slots.
const BAL_HOT: u64 = 1_000;
const BAL_HOT_SLOTS: u64 = 3_000;
const BAL_ACCOUNTS: u64 = 60;
const BAL_SLOTS: u64 = 40;

async fn bal_parent_state() -> (Store, BlockHeader) {
    let mut genesis = genesis(BAL_ACCOUNTS, BAL_SLOTS);
    let mut hot = generated_account(0, BAL_HOT_SLOTS);
    hot.nonce = 1;
    genesis.alloc.insert(address(BAL_HOT), hot);
    store_with(genesis).await
}

fn new_code(rng: &mut Rng) -> Code {
    Code::from_bytecode(
        Bytes::from(rng.next().to_be_bytes().to_vec()),
        &NativeCrypto,
    )
}

fn random_value(rng: &mut Rng, zero_percent: u64) -> U256 {
    if rng.chance(zero_percent) {
        U256::zero()
    } else {
        U256::from(1 + rng.below(1 << 32))
    }
}

/// One random BAL item for account `i`, which the parent has if `exists`.
fn random_bal_item(rng: &mut Rng, i: u64, exists: bool) -> BalSynthesisItem {
    let mut item = BalSynthesisItem::default();
    match rng.below(6) {
        // Info only.
        0 => {
            item.balance = Some(U256::from(1 + rng.below(1 << 40)));
            if rng.chance(50) {
                item.nonce = Some(1 + rng.below(50));
            }
        }
        // Slot overwrites, inserts and deletions.
        1 | 2 => {
            if !exists {
                let code = new_code(rng);
                item.nonce = Some(1);
                item.code_hash = Some(code.hash);
                item.code = Some(code);
            }
            for _ in 0..1 + rng.below(30) {
                let j = if exists && has_storage(i) && rng.chance(60) {
                    rng.below(BAL_SLOTS)
                } else {
                    BAL_SLOTS + rng.below(100)
                };
                item.added_storage.insert(slot(j), random_value(rng, 20));
            }
        }
        // Every parent slot cleared, so the storage trie ends up empty. Half
        // the time the account also loses its code, nonce and balance and
        // ends up empty too, so it is removed (EIP-161).
        3 if exists && has_storage(i) => {
            for j in 0..BAL_SLOTS {
                item.added_storage.insert(slot(j), U256::zero());
            }
            if rng.chance(50) {
                item.nonce = Some(0);
                item.balance = Some(U256::zero());
                item.code_hash = Some(*EMPTY_KECCAK_HASH);
            }
        }
        // Zero nonce and balance: an account without code or storage ends up
        // empty and is removed (EIP-161), including one the parent does not
        // have.
        3 | 4 => {
            item.nonce = Some(0);
            item.balance = Some(U256::zero());
        }
        // New code.
        _ => {
            let code = new_code(rng);
            item.nonce = Some(1 + rng.below(5));
            item.code_hash = Some(code.hash);
            item.code = Some(code);
        }
    }
    item
}

/// `n` slot indices whose hashed keys all start with `nibble`.
fn slots_in_bucket(nibble: u8, n: usize) -> Vec<u64> {
    (0u64..)
        .filter(|j| keccak(slot(*j)).as_bytes()[0] >> 4 == nibble)
        .take(n)
        .collect()
}

/// The BAL items for one seed: random items over existing and new accounts,
/// plus one hot-storage case chosen by the seed so every case runs.
fn random_bal_items(seed: u64) -> SeededHashMap<Address, BalSynthesisItem> {
    let mut rng = Rng(seed);
    let mut items = SeededHashMap::default();
    while items.len() < 50 {
        let i = rng.below(BAL_ACCOUNTS + BAL_ACCOUNTS / 2);
        let item = random_bal_item(&mut rng, i, i < BAL_ACCOUNTS);
        items.insert(address(i), item);
    }
    let mut hot = BalSynthesisItem::default();
    match seed % 5 {
        // Overwrites, inserts and deletions over the parent's slots.
        0 => {
            for _ in 0..2_500 {
                let j = rng.below(BAL_HOT_SLOTS + 1_000);
                hot.added_storage
                    .insert(slot(j), random_value(&mut rng, 10));
            }
        }
        // Every slot deleted: the split trie empties, which import computes
        // serially instead.
        1 => {
            for j in 0..BAL_HOT_SLOTS {
                hot.added_storage.insert(slot(j), U256::zero());
            }
        }
        // Every write in one bucket: import computes it serially instead.
        2 => {
            let nibble = u8::try_from(seed % 16).expect("a nibble");
            for j in slots_in_bucket(nibble, 2_100) {
                hot.added_storage
                    .insert(slot(j), random_value(&mut rng, 10));
            }
        }
        // Every slot outside one bucket deleted and some inside it changed,
        // so the split path runs and the root collapses onto that bucket:
        // there its node set differs from a serial one, and import keeps the
        // split path's.
        3 => {
            let kept = u8::try_from(seed % 16).expect("a nibble");
            for j in 0..BAL_HOT_SLOTS {
                if keccak(slot(j)).as_bytes()[0] >> 4 != kept {
                    hot.added_storage.insert(slot(j), U256::zero());
                } else if j % 3 == 0 {
                    hot.added_storage.insert(slot(j), U256::from(7 + j));
                }
            }
        }
        // A new account with a split trie from the start; the hot one only
        // changes its balance.
        _ => {
            let code = new_code(&mut rng);
            let mut fresh = BalSynthesisItem {
                nonce: Some(1),
                code_hash: Some(code.hash),
                code: Some(code),
                ..Default::default()
            };
            for j in 0..2_500 {
                fresh
                    .added_storage
                    .insert(slot(j), random_value(&mut rng, 0));
            }
            items.insert(address(2_000 + seed), fresh);
            hot.balance = Some(U256::from(seed));
        }
    }
    items.insert(address(BAL_HOT), hot);
    items
}

/// Import's BAL merkleizer on `ShardedStateTrie::finish` must give the same
/// root, the same state and storage node sets and the same code as the
/// merkleizer it replaced.
#[tokio::test]
async fn bal_path_matches_reference() {
    let (store, parent) = bal_parent_state().await;
    let blockchain = Blockchain::default_with_store(store);
    for seed in 1..=40 {
        let items = random_bal_items(seed);
        // A clone iterates in the same order, so both sides apply the items in
        // the same order, as one block's would be.
        let reference = blockchain
            .bal_merkleization_reference(items.clone(), &parent)
            .unwrap();
        let sharded = blockchain
            .handle_merkleization_bal_from_updates(items, &parent)
            .unwrap();
        let (root, state, storage, code) = normalized(sharded);
        let (expected_root, expected_state, expected_storage, expected_code) =
            normalized(reference);
        assert_eq!(root, expected_root, "seed {seed}: root");
        assert_eq!(state, expected_state, "seed {seed}: state nodes");
        assert_eq!(storage, expected_storage, "seed {seed}: storage nodes");
        assert_eq!(code, expected_code, "seed {seed}: code");
    }
}

/// When every account ends up empty, both sides return the empty root with
/// the `RLP_NULL` root entry (Stage D).
#[tokio::test]
async fn bal_path_matches_reference_when_the_state_empties() {
    let mut genesis = fixture_genesis();
    genesis.alloc.clear();
    let plain: Vec<u64> = (1..40).filter(|i| !has_storage(*i)).collect();
    for i in &plain {
        genesis.alloc.insert(address(*i), generated_account(*i, 0));
    }
    let (store, parent) = store_with(genesis).await;
    let blockchain = Blockchain::default_with_store(store);

    let mut items = SeededHashMap::default();
    for i in &plain {
        let item = BalSynthesisItem {
            nonce: Some(0),
            balance: Some(U256::zero()),
            ..Default::default()
        };
        items.insert(address(*i), item);
    }
    let reference = normalized(
        blockchain
            .bal_merkleization_reference(items.clone(), &parent)
            .unwrap(),
    );
    let sharded = normalized(
        blockchain
            .handle_merkleization_bal_from_updates(items, &parent)
            .unwrap(),
    );
    assert_eq!(sharded.0, *EMPTY_TRIE_HASH);
    assert_eq!(sharded, reference);
}

/// The payload seal's state root must equal the serial root of the same
/// account updates.
#[tokio::test]
async fn payload_seal_root_matches_serial() {
    let keys = read_private_keys(20);
    let accounts: Vec<Address> = keys.iter().map(address_for).collect();
    let store = setup_store(&accounts).await;
    let blockchain = blockchain_for(&store);
    for sk in &keys {
        for nonce in 0..10 {
            let tx = signed_transfer(sk, nonce).await;
            blockchain.add_transaction_to_pool(tx).await.unwrap();
        }
    }

    let result = blockchain.build_payload(payload_block(&store)).unwrap();
    assert_eq!(result.payload.body.transactions.len(), 200);
    let parent = store.get_block_header(0).unwrap().unwrap();
    assert_eq!(
        result.payload.header.state_root,
        serial_root(&store, &parent, &result.account_updates)
    );
}
