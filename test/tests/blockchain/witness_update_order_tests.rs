//! The host records a witness while applying a block's account updates to the state trie,
//! and the guest replays its own list of the same updates on the recorded nodes. Each list is
//! built by iterating a seeded map, so the two come in unrelated orders. Removing an account
//! from a branch with two children reads the remaining child, while inserting into that branch
//! first avoids the read: a host that inserts first records a witness that a guest removing
//! first cannot replay. Both sides apply the updates in address order, so every input order on
//! the host must produce a witness that every input order on the guest can replay.
//!
//! An account's storage changes come from a seeded map too, and the host applies them in that
//! map's order. The guest applies its inserts before its deletes, which reads a subset of the
//! nodes any order on the host reads, so here too every host order must produce a witness that
//! every guest order can replay.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    seeded_hash::SeededHashMap,
    types::{
        AccountInfo, AccountUpdate, BlockHeader, Genesis, GenesisAccount,
        block_execution_witness::{ExecutionWitness, GuestProgramState, RpcExecutionWitness},
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store, hash_address, hash_key};
use ethrex_trie::TrieLogger;

/// The first `N` candidates whose hashed keys start with different nibbles, in the order they
/// are generated. A trie holding any two of them is a root branch with two leaf children, and
/// each of the others lands in an empty slot of that branch.
fn with_distinct_first_nibbles<const N: usize, T: Debug>(
    candidate: impl Fn(u64) -> T,
    hashed: impl Fn(&T) -> Vec<u8>,
) -> [T; N] {
    let mut found = Vec::new();
    let mut nibbles = Vec::new();
    for i in 1u64.. {
        let item = candidate(i);
        let nibble = hashed(&item)[0] >> 4;
        if nibbles.contains(&nibble) {
            continue;
        }
        nibbles.push(nibble);
        found.push(item);
        if found.len() == N {
            break;
        }
    }
    found.try_into().expect("exactly N candidates found")
}

fn funded_account() -> GenesisAccount {
    GenesisAccount {
        code: Default::default(),
        storage: Default::default(),
        balance: U256::one(),
        nonce: 0,
    }
}

/// A store holding `alloc` as its genesis state, and the genesis header.
async fn genesis_store(alloc: BTreeMap<Address, GenesisAccount>) -> (Store, BlockHeader) {
    let genesis = Genesis {
        alloc,
        ..Default::default()
    };
    let parent = genesis.get_block().header;
    let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("initialize genesis");
    (store, parent)
}

/// Applies `updates` on the host exactly as witness generation does, and returns the witness
/// it recorded, state and storage nodes both, together with the state root it reached.
fn host_witness(
    store: &Store,
    parent: &BlockHeader,
    updates: &[AccountUpdate],
) -> (ExecutionWitness, H256) {
    let trie = store
        .state_trie(parent.hash())
        .expect("state trie lookup")
        .expect("parent state is stored");
    let (recorded, trie) = TrieLogger::open_trie(trie);
    let (storage_tries, applied) = store
        .apply_account_updates_from_trie_with_witness(trie, updates, HashMap::new())
        .expect("host applies the updates");
    let mut state: Vec<Bytes> = recorded
        .lock()
        .expect("witness lock")
        .values()
        .map(|node| node.encode_to_vec().into())
        .collect();
    for (storage_recorded, _) in storage_tries.values() {
        state.extend(
            storage_recorded
                .lock()
                .expect("witness lock")
                .values()
                .map(|node| Bytes::from(node.encode_to_vec())),
        );
    }
    let witness = RpcExecutionWitness {
        state,
        headers: vec![parent.encode_to_vec().into()],
        ..Default::default()
    }
    .into_execution_witness(
        store.get_chain_config(),
        parent.number + 1,
        std::slice::from_ref(parent),
        &NativeCrypto,
    )
    .expect("witness from the recorded nodes");
    (witness, applied.state_trie_hash)
}

/// Replays `witness` on a guest applying `updates`, and checks it reaches `expected_root`.
fn replay(witness: &ExecutionWitness, updates: &[AccountUpdate], expected_root: H256) {
    let mut guest = GuestProgramState::from_witness(witness.clone(), &NativeCrypto)
        .expect("guest state from witness");
    guest
        .apply_account_updates(updates, &NativeCrypto)
        .expect("the witness must hold every node this order needs");
    let root = guest.state_trie_root(&NativeCrypto).expect("guest root");
    assert_eq!(
        root, expected_root,
        "guest must reach the host's state root"
    );
}

