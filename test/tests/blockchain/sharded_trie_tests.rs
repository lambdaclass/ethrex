//! `ShardedStateTrie` must give the same root as the serial
//! `Store::apply_account_updates_batch` for the same net state change.

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use ethrex_blockchain::sharded_trie::{AccountWrite, ShardedStateTrie};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{AccountInfo, AccountUpdate, BlockHeader, Code, Genesis, GenesisAccount},
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};
use ethrex_trie::Node;

/// Deterministic generator, so a failure is reproducible from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Generated account `i`. Indices at or above the generated count are
/// accounts the parent does not have.
fn address(i: u64) -> Address {
    H160::from_low_u64_be(0xA000_0000 + i)
}

/// Storage slot `j`, as the VM keys it (unhashed).
fn slot(j: u64) -> H256 {
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
async fn parent_state(accounts: u64, slots: u64) -> (Store, BlockHeader) {
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
const HOT: u64 = 1_000;

/// `genesis(30, 10)` plus a contract at `address(HOT)` with `slots` slots.
async fn hot_parent_state(slots: u64) -> (Store, BlockHeader) {
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
