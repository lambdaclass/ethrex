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
//!
//! The second half of the file covers **Phase D — the flip**: from the first
//! block whose timestamp reaches `binaryTreeTime`, `header.state_root` *is*
//! the binary root, validated on import and produced by payload building,
//! while every earlier header keeps committing (and resolving through) the
//! MPT root forever. Its properties:
//!
//! 1. the flip lands on the first block at or after the timestamp, and not on
//!    the one before;
//! 2. that block commits a binary trie over the *full* carried-over state;
//! 3. a wrong root on an active block is rejected and records nothing;
//! 4. pre-activation blocks keep validating and reading through the MPT after
//!    the flip — the per-header rule, and the falsification target;
//! 5. payload building answers for the payload's own timestamp;
//! 6. competing branches across the boundary each resolve their own root.
//!
//! The last section covers **Phase D3 — execution reads through the binary
//! trie**: a `StoreVmDatabase` opened at an active header resolves accounts and
//! storage out of the binary trie at that header's root, so the chain continues
//! past the flip, while one opened at any earlier header keeps reading the MPT.
//! No block-hash -> MPT-root registry is involved anywhere.

use std::collections::{BTreeMap, HashMap};
use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    error::{ChainError, InvalidBlockError},
    payload::{BuildPayloadArgs, create_payload},
    vm::StoreVmDatabase,
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        AccountState, Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction,
        ELASTICITY_MULTIPLIER, Genesis, GenesisAccount, Transaction, TxKind,
    },
    utils::keccak,
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::VmDatabase;
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

/// The cadence `build_block` uses, so a test can predict block N's timestamp
/// as `genesis.timestamp + N * BLOCK_TIME` and place an activation between two
/// of them.
const BLOCK_TIME: u64 = 12;

/// Block index (1-based) whose timestamp the Phase D tests schedule activation
/// at exactly, so the block before it is the last MPT-committed one.
const FLIP_BLOCK: u64 = 3;

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
    load_funded_genesis_with(sender, binary_tree_time, &[])
}

/// [`load_funded_genesis`] with `extra` accounts added to the alloc, for tests
/// that need a state shape the fixture does not have.
fn load_funded_genesis_with(
    sender: Address,
    binary_tree_time: Option<u64>,
    extra: &[(Address, GenesisAccount)],
) -> Genesis {
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
    for (address, account) in extra {
        genesis.alloc.insert(*address, account.clone());
    }
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
    build_block_at(
        store,
        blockchain,
        parent_header,
        parent_header.timestamp + BLOCK_TIME,
    )
    .await
}

