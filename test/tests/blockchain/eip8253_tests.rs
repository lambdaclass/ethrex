//! EIP-8253: the first Hegota block sets the nonce of every account the chain config lists under
//! `zeroNonceStorageAccounts` to 1, before its system calls, and leaves balance, code and storage
//! alone. Its block access list records one nonce change per account at index 0.
//!
//! The blocks come from the payload builder and are imported again through the sequential and the
//! parallel (BAL) paths, so block building and every import path have to agree on the transition.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::{EMPTY_KECCAK_HASH, EMPTY_TRIE_HASH},
    types::{
        AccountState, Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis,
        GenesisAccount,
        block_access_list::{AccountChanges, BlockAccessList, NonceChange},
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

/// Listed, with the shape the EIP targets: zero nonce, no code, non-empty storage.
const LISTED: Address = Address::repeat_byte(0x53);
/// Listed but absent from the state, so the fork block creates it.
const LISTED_ABSENT: Address = Address::repeat_byte(0x54);
/// The targeted shape, but not listed.
const UNLISTED: Address = Address::repeat_byte(0x55);
const BALANCE: u64 = 1_000_000;
const BLOCK_TIME: u64 = 12;
/// Hegota activates this long after genesis. Block 1 comes before it, block 2 is the first block
/// at or past it (4 seconds past, so the fork block need not sit on the activation time), and
/// block 3 comes after the fork block.
const HEGOTA_DELAY: u64 = 20;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn targeted_shape() -> GenesisAccount {
    GenesisAccount {
        code: Bytes::new(),
        storage: BTreeMap::from([
            (U256::zero(), U256::from(0xdead)),
            (U256::one(), U256::from(7)),
        ]),
        balance: U256::from(BALANCE),
        nonce: 0,
    }
}

/// `l1-hegota.json` with `LISTED` and `UNLISTED` allocated, `accounts` written under the chain
/// config key the genesis generator uses (left out when `None`), and Hegota `hegota_delay`
/// seconds after genesis.
fn genesis(accounts: Option<&[Address]>, hegota_delay: u64) -> Genesis {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut json: serde_json::Value =
        serde_json::from_reader(BufReader::new(file)).expect("read l1-hegota genesis");
    if let Some(accounts) = accounts {
        json["config"]["zeroNonceStorageAccounts"] = serde_json::json!(accounts);
    }
    let mut genesis: Genesis = serde_json::from_value(json).expect("parse genesis");
    genesis.config.hegota_time = Some(genesis.timestamp + hegota_delay);
    for address in [LISTED, UNLISTED] {
        genesis.alloc.insert(address, targeted_shape());
    }
    genesis
}

async fn new_store(genesis: &Genesis) -> Store {
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis.clone())
        .await
        .expect("add genesis state");
    store
}

/// Builds `count` empty blocks on genesis with the payload builder, importing each so the next
/// one has its parent's state. Returns the store and each block with the BAL the builder
/// recorded for it.
async fn build_chain(genesis: &Genesis, count: u64) -> (Store, Vec<(Block, BlockAccessList)>) {
    let store = new_store(genesis).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let mut parent = store
        .get_block_header(0)
        .expect("read genesis header")
        .expect("genesis header is stored");
    let mut chain = Vec::new();
    for _ in 0..count {
        let args = BuildPayloadArgs {
            parent: parent.hash(),
            timestamp: parent.timestamp + BLOCK_TIME,
            fee_recipient: H160::zero(),
            random: H256::zero(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::zero()),
            slot_number: Some(parent.number + 1),
            version: 1,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        };
        let payload = create_payload(&args, &store, Bytes::new()).expect("create payload");
        let result = blockchain.build_payload(payload).expect("build payload");
        let bal = result
            .block_access_list
            .expect("an Amsterdam block has a BAL");
        let block = result.payload;
        blockchain
            .add_block(block.clone())
            .unwrap_or_else(|e| panic!("block {} must import: {e:?}", block.header.number));
        parent = block.header.clone();
        chain.push((block, bal));
    }
    (store, chain)
}

