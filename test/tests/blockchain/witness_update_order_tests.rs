//! The host records a witness while applying a block's account updates to the state trie,
//! and the guest replays its own list of the same updates on the recorded nodes. Each list is
//! built by iterating a seeded map, so the two come in unrelated orders. Removing an account
//! from a branch with two children reads the remaining child, while inserting into that branch
//! first avoids the read: a host that inserts first records a witness that a guest removing
//! first cannot replay. Both sides apply the updates in address order, so every input order on
//! the host must produce a witness that every input order on the guest can replay.

use std::collections::{BTreeMap, HashMap};

use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountInfo, AccountUpdate, BlockHeader, Genesis, GenesisAccount,
        block_execution_witness::{ExecutionWitness, GuestProgramState},
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store, hash_address};
use ethrex_trie::{Node, NodeRef, Trie, TrieLogger};

/// Three addresses whose hashed keys start with different nibbles. A state trie holding the
/// first two is a root branch with two leaf children, and the third lands in an empty slot
/// of that branch.
fn addresses_with_distinct_first_nibbles() -> [Address; 3] {
    let mut found = Vec::new();
    let mut nibbles = Vec::new();
    for candidate in 1u64.. {
        let address = Address::from_low_u64_be(candidate);
        let nibble = hash_address(&address)[0] >> 4;
        if nibbles.contains(&nibble) {
            continue;
        }
        nibbles.push(nibble);
        found.push(address);
        if found.len() == 3 {
            break;
        }
    }
    [found[0], found[1], found[2]]
}

fn funded_account() -> GenesisAccount {
    GenesisAccount {
        code: Default::default(),
        storage: Default::default(),
        balance: U256::one(),
        nonce: 0,
    }
}

/// Applies `updates` on the host exactly as witness generation does, and returns the witness
/// it recorded together with the state root it reached.
fn host_witness(
    store: &Store,
    parent: &BlockHeader,
    updates: &[AccountUpdate],
) -> (ExecutionWitness, H256) {
    let trie = store
        .state_trie(parent.hash())
        .expect("state trie lookup")
        .expect("parent state is stored");
    let initial_root = trie.hash_no_commit(&NativeCrypto);
    let (recorded, trie) = TrieLogger::open_trie(trie);
    let (_storage_tries, applied) = store
        .apply_account_updates_from_trie_with_witness(trie, updates, HashMap::new())
        .expect("host applies the updates");
    let nodes: BTreeMap<H256, Node> = recorded
        .lock()
        .expect("witness lock")
        .values()
        .map(|node| {
            (
                node.compute_hash(&NativeCrypto).finalize(&NativeCrypto),
                node.clone(),
            )
        })
        .collect();
    let NodeRef::Node(root, _) =
        Trie::get_embedded_root(&nodes, initial_root).expect("embedded root")
    else {
        panic!("the state trie root must be recorded");
    };
    let witness = ExecutionWitness {
        codes: Vec::new(),
        block_headers_bytes: vec![parent.encode_to_vec()],
        first_block_number: parent.number + 1,
        chain_config: store.get_chain_config(),
        state_trie_root: Some((*root).clone()),
        storage_trie_roots: BTreeMap::new(),
    };
    (witness, applied.state_trie_hash)
}

/// Records a witness on the host under each input order and replays it on a guest applying
/// each input order. `removed` and `untouched` share the root branch of the initial trie, and
/// `inserted` joins it.
async fn replay_under_every_order(removed: Address, untouched: Address, inserted: Address) {
    let mut alloc = BTreeMap::new();
    alloc.insert(removed, funded_account());
    alloc.insert(untouched, funded_account());
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
            let mut guest = GuestProgramState::from_witness(witness.clone(), &NativeCrypto)
                .expect("guest state from witness");
            guest
                .apply_account_updates(guest_order, &NativeCrypto)
                .expect("the witness must hold every node this order needs");
            let root = guest.state_trie_root(&NativeCrypto).expect("guest root");
            assert_eq!(
                root, expected_root,
                "guest must reach the host's state root"
            );
        }
    }
}

/// Both sides sort by address, so the sorted order is remove-first in one scenario and
/// insert-first in the other. Remove-first on the host records the sibling and hides a guest
/// that does not sort; insert-first on the host records the minimal witness and hides a host
/// that does not sort. Together the two scenarios fail if either side stops sorting.
#[tokio::test]
async fn guest_replays_the_witness_under_every_update_order() {
    let [first, second, third] = addresses_with_distinct_first_nibbles();
    assert!(
        first < second,
        "candidates are generated in ascending order"
    );
    replay_under_every_order(first, third, second).await;
    replay_under_every_order(second, third, first).await;
}