/// [`build_block`] with an explicit timestamp, so a test can place a payload on
/// either side of an activation boundary.
async fn build_block_at(
    store: &Store,
    blockchain: &Blockchain,
    parent_header: &BlockHeader,
    timestamp: u64,
) -> Block {
    let args = BuildPayloadArgs {
        parent: parent_header.hash(),
        timestamp,
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

        // Reads are unaffected too: with no schedule, no header can be active,
        // so every state read resolves through the MPT exactly as before.
        let head = blocks.last().unwrap();
        let db = StoreVmDatabase::new(store.clone(), head.header.clone())
            .unwrap_or_else(|err| panic!("{label}: the head must open against the MPT: {err:?}"));
        assert_eq!(
            db.get_account_state(sender_from_key(&test_secret_key()))
                .unwrap()
                .unwrap()
                .nonce,
            blocks.len() as u64,
            "{label}: the MPT read must show one transfer per block"
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

// ===========================================================================
// Phase D — the flip.
//
// From the first block at or after `binaryTreeTime`, `header.state_root` is
// the binary-trie root. Everything below turns on the *per-header* rule:
// activation is asked of the header being resolved, never of the chain. A
// chain-level "is scheduled" flag would make pre-activation blocks commit (and
// demand) a binary root, which is exactly what
// `pre_activation_blocks_still_validate_against_the_mpt_root` falsifies.
// ===========================================================================

/// A scheduled chain whose activation lands on block [`FLIP_BLOCK`], paired
/// with an **unscheduled** twin built from the same genesis with the same
/// transactions at the same timestamps.
///
/// The twin is the reference for "what an MPT-committing node produces". It is
/// deliberately unscheduled rather than scheduled-far-out, so that a
/// chain-level activation check would visibly diverge the two chains from
/// block 1 instead of from the flip block.
struct BoundaryChains {
    scheduled_genesis: Genesis,
    scheduled_store: Store,
    scheduled_blocks: Vec<Block>,
    twin_store: Store,
    twin_blocks: Vec<Block>,
    /// `genesis.timestamp + FLIP_BLOCK * BLOCK_TIME`: block `FLIP_BLOCK`'s own
    /// timestamp, so it is the first active block and block `FLIP_BLOCK - 1`
    /// is the last MPT-committed one.
    activation: u64,
}

async fn build_boundary_chains(count: u64) -> BoundaryChains {
    let sender = sender_from_key(&test_secret_key());
    let unscheduled = load_funded_genesis(sender, None);
    let activation = unscheduled.timestamp + FLIP_BLOCK * BLOCK_TIME;

    let scheduled_genesis = load_funded_genesis(sender, Some(activation));
    let chain_id = scheduled_genesis.config.chain_id;

    let scheduled_store = store_from_genesis(scheduled_genesis.clone()).await;
    let scheduled_chain = Blockchain::default_with_store(scheduled_store.clone());
    let scheduled_blocks = build_chain(&scheduled_store, &scheduled_chain, chain_id, count).await;

    let twin_store = store_from_genesis(unscheduled).await;
    let twin_chain = Blockchain::default_with_store(twin_store.clone());
    let twin_blocks = build_chain(&twin_store, &twin_chain, chain_id, count).await;

    BoundaryChains {
        scheduled_genesis,
        scheduled_store,
        scheduled_blocks,
        twin_store,
        twin_blocks,
        activation,
    }
}

/// The recorded binary root for `block`, which must exist on a scheduled chain.
fn binary_root(store: &Store, block: &Block) -> H256 {
    store
        .get_binary_trie_root(block.hash())
        .expect("binary root read")
        .unwrap_or_else(|| panic!("block {} must record a binary root", block.header.number))
}

// ---------------------------------------------------------------------------
// D.1 The flip lands on the first block at or after the timestamp — and not on
//     the one before or the one after.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_flip_lands_on_the_first_block_at_or_after_the_activation_timestamp() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    let flip_index = (FLIP_BLOCK - 1) as usize;
    let blocks = &chains.scheduled_blocks;

    // The boundary really is between these two blocks, otherwise nothing below
    // means what it says.
    let last_pre = &blocks[flip_index - 1];
    let flip = &blocks[flip_index];
    assert!(
        last_pre.header.timestamp < chains.activation,
        "block {} must be strictly before activation",
        last_pre.header.number
    );
    assert!(
        flip.header.timestamp >= chains.activation,
        "block {} must be at or after activation",
        flip.header.number
    );

    // Every pre-activation block is byte-identical to the one an unscheduled
    // node produced: same hash, therefore the same MPT state root.
    for (block, twin) in blocks[..flip_index]
        .iter()
        .zip(chains.twin_blocks[..flip_index].iter())
    {
        assert_eq!(
            block.hash(),
            twin.hash(),
            "pre-activation block {} must be identical to the one an unscheduled node produces",
            block.header.number
        );
        assert_ne!(
            block.header.state_root,
            binary_root(&chains.scheduled_store, block),
            "pre-activation block {} must not commit the binary root",
            block.header.number
        );
    }

    // The flip block commits the binary root, and it is not the MPT root the
    // unscheduled twin committed at the same height off the same parent.
    assert_eq!(
        flip.header.state_root,
        binary_root(&chains.scheduled_store, flip),
        "the first active block must commit the binary root"
    );
    assert_eq!(
        flip.header.parent_hash, chains.twin_blocks[flip_index].header.parent_hash,
        "the two chains must still share the flip block's parent"
    );
    assert_ne!(
        flip.header.state_root, chains.twin_blocks[flip_index].header.state_root,
        "the flip block's root must differ from the MPT root at the same height"
    );
}

// ---------------------------------------------------------------------------
// D.2 Carry-over: the flip block commits the *full* state.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_flip_block_commits_a_binary_trie_over_the_full_carried_over_state() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    let flip_index = (FLIP_BLOCK - 1) as usize;
    let flip = chains.scheduled_blocks.last().unwrap();
    let twin = &chains.twin_blocks[flip_index];

    // The twin executed the same block off the same parent, so its MPT holds
    // exactly the flip block's end state. Pin that down before relying on it:
    // the two headers must agree on everything except the state root.
    assert_eq!(flip.header.parent_hash, twin.header.parent_hash);
    assert_eq!(flip.header.timestamp, twin.header.timestamp);
    assert_eq!(flip.header.transactions_root, twin.header.transactions_root);
    assert_eq!(flip.header.receipts_root, twin.header.receipts_root);
    assert_eq!(flip.header.gas_used, twin.header.gas_used);
    assert_eq!(flip.header.logs_bloom, twin.header.logs_bloom);
    assert_ne!(flip.header.state_root, twin.header.state_root);

    let alloc = end_state_as_alloc(&chains.twin_store, twin.hash(), &known_addresses());

    // Guard against a vacuous pass: the state must be the real, mutated one.
    assert_eq!(
        alloc
            .get(&test_recipient())
            .expect("the transfer recipient must exist in the end state")
            .balance,
        U256::from(FLIP_BLOCK),
        "the end state must reflect one transfer per block"
    );
    assert!(
        alloc.len() > chains.scheduled_genesis.alloc.len(),
        "the end state must hold at least the genesis alloc plus the new recipient"
    );

    let oracle = Store::new("oracle-flip.db", EngineType::InMemory).expect("oracle store");
    let from_scratch = oracle
        .setup_genesis_binary_trie(alloc)
        .await
        .expect("build oracle binary trie");

    assert_eq!(
        flip.header.state_root, from_scratch,
        "the flip block must commit a binary trie over the full carried-over state, \
         not an empty-start overlay of its own diff"
    );
}

// ---------------------------------------------------------------------------
// D.3 A wrong root is rejected, and leaves nothing behind.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_tampered_active_state_root_is_rejected_and_records_no_binary_root() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    let flip_index = (FLIP_BLOCK - 1) as usize;

    // Replay the pre-activation blocks into a fresh scheduled store, then offer
    // a flip block whose state root has been corrupted.
    let store = store_from_genesis(chains.scheduled_genesis.clone()).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    for block in &chains.scheduled_blocks[..flip_index] {
        blockchain
            .add_block(block.clone())
            .expect("pre-activation blocks must import");
    }

    let mut tampered = chains.scheduled_blocks[flip_index].clone();
    tampered.header.state_root = H256::repeat_byte(0x77);
    // The header caches its hash; a mutated header must re-derive it.
    tampered.header.hash = Default::default();
    assert_ne!(
        tampered.hash(),
        chains.scheduled_blocks[flip_index].hash(),
        "the tampered block must hash differently"
    );

    let err = blockchain
        .add_block(tampered.clone())
        .expect_err("an active block with a wrong state root must be rejected");
    assert!(
        matches!(
            err,
            ChainError::InvalidBlock(InvalidBlockError::StateRootMismatch)
        ),
        "expected a state-root mismatch, got: {err:?}"
    );

    assert_eq!(
        store
            .get_binary_trie_root(tampered.hash())
            .expect("binary root read"),
        None,
        "a rejected block must leave no recorded binary root behind"
    );
}

