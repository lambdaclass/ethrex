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
use std::sync::Arc;
use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    error::{ChainError, InvalidBlockError, InvalidForkChoice},
    fork_choice::{apply_fork_choice, apply_fork_choice_with_deep_reorg},
    payload::{BuildPayloadArgs, create_payload},
    vm::StoreVmDatabase,
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        AccountInfo, AccountState, AccountUpdate, Block, BlockBody, BlockHeader,
        DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis,
        GenesisAccount, Transaction, TxKind,
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store, error::StoreError};
use ethrex_vm::{DynVmDatabase, Evm, VmDatabase};
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
// D3.4 The storage question across the flip.
//
// `AccountState` carries a `storage_root`; the binary trie has none, because
// storage is not a per-account subtrie there but leaves of the one unified
// tree. What consumers actually want out of that field is the boolean "does
// this account hold any storage" — EIP-7610's create-collision check and the
// destroyed-account storage wipe — and that the binary trie *can* answer,
// through the prefix existence check `pbt_state::has_storage` runs over each of
// the two storage zones.
//
// The answer therefore travels on its own channel, [`VmDatabase::has_storage`],
// rather than being smuggled through a field that has no honest value to hold.
// `storage_root` says only what it can say: on the binary path there is no
// root, so it reports [`EMPTY_TRIE_HASH`] — the same thing a reader gets for an
// account whose storage trie is genuinely empty, which is precisely why the
// boolean cannot ride on it. On the MPT path it stays a real root, and the two
// answers agree because there the root *is* the boolean.
//
// This test has pinned two earlier wrong answers, and exists to keep either
// from coming back:
//
//   * the binary read once reported the empty root unconditionally, so
//     EIP-7610 saw *no storage* for every account past the activation;
//   * it then reported a magic non-root (`0xbb…bb`) to mean "yes", which made
//     `storage_root` a field two readers disagreed about — and leaked into
//     `prewarm`, which tried to open a storage trie at it.
//
// Both accounts below are load-bearing: storage-only is the case the rule turns
// on, and the plain one is the control that says the answer is not simply
// always "yes".
// ---------------------------------------------------------------------------

/// An address the genesis gives **storage only**: no code, zero nonce, zero
/// balance. In the MPT that is an account with a non-empty `storage_root`. In
/// the binary trie it is a code-hash leaf plus one storage leaf — its basic
/// data encodes to 32 zero bytes and is therefore not even stored — and only a
/// prefix query over its storage zones can tell that it holds storage.
///
/// It is also exactly the shape EIP-7610 exists for: an account with storage,
/// no code and a zero nonce is one a `CREATE` must not be allowed to land on,
/// and post-EIP-161 a chain can only reach that shape through its genesis
/// alloc.
fn storage_only_account() -> Address {
    Address::from_low_u64_be(0x5707)
}

/// The control: an address the genesis gives a balance and nothing else. Its
/// storage answer must be *no*, or a read that always says "yes" would pass the
/// storage-only assertions below without meaning anything.
fn storageless_account() -> Address {
    Address::from_low_u64_be(0x5708)
}

const STORAGE_ONLY_SLOT: u64 = 7;
const STORAGE_ONLY_VALUE: u64 = 0x2a;

#[tokio::test]
async fn a_binary_read_reports_whether_the_account_has_storage() {
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
    let storageless = (
        storageless_account(),
        GenesisAccount {
            balance: U256::from(1_000u64),
            code: Bytes::new(),
            nonce: 0,
            storage: Default::default(),
        },
    );
    let extra = [storage_only.clone(), storageless];

    let unscheduled = load_funded_genesis_with(sender, None, &extra);
    let activation = unscheduled.timestamp + FLIP_BLOCK * BLOCK_TIME;
    let genesis = load_funded_genesis_with(sender, Some(activation), &extra);
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

    // Before the flip the MPT answers, and it says the storage-only account
    // has storage and the other does not. There `storage_root` is a genuine
    // root and carries the same answer `has_storage` does — asserting both is
    // the point, since the MPT is where the two must not drift apart.
    let mpt_db = StoreVmDatabase::new(store.clone(), pre_flip.header.clone())
        .expect("a pre-flip header opens against the MPT");
    let from_mpt = mpt_db
        .get_account_state(storage_only_account())
        .unwrap()
        .expect("the storage-only account exists in the MPT");
    assert_ne!(
        from_mpt.storage_root, *EMPTY_TRIE_HASH,
        "the MPT read reports the account's storage as a real root"
    );
    assert!(
        mpt_db.has_storage(storage_only_account()).unwrap(),
        "the MPT path must answer the storage question too"
    );
    assert_eq!(
        mpt_db
            .get_account_state(storageless_account())
            .unwrap()
            .expect("the control account exists in the MPT")
            .storage_root,
        *EMPTY_TRIE_HASH,
    );
    assert!(!mpt_db.has_storage(storageless_account()).unwrap());

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

    // ...and so is the storage question — on its own channel, because the
    // field has no honest value that could carry it.
    assert!(
        binary_db.has_storage(storage_only_account()).unwrap(),
        "a binary read must report that this account holds storage; \
         EIP-7610 reads exactly this, and a CREATE here must fail"
    );

    // The field itself now says only what the binary trie can back: there is
    // no per-account storage root, so it reports the empty one. This is the
    // assertion that fails if the `0xbb…bb` marker ever comes back, and the
    // reason the boolean above cannot be derived from it.
    assert_eq!(
        from_binary.storage_root, *EMPTY_TRIE_HASH,
        "a binary read must not invent a storage root it cannot back"
    );

    // And the control, which keeps the answer from being an unconditional yes.
    assert!(
        !binary_db.has_storage(storageless_account()).unwrap(),
        "an account with no storage must still read as having none"
    );
    assert_eq!(
        binary_db
            .get_account_state(storageless_account())
            .unwrap()
            .expect("the control account exists in the binary trie")
            .storage_root,
        *EMPTY_TRIE_HASH,
    );

    // An account that is not there at all answers "no" rather than erroring,
    // on both paths: `has_storage` is asked for every account execution loads,
    // including ones it is about to create.
    let absent = Address::from_low_u64_be(0x5709);
    assert!(binary_db.get_account_state(absent).unwrap().is_none());
    assert!(!binary_db.has_storage(absent).unwrap());
    assert!(!mpt_db.has_storage(absent).unwrap());
}

// ---------------------------------------------------------------------------
// D3.4b The same question, asked the way execution asks it.
//
// The test above reads `VmDatabase` directly. Execution does not: it goes
// through `DynVmDatabase` -> `CachingDatabase` -> `GeneralizedDatabase`, and
// each of those three layers has its own chance to answer from an MPT-shaped
// cache instead of forwarding. `CachingDatabase` in particular memoizes
// `AccountState`, whose `storage_root` is now honestly empty on the binary
// path — deriving the boolean there rather than forwarding would read "no
// storage" for every post-flip account, which is the pre-existing bug this
// whole change is undoing.
//
// What comes out the far end is a `LevmAccount`, and *two* of its fields are
// at stake:
//
//   * `has_storage`, which EIP-7610's `create_would_collide` reads;
//   * `exists`, which used to be derived as `state != AccountState::default()`
//     — true for a storage-only account on the MPT only because its
//     `storage_root` differed from empty. Making the field honest deletes that
//     signal, so `exists` has to take the boolean instead. An account holding
//     only storage must read as existing on both paths.
// ---------------------------------------------------------------------------

/// Load an address through the layering execution actually uses, and report
/// the two `LevmAccount` flags this section is about.
fn levm_account_flags(db: StoreVmDatabase, address: Address) -> (bool, bool) {
    use ethrex_levm::db::{CachingDatabase, Database as LevmDatabase, gen_db::GeneralizedDatabase};
    use ethrex_vm::DynVmDatabase;

    let dyn_db: DynVmDatabase = Box::new(db);
    let inner: std::sync::Arc<dyn LevmDatabase> = std::sync::Arc::new(dyn_db);
    let caching = std::sync::Arc::new(CachingDatabase::new(inner, false));
    let mut gen_db = GeneralizedDatabase::new(caching);
    let account = gen_db
        .get_account(address)
        .expect("the account read must succeed");
    (account.has_storage, account.exists)
}

