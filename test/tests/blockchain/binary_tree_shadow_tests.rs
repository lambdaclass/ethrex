//! Phase C — shadow tracking of the EIP-8297 binary trie.
//!
//! On a chain that has `binaryTreeTime` *scheduled* (set, but not yet
//! reached) every imported block must advance the persistent binary trie
//! alongside the MPT, so the binary state is complete and carried over when
//! the commitment activates. Nothing here validates a binary root against a
//! header — headers still commit MPT roots; that flip is Phase D.
//!
//! The properties under test:
//!
//! 1. after N blocks the recorded binary root equals a binary trie built
//!    from scratch over the same end state (the correctness heart);
//! 2. genesis seeds the first entry from the alloc, and block 1 extends
//!    *that* rather than an empty trie;
//! 3. an **unscheduled** chain does no binary-trie work at all;
//! 4. the plain, pipelined and batch import paths all agree;
//! 5. a missing parent entry is a hard error naming the parent, never a
//!    silent restart from an empty trie.

use std::collections::{BTreeMap, HashMap};
use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        AccountState, Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction,
        ELASTICITY_MULTIPLIER, Genesis, GenesisAccount, Transaction, TxKind,
    },
    utils::keccak,
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use tokio_util::sync::CancellationToken;

/// Test private key from fixtures/keys/private_keys_tests.txt.
const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 100_000;

/// Far enough ahead that `is_binary_tree_active` is false for every block any
/// test here builds: the whole point of Phase C is the *scheduled but not yet
/// active* regime.
const FAR_FUTURE_BINARY_TREE_TIME: u64 = 4_000_000_000;

/// Recipient of the value transfers the helpers below build.
fn test_recipient() -> Address {
    Address::from_low_u64_be(0xBEEF)
}

/// Fee recipient used by `build_block`; it accrues priority fees, so it is part
/// of the end state the oracle must reconstruct.
fn test_coinbase() -> Address {
    H160::zero()
}

fn test_secret_key() -> SecretKey {
    SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap()
}

fn sender_from_key(sk: &SecretKey) -> Address {
    LocalSigner::new(*sk).address
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Load the execution-api genesis, fund `sender`, and optionally schedule the
/// binary-tree commitment at `binary_tree_time`.
fn load_funded_genesis(sender: Address, binary_tree_time: Option<u64>) -> Genesis {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    genesis.alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10).pow(U256::from(20)), // 100 ETH
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );
    genesis.config.binary_tree_time = binary_tree_time;
    genesis
}

async fn store_from_genesis(genesis: Genesis) -> Store {
    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    store
}