// ---------------------------------------------------------------------------
// D.4 The per-header rule: pre-activation blocks keep resolving through the MPT.
//
// This pair is the falsification target. Swap `is_binary_tree_active(header
// .timestamp)` for a chain-level `binary_tree_scheduled()` and both fail:
// blocks produced by an MPT-committing node stop validating, and pre-flip
// state stops being readable once the flip has happened.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pre_activation_blocks_still_validate_against_the_mpt_root() {
    // The twin has no schedule at all, so its blocks commit MPT roots. A node
    // whose activation is still in the future must accept them unchanged.
    let chains = build_boundary_chains(FLIP_BLOCK - 1).await;

    let store = store_from_genesis(chains.scheduled_genesis.clone()).await;
    let blockchain = Blockchain::default_with_store(store);
    for block in &chains.twin_blocks {
        assert!(
            block.header.timestamp < chains.activation,
            "this test is only about pre-activation blocks"
        );
        blockchain.add_block(block.clone()).unwrap_or_else(|err| {
            panic!(
                "pre-activation block {} must validate against the MPT root, got: {err:?}",
                block.header.number
            )
        });
    }
}

#[tokio::test]
async fn pre_activation_state_stays_readable_after_the_flip() {
    let chains = build_boundary_chains(FLIP_BLOCK).await;
    let sender = sender_from_key(&test_secret_key());
    let hashed_sender = keccak(sender.as_bytes());

    // The flip has happened: the head commits a binary root.
    let head = chains.scheduled_blocks.last().unwrap();
    assert_eq!(
        head.header.state_root,
        binary_root(&chains.scheduled_store, head)
    );

    // Every pre-activation block must still resolve its state through the MPT,
    // addressed by the root its own header carries.
    for (index, block) in chains.scheduled_blocks.iter().enumerate() {
        if block.header.timestamp >= chains.activation {
            break;
        }
        let state_trie = chains
            .scheduled_store
            .state_trie(block.hash())
            .expect("state trie read")
            .unwrap_or_else(|| {
                panic!(
                    "pre-activation block {} must still have a readable MPT",
                    block.header.number
                )
            });
        let encoded = state_trie
            .get(hashed_sender.as_bytes())
            .expect("state trie lookup")
            .unwrap_or_else(|| {
                panic!(
                    "the funded sender must be present at pre-activation block {}",
                    block.header.number
                )
            });
        let account = AccountState::decode(&encoded).expect("decode account state");
        assert_eq!(
            account.nonce,
            index as u64 + 1,
            "block {} must show exactly one transfer per block so far",
            block.header.number
        );
    }
}