#[tokio::test]
async fn a_storage_only_account_exists_and_collides_on_both_paths() {
    let sender = sender_from_key(&test_secret_key());
    let extra = [
        (
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
        ),
        (
            storageless_account(),
            GenesisAccount {
                balance: U256::from(1_000u64),
                code: Bytes::new(),
                nonce: 0,
                storage: Default::default(),
            },
        ),
    ];

    let unscheduled = load_funded_genesis_with(sender, None, &extra);
    let activation = unscheduled.timestamp + FLIP_BLOCK * BLOCK_TIME;
    let genesis = load_funded_genesis_with(sender, Some(activation), &extra);
    let chain_id = genesis.config.chain_id;
    let store = store_from_genesis(genesis).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let blocks = build_chain(&store, &blockchain, chain_id, FLIP_BLOCK).await;

    let pre_flip = blocks[0].header.clone();
    let post_flip = blocks.last().unwrap().header.clone();
    assert!(pre_flip.timestamp < activation);
    assert_eq!(
        post_flip.state_root,
        binary_root(&store, blocks.last().unwrap()),
        "the head must be the flip block"
    );

    for (label, header) in [("MPT", pre_flip), ("binary", post_flip)] {
        let db = StoreVmDatabase::new(store.clone(), header)
            .expect("the header's own trie must hold its state");

        let (has_storage, exists) = levm_account_flags(db.clone(), storage_only_account());
        assert!(
            has_storage,
            "{label}: an account holding only storage must collide with a CREATE (EIP-7610)"
        );
        assert!(
            exists,
            "{label}: an account holding only storage exists, even though its \
             balance, nonce and code are all default"
        );

        let (has_storage, exists) = levm_account_flags(db.clone(), storageless_account());
        assert!(
            !has_storage,
            "{label}: the control account holds no storage"
        );
        assert!(exists, "{label}: the control account has a balance");

        let (has_storage, exists) = levm_account_flags(db, Address::from_low_u64_be(0x570a));
        assert!(!has_storage, "{label}: an absent account holds no storage");
        assert!(!exists, "{label}: an absent account does not exist");
    }
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
//     the tests above), and any root-guarded MPT reader still pointed at an
//     active header's root fails loudly instead of answering from some other
//     block's state.
//
// The state-reading RPCs no longer are pointed at it: Phase D4 below moves them
// onto the same per-header rule execution uses. This test stays because the MPT
// is still advancing underneath, and the day it is frozen the change should be
// deliberate.
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

// ===========================================================================
// Phase D4 — the state-reading RPCs follow the header past the flip.
//
// `eth_getBalance`, `eth_getTransactionCount`, `eth_getCode` and
// `eth_getStorageAt` are served by `Store` methods that used to open the MPT at
// `header.state_root` unconditionally. Past the flip that root names no MPT, so
// they failed loudly (correctly — see D3.5 — but the chain was unqueryable past
// the boundary). They now resolve the same way execution does: per header, from
// the timestamp of the header being queried.
//
// The tests below drive a real chain across the boundary rather than the
// degenerate activation-at-genesis shape the RPC-crate unit tests use, so they
// cover what only a mixed history can: a *pre*-flip block still answering out of
// the MPT while the head is already past the flip.
// ===========================================================================

/// Make `blocks` the canonical chain, which the helpers above deliberately do
/// not do: `add_block` imports and executes but leaves the canonical pointers
/// alone (that is the consensus layer's call). The state RPCs address blocks by
/// *number*, and a number only resolves through the canonical chain, so these
/// tests have to supply the forkchoice the other Phase D tests do not need.
async fn make_canonical(store: &Store, blocks: &[Block]) {
    let head = blocks.last().expect("a chain with at least one block");
    store
        .forkchoice_update(
            blocks
                .iter()
                .map(|block| (block.header.number, block.hash()))
                .collect(),
            head.header.number,
            head.hash(),
            None,
            None,
        )
        .await
        .expect("forkchoice update");
}

/// The four block-addressed state reads the RPC layer makes, gathered so the
/// scheduled chain and its MPT-committing twin can be compared field by field.
#[derive(Debug, PartialEq, Eq)]
struct StateReads {
    balance: U256,
    nonce: u64,
    code: Bytes,
    storage: Vec<Option<U256>>,
}

/// Reads `address` at `number` through exactly the `Store` entry points
/// `eth_getBalance` / `eth_getTransactionCount` / `eth_getCode` /
/// `eth_getStorageAt` use — three separate guarded call sites plus the storage
/// one, not one shared path, which is why all four are exercised.
async fn state_reads(store: &Store, number: u64, address: Address, slots: &[u64]) -> StateReads {
    let info = store
        .get_account_info(number, address)
        .await
        .expect("balance read")
        .unwrap_or_else(|| panic!("{address:#x} must exist at block {number}"));
    let nonce = store
        .get_nonce_by_account_address(number, address)
        .await
        .expect("nonce read")
        .unwrap_or_else(|| panic!("{address:#x} must exist at block {number}"));
    let code = store
        .get_code_by_account_address(number, address)
        .await
        .expect("code read")
        .map(|code| Bytes::copy_from_slice(code.code()))
        .unwrap_or_default();
    let storage = slots
        .iter()
        .map(|slot| {
            store
                .get_storage_at(number, address, H256::from_low_u64_be(*slot))
                .expect("storage read")
        })
        .collect();
    StateReads {
        balance: info.balance,
        nonce,
        code,
        storage,
    }
}

// ---------------------------------------------------------------------------
// D4.1 Balance, nonce, code and storage all read correctly at a post-flip
//      block — both for state written before the flip and state written after.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_rpc_reads_at_a_post_flip_block_match_an_mpt_committing_node() {
    let chains = build_boundary_chains(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;
    make_canonical(&chains.scheduled_store, &chains.scheduled_blocks).await;
    make_canonical(&chains.twin_store, &chains.twin_blocks).await;
    let blocks = FLIP_BLOCK + BLOCKS_PAST_THE_FLIP;
    let head = chains.scheduled_blocks.last().unwrap();
    assert_eq!(
        head.header.state_root,
        binary_root(&chains.scheduled_store, head),
        "the head must be past the flip, or this test is about the MPT"
    );

    let sender = sender_from_key(&test_secret_key());
    let slots = [1u64, 2, 3, 4];
    for address in [sender, test_recipient(), storage_fixture_account()] {
        let scheduled = state_reads(&chains.scheduled_store, blocks, address, &slots).await;
        let twin = state_reads(&chains.twin_store, blocks, address, &slots).await;
        assert_eq!(
            scheduled, twin,
            "{address:#x}: a post-flip read must agree with an MPT-committing node at the same height"
        );
    }

    // Guard against a vacuous pass. These values only exist because the chain
    // ran: the sender's nonce counts one transfer per block (state written both
    // before *and* after the flip), and the fixture account's slots are genesis
    // state carried across the boundary.
    let sender_reads = state_reads(&chains.scheduled_store, blocks, sender, &[]).await;
    assert_eq!(sender_reads.nonce, blocks);
    assert!(sender_reads.balance < U256::from(10).pow(U256::from(20)));
    let recipient_reads = state_reads(&chains.scheduled_store, blocks, test_recipient(), &[]).await;
    assert_eq!(recipient_reads.balance, U256::from(blocks));

    let fixture_reads = state_reads(
        &chains.scheduled_store,
        blocks,
        storage_fixture_account(),
        &slots,
    )
    .await;
    assert_eq!(
        fixture_reads.storage,
        vec![
            Some(U256::from(1u64)),
            Some(U256::from(2u64)),
            Some(U256::from(3u64)),
            None,
        ],
        "genesis storage must survive the flip, and an unwritten slot must stay absent"
    );

    // Code comes from the code table by hash, but reaching it needs the account,
    // and the account now comes from the binary trie.
    let beacon_roots =
        Address::from_slice(&hex::decode("000f3df6d732807ef1319fb7b8bb8522d0beac02").unwrap());
    let code = chains
        .scheduled_store
        .get_code_by_account_address(blocks, beacon_roots)
        .await
        .expect("code read past the flip")
        .expect("the beacon-roots contract must exist after the flip");
    assert!(!code.code().is_empty());
}

// ---------------------------------------------------------------------------
// D4.2 Pre-activation blocks keep resolving through the MPT after the chain has
//      moved past the flip. The per-header rule, and the falsification target:
//      a chain-level `binary_tree_scheduled()` here sends every one of these
//      reads at the binary trie holding an MPT root, and they all fail.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_rpc_reads_at_pre_flip_blocks_keep_using_the_mpt_after_the_flip() {
    let chains = build_boundary_chains(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;
    make_canonical(&chains.scheduled_store, &chains.scheduled_blocks).await;
    make_canonical(&chains.twin_store, &chains.twin_blocks).await;
    let sender = sender_from_key(&test_secret_key());
    let head = chains.scheduled_blocks.last().unwrap();
    assert_eq!(
        head.header.state_root,
        binary_root(&chains.scheduled_store, head),
        "the flip has happened"
    );

    let slots = [1u64, 2, 3];
    let mut pre_flip_blocks = 0;
    for (index, block) in chains.scheduled_blocks.iter().enumerate() {
        if block.header.timestamp >= chains.activation {
            break;
        }
        pre_flip_blocks += 1;
        let number = block.header.number;

        // Genesis' own alloc, addressed at a pre-flip height, out of the MPT.
        let scheduled = state_reads(
            &chains.scheduled_store,
            number,
            storage_fixture_account(),
            &slots,
        )
        .await;
        let twin = state_reads(
            &chains.twin_store,
            number,
            storage_fixture_account(),
            &slots,
        )
        .await;
        assert_eq!(
            scheduled, twin,
            "block {number}: a pre-flip block must read exactly as the unscheduled twin does"
        );

        // And the *history* at that height, which is what makes this a per-block
        // read rather than a read of whatever the head holds.
        assert_eq!(
            chains
                .scheduled_store
                .get_nonce_by_account_address(number, sender)
                .await
                .expect("nonce read")
                .expect("the sender exists"),
            index as u64 + 1,
            "block {number} must show exactly one transfer per block so far"
        );
        assert_eq!(
            chains
                .scheduled_store
                .get_account_info(number, test_recipient())
                .await
                .expect("balance read")
                .expect("the recipient exists")
                .balance,
            U256::from(index as u64 + 1),
            "block {number} must show the recipient's balance at that height"
        );
    }
    assert!(
        pre_flip_blocks > 0,
        "the chain must actually have pre-flip blocks for this test to mean anything"
    );

    // Genesis too, which is the oldest pre-activation header there is.
    assert_eq!(
        state_reads(&chains.scheduled_store, 0, sender, &[])
            .await
            .nonce,
        0,
        "genesis must still resolve against the MPT once the chain is past the flip"
    );
}

// ---------------------------------------------------------------------------
// D4.3 The staleness guard survives on both sides of the boundary.
//
// The guard these reads carry is the reason they were hardened in the first
// place: a block whose state this node no longer holds must error, not answer
// from whatever the single-version trie currently contains. Adding a binary
// branch must not loosen that — on the binary side the equivalent question is
// `has_binary_trie_state`, and it has to be asked.
// ---------------------------------------------------------------------------

/// Appends a canonical block on top of genesis at `timestamp` whose header
/// claims a state root no state of *either* shape stands behind.
async fn append_stateless_block(store: &Store, timestamp: u64) -> u64 {
    let genesis_hash = store
        .get_canonical_block_hash(0)
        .await
        .expect("genesis lookup")
        .expect("genesis is canonical");
    let header = BlockHeader {
        number: 1,
        parent_hash: genesis_hash,
        state_root: H256::repeat_byte(0xAA),
        timestamp,
        ..Default::default()
    };
    let hash = header.hash();
    store
        .add_block(Block::new(header, BlockBody::default()))
        .await
        .expect("add fabricated block");
    store
        .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
        .await
        .expect("make it canonical");
    1
}

#[track_caller]
fn assert_missing_state<T: std::fmt::Debug>(
    result: Result<T, ethrex_storage::error::StoreError>,
    label: &str,
) {
    match result {
        Err(err) => {
            let message = err.to_string();
            assert!(
                message.contains("state root missing"),
                "{label}: expected a missing-state error, got {message:?}"
            );
        }
        Ok(value) => panic!(
            "{label}: answered {value:?} from a state this node does not hold. \
             The staleness guard was lost."
        ),
    }
}

#[tokio::test]
async fn state_rpc_reads_still_refuse_a_block_whose_state_is_gone_on_both_sides_of_the_flip() {
    let sender = sender_from_key(&test_secret_key());
    let genesis = load_funded_genesis(sender, None);
    let activation = genesis.timestamp + FLIP_BLOCK * BLOCK_TIME;
    let scheduled_genesis = load_funded_genesis(sender, Some(activation));

    for (label, timestamp) in [
        // Before the flip: the MPT guard has to fire, exactly as it did before
        // the binary branch existed.
        ("pre-activation", activation - 1),
        // At and after the flip: the binary guard has to fire, because no binary
        // root was ever recorded for this fabricated block.
        ("post-activation", activation),
    ] {
        let store = store_from_genesis(scheduled_genesis.clone()).await;
        let number = append_stateless_block(&store, timestamp).await;

        assert_missing_state(
            store.get_account_info(number, sender).await,
            &format!("{label}: eth_getBalance"),
        );
        assert_missing_state(
            store.get_nonce_by_account_address(number, sender).await,
            &format!("{label}: eth_getTransactionCount"),
        );
        assert_missing_state(
            store.get_code_by_account_address(number, sender).await,
            &format!("{label}: eth_getCode"),
        );
        assert_missing_state(
            store.get_storage_at(number, storage_fixture_account(), H256::from_low_u64_be(1)),
            &format!("{label}: eth_getStorageAt"),
        );
    }
}

// ---------------------------------------------------------------------------
// D4.4 An unscheduled chain is untouched.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unscheduled_chains_read_exactly_as_they_did() {
    let chains = build_boundary_chains(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;
    make_canonical(&chains.twin_store, &chains.twin_blocks).await;
    let blocks = FLIP_BLOCK + BLOCKS_PAST_THE_FLIP;
    let sender = sender_from_key(&test_secret_key());
    let slots = [1u64, 2, 3];

    // No binary root is recorded anywhere on the twin, so nothing here can be
    // answering from one.
    for block in &chains.twin_blocks {
        assert_eq!(
            chains
                .twin_store
                .get_binary_trie_root(block.hash())
                .expect("binary root read"),
            None,
            "an unscheduled chain records no binary roots"
        );
    }

    for number in [0, 1, blocks] {
        let reads = state_reads(
            &chains.twin_store,
            number,
            storage_fixture_account(),
            &slots,
        )
        .await;
        assert_eq!(
            reads.storage,
            vec![
                Some(U256::from(1u64)),
                Some(U256::from(2u64)),
                Some(U256::from(3u64))
            ],
            "block {number}: the fixture account's genesis storage reads unchanged"
        );
    }
    assert_eq!(
        chains
            .twin_store
            .get_nonce_by_account_address(blocks, sender)
            .await
            .expect("nonce read")
            .expect("the sender exists"),
        blocks
    );
}

// ===========================================================================
// Phase E — binary-trie diff layers.
//
// Until now the binary trie committed straight to disk on every block while
// the MPT's nodes were staged into the in-memory layer chain and flushed only
// once a layer was deep enough to be safe. That was survivable while nothing
// read the binary trie; it is not survivable now that it is consensus state,
// because the store is path-keyed and single-version: a reorg would leave the
// abandoned branch's nodes on disk with no other version to fall back to, and
// two blocks at the same height would overwrite each other at shared paths.
//
// Phase E gives binary-trie nodes the same treatment: staged in memory per
// block in the *same* diff layer as that block's MPT nodes, indexed by binary
// root as well as by header state root, flushed on the same commit gate, and
// dropped together on reorg.
//
// The properties:
//
// 1. a just-imported block's binary state is readable before any flush (the
//    layer-chain read cascade);
// 2. nothing reaches `BINARY_TRIE_NODES` until the commit gate allows it;
// 3. a reorg discards the abandoned branch's binary nodes, and the surviving
//    branch's state is exactly what a node that only ever saw it would hold;
// 4. binary and MPT nodes land on disk at the same commit point, not
//    independently;
// 5. all three import paths still agree and an unscheduled chain still does
//    zero binary work (the existing tests above, unchanged).
// ===========================================================================

/// A scheduled chain that flips at [`FLIP_BLOCK`], together with the disk node
/// counts genesis left behind.
///
/// Genesis is the one binary-trie write that is *not* staged — there is no
/// block and therefore no diff layer to stage it into — so it is the floor
/// every "nothing has been flushed yet" assertion is measured against. Reading
/// it before any block is imported is what makes those assertions non-vacuous.
struct StagedChain {
    store: Store,
    blockchain: Blockchain,
    genesis: Genesis,
    blocks: Vec<Block>,
    /// `BINARY_TRIE_NODES` entries present after genesis and before block 1.
    genesis_binary_nodes: usize,
    /// `ACCOUNT_TRIE_NODES` entries present after genesis and before block 1.
    genesis_account_nodes: usize,
    activation: u64,
    chain_id: u64,
}

/// Build `count` blocks on a scheduled chain whose activation lands on
/// [`FLIP_BLOCK`], recording the on-disk node counts genesis produced first.
async fn build_staged_chain(count: u64) -> StagedChain {
    let sender = sender_from_key(&test_secret_key());
    let activation = load_funded_genesis(sender, None).timestamp + FLIP_BLOCK * BLOCK_TIME;
    let genesis = load_funded_genesis(sender, Some(activation));
    let chain_id = genesis.config.chain_id;

    let store = store_from_genesis(genesis.clone()).await;
    store.wait_for_persistence_idle().await.expect("idle");
    let genesis_binary_nodes = store.binary_trie_node_count_for_test().expect("node count");
    let genesis_account_nodes = store
        .account_trie_node_count_for_test()
        .expect("account node count");

    let blockchain = Blockchain::default_with_store(store.clone());
    let blocks = build_chain(&store, &blockchain, chain_id, count).await;
    store.wait_for_persistence_idle().await.expect("idle");

    StagedChain {
        store,
        blockchain,
        genesis,
        blocks,
        genesis_binary_nodes,
        genesis_account_nodes,
        activation,
        chain_id,
    }
}

/// Every `(key, value)` pair currently in `BINARY_TRIE_NODES`, as the ground
/// truth for "what actually reached disk".
fn binary_nodes_on_disk(store: &Store) -> BTreeMap<Vec<u8>, Vec<u8>> {
    store
        .binary_trie_nodes_for_test()
        .expect("binary node dump")
        .into_iter()
        .collect()
}

// ---------------------------------------------------------------------------
// E.1 A just-imported block's binary state is readable before any flush.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_just_imported_blocks_binary_state_is_readable_before_any_flush() {
    let chain = build_staged_chain(FLIP_BLOCK + 2).await;
    let head = chain.blocks.last().unwrap();
    assert_eq!(
        head.header.state_root,
        binary_root(&chain.store, head),
        "the head must be past the flip, so its reads resolve through the binary trie"
    );

    // Precondition, and the whole point: none of these blocks' binary nodes are
    // on disk. Anything read below therefore came out of the layer chain.
    assert_eq!(
        chain.store.binary_trie_node_count_for_test().unwrap(),
        chain.genesis_binary_nodes,
        "no block's binary nodes may reach disk before the commit gate fires"
    );

    let db = StoreVmDatabase::new(chain.store.clone(), head.header.clone())
        .expect("a post-flip header must open a readable state");
    let sender = sender_from_key(&test_secret_key());
    assert_eq!(
        db.get_account_state(sender).unwrap().unwrap().nonce,
        chain.blocks.len() as u64,
        "the sender's nonce must be readable from the layer chain alone"
    );
    assert_eq!(
        db.get_account_state(test_recipient())
            .unwrap()
            .unwrap()
            .balance,
        U256::from(chain.blocks.len()),
        "the recipient's balance must be readable from the layer chain alone"
    );
    // Genesis storage still resolves, which means the cascade falls through to
    // disk when the layers miss rather than reporting the key absent.
    for slot in 1u64..=3 {
        assert_eq!(
            db.get_storage_slot(storage_fixture_account(), H256::from_low_u64_be(slot))
                .expect("binary storage read"),
            Some(U256::from(slot)),
            "slot {slot} lives on disk from genesis and must survive the layer miss"
        );
    }

    // The state RPCs read through the same cascade.
    make_canonical(&chain.store, &chain.blocks).await;
    let reads = state_reads(
        &chain.store,
        head.header.number,
        storage_fixture_account(),
        &[1, 2, 3],
    )
    .await;
    assert_eq!(
        reads.storage,
        vec![
            Some(U256::from(1u64)),
            Some(U256::from(2u64)),
            Some(U256::from(3u64))
        ]
    );
}