/// Build a block on top of `parent_header`, including whatever is in the mempool.
async fn build_block(store: &Store, blockchain: &Blockchain, parent_header: &BlockHeader) -> Block {
    let args = BuildPayloadArgs {
        parent: parent_header.hash(),
        timestamp: parent_header.timestamp + 12,
        fee_recipient: test_coinbase(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };

    let block = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(block).unwrap().payload
}

async fn transfer_tx(chain_id: u64, nonce: u64, signer: &Signer) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to: TxKind::Call(test_recipient()),
        value: U256::from(1u64),
        data: Bytes::new(),
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

/// Build `count` blocks on top of genesis, importing each through
/// `add_block` so the next one can be built on the resulting state.
///
/// Returns the built blocks in order.
async fn build_chain(
    store: &Store,
    blockchain: &Blockchain,
    chain_id: u64,
    count: u64,
) -> Vec<Block> {
    let sk = test_secret_key();
    let signer: Signer = LocalSigner::new(sk).into();

    let mut parent_header = store.get_block_header(0).unwrap().unwrap();
    let mut blocks = Vec::with_capacity(count as usize);
    for nonce in 0..count {
        let tx = transfer_tx(chain_id, nonce, &signer).await;
        blockchain
            .add_transaction_to_pool(tx)
            .await
            .expect("tx should enter pool");

        let block = build_block(store, blockchain, &parent_header).await;
        blockchain
            .add_block(block.clone())
            .expect("block should import");
        blockchain
            .remove_block_transactions_from_pool(&block)
            .expect("remove block txs from pool");
        parent_header = block.header.clone();
        blocks.push(block);
    }
    blocks
}

// ---------------------------------------------------------------------------
// The independent oracle: reconstruct the end state from the MPT, then build a
// binary trie over it from scratch through the genesis path.
// ---------------------------------------------------------------------------

/// Storage slots any of these blocks can touch are small integers: EIP-4788
/// uses `timestamp % 8191` and `8191 + timestamp % 8191`, EIP-2935 uses
/// `number % 8191`, the request-queue contracts use single-digit indices, and
/// the fixture's own alloc uses 1/2/3. Precomputing keccak over `0..16384`
/// covers all of them; anything outside makes the reconstruction assert.
fn slot_preimages() -> HashMap<H256, U256> {
    (0u64..16_384)
        .map(|slot| {
            let value = U256::from(slot);
            (keccak(H256(value.to_big_endian()).as_bytes()), value)
        })
        .collect()
}

/// Rebuild `genesis.alloc`-shaped end state at `block_hash` by walking the MPT.
///
/// `known_addresses` must contain every address present in the final state;
/// the walk asserts that, so a state the test did not anticipate fails loudly
/// instead of silently producing a smaller (and trivially matching) alloc.
fn end_state_as_alloc(
    store: &Store,
    block_hash: H256,
    known_addresses: &[Address],
) -> BTreeMap<Address, GenesisAccount> {
    let by_hash: HashMap<H256, Address> = known_addresses
        .iter()
        .map(|address| (keccak(address.as_bytes()), *address))
        .collect();
    let slots = slot_preimages();

    let state_trie = store
        .state_trie(block_hash)
        .expect("state trie read")
        .expect("state trie present");

    let mut alloc = BTreeMap::new();
    for (path, value) in state_trie.into_iter().content() {
        let hashed_address = H256::from_slice(&path);
        let address = *by_hash.get(&hashed_address).unwrap_or_else(|| {
            panic!("end state holds an account this test cannot name: {hashed_address:#x}")
        });
        let account = AccountState::decode(&value).expect("decode account state");

        let code = store
            .get_account_code(account.code_hash)
            .expect("code read")
            .map(|code| Bytes::copy_from_slice(code.code()))
            .unwrap_or_default();

        let mut storage = BTreeMap::new();
        let storage_trie = store
            .storage_trie(block_hash, address)
            .expect("storage trie read")
            .expect("storage trie present");
        for (slot_path, slot_value) in storage_trie.into_iter().content() {
            let hashed_slot = H256::from_slice(&slot_path);
            let slot = *slots.get(&hashed_slot).unwrap_or_else(|| {
                panic!("end state holds a storage slot this test cannot name: {hashed_slot:#x}")
            });
            let value = U256::decode(&slot_value).expect("decode storage value");
            storage.insert(slot, value);
        }

        alloc.insert(
            address,
            GenesisAccount {
                code,
                storage,
                balance: account.balance,
                nonce: account.nonce,
            },
        );
    }
    alloc
}

// ---------------------------------------------------------------------------
// 1. Correctness: incremental advance == from-scratch trie over the end state.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scheduled_chain_binary_root_matches_a_trie_built_over_the_end_state() {
    let sender = sender_from_key(&test_secret_key());
    let genesis = load_funded_genesis(sender, Some(FAR_FUTURE_BINARY_TREE_TIME));
    let chain_id = genesis.config.chain_id;

    let store = store_from_genesis(genesis.clone()).await;
    let blockchain = Blockchain::default_with_store(store.clone());

    let blocks = build_chain(&store, &blockchain, chain_id, 3).await;
    let head = blocks.last().unwrap();

    let recorded = store
        .get_binary_trie_root(head.hash())
        .expect("binary root read")
        .expect("a scheduled chain must record a binary root for every block");

    // Independent oracle: read the end state back out of the MPT and build a
    // fresh binary trie over it, from empty, through the genesis path.
    let alloc = end_state_as_alloc(&store, head.hash(), &known_addresses());

    // Guard against a vacuous pass: the comparison is only meaningful if the
    // reconstructed end state is the real, mutated one. The recipient did not
    // exist at genesis and must now hold exactly the three transferred wei, and
    // the whole genesis alloc must still be there.
    assert_eq!(
        alloc
            .get(&test_recipient())
            .expect("the transfer recipient must exist in the end state")
            .balance,
        U256::from(3u64),
        "the end state must reflect all three transfers"
    );
    assert!(
        alloc.len() > genesis.alloc.len(),
        "the end state must hold at least the genesis alloc plus the new recipient (got {} vs {})",
        alloc.len(),
        genesis.alloc.len()
    );
    let oracle_store = Store::new("oracle.db", EngineType::InMemory).expect("oracle store");
    let from_scratch = oracle_store
        .setup_genesis_binary_trie(alloc)
        .await
        .expect("build oracle binary trie");

    assert_eq!(
        recorded,
        from_scratch,
        "the shadow-tracked binary root after {} blocks must equal a binary trie built from scratch over the same end state",
        blocks.len()
    );
}