// ---------------------------------------------------------------------------
// D.5 Payload building commits the root its own timestamp calls for.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn payload_building_resolves_the_root_from_the_payload_timestamp() {
    // Two blocks, both pre-activation, so both chains still agree on the head
    // the payloads are built on.
    let chains = build_boundary_chains(FLIP_BLOCK - 1).await;
    let parent = chains.scheduled_blocks.last().unwrap().header.clone();
    let twin_parent = chains.twin_blocks.last().unwrap().header.clone();
    assert_eq!(parent.hash(), twin_parent.hash());

    let scheduled_chain = Blockchain::default_with_store(chains.scheduled_store.clone());
    let twin_chain = Blockchain::default_with_store(chains.twin_store.clone());

    // One second before the boundary: still the MPT root, byte-identical to
    // what an unscheduled node builds.
    let before = build_block_at(
        &chains.scheduled_store,
        &scheduled_chain,
        &parent,
        chains.activation - 1,
    )
    .await;
    let twin_before = build_block_at(
        &chains.twin_store,
        &twin_chain,
        &twin_parent,
        chains.activation - 1,
    )
    .await;
    assert_eq!(
        before.header.state_root, twin_before.header.state_root,
        "a payload one second before the boundary must still commit the MPT root"
    );

    // At the boundary second: the binary root instead.
    let at = build_block_at(
        &chains.scheduled_store,
        &scheduled_chain,
        &parent,
        chains.activation,
    )
    .await;
    let twin_at = build_block_at(
        &chains.twin_store,
        &twin_chain,
        &twin_parent,
        chains.activation,
    )
    .await;
    assert_ne!(
        at.header.state_root, twin_at.header.state_root,
        "a payload at the boundary second must not commit the MPT root"
    );

    // Producer and validator must agree: importing the payload validates its
    // own root against shadow tracking's answer.
    scheduled_chain
        .add_block(at.clone())
        .expect("a payload built at the boundary must validate on import");
    assert_eq!(
        at.header.state_root,
        binary_root(&chains.scheduled_store, &at),
        "the payload's root must be the binary root the importer computes"
    );
}

// ---------------------------------------------------------------------------
// D.6 Reorging across the boundary.
//
// Two competing first-active blocks off the same last pre-activation parent.
// Each must validate against *its own* binary root — the parent is pre-flip, so
// both branches start from the same recorded root and diverge from there.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn competing_branches_across_the_boundary_each_resolve_their_own_root() {
    let chains = build_boundary_chains(FLIP_BLOCK - 1).await;
    let parent = chains.scheduled_blocks.last().unwrap().header.clone();
    let blockchain = Blockchain::default_with_store(chains.scheduled_store.clone());

    // Two flip-block candidates on the same parent, differing only in
    // timestamp — enough to give them different state (EIP-4788 writes the
    // beacon root at `timestamp % 8191`) and therefore different roots.
    let branch_a = build_block_at(
        &chains.scheduled_store,
        &blockchain,
        &parent,
        chains.activation,
    )
    .await;
    blockchain
        .add_block(branch_a.clone())
        .expect("the first branch's flip block must import");

    let branch_b = build_block_at(
        &chains.scheduled_store,
        &blockchain,
        &parent,
        chains.activation + 1,
    )
    .await;
    assert_ne!(branch_a.hash(), branch_b.hash());
    assert_eq!(branch_a.header.parent_hash, branch_b.header.parent_hash);

    blockchain
        .add_block(branch_b.clone())
        .expect("the sibling branch's flip block must import too");

    for block in [&branch_a, &branch_b] {
        assert_eq!(
            block.header.state_root,
            binary_root(&chains.scheduled_store, block),
            "each branch must resolve the binary root recorded for its own block hash"
        );
    }
    assert_ne!(
        binary_root(&chains.scheduled_store, &branch_a),
        binary_root(&chains.scheduled_store, &branch_b),
        "the two branches must not collapse onto the same recorded root"
    );
}