// ---------------------------------------------------------------------------
// E.2 Nothing is written to BINARY_TRIE_NODES until the commit gate allows it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn binary_nodes_are_not_written_until_the_commit_gate_allows_it() {
    // Fewer blocks than the commit threshold, and no forkchoice update, so the
    // safe-commit root never advances and nothing is committable.
    let chain = build_staged_chain(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;
    assert_eq!(
        chain.store.binary_trie_node_count_for_test().unwrap(),
        chain.genesis_binary_nodes,
        "{} blocks of binary-trie writes must still be in memory",
        chain.blocks.len()
    );
    // Not vacuous: the blocks really did produce binary nodes, they are just
    // staged. Committing them puts them on disk.
    let head = chain.blocks.last().unwrap();
    chain
        .store
        .commit_trie_layers_for_test(head.header.state_root)
        .await
        .expect("forcing the gate must flush the whole backlog");
    assert!(
        chain.store.binary_trie_node_count_for_test().unwrap() > chain.genesis_binary_nodes,
        "flushing the layer backlog must put this chain's binary nodes on disk"
    );
}

// ---------------------------------------------------------------------------
// E.3 A reorg discards the abandoned branch's binary nodes.
// ---------------------------------------------------------------------------

/// Two competing branches past the activation boundary, one of which is then
/// made canonical and flushed. The surviving branch's on-disk binary trie must
/// be byte-for-byte what a node that only ever saw that branch would hold —
/// which is only true if the abandoned branch's nodes never reached disk.
///
/// This is the heavier divergence the single-flip-block version of this test
/// could not exercise: each branch carries several blocks of its own
/// transactions, so the two write overlapping paths at many depths.
#[tokio::test]
async fn a_reorg_discards_the_abandoned_branchs_binary_nodes() {
    let chain = build_staged_chain(FLIP_BLOCK).await;
    let fork_point = chain.blocks.last().unwrap().header.clone();
    let sender = sender_from_key(&test_secret_key());
    let signer: Signer = LocalSigner::new(test_secret_key()).into();
    let base_nonce = chain.blocks.len() as u64;

    // Branch A: three blocks, one transfer each, starting one second after the
    // fork point so it is distinguishable from B.
    // Branch B: three blocks with a different cadence, so every block's state
    // (and therefore every binary root) differs from A's at the same height.
    let mut branch_a = Vec::new();
    let mut branch_b = Vec::new();
    for (branch, offset) in [(&mut branch_a, 1u64), (&mut branch_b, 2)] {
        let mut parent = fork_point.clone();
        for i in 0..3u64 {
            let tx = transfer_tx(chain.chain_id, base_nonce + i, &signer).await;
            chain
                .blockchain
                .add_transaction_to_pool(tx)
                .await
                .expect("tx should enter pool");
            let block = build_block_at(
                &chain.store,
                &chain.blockchain,
                &parent,
                parent.timestamp + BLOCK_TIME + offset,
            )
            .await;
            chain
                .blockchain
                .add_block(block.clone())
                .unwrap_or_else(|err| panic!("branch block must import: {err:?}"));
            chain
                .blockchain
                .remove_block_transactions_from_pool(&block)
                .expect("remove block txs from pool");
            parent = block.header.clone();
            branch.push(block);
        }
    }

    // Both branches really are past the flip, really do fork at the same point,
    // and really do differ — otherwise nothing below means anything.
    for block in branch_a.iter().chain(branch_b.iter()) {
        assert!(block.header.timestamp >= chain.activation);
        assert_eq!(
            block.header.state_root,
            binary_root(&chain.store, block),
            "block {} must commit its own binary root",
            block.header.number
        );
    }
    assert_eq!(
        branch_a[0].header.parent_hash,
        branch_b[0].header.parent_hash
    );
    for (a, b) in branch_a.iter().zip(branch_b.iter()) {
        assert_ne!(
            a.header.state_root, b.header.state_root,
            "the branches must diverge at every height, not just the first"
        );
    }

    // Branch B wins: make it canonical and flush its layers to disk.
    let canonical: Vec<Block> = chain
        .blocks
        .iter()
        .cloned()
        .chain(branch_b.iter().cloned())
        .collect();
    make_canonical(&chain.store, &canonical).await;
    let winner = branch_b.last().unwrap();
    chain
        .store
        .commit_trie_layers_for_test(winner.header.state_root)
        .await
        .expect("flush the surviving branch");

    // The oracle: a fresh node that only ever saw branch B, flushed the same way.
    let clean_store = store_from_genesis(chain.genesis.clone()).await;
    let clean_chain = Blockchain::default_with_store(clean_store.clone());
    for block in &canonical {
        clean_chain
            .add_block(block.clone())
            .unwrap_or_else(|err| panic!("clean replay of branch B must import: {err:?}"));
    }
    make_canonical(&clean_store, &canonical).await;
    clean_store
        .commit_trie_layers_for_test(winner.header.state_root)
        .await
        .expect("flush the clean replay");

    assert_eq!(
        binary_nodes_on_disk(&chain.store),
        binary_nodes_on_disk(&clean_store),
        "the reorged node's on-disk binary trie must be exactly what a node that \
         only ever saw the surviving branch holds: any extra or differing entry is \
         an abandoned-branch node that reached disk"
    );

    // And the surviving branch's state is correct after the flush, read from
    // disk now rather than from a layer.
    let db = StoreVmDatabase::new(chain.store.clone(), winner.header.clone())
        .expect("the surviving head must open");
    assert_eq!(
        db.get_account_state(sender).unwrap().unwrap().nonce,
        canonical.len() as u64,
        "the surviving branch's sender nonce must count one transfer per canonical block"
    );
    assert_eq!(
        db.get_account_state(test_recipient())
            .unwrap()
            .unwrap()
            .balance,
        U256::from(canonical.len()),
        "the surviving branch's recipient balance must count one wei per canonical block"
    );
}

/// The pre-existing sibling-branch test, strengthened: with the nodes staged
/// per layer rather than written straight through, the two branches no longer
/// merely record different roots — neither branch's nodes are on disk at all,
/// and each root's state is separately readable.
#[tokio::test]
async fn competing_branches_keep_separate_readable_binary_state() {
    let chain = build_staged_chain(FLIP_BLOCK - 1).await;
    let parent = chain.blocks.last().unwrap().header.clone();

    let branch_a = build_block_at(&chain.store, &chain.blockchain, &parent, chain.activation).await;
    chain
        .blockchain
        .add_block(branch_a.clone())
        .expect("the first branch's flip block must import");
    let branch_b = build_block_at(
        &chain.store,
        &chain.blockchain,
        &parent,
        chain.activation + 1,
    )
    .await;
    chain
        .blockchain
        .add_block(branch_b.clone())
        .expect("the sibling branch's flip block must import too");
    chain.store.wait_for_persistence_idle().await.unwrap();

    assert_ne!(branch_a.hash(), branch_b.hash());
    assert_eq!(branch_a.header.parent_hash, branch_b.header.parent_hash);
    assert_ne!(
        binary_root(&chain.store, &branch_a),
        binary_root(&chain.store, &branch_b),
        "the two branches must not collapse onto the same recorded root"
    );

    // Neither branch wrote a node: both are staged, so neither can have
    // overwritten the other at a shared path.
    assert_eq!(
        chain.store.binary_trie_node_count_for_test().unwrap(),
        chain.genesis_binary_nodes,
        "competing blocks must not write binary nodes at all before the gate fires"
    );

    // Each branch's state resolves through its own layer, at its own root.
    // EIP-4788 writes the beacon root at `timestamp % 8191`, and the two blocks
    // have different timestamps, so each branch has a slot the other does not.
    let beacon_roots =
        Address::from_slice(&hex::decode("000f3df6d732807ef1319fb7b8bb8522d0beac02").unwrap());
    for (label, block) in [("a", &branch_a), ("b", &branch_b)] {
        let db = StoreVmDatabase::new(chain.store.clone(), block.header.clone())
            .unwrap_or_else(|err| panic!("branch {label} must open: {err:?}"));
        let slot = H256::from_low_u64_be(block.header.timestamp % 8191);
        assert_eq!(
            db.get_storage_slot(beacon_roots, slot).unwrap(),
            Some(U256::from_big_endian(
                block.header.timestamp.to_be_bytes().as_slice()
            )),
            "branch {label} must read its own EIP-4788 timestamp slot"
        );
    }
}

// ---------------------------------------------------------------------------
// E.4 Flush parity: binary and MPT nodes land at the same commit point.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn binary_and_mpt_nodes_land_on_disk_at_the_same_commit_point() {
    let chain = build_staged_chain(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;

    // Before the gate: neither trie has advanced past genesis.
    assert_eq!(
        chain.store.binary_trie_node_count_for_test().unwrap(),
        chain.genesis_binary_nodes,
        "binary nodes must be staged, not written"
    );
    assert_eq!(
        chain.store.account_trie_node_count_for_test().unwrap(),
        chain.genesis_account_nodes,
        "MPT nodes must be staged, not written — the parity baseline"
    );

    // Flush the layer containing block N only. Both node sets for that block
    // must appear, and neither trie may run ahead of the other.
    let target = &chain.blocks[1];
    chain
        .store
        .commit_trie_layers_for_test(target.header.state_root)
        .await
        .expect("commit up to block 2");

    let binary_after_first = chain.store.binary_trie_node_count_for_test().unwrap();
    let account_after_first = chain.store.account_trie_node_count_for_test().unwrap();
    assert!(
        binary_after_first > chain.genesis_binary_nodes,
        "the flushed layers' binary nodes must be on disk"
    );
    assert!(
        account_after_first > chain.genesis_account_nodes,
        "the flushed layers' MPT nodes must be on disk"
    );

    // The two are one write: the binary trie must be at exactly the block the
    // MPT is at, not ahead of it. Reading the binary state at the flushed
    // block's root with the layer cache emptied of everything below it proves
    // that — and reading it at the *unflushed* head still works because those
    // layers are still resident.
    let head = chain.blocks.last().unwrap();
    let head_db = StoreVmDatabase::new(chain.store.clone(), head.header.clone())
        .expect("the unflushed head must still open");
    assert_eq!(
        head_db
            .get_account_state(test_recipient())
            .unwrap()
            .unwrap()
            .balance,
        U256::from(chain.blocks.len()),
        "the head's state is still layer-resident and must read correctly after a partial flush"
    );

    // Flushing the rest advances both again, together.
    chain
        .store
        .commit_trie_layers_for_test(head.header.state_root)
        .await
        .expect("commit the rest");
    assert!(
        chain.store.binary_trie_node_count_for_test().unwrap() >= binary_after_first,
        "binary nodes must only ever grow across commits"
    );
    assert!(
        chain.store.account_trie_node_count_for_test().unwrap() >= account_after_first,
        "MPT nodes must only ever grow across commits"
    );

    // After the full flush the whole chain's state is on disk and still correct.
    let disk_db = StoreVmDatabase::new(chain.store.clone(), head.header.clone())
        .expect("the fully flushed head must open");
    assert_eq!(
        disk_db
            .get_account_state(sender_from_key(&test_secret_key()))
            .unwrap()
            .unwrap()
            .nonce,
        chain.blocks.len() as u64,
        "the flushed binary trie must hold the same state the layers served"
    );
}