// ---------------------------------------------------------------------------
// 2. Genesis seeding, and block 1 extending it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn genesis_seeds_the_binary_trie_and_the_first_block_extends_it() {
    let sender = sender_from_key(&test_secret_key());
    let genesis = load_funded_genesis(sender, Some(FAR_FUTURE_BINARY_TREE_TIME));
    let chain_id = genesis.config.chain_id;
    let alloc = genesis.alloc.clone();

    let store = store_from_genesis(genesis).await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_hash = genesis_header.hash();

    let genesis_binary_root = store
        .get_binary_trie_root(genesis_hash)
        .expect("binary root read")
        .expect("genesis must record a binary root on a scheduled chain");

    // It is the alloc's root, not an empty trie.
    let expected = {
        let oracle = Store::new("oracle-genesis.db", EngineType::InMemory).expect("oracle store");
        oracle
            .setup_genesis_binary_trie(alloc)
            .await
            .expect("seed oracle")
    };
    assert_eq!(
        genesis_binary_root, expected,
        "genesis binary root must be the trie over the genesis alloc"
    );

    // Block 1 must extend the *seeded* root. Applying block 1's updates on top
    // of genesis gives the recorded root; applying them on top of an empty trie
    // must not.
    let blockchain = Blockchain::default_with_store(store.clone());
    let blocks = build_chain(&store, &blockchain, chain_id, 1).await;
    let block_root = store
        .get_binary_trie_root(blocks[0].hash())
        .expect("binary root read")
        .expect("block 1 must record a binary root");

    assert_ne!(
        block_root, genesis_binary_root,
        "block 1 changes state, so its binary root must differ from genesis'"
    );

    // Reconstructing block 1's end state from scratch reproduces `block_root`,
    // which is only possible if block 1 built on the genesis alloc rather than
    // starting empty.
    let alloc = end_state_as_alloc(&store, blocks[0].hash(), &known_addresses());
    let oracle_store = Store::new("oracle-block1.db", EngineType::InMemory).expect("oracle store");
    let from_scratch = oracle_store
        .setup_genesis_binary_trie(alloc)
        .await
        .expect("build oracle binary trie");
    assert_eq!(
        block_root, from_scratch,
        "block 1's binary root must be the full carried-over state, not just its own diff"
    );
}

/// Every address any chain built here can end up holding: the genesis alloc
/// (system contracts included), the funded sender, the transfer recipient and
/// the fee recipient.
fn known_addresses() -> Vec<Address> {
    let genesis = load_funded_genesis(sender_from_key(&test_secret_key()), None);
    let mut addresses: Vec<Address> = genesis.alloc.keys().copied().collect();
    addresses.push(test_recipient());
    addresses.push(test_coinbase());
    addresses
}