// ===========================================================================
// Phase D3 — execution reads through the binary trie.
//
// Phase D flipped what a header *commits to*. D3 flips how state is *resolved*:
// a `StoreVmDatabase` opened at a header whose timestamp has reached
// `binaryTreeTime` reads accounts and storage out of the binary trie at
// `header.state_root`, and one opened at any earlier header keeps reading the
// MPT at its own root — forever, across restarts and reorgs. There is no
// block-hash -> MPT-root registry anywhere: each header names the trie that
// answers for it.
//
// The properties:
//
// 1. the chain keeps going past the flip: blocks built on a binary-committed
//    parent execute, import and commit their own binary roots, on every import
//    path;
// 2. state written before the flip reads back correctly after it, and
//    transactions that read it and write it produce the same end state an
//    MPT-committing node produces;
// 3. pre-flip headers still resolve through the MPT after the flip has
//    happened, including for a *new* block built on a pre-flip parent;
// 4. an unscheduled chain still does nothing at all (the existing zero-work
//    assertions, above);
// 5. and the one thing a binary read cannot answer honestly: `storage_root`.
// ===========================================================================

/// How far past the flip block the D3 tests drive the chain.
///
/// Two blocks would cover the seams — the flip block's own child, which is the
/// first block whose *parent* commits a binary root, and one whose parent was
/// itself executed against the binary trie. A dozen is cheap (the whole file
/// runs in well under a second) and keeps the chain going long enough that
/// nothing here depends on the pre-flip state still being one block away.
const BLOCKS_PAST_THE_FLIP: u64 = 12;

/// A genesis-alloc account of the fixture that holds storage — slots 1, 2 and 3
/// — so a post-flip read has pre-flip storage to find.
fn storage_fixture_account() -> Address {
    Address::from_slice(&hex::decode("8bebc8ba651aee624937e7d897853ac30c95a067").unwrap())
}

// ---------------------------------------------------------------------------
// D3.1 The chain continues past the flip.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn blocks_built_on_the_flip_block_execute_import_and_commit_binary_roots() {
    let chains = build_boundary_chains(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;
    let blocks = &chains.scheduled_blocks;
    assert_eq!(
        blocks.len() as u64,
        FLIP_BLOCK + BLOCKS_PAST_THE_FLIP,
        "every block must have been built and imported"
    );

    // A chain, not a set of siblings: each block's parent is the previous one,
    // so every block after the flip executed on a binary-committed parent.
    for pair in blocks.windows(2) {
        assert_eq!(pair[1].header.parent_hash, pair[0].hash());
    }

    let active: Vec<&Block> = blocks
        .iter()
        .filter(|block| block.header.timestamp >= chains.activation)
        .collect();
    assert_eq!(
        active.len() as u64,
        BLOCKS_PAST_THE_FLIP + 1,
        "the flip block plus {BLOCKS_PAST_THE_FLIP} blocks built on top of it"
    );
    for block in &active {
        assert_eq!(
            block.header.state_root,
            binary_root(&chains.scheduled_store, block),
            "active block {} must commit its own binary root",
            block.header.number
        );
    }

    // The producer is not the only node that can execute them: replay the whole
    // chain into fresh scheduled stores through every import path.
    let plain_store = store_from_genesis(chains.scheduled_genesis.clone()).await;
    let plain_chain = Blockchain::default_with_store(plain_store.clone());
    for block in blocks {
        plain_chain.add_block(block.clone()).unwrap_or_else(|err| {
            panic!(
                "plain import of block {} past the flip failed: {err:?}",
                block.header.number
            )
        });
    }

    let pipeline_store = store_from_genesis(chains.scheduled_genesis.clone()).await;
    let pipeline_chain = Blockchain::default_with_store(pipeline_store.clone());
    for block in blocks {
        pipeline_chain
            .add_block_pipeline(block.clone(), None)
            .unwrap_or_else(|err| {
                panic!(
                    "pipelined import of block {} past the flip failed: {err:?}",
                    block.header.number
                )
            });
    }

    let batch_store = store_from_genesis(chains.scheduled_genesis.clone()).await;
    let batch_chain = Blockchain::default_with_store(batch_store.clone());
    batch_chain
        .add_blocks_in_batch(blocks.clone(), &[], CancellationToken::new())
        .await
        .expect("batch import must carry the chain past the flip too");

    for (label, store) in [
        ("plain", &plain_store),
        ("pipelined", &pipeline_store),
        ("batch", &batch_store),
    ] {
        for block in &active {
            assert_eq!(
                binary_root(store, block),
                block.header.state_root,
                "{label}: block {} must record the binary root its header commits to",
                block.header.number
            );
        }
    }
}