// ---------------------------------------------------------------------------
// E.5 All three import paths still agree once the nodes are staged, and an
//     unscheduled chain still does zero binary work through the whole gate.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_import_paths_agree_and_flush_the_same_binary_nodes() {
    let chain = build_staged_chain(FLIP_BLOCK + 2).await;
    let head = chain.blocks.last().unwrap();
    make_canonical(&chain.store, &chain.blocks).await;
    chain
        .store
        .commit_trie_layers_for_test(head.header.state_root)
        .await
        .expect("flush the plain path");

    // Path B: the pipelined engine-API route. Path C: batch import.
    let pipeline_store = store_from_genesis(chain.genesis.clone()).await;
    let pipeline_chain = Blockchain::default_with_store(pipeline_store.clone());
    for block in &chain.blocks {
        pipeline_chain
            .add_block_pipeline(block.clone(), None)
            .expect("pipelined import past the flip");
    }
    make_canonical(&pipeline_store, &chain.blocks).await;
    pipeline_store
        .commit_trie_layers_for_test(head.header.state_root)
        .await
        .expect("flush the pipelined path");

    let batch_store = store_from_genesis(chain.genesis.clone()).await;
    let batch_chain = Blockchain::default_with_store(batch_store.clone());
    batch_chain
        .add_blocks_in_batch(chain.blocks.clone(), &[], CancellationToken::new())
        .await
        .expect("batch import past the flip");
    make_canonical(&batch_store, &chain.blocks).await;
    batch_store
        .commit_trie_layers_for_test(head.header.state_root)
        .await
        .expect("flush the batch path");

    let expected = binary_nodes_on_disk(&chain.store);
    assert!(
        expected.len() > chain.genesis_binary_nodes,
        "the plain path must have flushed something"
    );
    for (label, store) in [("pipelined", &pipeline_store), ("batch", &batch_store)] {
        for block in &chain.blocks {
            assert_eq!(
                binary_root(store, block),
                binary_root(&chain.store, block),
                "{label}: block {} must record the same binary root",
                block.header.number
            );
        }
        assert_eq!(
            binary_nodes_on_disk(store),
            expected,
            "{label}: the flushed binary trie must be byte-for-byte the plain path's"
        );
    }
}

#[tokio::test]
async fn an_unscheduled_chain_stages_and_flushes_no_binary_nodes() {
    let sender = sender_from_key(&test_secret_key());
    let genesis = load_funded_genesis(sender, None);
    let chain_id = genesis.config.chain_id;
    let store = store_from_genesis(genesis).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let blocks = build_chain(&store, &blockchain, chain_id, 4).await;

    make_canonical(&store, &blocks).await;
    store
        .commit_trie_layers_for_test(blocks.last().unwrap().header.state_root)
        .await
        .expect("flush an unscheduled chain");
    store.wait_for_persistence_idle().await.unwrap();

    assert_eq!(
        store.binary_trie_node_count_for_test().unwrap(),
        0,
        "an unscheduled chain must write no binary-trie nodes, even across a flush"
    );
    assert_eq!(
        store.binary_trie_root_count_for_test().unwrap(),
        0,
        "an unscheduled chain must record no binary-trie roots"
    );
    assert!(
        store.account_trie_node_count_for_test().unwrap() > 0,
        "its MPT must still have been flushed, so the gate really did fire"
    );
}

// ---------------------------------------------------------------------------
// E.6 A reorg *deeper* than the layer window unwinds the on-disk binary trie.
//
// E.3 covers every reorg inside the diff-layer window, where the abandoned
// branch's binary nodes are discarded before they are ever written. This is the
// case underneath it: the abandoned branch was flushed, the layer cache is gone
// (a restart, or simply enough commits past the fork point), and the only thing
// that can put the binary trie back at the pivot is the `STATE_HISTORY`
// reverse-diff journal — which is exactly what the binary sections of the
// journal exist for.
// ---------------------------------------------------------------------------

/// The shape both deep-reorg tests below drive: a scheduled chain past the
/// flip, forked at `fork_index`, with the losing branch already flushed to disk
/// and every diff layer dropped, so recovering the winner is only possible
/// through the journal.
struct DeepReorgFixture {
    chain: StagedChain,
    /// The winning branch's blocks, in order, excluding the shared prefix.
    winner: Vec<Block>,
    /// The full winning chain from block 1, for the clean-replay oracle.
    canonical: Vec<Block>,
    /// Blocks of the branch that was flushed to disk and must be unwound.
    loser: Vec<Block>,
}

/// Builds two competing branches past the activation, flushes the *loser* to
/// disk, then drops every diff layer.
///
/// The drop is what makes this a deep reorg rather than the E.3 case: with no
/// layers left, the pivot's state exists only on disk (where the loser has
/// overwritten it) plus the journal, so nothing short of an unwind can serve
/// the winner's first block its parent state.
///
/// The two branch lengths are separate on purpose. When the winner is at least
/// as long as the loser it tends to rewrite every path the loser touched, so the
/// unwind is invisible in the end state; a **shorter** winner leaves the loser's
/// top blocks with nothing above them, which is what forces the reconciliation
/// bridge to carry those keys back to their pivot values.
async fn build_deep_reorg_fixture(
    prefix_len: u64,
    loser_len: u64,
    winner_len: u64,
) -> DeepReorgFixture {
    let chain = build_staged_chain(prefix_len).await;
    let fork_point = chain.blocks.last().unwrap().header.clone();
    let signer: Signer = LocalSigner::new(test_secret_key()).into();
    let base_nonce = chain.blocks.len() as u64;

    // Two branches from the same parent with different cadences, so every
    // block's state — and therefore every binary root — differs at every height.
    let mut branches: Vec<Vec<Block>> = Vec::new();
    for (offset, branch_len) in [(1u64, loser_len), (2, winner_len)] {
        let mut parent = fork_point.clone();
        let mut branch = Vec::new();
        for i in 0..branch_len {
            let tx = transfer_tx(chain.chain_id, base_nonce + i, &signer).await;
            chain
                .blockchain
                .add_transaction_to_pool(tx)
                .await
                .expect("tx should enter pool");
            let block = build_block_at(
                &chain.store,
                &chain.blockchain,
                &parent,
                parent.timestamp + BLOCK_TIME + offset,
            )
            .await;
            chain
                .blockchain
                .add_block(block.clone())
                .unwrap_or_else(|err| panic!("branch block must import: {err:?}"));
            chain
                .blockchain
                .remove_block_transactions_from_pool(&block)
                .expect("remove block txs from pool");
            parent = block.header.clone();
            branch.push(block);
        }
        branches.push(branch);
    }
    let winner = branches.pop().expect("two branches");
    let loser = branches.pop().expect("two branches");

    // Every branch block is past the flip whatever the fork point is, so the
    // reorg really does turn on the binary trie rather than the MPT.
    for block in winner.iter().chain(loser.iter()) {
        assert!(block.header.timestamp >= chain.activation);
        assert_eq!(
            block.header.state_root,
            binary_root(&chain.store, block),
            "block {} must commit its own binary root",
            block.header.number
        );
    }

    // The loser becomes canonical and is flushed: its binary nodes reach disk,
    // and the journal records the reverse diff of every block above the pivot.
    let loser_chain: Vec<Block> = chain.blocks.iter().cloned().chain(loser.clone()).collect();
    make_canonical(&chain.store, &loser_chain).await;
    flush_every_layer(&chain.store, loser_chain.last().unwrap().header.state_root).await;

    // And now the layers go away, exactly as a restart loses them. Everything
    // below has to come out of disk plus the journal.
    chain.store.drop_trie_layers_for_test().unwrap();

    // The journal has to reach back past the pivot, or the deep path has nothing
    // to unwind with and declines with `StateNotReachable` for a reason that has
    // nothing to do with the binary trie.
    let pivot_number = fork_point.number;
    assert_eq!(
        chain
            .store
            .highest_state_history_block_number()
            .unwrap()
            .expect("the flush must have journaled"),
        loser_chain.last().unwrap().header.number,
        "the journal's edge must be the flushed head"
    );
    assert!(
        chain
            .store
            .lowest_state_history_block_number()
            .unwrap()
            .expect("the flush must have journaled")
            <= pivot_number + 1,
        "the journal must reach back to the block above the pivot"
    );
    assert!(
        chain.store.flatkeyvalue_fully_generated().unwrap(),
        "the deep path defers while flat-KV generation is in flight"
    );

    let canonical: Vec<Block> = chain.blocks.iter().cloned().chain(winner.clone()).collect();
    DeepReorgFixture {
        chain,
        winner,
        canonical,
        loser,
    }
}

/// Force the commit gate at `root` until no layer is left below it.
///
/// One pass is not always enough: while a deep-reorg overlay is installed
/// `commit_to_disk` deliberately commits only the bottom layer per pass, so the
/// backlog above it drains on subsequent passes. Iterating to a fixed point
/// keeps the tests' "what is on disk" assertions honest either way.
async fn flush_every_layer(store: &Store, root: H256) {
    let mut previous = usize::MAX;
    for _ in 0..8 {
        store
            .commit_trie_layers_for_test(root)
            .await
            .expect("forcing the commit gate");
        let count = store.binary_trie_node_count_for_test().unwrap();
        if !store.is_state_in_layer_cache(root).unwrap() || count == previous {
            break;
        }
        previous = count;
    }
}

/// A node that only ever saw `blocks`, flushed the same way, as the oracle for
/// what the reorged node's disk must hold.
async fn clean_replay(genesis: &Genesis, blocks: &[Block]) -> Store {
    let store = store_from_genesis(genesis.clone()).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    for block in blocks {
        blockchain
            .add_block(block.clone())
            .unwrap_or_else(|err| panic!("clean replay must import: {err:?}"));
    }
    make_canonical(&store, blocks).await;
    flush_every_layer(&store, blocks.last().unwrap().header.state_root).await;
    store
}

/// A reorg deeper than the layer window must leave the on-disk binary trie
/// byte-for-byte what a node that only ever saw the winning branch holds.
///
/// Without a binary reverse-diff journal the deep path cannot even get started:
/// the pivot's binary root is not what the on-disk trie resolves to (the loser
/// overwrote it), so the very first replayed block fails to open its parent
/// state. With one, the overlay serves the pivot's binary nodes to the replay
/// and the reconciliation folds the unwound nodes into the same atomic write as
/// the new chain's.
#[tokio::test]
async fn a_deep_reorg_unwinds_the_on_disk_binary_trie() {
    // Three blocks abandoned, one adopted: the reorged node's head is *below*
    // the height the abandoned branch reached, so two blocks' worth of on-disk
    // binary nodes have nothing above them to overwrite them and must be
    // unwound outright.
    let fixture = build_deep_reorg_fixture(FLIP_BLOCK + 1, 3, 1).await;
    let head = fixture.winner.last().unwrap();

    // Precondition: this really is deeper than the layer window. The pivot's
    // state is unreachable without an unwind, which is what makes the shallow
    // path decline it.
    let pivot = fixture.chain.blocks.last().unwrap();
    assert!(
        !fixture
            .chain
            .store
            .is_state_in_layer_cache(pivot.header.state_root)
            .unwrap(),
        "the pivot must have no diff layer, or this is the shallow reorg E.3 already covers"
    );
    assert!(
        !fixture
            .chain
            .store
            .has_binary_trie_state(pivot.hash(), pivot.header.state_root)
            .unwrap(),
        "the on-disk binary trie must currently be on the abandoned branch, or there is \
         nothing here to unwind"
    );

    apply_fork_choice_with_deep_reorg(
        &fixture.chain.blockchain,
        head.hash(),
        H256::zero(),
        H256::zero(),
    )
    .await
    .expect("the deep reorg onto the winning branch must succeed");

    // Flush whatever the reconciliation left in layers, so the comparison below
    // is against a fully-persisted trie on both sides.
    flush_every_layer(&fixture.chain.store, head.header.state_root).await;

    let clean = clean_replay(&fixture.chain.genesis, &fixture.canonical).await;
    assert_eq!(
        binary_nodes_on_disk(&fixture.chain.store),
        binary_nodes_on_disk(&clean),
        "after a deep reorg the on-disk binary trie must be exactly what a node that only \
         ever saw the winning branch holds: any extra or differing entry is an abandoned-branch \
         node the journal failed to unwind"
    );

    // And the winner's state is readable at its own root, from disk.
    let db = StoreVmDatabase::new(fixture.chain.store.clone(), head.header.clone())
        .expect("the reorged head must open");
    assert_eq!(
        db.get_account_state(sender_from_key(&test_secret_key()))
            .unwrap()
            .unwrap()
            .nonce,
        fixture.canonical.len() as u64,
        "the winning chain's sender nonce must count one transfer per canonical block"
    );
    assert_eq!(
        db.get_account_state(test_recipient())
            .unwrap()
            .unwrap()
            .balance,
        U256::from(fixture.canonical.len()),
        "the winning chain's recipient balance must count one wei per canonical block"
    );

    // The abandoned branch's roots must no longer resolve: its nodes are gone.
    for block in &fixture.loser {
        assert!(
            !fixture
                .chain
                .store
                .has_binary_trie_state(block.hash(), block.header.state_root)
                .unwrap(),
            "abandoned block {} must not still have readable binary state",
            block.header.number
        );
    }
}