fn blockchain(store: Store, bal_parallel_exec_enabled: bool) -> Blockchain {
    Blockchain::new(
        store,
        BlockchainOptions {
            bal_parallel_exec_enabled,
            ..Default::default()
        },
    )
}

/// Imports `chain` into fresh stores through the sequential pipeline and through the parallel
/// path, which executes against each block's own BAL.
async fn import_through_both_pipelines(genesis: &Genesis, chain: &[(Block, BlockAccessList)]) {
    for parallel in [false, true] {
        let blockchain = blockchain(new_store(genesis).await, parallel);
        for (block, bal) in chain {
            blockchain
                .add_block_pipeline_bal(block.clone(), Some(Arc::new(bal.clone())))
                .unwrap_or_else(|e| {
                    panic!(
                        "block {} must import (parallel: {parallel}): {e:?}",
                        block.header.number
                    )
                });
        }
    }
}

fn account(store: &Store, header: &BlockHeader, address: Address) -> Option<AccountState> {
    store
        .get_account_state_by_root(header.state_root, address)
        .expect("read account state")
}

fn bal_entry(bal: &BlockAccessList, address: Address) -> Option<&AccountChanges> {
    bal.accounts().iter().find(|entry| entry.address == address)
}

/// The fork block's BAL entry of a listed account nothing else touches.
fn bump_only(address: Address) -> AccountChanges {
    AccountChanges::new(address).with_nonce_changes(vec![NonceChange {
        block_access_index: 0,
        post_nonce: 1,
    }])
}

#[test]
fn genesis_names_the_accounts_under_zero_nonce_storage_accounts() {
    let listed = genesis(Some(&[LISTED, LISTED_ABSENT]), HEGOTA_DELAY);
    assert_eq!(
        listed.config.zero_nonce_storage_accounts,
        [LISTED, LISTED_ABSENT]
    );

    // The store keeps the chain config as JSON, so the list must survive a round trip, and a
    // chain without one must not grow the key.
    let json = serde_json::to_string(&listed.config).expect("serialize chain config");
    assert!(json.contains("\"zeroNonceStorageAccounts\""));
    let reloaded: ethrex_common::types::ChainConfig =
        serde_json::from_str(&json).expect("parse chain config");
    assert_eq!(reloaded, listed.config);

    let unlisted = genesis(None, HEGOTA_DELAY);
    assert!(unlisted.config.zero_nonce_storage_accounts.is_empty());
    let json = serde_json::to_string(&unlisted.config).expect("serialize chain config");
    assert!(!json.contains("zeroNonceStorageAccounts"));
}

#[tokio::test]
async fn the_first_hegota_block_sets_each_listed_nonce_to_one() {
    let genesis = genesis(Some(&[LISTED, LISTED_ABSENT]), HEGOTA_DELAY);
    let (store, chain) = build_chain(&genesis, 3).await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let [
        (pre_fork, pre_fork_bal),
        (fork, fork_bal),
        (post_fork, post_fork_bal),
    ] = &chain[..]
    else {
        unreachable!("three blocks were built")
    };
    assert!(
        !genesis
            .config
            .is_hegota_activated(pre_fork.header.timestamp)
    );
    assert!(fork.header.timestamp > genesis.config.hegota_time.unwrap());

    let initial = account(&store, &genesis_header, LISTED).expect("LISTED is allocated");
    assert_eq!(initial.nonce, 0);
    assert_ne!(initial.storage_root, *EMPTY_TRIE_HASH);

    // Before Hegota nothing changes.
    assert_eq!(account(&store, &pre_fork.header, LISTED), Some(initial));
    assert_eq!(account(&store, &pre_fork.header, LISTED_ABSENT), None);
    assert!(bal_entry(pre_fork_bal, LISTED).is_none());

    // The fork block bumps the nonce and nothing else, and creates a listed account that is
    // missing from the state.
    let bumped = AccountState {
        nonce: 1,
        ..initial
    };
    assert_eq!(account(&store, &fork.header, LISTED), Some(bumped));
    assert_eq!(
        account(&store, &fork.header, LISTED_ABSENT),
        Some(AccountState {
            nonce: 1,
            balance: U256::zero(),
            storage_root: *EMPTY_TRIE_HASH,
            code_hash: *EMPTY_KECCAK_HASH,
        })
    );
    assert_eq!(account(&store, &fork.header, UNLISTED), Some(initial));
    assert_eq!(bal_entry(fork_bal, LISTED), Some(&bump_only(LISTED)));
    assert_eq!(
        bal_entry(fork_bal, LISTED_ABSENT),
        Some(&bump_only(LISTED_ABSENT))
    );
    assert!(bal_entry(fork_bal, UNLISTED).is_none());

    // Later blocks leave the accounts alone.
    assert_eq!(account(&store, &post_fork.header, LISTED), Some(bumped));
    assert!(bal_entry(post_fork_bal, LISTED).is_none());
    assert!(bal_entry(post_fork_bal, LISTED_ABSENT).is_none());

    import_through_both_pipelines(&genesis, &chain).await;
}