// ---------------------------------------------------------------------------
// D3.2 Values written before the flip read back correctly after it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_written_before_the_flip_is_read_back_through_the_binary_trie() {
    let chains = build_boundary_chains(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;
    let blocks = FLIP_BLOCK + BLOCKS_PAST_THE_FLIP;
    let head = chains.scheduled_blocks.last().unwrap();
    let twin_head = chains.twin_blocks.last().unwrap();
    assert_eq!(head.header.number, twin_head.header.number);
    assert_eq!(
        head.header.state_root,
        binary_root(&chains.scheduled_store, head),
        "the head must be past the flip"
    );

    let binary_db = StoreVmDatabase::new(chains.scheduled_store.clone(), head.header.clone())
        .expect("a post-flip header must open a readable state");
    // The twin is unscheduled, so this one reads the MPT: the reference for what
    // the values should be.
    let mpt_db = StoreVmDatabase::new(chains.twin_store.clone(), twin_head.header.clone())
        .expect("the unscheduled twin opens against the MPT");

    let sender = sender_from_key(&test_secret_key());
    // Accounts whose end state does not depend on block hashes — EIP-2935 writes
    // the parent hash into its own storage, and the two chains' hashes diverge at
    // the flip, so the history contract is deliberately not compared.
    for address in [
        sender,
        test_recipient(),
        test_coinbase(),
        storage_fixture_account(),
    ] {
        let from_binary = binary_db
            .get_account_state(address)
            .expect("binary account read")
            .unwrap_or_else(|| panic!("{address:#x} must exist after the flip"));
        let from_mpt = mpt_db
            .get_account_state(address)
            .expect("MPT account read")
            .unwrap_or_else(|| panic!("{address:#x} must exist on the twin"));
        assert_eq!(
            from_binary.nonce, from_mpt.nonce,
            "{address:#x}: nonce read after the flip"
        );
        assert_eq!(
            from_binary.balance, from_mpt.balance,
            "{address:#x}: balance read after the flip"
        );
        assert_eq!(
            from_binary.code_hash, from_mpt.code_hash,
            "{address:#x}: code hash read after the flip"
        );
    }

    // Guard against a vacuous pass: these are the mutated values, and the
    // transactions that produced them read the previous ones out of the same
    // trie — one transfer per block, each one nonce-checked against the state
    // the previous block left.
    let sender_state = binary_db.get_account_state(sender).unwrap().unwrap();
    assert_eq!(
        sender_state.nonce, blocks,
        "the sender's nonce must count one transfer per block, read through the binary trie"
    );
    assert!(
        sender_state.balance < U256::from(10).pow(U256::from(20)),
        "the sender must have paid for those transfers"
    );
    assert_eq!(
        binary_db
            .get_account_state(test_recipient())
            .unwrap()
            .unwrap()
            .balance,
        U256::from(blocks),
        "the recipient must hold one wei per block"
    );

    // Storage written at genesis, read back after the flip.
    for slot in 1u64..=3 {
        assert_eq!(
            binary_db
                .get_storage_slot(storage_fixture_account(), H256::from_low_u64_be(slot))
                .expect("binary storage read"),
            Some(U256::from(slot)),
            "slot {slot} of the fixture account must survive the flip"
        );
        assert_eq!(
            binary_db
                .get_storage_slot(storage_fixture_account(), H256::from_low_u64_be(slot))
                .unwrap(),
            mpt_db
                .get_storage_slot(storage_fixture_account(), H256::from_low_u64_be(slot))
                .unwrap(),
            "slot {slot} must read the same as it does on an MPT-committing node"
        );
    }
    // An unwritten slot reads as absent, not as some other slot's value.
    assert_eq!(
        binary_db
            .get_storage_slot(storage_fixture_account(), H256::from_low_u64_be(4))
            .unwrap(),
        None
    );

    // Code is unaffected by the flip: it comes from the code table by hash, not
    // from either trie. The EIP-4788 beacon-roots contract is a genesis account
    // with real bytecode, and every block calls it.
    let beacon_roots =
        Address::from_slice(&hex::decode("000f3df6d732807ef1319fb7b8bb8522d0beac02").unwrap());
    let code_hash = binary_db
        .get_account_state(beacon_roots)
        .unwrap()
        .expect("the beacon-roots contract must exist after the flip")
        .code_hash;
    assert_eq!(
        code_hash,
        mpt_db
            .get_account_state(beacon_roots)
            .unwrap()
            .unwrap()
            .code_hash,
        "the binary read must report the same code hash the MPT does"
    );
    assert!(
        !binary_db
            .get_account_code(code_hash)
            .expect("code read")
            .code()
            .is_empty(),
        "bytecode must still be fetchable by hash after the flip"
    );
}