/// The same unwind with the pivot on the **other side of the flip**: the block
/// the reorg returns to commits an MPT root, while every block above it commits
/// a binary one.
///
/// This is the per-header rule under the deep-reorg path. The overlay has to
/// serve two different roots for the same pivot — `serves_root` for the MPT the
/// pivot's header names, `serves_binary_root` for the shadow-tracked binary trie
/// the blocks above it extend — and nothing in the pivot's header names the
/// second one. A design that reused the header state root for both would find
/// nothing here and fall through to the abandoned branch on disk.
#[tokio::test]
async fn a_deep_reorg_whose_pivot_is_before_the_flip_still_unwinds_the_binary_trie() {
    let fixture = build_deep_reorg_fixture(FLIP_BLOCK - 1, 3, 1).await;
    let head = fixture.winner.last().unwrap();
    let pivot = fixture.chain.blocks.last().unwrap();

    // The premise: the pivot is pre-flip and commits its MPT root, so its binary
    // root exists only in `BINARY_TRIE_ROOTS`.
    assert!(pivot.header.timestamp < fixture.chain.activation);
    let pivot_binary_root = binary_root(&fixture.chain.store, pivot);
    assert_ne!(
        pivot.header.state_root, pivot_binary_root,
        "the pivot must be an MPT-committing header, or this is the same case as above"
    );

    apply_fork_choice_with_deep_reorg(
        &fixture.chain.blockchain,
        head.hash(),
        H256::zero(),
        H256::zero(),
    )
    .await
    .expect("a deep reorg across the flip boundary must succeed");

    flush_every_layer(&fixture.chain.store, head.header.state_root).await;

    let clean = clean_replay(&fixture.chain.genesis, &fixture.canonical).await;
    assert_eq!(
        binary_nodes_on_disk(&fixture.chain.store),
        binary_nodes_on_disk(&clean),
        "unwinding to a pre-flip pivot must land the binary trie exactly where a node that \
         only ever saw the winning branch would have it"
    );

    // The winner's post-flip head — whose parent is the pre-flip pivot — reads
    // its own state back off disk, which is only possible if the unwind put the
    // pivot's *binary* trie back before that block re-executed.
    let db = StoreVmDatabase::new(fixture.chain.store.clone(), head.header.clone())
        .expect("the reorged head must open");
    assert_eq!(
        db.get_account_state(test_recipient())
            .unwrap()
            .unwrap()
            .balance,
        U256::from(fixture.canonical.len()),
        "the winning chain's recipient balance must count one wei per canonical block"
    );

    // The pivot's shadow root is untouched by the reorg — nothing rewrote the
    // pre-flip block's bookkeeping — and it is still the root the winner's first
    // block extended.
    assert_eq!(
        binary_root(&fixture.chain.store, pivot),
        pivot_binary_root,
        "the pre-flip pivot's recorded binary root must survive the reorg unchanged"
    );

    // The abandoned branch is gone from the binary trie as before, and the
    // *pre-flip* pivot's MPT state is not resurrected either: disk is
    // single-version on both sides, and the reconciliation advanced it to the
    // winner's head rather than leaving it at the pivot.
    for block in &fixture.loser {
        assert!(
            !fixture
                .chain
                .store
                .has_binary_trie_state(block.hash(), block.header.state_root)
                .unwrap(),
            "abandoned block {} must not still have readable binary state",
            block.header.number
        );
    }
}

// ===========================================================================
// Phase D6 — forkchoice reachability across the boundary.
//
// Importing a block is only half of accepting it: the consensus layer then
// sends a forkchoice update naming it as head, and `apply_fork_choice` gates
// that on "do I actually hold this branch's state". That gate reads
// `Store::has_state_root`, which opens the **MPT** and hashes its root node.
// An active header's `state_root` is a binary-trie root, so the gate has to
// ask the per-header question (`header_addresses_binary_trie`) exactly as
// block execution and the state RPCs do, or it answers "not held" for state
// the node holds perfectly well.
//
// Found on a devnet, not here: every other test in this file drives
// `add_block` directly, and the whole engine-API forkchoice path — the thing
// a real CL exercises once per slot — had no coverage across the flip. The
// symptom was a chain that executed the flip block cleanly, with correct
// roots on all three nodes, and then simply stopped advancing its head.
// ===========================================================================

/// The forkchoice update the consensus layer sends for `block`, with the
/// genesis as safe and finalized so that only the head's own reachability is
/// under test.
async fn forkchoice_to(store: &Store, block: &Block) -> Result<BlockHeader, InvalidForkChoice> {
    apply_fork_choice(store, block.hash(), H256::zero(), H256::zero(), None).await
}

/// A scheduled chain must accept a forkchoice update at every block, including
/// the flip block and the ones after it.
///
/// The updates are applied **one block at a time**, which is not incidental:
/// the gate reads the *link block* — the deepest block of the branch being
/// made canonical — and only a per-block forkchoice makes that link block the
/// new block itself. Applying a single update at the final head would link at
/// block 1, whose root is an MPT root, and the test would pass without ever
/// putting a binary root in front of the gate. `a_batched_forkchoice_would_not_
/// have_caught_this` below pins that distinction so the loop is not later
/// "simplified" into a vacuous test.
#[tokio::test]
async fn forkchoice_accepts_every_block_across_the_flip() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;

    for block in &chains.scheduled_blocks {
        let number = block.header.number;
        let active = number >= FLIP_BLOCK;
        let header = forkchoice_to(&chains.scheduled_store, block)
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "forkchoice at block {number} ({}) must be accepted, got {err:?}",
                    if active { "active" } else { "pre-activation" }
                )
            });
        assert_eq!(
            header.hash(),
            block.hash(),
            "forkchoice at block {number} must return that block's header"
        );
        assert_eq!(
            chains
                .scheduled_store
                .get_latest_block_number()
                .await
                .expect("latest block number"),
            number,
            "the head must advance to block {number}; a chain that executes the \
             flip block but leaves the head behind is the devnet halt"
        );
    }
}

/// The unscheduled twin must behave identically, so a failure above is about
/// the binary trie and not about the per-block forkchoice loop itself.
#[tokio::test]
async fn forkchoice_accepts_every_block_on_an_unscheduled_chain() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;

    for block in &chains.twin_blocks {
        forkchoice_to(&chains.twin_store, block)
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "forkchoice at block {} on an MPT-committing chain must be \
                     accepted, got {err:?}",
                    block.header.number
                )
            });
    }
}

/// Falsification: a single forkchoice update at the final head links at block
/// 1, never presents an active header's root to the reachability gate, and so
/// passes even when the gate is MPT-only.
///
/// This exists to keep the per-block loop above honest. If this test ever
/// fails, the link-block reasoning it encodes has changed and the loop's
/// justification needs revisiting — not the other way around.
#[tokio::test]
async fn a_batched_forkchoice_would_not_have_caught_this() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let head = chains.scheduled_blocks.last().expect("a non-empty chain");

    let link = chains
        .scheduled_blocks
        .first()
        .expect("a non-empty chain")
        .header
        .clone();
    assert!(
        link.number < FLIP_BLOCK,
        "the batched update's link block ({}) must be pre-activation for this \
         test to say anything",
        link.number
    );

    forkchoice_to(&chains.scheduled_store, head)
        .await
        .expect("a single forkchoice at the head links pre-activation and passes");
}

/// The *other* reachability gate: the "head is already canonical, skip this
/// FCU" fast path (`fork_choice.rs`, the `NewHeadAlreadyCanonical` arm) asks
/// the same question about `head` that the link-block gate asks about the
/// link, and needs the same per-header answer.
///
/// The loop above never reaches it — it passes no finalized block, and the
/// skip requires one. Reaching it needs a finalized block at or above the head
/// under test, with a further block on top so the head is a strict ancestor.
///
/// A root-only check here is not a halt but a silent behaviour change: the skip
/// is declined, the FCU falls through to the full path and re-runs a forkchoice
/// update that was supposed to be a no-op. Asserting the skip *fires* is what
/// pins it.
#[tokio::test]
async fn the_already_canonical_skip_fires_at_a_post_flip_head() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    let blocks = &chains.scheduled_blocks;

    let head = blocks.last().expect("a non-empty chain");
    let finalized = &blocks[blocks.len() - 2];
    assert!(
        finalized.header.number > FLIP_BLOCK,
        "the finalized block must be past the flip for this test to say anything"
    );

    for block in blocks {
        apply_fork_choice(store, block.hash(), H256::zero(), H256::zero(), None)
            .await
            .expect("building the canonical chain");
    }
    apply_fork_choice(store, head.hash(), finalized.hash(), finalized.hash(), None)
        .await
        .expect("finalizing a post-flip block");

    let err = apply_fork_choice(
        store,
        finalized.hash(),
        finalized.hash(),
        finalized.hash(),
        None,
    )
    .await
    .expect_err("re-issuing an FCU at a finalized canonical head must be skipped");
    assert!(
        matches!(err, InvalidForkChoice::NewHeadAlreadyCanonical),
        "the skip must fire at a post-flip head; a root-only reachability check \
         declines it and silently re-runs the whole forkchoice path, got {err:?}"
    );
}

// ===========================================================================
// Phase D7 — the startup state walk.
//
// `regenerate_head_state` runs on every node start. It walks *backwards* from
// the durable head looking for a block whose post-state this node holds, then
// re-executes forward from there. On a post-activation chain a root-only check
// reports "not held" for every header it tests, so the walk runs all the way to
// genesis and the node re-executes its entire history on every restart.
//
// Measured on the 2026-08-07 devnet: a node stopped at head 73 and restarted
// logged `Regenerating state from block 0 to 74` and re-executed blocks 1-74.
// It did not fail — genesis state is always available, so the walk bottoms out
// there and the node recovers by full replay. The cost is the whole point of
// persisting the binary trie: state is on disk and correct, but the startup
// check cannot see it.
// ===========================================================================

/// On a scheduled chain past the flip, the walk must stop at the head — the
/// node holds that state and has nothing to re-execute.
#[tokio::test]
async fn the_startup_state_walk_stops_at_a_post_flip_head() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let blocks = &chains.scheduled_blocks;
    make_canonical(&chains.scheduled_store, blocks).await;

    let head = blocks.last().expect("a non-empty chain").header.number;
    assert!(
        head > FLIP_BLOCK,
        "the head must be past the flip for this test to say anything"
    );

    let resume_from = ethrex::initializers::last_block_with_state(&chains.scheduled_store)
        .await
        .expect("the startup walk must succeed");

    assert_eq!(
        resume_from,
        head,
        "the startup walk must stop at the post-flip head; walking below it \
         means the node re-executes blocks {}..={head} on every restart, which \
         is what persisting the binary trie exists to avoid",
        resume_from + 1
    );
}

/// The unscheduled twin must behave identically, so a failure above is about
/// the binary trie and not about the walk or the test harness.
#[tokio::test]
async fn the_startup_state_walk_stops_at_the_head_on_an_unscheduled_chain() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let blocks = &chains.twin_blocks;
    make_canonical(&chains.twin_store, blocks).await;

    let head = blocks.last().expect("a non-empty chain").header.number;
    let resume_from = ethrex::initializers::last_block_with_state(&chains.twin_store)
        .await
        .expect("the startup walk must succeed");

    assert_eq!(
        resume_from, head,
        "an MPT-committing chain's walk must stop at the head"
    );
}

// ===========================================================================
// Phase D8 — the remaining root-only `has_state_root` callers.
//
// The forkchoice gates and the startup walk were the first two found, but the
// same question is asked in several other places, each passing a header's
// `state_root` to the MPT-only `Store::has_state_root`. All of them answer
// "not held" for every post-activation header. They sit on paths a healthy
// steady-state chain does not take — sync resume, payload re-execution,
// tracing — which is why neither the devnet nor the earlier tests reached
// them.
//
// These two are the ones reachable as units. The rest are fixed alongside and
// pinned by `no_caller_asks_has_state_root_about_a_header` below.
// ===========================================================================

/// Full sync skips blocks it has already executed by asking `is_resume_point`.
/// A post-flip block whose state this node holds must qualify — otherwise full
/// sync re-downloads and re-executes the entire post-activation chain on every
/// cycle, and `--syncmode=full` is mandatory on a scheduled chain.
#[tokio::test]
async fn a_post_flip_block_is_a_full_sync_resume_point() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    make_canonical(&chains.scheduled_store, &chains.scheduled_blocks).await;

    let head = chains.scheduled_blocks.last().expect("a non-empty chain");
    assert!(
        head.header.number > FLIP_BLOCK,
        "the head must be past the flip for this test to say anything"
    );

    assert!(
        ethrex_p2p::sync::is_resume_point(&chains.scheduled_store, &head.header)
            .expect("resume-point check"),
        "a post-flip block whose state this node holds must be a full-sync \
         resume point; treating it as stateless makes full sync re-execute \
         every block after the activation, forever"
    );
}