/// Records a witness on the host under each input order and replays it on a guest applying
/// each input order. `removed` and `untouched` share the root branch of the initial trie, and
/// `inserted` joins it.
async fn replay_under_every_order(removed: Address, untouched: Address, inserted: Address) {
    let mut alloc = BTreeMap::new();
    alloc.insert(removed, funded_account());
    alloc.insert(untouched, funded_account());
    let (store, parent) = genesis_store(alloc).await;

    let remove = AccountUpdate::removed(removed);
    let mut insert = AccountUpdate::new(inserted);
    insert.info = Some(AccountInfo {
        balance: U256::one(),
        ..Default::default()
    });
    let orders = [vec![insert.clone(), remove.clone()], vec![remove, insert]];

    for host_order in &orders {
        let (witness, expected_root) = host_witness(&store, &parent, host_order);
        for guest_order in &orders {
            replay(&witness, guest_order, expected_root);
        }
    }
}

/// Both sides sort by address, so the sorted order is remove-first in one scenario and
/// insert-first in the other. Remove-first on the host records the sibling and hides a guest
/// that does not sort; insert-first on the host records the minimal witness and hides a host
/// that does not sort. Together the two scenarios fail if either side stops sorting.
#[tokio::test]
async fn guest_replays_the_witness_under_every_update_order() {
    let [first, second, third] =
        with_distinct_first_nibbles(Address::from_low_u64_be, hash_address);
    assert!(
        first < second,
        "candidates are generated in ascending order"
    );
    replay_under_every_order(first, third, second).await;
    replay_under_every_order(second, third, first).await;
}

/// `changes` as an `added_storage` map that iterates them in exactly this order. Each seeded
/// map draws its own seed, so a map of two entries comes out in either order about half the
/// time; this builds maps until one comes out in the order asked for.
fn added_storage_in_order(changes: [(H256, U256); 2]) -> SeededHashMap<H256, U256> {
    for _ in 0..1000 {
        let map: SeededHashMap<H256, U256> = changes.into_iter().collect();
        if map.keys().eq(changes.iter().map(|(slot, _)| slot)) {
            return map;
        }
    }
    panic!("no map out of 1000 iterated the changes in the order asked for");
}

/// Gives one account the storage `initial`, records a witness on the host for `changes` under
/// each order of its `added_storage`, and replays it on a guest whose `added_storage` comes in
/// each order.
async fn replay_storage_under_every_order(initial: &[H256], changes: [(H256, U256); 2]) {
    let owner = Address::from_low_u64_be(0x5107);
    let mut account = funded_account();
    account.storage = initial
        .iter()
        .map(|slot| (U256::from_big_endian(slot.as_bytes()), U256::one()))
        .collect();
    let (store, parent) = genesis_store(BTreeMap::from([(owner, account)])).await;

    let update = |order: [(H256, U256); 2]| {
        let mut update = AccountUpdate::new(owner);
        update.added_storage = added_storage_in_order(order);
        update
    };
    let [first, second] = changes;
    let orders = [[first, second], [second, first]];

    for host_order in orders {
        let (witness, expected_root) = host_witness(&store, &parent, &[update(host_order)]);
        for guest_order in orders {
            replay(&witness, &[update(guest_order)], expected_root);
        }
    }
}

/// A delete that leaves the root branch with one child reads that child, unless an insert
/// into the branch came first. The host records the child only when it deletes first, so a
/// guest that stopped putting inserts first fails against the host that inserted first.
#[tokio::test]
async fn guest_replays_storage_inserts_and_deletes_under_every_order() {
    let [removed, untouched, inserted] =
        with_distinct_first_nibbles(H256::from_low_u64_be, hash_key);
    replay_storage_under_every_order(
        &[removed, untouched],
        [(removed, U256::zero()), (inserted, U256::one())],
    )
    .await;
}

/// Two deletes from one branch: whichever comes second leaves the branch with one child and
/// reads it, so the host records the same nodes in either order and the guest needs them in
/// either order.
#[tokio::test]
async fn guest_replays_storage_deletes_under_every_order() {
    let [first, second, untouched] = with_distinct_first_nibbles(H256::from_low_u64_be, hash_key);
    replay_storage_under_every_order(
        &[first, second, untouched],
        [(first, U256::zero()), (second, U256::zero())],
    )
    .await;
}