// ---------------------------------------------------------------------------
// D3.3 Pre-flip headers keep resolving through the MPT — the per-header rule,
//      and the falsification target for the read path.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pre_flip_headers_keep_executing_against_the_mpt_after_the_flip() {
    let chains = build_boundary_chains(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;
    let sender = sender_from_key(&test_secret_key());
    let head = chains.scheduled_blocks.last().unwrap();
    assert_eq!(
        head.header.state_root,
        binary_root(&chains.scheduled_store, head),
        "the flip has happened"
    );

    // Each pre-flip header resolves the state *that block* left behind, out of
    // the MPT, addressed by the root its own header carries.
    for (index, block) in chains.scheduled_blocks.iter().enumerate() {
        if block.header.timestamp >= chains.activation {
            break;
        }
        let db = StoreVmDatabase::new(chains.scheduled_store.clone(), block.header.clone())
            .unwrap_or_else(|err| {
                panic!(
                    "pre-flip block {} must still open against the MPT, got: {err:?}",
                    block.header.number
                )
            });
        assert_eq!(
            db.get_account_state(sender).unwrap().unwrap().nonce,
            index as u64 + 1,
            "block {} must show exactly one transfer per block so far",
            block.header.number
        );
        assert_eq!(
            db.get_account_state(test_recipient())
                .unwrap()
                .unwrap()
                .balance,
            U256::from(index as u64 + 1),
            "block {} must show the recipient's balance at that height",
            block.header.number
        );
    }

    // And a *new* pre-activation block, built and imported while the head is
    // already past the flip, still executes against the MPT and commits its root.
    let blockchain = Blockchain::default_with_store(chains.scheduled_store.clone());
    let parent = chains.scheduled_blocks[0].header.clone();
    let late = build_block_at(
        &chains.scheduled_store,
        &blockchain,
        &parent,
        chains.activation - 1,
    )
    .await;
    assert!(late.header.timestamp < chains.activation);
    blockchain
        .add_block(late.clone())
        .expect("a pre-activation block must still import after the flip has happened");
    assert_ne!(
        late.header.state_root,
        binary_root(&chains.scheduled_store, &late),
        "a pre-activation block commits the MPT root, not the binary one"
    );
}

// ---------------------------------------------------------------------------
// D3.4 The `storage_root` gap, pinned.
//
// `AccountState` carries a `storage_root`; the binary trie has none, because
// storage is not a per-account subtrie there but leaves of the one unified
// tree. A binary read therefore reports `EMPTY_TRIE_HASH`, which every
// "does this account have storage" consumer reads as *no storage* — see the
// comment on the read seam in `crates/blockchain/vm.rs`. The storage itself is
// perfectly readable; only the summary field is absent.
// ---------------------------------------------------------------------------

/// An address the gap test's genesis gives **storage only**: no code, zero
/// nonce, zero balance. In the MPT that is an account with a non-empty
/// `storage_root`. In the binary trie it is a code-hash leaf plus one storage
/// leaf — its basic data encodes to 32 zero bytes and is therefore not even
/// stored — and nothing in it can answer "does this account have storage".
fn storage_only_account() -> Address {
    Address::from_low_u64_be(0x5707)
}

const STORAGE_ONLY_SLOT: u64 = 7;
const STORAGE_ONLY_VALUE: u64 = 0x2a;