/// The unscheduled twin must qualify too, so a failure above is about the
/// binary trie rather than the resume-point predicate itself.
#[tokio::test]
async fn a_block_on_an_unscheduled_chain_is_a_full_sync_resume_point() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    make_canonical(&chains.twin_store, &chains.twin_blocks).await;

    let head = chains.twin_blocks.last().expect("a non-empty chain");
    assert!(
        ethrex_p2p::sync::is_resume_point(&chains.twin_store, &head.header)
            .expect("resume-point check"),
        "an MPT-committing chain's head must be a resume point"
    );
}

/// `debug_trace*` re-executes parents until it finds one whose state it holds.
/// At a post-flip block that walk must stop immediately: the parent's state is
/// in the binary trie. Otherwise tracing re-executes back to the flip and then
/// fails once the walk exceeds its `reexec` budget.
#[tokio::test]
async fn tracing_finds_no_missing_state_parents_at_a_post_flip_block() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    make_canonical(&chains.scheduled_store, &chains.scheduled_blocks).await;

    let head = chains.scheduled_blocks.last().expect("a non-empty chain");
    let missing = ethrex_blockchain::tracing::get_missing_state_parents(
        head.header.parent_hash,
        &chains.scheduled_store,
        128,
    )
    .await
    .expect("the parent walk must not exhaust its re-execution budget");

    assert!(
        missing.is_empty(),
        "tracing must find the post-flip parent's state immediately; it instead \
         wants to re-execute {} block(s), walking back across the activation",
        missing.len()
    );
}

/// Every remaining site is fixed the same way, but most sit inside large async
/// sync and engine-API handlers that no unit test reaches. This pins the class
/// instead of each site: no source file may ask the root-only
/// `has_state_root` about a *header's* `state_root`.
///
/// Two answers are legitimate and allowlisted below. Everything else asking
/// this question about a header is the bug that halted a devnet at the flip
/// block, made every restart replay from genesis, and would have made full sync
/// re-execute the whole post-activation chain.
#[test]
fn no_caller_asks_has_state_root_about_a_header() {
    /// (path suffix, why it is allowed)
    const ALLOWED: &[(&str, &str)] = &[
        (
            "crates/storage/store.rs",
            "the MPT branch inside `has_state_for_header` itself, plus tests \
             that deliberately probe unheld roots",
        ),
        (
            "crates/blockchain/vm.rs",
            "already branches on `binary_tree_active` explicitly, which is the \
             pattern `has_state_for_header` packages",
        ),
    ];

    fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
        let entries = std::fs::read_dir(dir).expect("readable directory");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan(&path, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("readable source file");
                let relative = path.to_string_lossy().replace('\\', "/");
                if ALLOWED
                    .iter()
                    .any(|(allowed, _)| relative.ends_with(allowed))
                {
                    continue;
                }
                for (index, line) in text.lines().enumerate() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") {
                        continue;
                    }
                    if line.contains("has_state_root(") && line.contains(".state_root") {
                        hits.push(format!("{relative}:{}: {}", index + 1, line.trim()));
                    }
                }
            }
        }
    }

    let root = workspace_root();
    let mut hits = Vec::new();
    // `tooling` is in scope deliberately: the ef-test runner had exactly this
    // bug and escaped an earlier sweep because the scan stopped at the crates.
    for area in ["crates", "cmd", "tooling"] {
        scan(&root.join(area), &mut hits);
    }

    assert!(
        hits.is_empty(),
        "these callers ask the MPT-only `has_state_root` about a header's \
         state_root, which answers `false` for every block after \
         `binaryTreeTime`. Use `Store::has_state_for_header(block_hash, \
         header)` instead:\n{}",
        hits.join("\n")
    );
}

// ---------------------------------------------------------------------------
// D8.2 The predicates that were previously inline.
//
// `engine_newPayload`'s state-materialization check, `eth_syncing`'s
// stateful-head check and the L2 committer's resume walk were all buried in
// handlers no unit test reaches, and were covered only by the source-scan
// guard. Each is now a named function, so each can be asserted on directly.
// ---------------------------------------------------------------------------

/// `newPayload` replies VALID without re-execution when a known block's state
/// is materialized, and stashes a payload as ACCEPTED when its parent's is not.
/// Both turn on this predicate; at a post-flip block it must say "materialized".
#[tokio::test]
async fn a_post_flip_blocks_state_reads_as_materialized() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    make_canonical(store, &chains.scheduled_blocks).await;

    let head = chains.scheduled_blocks.last().expect("a non-empty chain");

    // Flush first, and assert the layer cache does NOT answer for the head.
    // `state_is_materialized` is `in_layer_cache || on_disk`; with a warm cache
    // the first disjunct short-circuits and the disk check — the half that was
    // wrong — is never reached, so the assertion below would hold no matter
    // what that half returned.
    store
        .commit_trie_layers_for_test(head.header.state_root)
        .await
        .expect("flush the scheduled chain");
    store.wait_for_persistence_idle().await.expect("idle");
    assert!(
        !store
            .is_state_in_layer_cache(head.header.state_root)
            .expect("layer-cache probe"),
        "the head must have left the layer cache, or this test proves nothing \
         about the on-disk half of the predicate"
    );

    assert!(
        head.header.number > FLIP_BLOCK,
        "the head must be past the flip for this test to say anything"
    );
    assert!(
        ethrex_rpc::engine::payload::state_is_materialized(store, head.hash(), &head.header)
            .expect("materialization check"),
        "the post-flip head must read as materialized from disk; reading it as \
         absent makes newPayload re-execute or stash blocks whose state this \
         node holds"
    );
}

/// `eth_syncing` reports the canonical head only when its state is held, and
/// falls back to the executed head otherwise. Past the flip the canonical head
/// is held, so the node must not under-report itself as still syncing.
#[tokio::test]
async fn a_post_flip_canonical_head_reads_as_stateful() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    make_canonical(&chains.scheduled_store, &chains.scheduled_blocks).await;

    let head = chains.scheduled_blocks.last().expect("a non-empty chain");
    assert!(
        head.header.number > FLIP_BLOCK,
        "the head must be past the flip for this test to say anything"
    );

    assert!(
        ethrex_rpc::canonical_head_is_stateful(&chains.scheduled_store, head.header.number,)
            .await
            .expect("stateful-head check"),
        "the post-flip canonical head holds its state; reporting otherwise \
         makes eth_syncing advertise a head behind the real one"
    );
}

/// The L2 committer's resume walk now shares the L1 one. An L2 sets no
/// `binaryTreeTime`, so this asserts the shared walk stayed correct for the
/// unscheduled case rather than fixing a live L2 bug.
#[tokio::test]
async fn the_shared_resume_walk_stops_at_the_head_on_both_chains() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    make_canonical(&chains.scheduled_store, &chains.scheduled_blocks).await;
    make_canonical(&chains.twin_store, &chains.twin_blocks).await;

    for (label, store, blocks) in [
        (
            "scheduled",
            &chains.scheduled_store,
            &chains.scheduled_blocks,
        ),
        ("unscheduled", &chains.twin_store, &chains.twin_blocks),
    ] {
        let head = blocks.last().expect("a non-empty chain").header.number;
        let resume_from =
            ethrex_l2::sequencer::l1_committer::find_last_known_state_root(store, head)
                .await
                .expect("the shared resume walk must succeed");

        assert_eq!(
            resume_from, head,
            "the {label} chain's resume walk must stop at the head"
        );
    }
}

// ===========================================================================
// Phase D9 — `has_binary_trie_state` must be a presence check, not bookkeeping.
//
// `advance_binary_trie_for_block` writes the block -> root mapping into
// `BINARY_TRIE_ROOTS` *immediately and durably*, while the trie nodes backing
// that root are only staged into the in-memory diff layer. `Store::shutdown`
// deliberately leaves those layers in memory. So after a restart the mapping
// row survives for every block the node ever executed, and the nodes behind it
// do not.
//
// A predicate that only compares the recorded root therefore claims state the
// node cannot read. Measured on the 2026-08-07 devnet: a node restarted at head
// 54, resumed without replaying (the startup walk believed it held the state),
// then served *genesis alloc values* at every post-flip block and wedged
// permanently on `Insufficient account funds` while its peers ran to 182.
//
// That is strictly worse than the bug it replaced: the root-only check made the
// node re-execute from genesis, which was wasteful but recovered. This makes it
// fast and wrong. The MPT side has always had the real check — `has_state_root`
// reads the root node and hashes it, precisely because trie nodes are keyed by
// path, not by hash. The binary side needs the same.
// ===========================================================================

/// After the diff layers are lost — exactly what a restart does — a block whose
/// binary nodes never reached disk must NOT report its state as held.
#[tokio::test]
async fn binary_state_lost_with_the_diff_layers_is_not_reported_as_held() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    let head = chains.scheduled_blocks.last().expect("a non-empty chain");
    assert!(
        head.header.number > FLIP_BLOCK,
        "the head must be past the flip for this test to say anything"
    );

    // Nothing was flushed, so this block's nodes live only in the diff layers.
    // (Genesis seeding writes its own nodes straight to disk, so the table is
    // not empty; what matters is that *this* block's are not in it.)
    assert!(
        store
            .has_binary_trie_state(head.hash(), head.header.state_root)
            .unwrap(),
        "precondition: with the layers warm the state is genuinely readable"
    );

    // What a restart does: layers gone, disk keeps whatever reached it.
    store.drop_trie_layers_for_test().unwrap();

    // The bookkeeping row is durable and survives; this is the mechanism.
    assert_eq!(
        store.get_binary_trie_root(head.hash()).unwrap(),
        Some(head.header.state_root),
        "the block -> root mapping is written durably at import, so it outlives \
         the diff layers holding the nodes it names"
    );

    assert!(
        !store
            .has_binary_trie_state(head.hash(), head.header.state_root)
            .unwrap(),
        "the block -> root mapping outlives the nodes it refers to, so a \
         bookkeeping-only check claims state this node can no longer read. A \
         restarted node then resumes on absent state and wedges forever \
         instead of re-executing."
    );
}

/// The same, one level up: the predicate every reachability caller now uses
/// must not claim a post-flip header whose nodes are gone.
#[tokio::test]
async fn a_post_flip_header_with_no_nodes_on_disk_is_not_held() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    let head = chains.scheduled_blocks.last().expect("a non-empty chain");

    store.drop_trie_layers_for_test().unwrap();

    assert!(
        !store
            .has_state_for_header(head.hash(), &head.header)
            .unwrap(),
        "has_state_for_header must answer for state the node can actually read"
    );
}

/// The other direction, so the fix cannot be "always return false": once the
/// nodes are genuinely flushed, the state survives losing the diff layers.
#[tokio::test]
async fn binary_state_flushed_to_disk_survives_losing_the_diff_layers() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    let head = chains.scheduled_blocks.last().expect("a non-empty chain");

    store
        .commit_trie_layers_for_test(head.header.state_root)
        .await
        .expect("flush the scheduled chain");
    store.wait_for_persistence_idle().await.expect("idle");
    assert!(
        store.binary_trie_node_count_for_test().unwrap() > 0,
        "precondition: the flush really did write binary nodes"
    );

    store.drop_trie_layers_for_test().unwrap();

    assert!(
        store
            .has_state_for_header(head.hash(), &head.header)
            .unwrap(),
        "flushed binary state must still be held after the layers are dropped, \
         or every restart replays the whole chain again"
    );
}

// ---------------------------------------------------------------------------
// D8.3 The sync cycle's parent-state checks.
//
// The last three sites of this class. They gate whether the sync cycle may
// execute the blocks stacked on top of a given block, and all three were
// spelled out inline — which is why all three had to be found by hand, and why
// none of them had a test until now.
// ---------------------------------------------------------------------------

/// A post-flip block whose state this node holds is a base the sync cycle can
/// execute on, addressed either by hash or by number.
#[tokio::test]
async fn state_is_available_at_a_post_flip_block_by_hash_and_by_number() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    make_canonical(store, &chains.scheduled_blocks).await;

    let head = chains.scheduled_blocks.last().expect("a non-empty chain");
    assert!(
        head.header.number > FLIP_BLOCK,
        "the head must be past the flip for this test to say anything"
    );

    assert!(
        ethrex_p2p::sync::state_available_at(store, head.hash()).expect("by-hash check"),
        "the sync cycle must see the post-flip head's state; not seeing it \
         makes it skip or re-download blocks it can already build on"
    );
    assert!(
        ethrex_p2p::sync::state_available_at_number(store, head.header.number)
            .expect("by-number check"),
        "the by-number form must agree with the by-hash one"
    );
}

/// An unknown block is not a base to build on, and must not be reported as one.
#[tokio::test]
async fn state_is_not_available_at_an_unknown_block() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;

    assert!(
        !ethrex_p2p::sync::state_available_at(store, H256::repeat_byte(0xAB))
            .expect("by-hash check"),
        "an unknown header must answer false, not error and not true"
    );
    assert!(
        !ethrex_p2p::sync::state_available_at_number(store, 9_999).expect("by-number check"),
        "an unknown block number must answer false"
    );
}