#[tokio::test]
async fn without_a_list_the_fork_block_changes_nothing() {
    let genesis = genesis(None, HEGOTA_DELAY);
    let (store, chain) = build_chain(&genesis, 2).await;
    let (fork, fork_bal) = &chain[1];
    assert!(
        genesis
            .config
            .is_hegota_fork_block(chain[0].0.header.timestamp, fork.header.timestamp)
    );

    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    for address in [LISTED, UNLISTED] {
        assert_eq!(
            account(&store, &fork.header, address),
            account(&store, &genesis_header, address)
        );
        assert!(bal_entry(fork_bal, address).is_none());
    }
    assert!(
        fork_bal
            .accounts()
            .iter()
            .all(|entry| entry.nonce_changes.is_empty())
    );

    import_through_both_pipelines(&genesis, &chain).await;
}

#[tokio::test]
async fn a_chain_that_starts_in_hegota_has_no_fork_block() {
    let genesis = genesis(Some(&[LISTED, LISTED_ABSENT]), 0);
    let (store, chain) = build_chain(&genesis, 2).await;

    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let initial = account(&store, &genesis_header, LISTED);
    for (block, bal) in &chain {
        assert_eq!(account(&store, &block.header, LISTED), initial);
        assert_eq!(account(&store, &block.header, LISTED_ABSENT), None);
        assert!(bal_entry(bal, LISTED).is_none());
    }

    import_through_both_pipelines(&genesis, &chain).await;
}

#[tokio::test]
async fn a_fork_block_whose_bal_omits_the_bump_is_rejected() {
    let genesis = genesis(Some(&[LISTED, LISTED_ABSENT]), HEGOTA_DELAY);
    let (_, chain) = build_chain(&genesis, 2).await;
    let (pre_fork, pre_fork_bal) = &chain[0];
    let (fork, fork_bal) = &chain[1];

    let trimmed = BlockAccessList::from_accounts(
        fork_bal
            .accounts()
            .iter()
            .filter(|entry| entry.address != LISTED)
            .cloned()
            .collect(),
    );
    let mut forged = fork.clone();
    forged.header.block_access_list_hash = Some(trimmed.compute_hash(&NativeCrypto));

    // The sequential path rebuilds the BAL and compares hashes. The parallel path takes state
    // from the BAL, so it checks the BAL covers everything the pre-transaction phase touched.
    for (parallel, reason) in [
        (false, "BlockAccessListHashMismatch"),
        (true, "missing from BAL"),
    ] {
        let blockchain = blockchain(new_store(&genesis).await, parallel);
        blockchain
            .add_block_pipeline_bal(pre_fork.clone(), Some(Arc::new(pre_fork_bal.clone())))
            .expect("the block before the fork imports");
        let err = blockchain
            .add_block_pipeline_bal(forged.clone(), Some(Arc::new(trimmed.clone())))
            .expect_err("a fork block whose BAL omits a bump must be rejected");
        assert!(
            format!("{err:?}").contains(reason),
            "rejected for the wrong reason (parallel: {parallel}): {err:?}"
        );
    }
}