// ---------------------------------------------------------------------------
// 3. Unscheduled chains do nothing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unscheduled_chain_does_no_binary_trie_work() {
    let sender = sender_from_key(&test_secret_key());
    let genesis = load_funded_genesis(sender, None);
    let chain_id = genesis.config.chain_id;

    let store = store_from_genesis(genesis).await;
    let blockchain = Blockchain::default_with_store(store.clone());

    let blocks = build_chain(&store, &blockchain, chain_id, 3).await;
    // Also drive the pipelined path on a fresh unscheduled store.
    let pipelined_store = store_from_genesis(load_funded_genesis(sender, None)).await;
    let pipelined_chain = Blockchain::default_with_store(pipelined_store.clone());
    for block in &blocks {
        pipelined_chain
            .add_block_pipeline(block.clone(), None)
            .expect("pipelined import on an unscheduled chain");
    }

    for (label, store) in [("plain", &store), ("pipelined", &pipelined_store)] {
        assert_eq!(
            store.binary_trie_node_count_for_test().unwrap(),
            0,
            "{label}: an unscheduled chain must write no binary-trie nodes"
        );
        assert_eq!(
            store.binary_trie_root_count_for_test().unwrap(),
            0,
            "{label}: an unscheduled chain must record no binary-trie roots"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Every import path agrees.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_import_paths_produce_the_same_binary_roots() {
    let sender = sender_from_key(&test_secret_key());
    let genesis = load_funded_genesis(sender, Some(FAR_FUTURE_BINARY_TREE_TIME));
    let chain_id = genesis.config.chain_id;

    // Path A: plain `add_block`.
    let plain_store = store_from_genesis(genesis.clone()).await;
    let plain_chain = Blockchain::default_with_store(plain_store.clone());
    let blocks = build_chain(&plain_store, &plain_chain, chain_id, 3).await;
    let plain_roots: Vec<H256> = blocks
        .iter()
        .map(|block| {
            plain_store
                .get_binary_trie_root(block.hash())
                .unwrap()
                .expect("plain path must record a binary root")
        })
        .collect();

    // Path B: `add_block_pipeline` (the engine-API route).
    let pipeline_store = store_from_genesis(genesis.clone()).await;
    let pipeline_chain = Blockchain::default_with_store(pipeline_store.clone());
    for block in &blocks {
        pipeline_chain
            .add_block_pipeline(block.clone(), None)
            .expect("pipelined import");
    }
    let pipeline_roots: Vec<H256> = blocks
        .iter()
        .map(|block| {
            pipeline_store
                .get_binary_trie_root(block.hash())
                .unwrap()
                .expect("pipelined path must record a binary root")
        })
        .collect();

    // Path C: `add_blocks_in_batch` (full sync / block import).
    let batch_store = store_from_genesis(genesis).await;
    let batch_chain = Blockchain::default_with_store(batch_store.clone());
    batch_chain
        .add_blocks_in_batch(blocks.clone(), &[], CancellationToken::new())
        .await
        .expect("batch import");
    let batch_roots: Vec<H256> = blocks
        .iter()
        .map(|block| {
            batch_store
                .get_binary_trie_root(block.hash())
                .unwrap()
                .expect("batch path must record a binary root")
        })
        .collect();

    assert_eq!(
        plain_roots, pipeline_roots,
        "the pipelined path must shadow-track the same binary roots as the plain path"
    );
    assert_eq!(
        plain_roots, batch_roots,
        "batch import must shadow-track the same binary roots as the plain path"
    );
}

/// The BAL-driven parallel-trie route through the pipeline, which is the one
/// live Amsterdam blocks actually take.
///
/// This is the path an earlier branch got wrong. With a BAL in hand and
/// `bal_parallel_trie_enabled` (the default), the merkleizer normally skips the
/// streaming channel and builds the MPT from BAL-synthesized updates — which
/// never carry `removed` / `removed_storage` and so cannot describe account
/// deletion to the binary trie. A scheduled chain must therefore force the
/// streaming branch, exactly as witness collection does. Without that, this
/// test reaches `store_block_with_depth` with no raw updates at all.
#[tokio::test]
async fn scheduled_chain_shadow_tracks_on_the_bal_parallel_trie_path() {
    async fn amsterdam_store(binary_tree_time: Option<u64>) -> Store {
        let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
            .expect("open l1-bal genesis");
        let mut genesis: Genesis =
            serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
        genesis.config.binary_tree_time = binary_tree_time;
        store_from_genesis(genesis).await
    }

    // Build one valid Amsterdam block plus the canonical BAL its producer recorded.
    let build_store = amsterdam_store(None).await;
    let builder = Blockchain::default_with_store(build_store.clone());
    let genesis_header = build_store.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: test_coinbase(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        // EIP-7843: Amsterdam headers must carry a slot number.
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, &build_store, Bytes::new()).unwrap();
    let built = builder.build_payload(payload).unwrap();
    let block = built.payload;
    let bal = std::sync::Arc::new(
        built
            .block_access_list
            .expect("an Amsterdam block must produce a BAL"),
    );
    assert!(
        !bal.accounts().is_empty(),
        "the BAL must be non-empty (EIP-4788 system call), else the parallel-trie path is not exercised"
    );

    // Plain path, for the expected root.
    let plain_store = amsterdam_store(Some(FAR_FUTURE_BINARY_TREE_TIME)).await;
    Blockchain::default_with_store(plain_store.clone())
        .add_block(block.clone())
        .expect("plain import");
    let expected = plain_store
        .get_binary_trie_root(block.hash())
        .unwrap()
        .expect("plain path must record a binary root");

    // Pipelined path with the BAL supplied — the parallel-trie route.
    let pipeline_store = amsterdam_store(Some(FAR_FUTURE_BINARY_TREE_TIME)).await;
    Blockchain::default_with_store(pipeline_store.clone())
        .add_block_pipeline(block.clone(), Some(bal))
        .expect("BAL-driven pipelined import must still shadow-track");
    let actual = pipeline_store
        .get_binary_trie_root(block.hash())
        .unwrap()
        .expect("the BAL-driven path must record a binary root");

    assert_eq!(
        expected, actual,
        "the BAL parallel-trie path must shadow-track the same binary root as the plain path"
    );
}

// ---------------------------------------------------------------------------
// 5. A missing parent entry is a hard error.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_missing_parent_binary_root_is_a_hard_error() {
    let sender = sender_from_key(&test_secret_key());
    let genesis = load_funded_genesis(sender, Some(FAR_FUTURE_BINARY_TREE_TIME));
    let store = store_from_genesis(genesis).await;

    let unknown_parent = H256::repeat_byte(0xAB);
    let err = store
        .advance_binary_trie_for_block(H256::repeat_byte(0xCD), unknown_parent, &[])
        .expect_err("a scheduled chain must refuse to extend an unknown parent");

    let message = err.to_string();
    assert!(
        message.contains(&format!("{unknown_parent:#x}")),
        "the error must name the parent whose binary root is missing, got: {message}"
    );
}