/// And it must track real availability: state that never reached disk is gone
/// once the diff layers are, so the sync cycle must not try to build on it.
#[tokio::test]
async fn state_is_not_available_once_the_diff_layers_are_lost() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    make_canonical(store, &chains.scheduled_blocks).await;

    let head = chains.scheduled_blocks.last().expect("a non-empty chain");
    store.drop_trie_layers_for_test().unwrap();

    assert!(
        !ethrex_p2p::sync::state_available_at(store, head.hash()).expect("by-hash check"),
        "unflushed state is gone after a restart; reporting it as available \
         makes the sync cycle execute against state that is not there"
    );
}

// ---------------------------------------------------------------------------
// eth_syncing must not hide a node that is behind.
//
// `highestBlock` is the only number in the response that says how far the
// chain has actually got. When the last forkchoice head cannot be resolved
// locally — precisely the situation of a node too far behind to have
// downloaded it — the old code fell straight back to the local canonical head,
// so the target collapsed onto the current block and the node reported that it
// had arrived. Observed on the 2026-08-07 devnet: a node stuck 128 blocks
// behind, failing every sync cycle, answering
// `{"currentBlock":"0x36","highestBlock":"0x36"}`.
// ---------------------------------------------------------------------------

/// The regression: an unresolvable forkchoice head must report the sync
/// cycle's recorded target, not the local head.
#[tokio::test]
async fn eth_syncing_reports_the_recorded_target_when_the_head_is_unknown() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    make_canonical(store, &chains.scheduled_blocks).await;

    let canonical_head = store.get_latest_block_number().await.unwrap();
    let target = canonical_head + 128;

    let highest = ethrex_rpc::resolve_highest_block(
        store,
        H256::repeat_byte(0xAB), // a head this node has never seen
        Some(target),
        canonical_head,
    )
    .await
    .expect("highest-block resolution")
    .number();

    assert_eq!(
        highest, target,
        "a node that cannot resolve the forkchoice head is behind, not \
         arrived; reporting the local head as the target is what made a \
         wedged node advertise itself as caught up"
    );
}

/// A resolvable head still wins — the recorded target must not override what
/// the node actually knows.
#[tokio::test]
async fn eth_syncing_prefers_a_resolvable_forkchoice_head() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    make_canonical(store, &chains.scheduled_blocks).await;

    let head = chains.scheduled_blocks.last().expect("a non-empty chain");
    let canonical_head = store.get_latest_block_number().await.unwrap();

    let highest = ethrex_rpc::resolve_highest_block(
        store,
        head.hash(),
        Some(canonical_head + 500), // a stale/bogus recorded target
        canonical_head,
    )
    .await
    .expect("highest-block resolution")
    .number();

    assert_eq!(
        highest, head.header.number,
        "the resolved forkchoice head is authoritative when it is known"
    );
}

/// With no target ever recorded, the canonical head is the only honest answer.
#[tokio::test]
async fn eth_syncing_falls_back_to_the_canonical_head_with_no_target() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    make_canonical(store, &chains.scheduled_blocks).await;

    let canonical_head = store.get_latest_block_number().await.unwrap();
    let highest =
        ethrex_rpc::resolve_highest_block(store, H256::repeat_byte(0xAB), None, canonical_head)
            .await
            .expect("highest-block resolution")
            .number();

    assert_eq!(highest, canonical_head);
}

/// A stale recorded target must never drag the reported tip below the node's
/// own head, which would make a caught-up node look ahead of the chain.
#[tokio::test]
async fn eth_syncing_never_reports_a_target_below_the_local_head() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    make_canonical(store, &chains.scheduled_blocks).await;

    let canonical_head = store.get_latest_block_number().await.unwrap();
    let highest = ethrex_rpc::resolve_highest_block(
        store,
        H256::repeat_byte(0xAB),
        Some(1), // long-since-passed target
        canonical_head,
    )
    .await
    .expect("highest-block resolution")
    .number();

    assert_eq!(highest, canonical_head);
}

/// **The test the previous attempt should have been.** It supplies no
/// `recorded_target`, because that is production's actual state: `sync_target`
/// is written only by the full-sync cycle, and a restarted node behind a
/// healthy consensus client is fed its missing blocks by `engine_newPayload`
/// and never runs one. Measured on the 2026-08-07 devnet — a restart 145 blocks
/// behind logged zero sync cycles and caught up entirely through newPayload.
///
/// It also passes a zero `head_hash`, because `last_fcu_head` does not survive
/// a restart: it is an `Arc<Mutex<H256>>` initialised to `H256::zero()`.
///
/// So this is exactly what the handler sees in the seconds after a restart, and
/// in that state the node does not know where the chain head is. It must say so.
#[tokio::test]
async fn a_node_with_no_known_target_is_never_reported_synced() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    make_canonical(store, &chains.scheduled_blocks).await;
    let canonical_head = store.get_latest_block_number().await.unwrap();

    let target = ethrex_rpc::resolve_highest_block(store, H256::zero(), None, canonical_head)
        .await
        .expect("highest-block resolution");

    assert!(
        !target.is_known(),
        "a node that cannot resolve the forkchoice head and has never been \
         told a target does not know where the chain is; reporting its own \
         head as the target is a stand-in, not a measurement"
    );
    assert!(
        !ethrex_rpc::is_reported_synced(true, canonical_head, &target),
        "with the is_synced() latch set and highestBlock collapsed onto \
         currentBlock, the distance test is trivially true — which is how a \
         node 130 blocks behind answered `false` on the devnet"
    );
}

/// The complement: with a genuinely known target, the latch and the distance
/// test decide as before. Without this the fix could degenerate into "never
/// report synced".
#[tokio::test]
async fn a_node_at_a_known_target_is_reported_synced() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    make_canonical(store, &chains.scheduled_blocks).await;

    let head = chains.scheduled_blocks.last().expect("a non-empty chain");
    let canonical_head = store.get_latest_block_number().await.unwrap();

    let target = ethrex_rpc::resolve_highest_block(store, head.hash(), None, canonical_head)
        .await
        .expect("highest-block resolution");

    assert!(
        target.is_known(),
        "a resolvable forkchoice head is a real target"
    );
    assert!(
        ethrex_rpc::is_reported_synced(true, canonical_head, &target),
        "a latched node that has reached a known target is synced"
    );
    // Far below a known target. Expressed as a distant target rather than a
    // negative offset, because these chains are shorter than
    // SYNCED_HEAD_TOLERANCE allows for.
    let distant = ethrex_rpc::SyncTarget::Known(canonical_head + 500);
    assert!(
        !ethrex_rpc::is_reported_synced(true, canonical_head, &distant),
        "a node far below a known target is not synced, latch or no latch"
    );
    assert!(
        !ethrex_rpc::is_reported_synced(false, canonical_head, &target),
        "an unlatched node is not synced even at the target"
    );
}

// ===========================================================================
// Phase D5 — the MPT on a node that never had one.
//
// `store_block_inner` keeps merkleizing into the MPT after the flip; D3.5
// above pins that on a node which executed every block the MPT stays
// *correct*, just unaddressable by header root. A `pbtsnap/1` snap-sync
// client lands somewhere no full-sync node is ever in: it holds binary-trie
// state and `ACCOUNT_CODES` for a post-flip pivot and executed none of the
// blocks beneath it, so its MPT never advanced past the genesis alloc that
// `add_initial_state` writes at startup.
//
// The snap plan predicts that importing on such a pivot merely computes a
// meaningless MPT root nobody reads. These tests establish that the outcome
// is that, but not for the reason the prediction assumes, and the difference
// is the part worth pinning: MPT nodes are keyed by **path**, not by hash
// (`BackendTrieDB`, `TrieWrapper::get`), so opening the state trie at a root
// the store does not hold neither fails nor yields an empty trie — it reads
// whatever node sits at the root path, which on a snap-landed node is the
// *genesis* root node. Merkleization therefore succeeds by silently
// resuming from genesis state, and the root it commits is a hybrid of
// genesis and the blocks imported since.
//
// Had the read been hash-keyed, the same import would have failed with
// `TrieError::InconsistentTree` instead: `Trie::get` and `Trie::insert` do
// raise `RootNotFound` when the root path is *empty*, which is the shape a
// store with no genesis alloc would be in. Ethrex always writes a genesis
// alloc, so the silent-resume branch is the one a real snap node takes.
// ===========================================================================

/// Re-execute `blocks` against `donor` — which holds every trie — and return
/// the account updates each produced.
///
/// This is the whole of what a second store needs to reach the same *binary*
/// state, and it is deliberately all it gets: nothing here writes an MPT.
fn account_updates_for(donor: &Store, blocks: &[Block]) -> Vec<Vec<AccountUpdate>> {
    blocks
        .iter()
        .map(|block| {
            let parent = donor
                .get_block_header_by_hash(block.header.parent_hash)
                .expect("parent header read")
                .expect("parent header present");
            let vm_db: DynVmDatabase =
                Box::new(StoreVmDatabase::new(donor.clone(), parent).expect("vm db"));
            let mut vm = Evm::new_from_db_for_l1(Arc::new(vm_db), Arc::new(NativeCrypto));
            vm.execute_block(block).expect("re-execution must succeed");
            vm.get_state_transitions().expect("state transitions")
        })
        .collect()
}

/// A store in the shape `pbtsnap/1`'s landing leaves behind (plan Decision 8):
/// binary-trie state for the pivot written straight to the backend, bytecode in
/// `ACCOUNT_CODES`, the ancestry's headers and bodies, canonical pointers — and
/// an MPT holding nothing but the genesis alloc, because the node executed no
/// pre-pivot block.
///
/// The binary half is reached by replaying account updates rather than by a real
/// range download, and the writes go through
/// `apply_account_updates_to_binary_trie_blocking`, which commits nodes to the
/// backend with no diff layer — the state a landed snapshot is in. That the
/// result really is the pivot's state is checked against the donor's recorded
/// root by `a_snap_landed_store_holds_the_pivots_binary_state_over_an_empty_mpt`,
/// which is the same equality the plan's landing performs against
/// `pivot_header.state_root`.
async fn snap_landed_store(chains: &BoundaryChains, pivot: usize) -> Store {
    let blocks = &chains.scheduled_blocks[..=pivot];
    let updates = account_updates_for(&chains.scheduled_store, blocks);

    let store = store_from_genesis(chains.scheduled_genesis.clone()).await;
    let genesis_hash = store
        .get_block_header(0)
        .expect("genesis header read")
        .expect("genesis header present")
        .hash();
    let mut root = store
        .get_binary_trie_root(genesis_hash)
        .expect("genesis binary root read")
        .expect("genesis seeds a binary root");

    for (block, block_updates) in blocks.iter().zip(updates.iter()) {
        root = store
            .apply_account_updates_to_binary_trie_blocking(root, block_updates)
            .expect("binary-trie advance");
        store
            .set_binary_trie_root(block.hash(), root)
            .expect("record binary root");
    }

    store
        .add_blocks(blocks.to_vec())
        .await
        .expect("write the ancestry's headers and bodies");
    make_canonical(&store, blocks).await;
    store
}

/// The account the MPT answers with at `block`, through the same `state_trie`
/// entry point `apply_account_updates_batch` uses.
fn mpt_account_at(store: &Store, block: &Block, address: Address) -> Option<AccountState> {
    let trie = store
        .state_trie(block.hash())
        .expect("state trie open")
        .expect("state trie present");
    trie.get(keccak(address.as_bytes()).as_bytes())
        .expect("state trie lookup")
        .map(|encoded| AccountState::decode(&encoded).expect("decode account state"))
}

/// The EIP-4788 beacon-roots contract. It is in the fixture's alloc and its
/// storage is written by the system call in *every* block, so its storage root
/// is a fingerprint of how much history a trie has actually seen — which is
/// what separates a fabricated MPT from a real one.
fn beacon_roots_contract() -> Address {
    Address::from_slice(&hex::decode("000f3df6d732807ef1319fb7b8bb8522d0beac02").unwrap())
}

/// A post-flip pivot, a snap-landed store sitting at it, and the next block a
/// node holding the whole chain would import on top.
struct SnapLanded {
    chains: BoundaryChains,
    store: Store,
    pivot: Block,
    next: Block,
}

async fn snap_landed_at_post_flip_pivot() -> SnapLanded {
    let chains = build_boundary_chains(FLIP_BLOCK + BLOCKS_PAST_THE_FLIP).await;
    let pivot_index = chains.scheduled_blocks.len() - 1;
    let pivot = chains.scheduled_blocks[pivot_index].clone();

    // Built by the donor, which holds every trie, so `next` is a block a real
    // network would gossip rather than one the node under test produced.
    let donor_chain = Blockchain::default_with_store(chains.scheduled_store.clone());
    let signer: Signer = LocalSigner::new(test_secret_key()).into();
    let tx = transfer_tx(
        chains.scheduled_genesis.config.chain_id,
        chains.scheduled_blocks.len() as u64,
        &signer,
    )
    .await;
    donor_chain
        .add_transaction_to_pool(tx)
        .await
        .expect("tx should enter pool");
    let next = build_block(&chains.scheduled_store, &donor_chain, &pivot.header).await;

    let store = snap_landed_store(&chains, pivot_index).await;
    SnapLanded {
        chains,
        store,
        pivot,
        next,
    }
}