#[tokio::test]
async fn a_binary_read_reports_no_storage_root_even_when_the_account_has_storage() {
    let sender = sender_from_key(&test_secret_key());
    let storage_only = (
        storage_only_account(),
        GenesisAccount {
            balance: U256::zero(),
            code: Bytes::new(),
            nonce: 0,
            storage: [(
                U256::from(STORAGE_ONLY_SLOT),
                U256::from(STORAGE_ONLY_VALUE),
            )]
            .into_iter()
            .collect(),
        },
    );

    let unscheduled = load_funded_genesis_with(sender, None, std::slice::from_ref(&storage_only));
    let activation = unscheduled.timestamp + FLIP_BLOCK * BLOCK_TIME;
    let genesis = load_funded_genesis_with(
        sender,
        Some(activation),
        std::slice::from_ref(&storage_only),
    );
    let chain_id = genesis.config.chain_id;
    let store = store_from_genesis(genesis).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let blocks = build_chain(&store, &blockchain, chain_id, FLIP_BLOCK).await;

    let pre_flip = &blocks[0];
    assert!(pre_flip.header.timestamp < activation);
    let head = blocks.last().unwrap();
    assert_eq!(
        head.header.state_root,
        binary_root(&store, head),
        "the head must be the flip block"
    );

    // Before the flip, the MPT answers honestly: the account has storage.
    let mpt_db = StoreVmDatabase::new(store.clone(), pre_flip.header.clone())
        .expect("a pre-flip header opens against the MPT");
    let from_mpt = mpt_db
        .get_account_state(storage_only_account())
        .unwrap()
        .expect("the storage-only account exists in the MPT");
    assert_ne!(
        from_mpt.storage_root, *EMPTY_TRIE_HASH,
        "the MPT read reports the account's storage"
    );

    // After it, the account is still found and its storage is still readable...
    let binary_db = StoreVmDatabase::new(store.clone(), head.header.clone())
        .expect("a post-flip header opens against the binary trie");
    let from_binary = binary_db
        .get_account_state(storage_only_account())
        .unwrap()
        .expect(
            "an account whose basic data collapses to zero must still be found by its code-hash leaf",
        );
    assert_eq!(from_binary.nonce, 0);
    assert_eq!(from_binary.balance, U256::zero());
    assert_eq!(
        binary_db
            .get_storage_slot(
                storage_only_account(),
                H256::from_low_u64_be(STORAGE_ONLY_SLOT)
            )
            .unwrap(),
        Some(U256::from(STORAGE_ONLY_VALUE)),
        "the storage itself is readable through the binary trie"
    );

    // ...but the summary field is not there to be reported. This is the gap:
    // every consumer of `storage_root` (EIP-7610's create-collision check, and
    // the destroyed-account storage wipe) reads "no storage" for this account
    // after the flip.
    assert_eq!(
        from_binary.storage_root, *EMPTY_TRIE_HASH,
        "a binary-trie read has no storage root to report, and says so by \
         reporting the empty one"
    );
}

// ---------------------------------------------------------------------------
// D3.5 What became of the MPT, pinned so it is visible rather than assumed.
//
// Phase D2 decided to keep the MPT advancing after the flip. D3 does not change
// that, and this records what "advancing" now means, because it is not obvious:
//
//   * the MPT is still correct — post-flip merkleization reads its parent state
//     through the trie-layer chain, which is keyed by *header* state roots and
//     therefore stays continuous across the boundary, so the MPT holds the same
//     state an MPT-committing node's does;
//   * but it is no longer *addressable*: `has_state_root(header.state_root)` is
//     false for every active header, because that root belongs to the binary
//     trie. Execution must therefore not resolve through it (it does not — see
//     the tests above), and every root-guarded MPT reader, which is most of the
//     state-reading RPC surface, fails loudly at a post-flip block instead of
//     answering from some other block's state.
//
// Serving those RPC reads from the binary trie is its own piece of work; this
// test is here so that the day the MPT is frozen (or the RPC reads are moved),
// the change is deliberate.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_mpt_keeps_advancing_after_the_flip_but_is_no_longer_addressable_by_header_root() {
    let chains = build_boundary_chains(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;
    let head = chains.scheduled_blocks.last().unwrap();
    let twin_head = chains.twin_blocks.last().unwrap();
    let hashed_sender = keccak(sender_from_key(&test_secret_key()).as_bytes());

    assert!(
        !chains
            .scheduled_store
            .has_state_root(head.header.state_root)
            .expect("root check"),
        "an active header's root is a binary root, so the MPT does not hold it"
    );

    // Block-addressed, which skips the root guard: the MPT is still there and
    // still right.
    let mpt_account = |store: &Store, block: &Block| {
        let trie = store
            .state_trie(block.hash())
            .expect("state trie read")
            .expect("state trie present");
        AccountState::decode(
            &trie
                .get(hashed_sender.as_bytes())
                .expect("state trie lookup")
                .expect("the funded sender must be present"),
        )
        .expect("decode account state")
    };
    let scheduled = mpt_account(&chains.scheduled_store, head);
    let twin = mpt_account(&chains.twin_store, twin_head);
    assert_eq!(
        (scheduled.nonce, scheduled.balance),
        (twin.nonce, twin.balance),
        "the MPT must still hold the state an MPT-committing node holds at this height"
    );
    assert_eq!(
        scheduled.nonce,
        FLIP_BLOCK + BLOCKS_PAST_THE_FLIP,
        "guard against a vacuous pass: this is the end state, not a stale one"
    );
}