// ---------------------------------------------------------------------------
// D5.1 The fixture really is the condition under investigation.
//
// Everything below is worthless if the simulated node either lacks the pivot's
// binary state or secretly has MPT ancestry, so both halves are asserted
// before any conclusion is drawn from them.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_snap_landed_store_holds_the_pivots_binary_state_over_an_empty_mpt() {
    let SnapLanded {
        chains,
        store,
        pivot,
        ..
    } = snap_landed_at_post_flip_pivot().await;
    let sender = sender_from_key(&test_secret_key());

    assert!(
        pivot.header.timestamp >= chains.activation,
        "the pivot must be post-flip, which is what Decision 12 requires of one"
    );
    assert!(
        store
            .has_binary_trie_state(pivot.hash(), pivot.header.state_root)
            .expect("binary state check"),
        "the landed store must hold the pivot's binary state by the predicate the \
         whole plan gates on"
    );

    // The MPT half: block-addressed reads skip the root guard, so this is the
    // state merkleization will actually resume from. The donor is at the end
    // state; the snap-landed node is still at the genesis alloc.
    let landed = mpt_account_at(&store, &pivot, sender).expect("the funded sender is in the alloc");
    let donor = mpt_account_at(&chains.scheduled_store, &pivot, sender)
        .expect("the funded sender is in the alloc");

    assert_eq!(
        landed.nonce, 0,
        "a snap-landed node's MPT has seen no block, so the sender is still at its alloc nonce"
    );
    assert_eq!(
        donor.nonce,
        FLIP_BLOCK + BLOCKS_PAST_THE_FLIP,
        "guard against a vacuous pass: the donor really did advance its MPT this far"
    );
    assert_ne!(
        landed.balance, donor.balance,
        "the two MPTs must not coincidentally agree, or nothing below distinguishes them"
    );
}

// ---------------------------------------------------------------------------
// D5.2 Import succeeds, and consensus is unaffected either way.
//
// The answer to the plan's open question: post-flip MPT maintenance on a node
// with no MPT ancestry neither errors nor panics. It cannot, because the MPT
// read is path-keyed and finds the genesis root node.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn importing_on_a_snap_landed_pivot_succeeds_and_still_validates_the_binary_root() {
    let SnapLanded {
        store, next, pivot, ..
    } = snap_landed_at_post_flip_pivot().await;

    Blockchain::default_with_store(store.clone())
        .add_block(next.clone())
        .expect("importing on a snap-landed pivot must succeed, MPT ancestry or not");

    assert_eq!(
        store
            .get_binary_trie_root(next.hash())
            .expect("binary root read"),
        Some(next.header.state_root),
        "the header's root is the binary root, and the node reproduced it"
    );
    assert!(
        store
            .has_state_for_header(next.hash(), &next.header)
            .expect("state reachability"),
        "the imported block's state must be reachable, which is what forkchoice asks"
    );

    // The other half of "consensus is unaffected": a wrong root is still
    // refused. Without this the test above would also pass on a node that had
    // stopped checking anything.
    let mut tampered = next.clone();
    tampered.header.state_root = H256::repeat_byte(0x99);
    let err = Blockchain::default_with_store(store.clone())
        .add_block(tampered.clone())
        .expect_err("a block whose header commits the wrong binary root must be rejected");
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

    // And the MPT root the import computed addresses nothing: the header names
    // the binary root, and the MPT does not hold it.
    assert!(
        !store
            .has_state_root(next.header.state_root)
            .expect("root check"),
        "an active header's root is a binary root, so no MPT reader can resolve through it"
    );
    assert!(
        pivot.header.number > 0,
        "sanity: the pivot is a real block, not the genesis the MPT is stuck at"
    );
}

// ---------------------------------------------------------------------------
// D5.3 What the MPT it maintains actually contains.
//
// Not an empty trie and not the right one: a hybrid of the genesis alloc and
// the blocks imported since the pivot. Touched accounts get correct absolute
// values (the updates come from execution, which reads the binary trie), while
// everything the pre-pivot history changed and this block did not stays at its
// alloc value. This is the assertion that fails the moment post-flip MPT
// maintenance is skipped, which is why it is the mutation target for the
// design change D5 exists to inform.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_mpt_a_snap_landed_node_maintains_is_a_fabrication_over_genesis_state() {
    let SnapLanded {
        chains,
        store,
        next,
        ..
    } = snap_landed_at_post_flip_pivot().await;
    let sender = sender_from_key(&test_secret_key());

    Blockchain::default_with_store(store.clone())
        .add_block(next.clone())
        .expect("import");

    // The donor imports the same block, so the two MPTs differ only in the
    // history beneath them.
    Blockchain::default_with_store(chains.scheduled_store.clone())
        .add_block(next.clone())
        .expect("donor import");

    // The MPT did advance — it is not frozen and not empty. This is the line
    // that dies if `store_block_inner` stops merkleizing past the flip.
    let landed =
        mpt_account_at(&store, &next, sender).expect("the sender is in the fabricated MPT");
    assert_eq!(
        landed.nonce,
        FLIP_BLOCK + BLOCKS_PAST_THE_FLIP + 1,
        "the imported block's own updates carry absolute post-state values, so a \
         touched account lands at its true nonce even on a fabricated MPT"
    );

    // But it is a fabrication: the beacon-roots contract's storage ring has
    // seen exactly one block here and the whole chain on the donor, so the two
    // storage roots cannot agree.
    let landed_beacon = mpt_account_at(&store, &next, beacon_roots_contract())
        .expect("the beacon-roots contract is in the alloc");
    let donor_beacon = mpt_account_at(&chains.scheduled_store, &next, beacon_roots_contract())
        .expect("the beacon-roots contract is in the alloc");
    assert_ne!(
        landed_beacon.storage_root, donor_beacon.storage_root,
        "a snap-landed node's MPT must not match the one a node with real ancestry holds — \
         if these agree the fixture is not exercising a missing history"
    );
    assert_ne!(
        landed_beacon.storage_root, *EMPTY_TRIE_HASH,
        "guard against a vacuous pass: the fabricated MPT really did write storage"
    );
}
// Phase E — the single-version trie parked *past* the root it is asked to
// extend.
//
// Phase D9 covered one way a `BINARY_TRIE_ROOTS` row outlives the nodes it
// names: the nodes never reached disk. This is the other way round — the nodes
// reached disk and were then overwritten, because the binary trie is path-keyed
// and holds exactly one version of state.
//
// Why it matters beyond a curiosity: a `pbtsnap/1` install would land the
// pivot's state into that single slot, while genesis keeps the row
// `add_initial_state` wrote for it. A node that then falls back to full sync
// walks forward from genesis, and every pre-activation block shadow-tracks
// through `advance_binary_trie_for_block`, which gates on the recorded parent
// root and on nothing else.
//
// The tests below need no snap-sync seam: flushing a short chain's layers and
// dropping them parks the trie at block 3 with today's machinery, which is the
// same shape the seam would produce.
// ===========================================================================

/// A single fresh account, so the root that comes out depends entirely on the
/// base it was applied to.
fn wrong_base_probe_updates() -> Vec<AccountUpdate> {
    vec![AccountUpdate {
        address: Address::from_low_u64_be(0xD00D),
        info: Some(AccountInfo {
            balance: U256::from(7u64),
            nonce: 1,
            ..Default::default()
        }),
        ..Default::default()
    }]
}

/// Park the binary trie at block 3, then ask it to extend genesis.
///
/// Establishes the answer to "clean error, panic, or silent wrong base": it is
/// the third. `has_binary_trie_state` sees the problem perfectly well — the
/// precondition below asserts that — but `advance_binary_trie_for_block` never
/// asks it.
///
/// **This is a characterization test of a gap, not a specification.** It pins
/// down what the code does today so the gap cannot close or widen unnoticed;
/// the behaviour it records is the behaviour we want to remove. When
/// `advance_binary_trie_for_block` grows the missing presence check, this test
/// must be *inverted* — the `advance` below becomes an `expect_err` — not
/// deleted. Failing here after such a change is the guard working.
///
/// The check has to read through the gated DB `advance` already builds, not
/// through `binary_trie_holds_root`, whose gate is the root itself: for a
/// pre-activation block the layer is keyed by the parent's *header* (MPT) root,
/// so gating on the binary root waits for nothing and can miss a parent layer
/// that has not been installed yet.
#[tokio::test]
async fn extending_a_root_the_parked_binary_trie_no_longer_holds_is_refused() {
    let sender = sender_from_key(&test_secret_key());
    let genesis = load_funded_genesis(sender, Some(FAR_FUTURE_BINARY_TREE_TIME));
    let chain_id = genesis.config.chain_id;

    // The node under test: trie driven to block 3, flushed, layers dropped, so
    // the one on-disk version is block 3's.
    let store = store_from_genesis(genesis.clone()).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let blocks = build_chain(&store, &blockchain, chain_id, 3).await;
    let head = blocks.last().expect("a non-empty chain");

    let genesis_hash = store.get_block_header(0).unwrap().unwrap().hash();
    let genesis_binary_root = store
        .get_binary_trie_root(genesis_hash)
        .unwrap()
        .expect("genesis seeding records a binary root");

    store
        .commit_trie_layers_for_test(head.header.state_root)
        .await
        .unwrap();
    store.drop_trie_layers_for_test().unwrap();

    // Genesis's bookkeeping row is durable and still there ...
    assert_eq!(
        store.get_binary_trie_root(genesis_hash).unwrap(),
        Some(genesis_binary_root),
        "genesis keeps the binary root `add_initial_state` recorded for it"
    );
    // ... while the nodes behind it were overwritten in place by block 3.
    // This is the presence check doing its job, and it is the whole reason the
    // failure below is a gap in one caller rather than a gap in the predicate.
    assert!(
        !store
            .has_binary_trie_state(genesis_hash, genesis_binary_root)
            .unwrap(),
        "precondition: the presence check must see that genesis's binary state \
         is not what the single-version trie holds any more"
    );

    // A store whose trie really is at genesis extends it without complaint —
    // so the refusal below is about the trie's contents, not about the call.
    let clean = store_from_genesis(genesis.clone()).await;
    let clean_genesis_hash = clean.get_block_header(0).unwrap().unwrap().hash();
    clean
        .advance_binary_trie_for_block(
            H256::repeat_byte(0x11),
            clean_genesis_hash,
            &wrong_base_probe_updates(),
        )
        .expect("a trie that holds genesis extends it");

    // The parked node, asked the same question, must refuse. Before the guard
    // it answered — silently, with a root computed over block 3's state while
    // still reporting genesis as its parent, so nothing downstream could tell.
    let err = store
        .advance_binary_trie_for_block(
            H256::repeat_byte(0x11),
            genesis_hash,
            &wrong_base_probe_updates(),
        )
        .expect_err(
            "extending a root the trie no longer holds must refuse: the \
             recorded-root row outlives the nodes it names, and a path-keyed \
             open would resolve whatever is on disk instead",
        );

    assert!(
        matches!(
            err,
            StoreError::BinaryTrieRootNotHeld { parent_root, .. }
                if parent_root == genesis_binary_root
        ),
        "the refusal must name the root that is missing, not a generic error: {err:?}"
    );

    // Before the guard this call succeeded, and the base it silently used was
    // not arbitrary: it produced exactly block 3's successor, which a full-sync
    // fallback after a snapshot install would have recorded as block 1's root
    // and carried forward, with nothing detecting it until the flip block.
}

/// The same parked node at startup: the resume walk is *not* where this bites.
///
/// `last_block_with_state` asks `has_state_for_header`, which is the real
/// presence check on both tries, so it lands on a block whose state is genuinely
/// held. The hole is forward-only — in the shadow-track path, not the walk.
#[tokio::test]
async fn the_resume_walk_on_a_parked_binary_trie_lands_on_state_it_really_holds() {
    let chains = build_boundary_chains(FLIP_BLOCK + 2).await;
    let store = &chains.scheduled_store;
    let head = chains.scheduled_blocks.last().expect("a non-empty chain");
    assert!(
        head.header.number > FLIP_BLOCK,
        "the head must be past the flip, or the binary trie is never consulted"
    );

    // The commit gate only flushes a *canonical* root, exactly as the real one
    // driven by `forkchoice_update` does.
    make_canonical(store, &chains.scheduled_blocks).await;
    store
        .commit_trie_layers_for_test(head.header.state_root)
        .await
        .unwrap();
    store.drop_trie_layers_for_test().unwrap();

    // The trie is parked at the head, so the head is where the walk stops.
    assert_eq!(
        store.last_block_with_state(head.header.number).unwrap(),
        Some(head.header.number),
        "the resume walk stops at the head whose state the parked trie holds"
    );

    // Every *earlier* post-flip block still has its durable root row, and the
    // presence check refuses all of them, because the single-version trie moved
    // on. This is the behaviour that makes the resume walk safe, and it is
    // exactly the check `advance_binary_trie_for_block` skips.
    for block in &chains.scheduled_blocks {
        if block.header.number >= head.header.number || block.header.number < FLIP_BLOCK {
            continue;
        }
        assert_eq!(
            store.get_binary_trie_root(block.hash()).unwrap(),
            Some(block.header.state_root),
            "block {} keeps its durable root row",
            block.header.number
        );
        assert!(
            !store
                .has_binary_trie_state(block.hash(), block.header.state_root)
                .unwrap(),
            "block {}'s binary state was overwritten by the head's, and the \
             presence check must say so",
            block.header.number
        );
    }
}
