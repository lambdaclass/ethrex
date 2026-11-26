//! EIP-8288 dependency proof, from the pool to a block and its import.
//!
//! Accounts controlled by leanSPHINCS keys send `[VERIFY, SENDER]` transactions
//! whose signature list declares their dependencies, as before LStar: each one
//! is submitted with its signature and kept with it in the pool. From LStar the
//! block does not carry the signatures.
//! Its builder aggregates them into one proof, the header commits to the proof
//! and to the dependencies it proves, and every node that imports the block
//! verifies the proof against the dependencies of the block's transactions.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    dependency_proof::prove_dependencies,
    error::ChainError,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    errors::InvalidBlockError,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, Dependency, ELASTICITY_MULTIPLIER, Frame,
        FrameMode, FrameTransaction, Genesis, GenesisAccount, InvalidBlockBodyError,
        InvalidBlockHeaderError, LEANSPHINCS_SCHEME, LEANSPHINCS_SIGNATURE_SIZE,
        LEANSPHINCS_WITNESS_GAS, LEANSTARK_VERIFICATION_GAS, MAX_DEPENDENCY_PROOF_BYTES,
        ProofWrapper, Transaction, block_access_list::BlockAccessList, block_dependencies,
        compute_dependency_proof_hash, compute_dependency_witnesses_hash, compute_deps_hash,
        validate_block_body,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_p2p::rlpx::{
    eth::blocks::{BlockBodies, GetBlockBodies},
    message::RLPxMessage,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::{
    engine::payload::{GetPayloadV6Request, NewPayloadV5Request},
    rpc::RpcHandler,
    test_utils::default_context_with_storage,
    types::payload::ExecutionPayload,
};
use ethrex_storage::{EngineType, Store};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::levm::eip8504_tests::{account_code, dependency_entries};

const RECIPIENT: Address = H160([0x86; 20]);
const GWEI: u64 = 1_000_000_000;
/// Seconds between a block and its parent on the chains of these tests.
const BLOCK_TIME: u64 = 12;

/// A leanSPHINCS key pair and the account it controls.
struct Key {
    secret: [u8; 32],
    public: H256,
    account: Address,
}

fn key(seed: u8) -> Key {
    let secret = [seed; 32];
    Key {
        secret,
        public: H256(lean_sphincs::key_gen(&secret).to_bytes()),
        account: H160([seed; 20]),
    }
}

fn sign(key: &Key, message: H256) -> Bytes {
    Bytes::from(
        lean_sphincs::sign(&key.secret, message.as_fixed_bytes())
            .to_bytes()
            .to_vec(),
    )
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `l1-hegota.json` (every fork through Hegota active at genesis) with each
/// key's account funded and running the validation code for that key. LStar
/// activates `lstar_after` seconds after genesis, or never when it is `None`.
async fn setup_store(keys: &[&Key], lstar_after: Option<u64>) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    genesis.config.lstar_time = lstar_after.map(|seconds| genesis.timestamp + seconds);
    for key in keys {
        genesis.alloc.insert(
            key.account,
            GenesisAccount {
                code: Bytes::from(account_code(key.public)),
                storage: BTreeMap::new(),
                balance: U256::from(10u64).pow(U256::from(18u64)),
                nonce: 0,
            },
        );
    }
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

/// LStar from the first block on: the genesis block is the only Hegota block.
const LSTAR_AT_FIRST_BLOCK: Option<u64> = Some(BLOCK_TIME);

fn blockchain(store: Store) -> Blockchain {
    Blockchain::new(
        store,
        BlockchainOptions {
            min_tip_wei: 0,
            ..Default::default()
        },
    )
}

/// `[VERIFY, SENDER]` from the key's account: its validation frame and a call to
/// `RECIPIENT`. The signature list declares a signature of the transaction's own
/// signature hash under the key, followed by `more_declarations`.
fn dependency_transaction(
    chain_id: u64,
    key: &Key,
    nonce: u64,
    more_declarations: &[(H256, H256)],
) -> FrameTransaction {
    let mut declarations = vec![(H256::zero(), key.public)];
    declarations.extend_from_slice(more_declarations);
    // The validation frame points at entry 0 of the signature list.
    let verify_data = [0u8; 32];
    let mut tx = FrameTransaction {
        chain_id,
        nonce_keys: vec![],
        nonce_seq: nonce,
        sender: key.account,
        frames: vec![
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0x03,
                target: Some(key.account),
                gas_limit: 50_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::from(verify_data.to_vec()),
            },
            Frame {
                mode: u8::from(FrameMode::Sender),
                flags: 0,
                target: Some(RECIPIENT),
                gas_limit: 50_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ],
        signatures: dependency_entries(&declarations),
        max_priority_fee_per_gas: U256::from(GWEI),
        blob_versioned_hashes: Vec::new(),
        ..Default::default()
    };
    tx.max_fee = U256::from(10 * GWEI) * U256::from(tx.max_gas());
    tx
}

/// The transaction's wrapper, with each dependency signed by the key it names.
fn wrapper(tx: FrameTransaction, keys: &[&Key]) -> ProofWrapper {
    let witnesses = tx
        .dependencies()
        .iter()
        .map(|dependency| {
            let key = keys
                .iter()
                .find(|key| key.public == dependency.verification_key_hash)
                .expect("a key for every dependency");
            sign(key, dependency.data_hash)
        })
        .collect();
    ProofWrapper::new(tx, None, witnesses)
}

/// The block a builder starts from on top of `parent`.
fn payload_on(store: &Store, parent: &BlockHeader) -> Block {
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
    create_payload(&args, store, Bytes::new()).unwrap()
}

fn build_block(store: &Store, blockchain: &Blockchain, parent: &BlockHeader) -> PayloadBuildResult {
    blockchain.build_payload(payload_on(store, parent)).unwrap()
}

fn genesis_header(store: &Store) -> BlockHeader {
    store.get_block_header(0).unwrap().unwrap()
}

/// The dependency a signature of `message` under `key` satisfies.
fn dependency(key: &Key, message: H256) -> Dependency {
    Dependency {
        scheme: LEANSPHINCS_SCHEME,
        data_hash: message,
        verification_key_hash: key.public,
    }
}

/// A proof of the given `(key, message)` pairs, each signed here.
fn proof_of(claims: &[(&Key, H256)]) -> Bytes {
    let signed: Vec<_> = claims
        .iter()
        .map(|(key, message)| (dependency(key, *message), sign(key, *message)))
        .collect();
    prove_dependencies(&signed).expect("valid signatures aggregate")
}

/// The block with its dependency proof replaced and the header committing to the
/// new proof, so only the proof itself can make it invalid.
fn with_proof(mut block: Block, proof: Bytes) -> Block {
    block.header.dependency_proof_hash = Some(compute_dependency_proof_hash(&proof, &NativeCrypto));
    block.body.dependency_proof = Some(proof);
    block.header.hash = Default::default();
    block
}

/// Imports the block into two fresh nodes, on the two paths a block takes, and
/// returns what each one answered: the plain import first, then the pipeline.
async fn import(
    block: &Block,
    bal: &BlockAccessList,
    keys: &[&Key],
    lstar_after: Option<u64>,
) -> [Result<(), ChainError>; 2] {
    let (store, _) = setup_store(keys, lstar_after).await;
    let plain = blockchain(store).add_block(block.clone());
    let (store, _) = setup_store(keys, lstar_after).await;
    let pipeline = blockchain(store).add_block_pipeline(block.clone(), Some(Arc::new(bal.clone())));
    [plain, pipeline]
}

/// Asserts that both import paths reject the block with an error matching the
/// pattern.
macro_rules! assert_rejected {
    ($results:expr, $pattern:pat) => {
        for result in $results {
            assert!(
                matches!(result, Err(ChainError::InvalidBlock($pattern))),
                "got {result:?}"
            );
        }
    };
}

/// A node with two dependency transactions in its pool, from `first` and from
/// `second`, and the LStar block it builds from them: two dependencies, one for
/// each transaction's signature hash.
struct LStarBlock {
    block: Block,
    bal: BlockAccessList,
    /// The signature hashes of the two transactions, in the order of the keys.
    messages: [H256; 2],
}

async fn lstar_block(first: &Key, second: &Key) -> LStarBlock {
    let (store, chain_id) = setup_store(&[first, second], LSTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());
    let mut messages = [H256::zero(); 2];
    for (index, sender) in [first, second].into_iter().enumerate() {
        let tx = dependency_transaction(chain_id, sender, 0, &[]);
        messages[index] = tx.compute_sig_hash();
        blockchain
            .add_dependency_transaction_to_pool(wrapper(tx, &[sender]))
            .await
            .expect("the dependency transaction enters the pool");
    }
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    assert_eq!(built.payload.body.transactions.len(), 2);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));
    LStarBlock {
        block: built.payload,
        bal: built.block_access_list.expect("an LStar block has a BAL"),
        messages,
    }
}

#[tokio::test]
async fn lstar_block_carries_one_proof_for_its_dependencies_and_imports() {
    let first = key(0x61);
    let second = key(0x62);
    let third = key(0x63);
    let shared = key(0x64);
    let keys = [&first, &second, &third];
    let (store, chain_id) = setup_store(&keys, LSTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());

    // The first two transactions also depend on a signature of the same message
    // under a fourth key; the third depends on its own signature hash alone.
    let message = H256::repeat_byte(0x77);
    let mut wrappers = Vec::new();
    for sender in [&first, &second] {
        let tx = dependency_transaction(chain_id, sender, 0, &[(message, shared.public)]);
        wrappers.push(wrapper(tx, &[sender, &shared]));
    }
    wrappers.push(wrapper(
        dependency_transaction(chain_id, &third, 0, &[]),
        &[&third],
    ));
    for wrapper in &wrappers {
        blockchain
            .add_dependency_transaction_to_pool(wrapper.clone())
            .await
            .expect("the dependency transaction enters the pool with its witnesses");
    }

    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 3);
    assert!(
        built.receipts.iter().all(|receipt| receipt.succeeded),
        "the transactions must succeed: {:?}",
        built.receipts
    );

    // Five entries of the signature lists, four distinct dependencies: the
    // shared one counts once.
    let dependencies = block_dependencies(&block.body.transactions);
    assert_eq!(dependencies.len(), 4);

    // The block has no witness list and no commitment to one. It carries a proof,
    // and the header commits to it and to the dependencies.
    assert_eq!(block.body.dependency_witnesses, None);
    assert_eq!(block.header.dependency_witnesses_hash, None);
    let proof = block
        .body
        .dependency_proof
        .clone()
        .expect("an LStar block has a dependency proof");
    assert!(!proof.is_empty());
    assert!(proof.len() <= MAX_DEPENDENCY_PROOF_BYTES);
    assert!(built.unproven_dependencies.is_empty());
    assert_eq!(
        block.header.dependency_proof_hash,
        Some(compute_dependency_proof_hash(&proof, &NativeCrypto))
    );
    assert_eq!(
        block.header.block_deps_hash,
        Some(compute_deps_hash(&dependencies, &NativeCrypto))
    );

    // The transactions stay in the pool with their witnesses until the block is
    // imported: the block did not take the signatures, it proved them.
    for wrapper in &wrappers {
        let tx_hash = Transaction::FrameTransaction(wrapper.tx.clone()).hash(&NativeCrypto);
        assert_eq!(
            blockchain
                .mempool
                .get_dependency_witnesses(tx_hash)
                .unwrap(),
            Some(wrapper.witnesses.clone())
        );
    }

    // It imports on every path.
    let bal = built.block_access_list.expect("an LStar block has a BAL");
    for result in import(&block, &bal, &keys, LSTAR_AT_FIRST_BLOCK).await {
        result.expect("the block imports");
    }

    // The same transactions in a Hegota block, on a chain without LStar: the
    // block carries a witness for each dependency.
    let (hegota_store, _) = setup_store(&keys, None).await;
    let hegota_blockchain = self::blockchain(hegota_store.clone());
    for wrapper in &wrappers {
        hegota_blockchain
            .add_dependency_transaction_to_pool(wrapper.clone())
            .await
            .expect("the dependency transaction enters the pool with its witnesses");
    }
    let hegota_built = build_block(
        &hegota_store,
        &hegota_blockchain,
        &genesis_header(&hegota_store),
    );
    let hegota_block = hegota_built.payload;
    assert_eq!(
        hegota_block.body.transactions.len(),
        block.body.transactions.len()
    );
    for tx in &block.body.transactions {
        assert!(hegota_block.body.transactions.contains(tx));
    }
    assert_eq!(hegota_block.body.dependency_proof, None);
    assert_eq!(
        hegota_block
            .body
            .dependency_witnesses
            .as_ref()
            .map(|witnesses| witnesses.len()),
        Some(4)
    );
    // The proof is not part of the RLP block; the witnesses are.
    println!(
        "LStar block: {} bytes of RLP block and {} bytes of dependency proof, {} gas used. \
         Hegota block with the same transactions: {} bytes of RLP block, of which {} bytes \
         are witnesses, {} gas used.",
        block.length_without_dependency_proof(),
        proof.len(),
        block.header.gas_used,
        hegota_block.length(),
        4 * LEANSPHINCS_SIGNATURE_SIZE,
        hegota_block.header.gas_used,
    );
    assert!(block.length_without_dependency_proof() < hegota_block.length());
}

#[tokio::test]
async fn dependency_proof_is_stored_and_served_with_the_block_body() {
    let first = key(0x7f);
    let second = key(0x80);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second).await;
    let proof = built.block.body.dependency_proof.clone().unwrap();

    // A node that imports the block keeps the proof with the block's body.
    let (store, _) = setup_store(&keys, LSTAR_AT_FIRST_BLOCK).await;
    blockchain(store.clone())
        .add_block(built.block.clone())
        .expect("the block imports");
    let stored = store
        .get_block_body_by_hash(built.block.hash())
        .await
        .unwrap()
        .expect("the body is stored");
    assert_eq!(stored.dependency_proof, Some(proof.clone()));
    assert_eq!(stored.dependency_witnesses, None);

    // It serves the body with the proof to a peer that asks for it, and the
    // peer checks what it receives against the header it already has.
    let request = GetBlockBodies::new(1, vec![built.block.hash()]);
    let response = BlockBodies::new(1, request.fetch_blocks(&store).await);
    let mut encoded = Vec::new();
    response.encode(&mut encoded).unwrap();
    let received = BlockBodies::decode(&encoded).unwrap().block_bodies;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].dependency_proof, Some(proof));
    validate_block_body(&built.block.header, &received[0], &NativeCrypto)
        .expect("the body matches the header");

    // A body served with another proof is not the body of that header.
    let mut tampered = received[0].clone();
    tampered.dependency_proof = Some(Bytes::from(vec![0xEE; 10]));
    let result = validate_block_body(&built.block.header, &tampered, &NativeCrypto);
    assert!(
        matches!(
            result,
            Err(InvalidBlockBodyError::DependencyProofHashNotMatch)
        ),
        "got {result:?}"
    );
}

#[tokio::test]
async fn block_whose_proof_misses_a_dependency_is_rejected() {
    let first = key(0x65);
    let second = key(0x66);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second).await;

    // A proof that the first transaction's dependency is satisfied says nothing
    // about the second's.
    let partial = proof_of(&[(&first, built.messages[0])]);
    let results = import(
        &with_proof(built.block, partial),
        &built.bal,
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(results, InvalidBlockError::InvalidDependencyProof(2));
}

#[tokio::test]
async fn block_whose_proof_is_for_another_message_or_key_is_rejected() {
    let first = key(0x67);
    let second = key(0x68);
    let other = key(0x69);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second).await;

    // The first key signed another message than the transaction's signature hash.
    let another_message = proof_of(&[
        (&first, H256::repeat_byte(0x11)),
        (&second, built.messages[1]),
    ]);
    let results = import(
        &with_proof(built.block.clone(), another_message),
        &built.bal,
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(results, InvalidBlockError::InvalidDependencyProof(2));

    // Another key signed the first transaction's signature hash.
    let another_key = proof_of(&[(&other, built.messages[0]), (&second, built.messages[1])]);
    let results = import(
        &with_proof(built.block.clone(), another_key),
        &built.bal,
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(results, InvalidBlockError::InvalidDependencyProof(2));

    // A proof of the block's dependencies and of one more claim is not a proof
    // of exactly the block's dependencies.
    let one_more = proof_of(&[
        (&first, built.messages[0]),
        (&second, built.messages[1]),
        (&other, H256::repeat_byte(0x22)),
    ]);
    let results = import(
        &with_proof(built.block, one_more),
        &built.bal,
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(results, InvalidBlockError::InvalidDependencyProof(2));
}

#[tokio::test]
async fn block_with_a_damaged_or_missing_proof_is_rejected() {
    let first = key(0x6a);
    let second = key(0x6b);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second).await;
    let proof = built.block.body.dependency_proof.clone().unwrap();

    // One byte flipped.
    let mut flipped = proof.to_vec();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0x01;
    // Cut short.
    let truncated = proof.slice(..proof.len() - 1);
    let half = proof.slice(..proof.len() / 2);
    for damaged in [Bytes::from(flipped), truncated, half] {
        let results = import(
            &with_proof(built.block.clone(), damaged),
            &built.bal,
            &keys,
            LSTAR_AT_FIRST_BLOCK,
        )
        .await;
        assert_rejected!(results, InvalidBlockError::InvalidDependencyProof(2));
    }

    // The empty proof, which only a block without dependencies carries.
    let results = import(
        &with_proof(built.block.clone(), Bytes::new()),
        &built.bal,
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(results, InvalidBlockError::InvalidDependencyProof(2));

    // No proof at all, with the header still committing to one.
    let mut proofless = built.block;
    proofless.body.dependency_proof = None;
    let results = import(&proofless, &built.bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(results, InvalidBlockError::DependencyProofNotPresent);
}

#[tokio::test]
async fn block_without_dependencies_must_carry_an_empty_proof() {
    let first = key(0x6c);
    let second = key(0x6d);
    let keys = [&first, &second];
    // A valid proof, of the dependencies of another block.
    let other = lstar_block(&first, &second).await;
    let proof = other.block.body.dependency_proof.unwrap();

    // An LStar block without transactions: its proof is empty, and the header
    // commits to the empty proof and to the empty list of dependencies.
    let (store, _) = setup_store(&keys, LSTAR_AT_FIRST_BLOCK).await;
    let built = build_block(&store, &blockchain(store.clone()), &genesis_header(&store));
    let block = built.payload;
    let bal = built.block_access_list.expect("an LStar block has a BAL");
    assert!(block.body.transactions.is_empty());
    assert_eq!(block.body.dependency_proof, Some(Bytes::new()));
    assert_eq!(block.body.dependency_witnesses, None);
    assert_eq!(
        block.header.dependency_proof_hash,
        Some(compute_dependency_proof_hash(&[], &NativeCrypto))
    );
    assert_eq!(
        block.header.block_deps_hash,
        Some(compute_deps_hash(&[], &NativeCrypto))
    );
    for result in import(&block, &bal, &keys, LSTAR_AT_FIRST_BLOCK).await {
        result.expect("the block imports");
    }

    // With a proof it has nothing to prove, it is invalid, however valid the
    // proof is for other dependencies.
    let proof_len = proof.len();
    let results = import(&with_proof(block, proof), &bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    for result in results {
        assert!(
            matches!(
                result,
                Err(ChainError::InvalidBlock(InvalidBlockError::DependencyProofNotEmpty(len)))
                    if len == proof_len
            ),
            "got {result:?}"
        );
    }
}

#[tokio::test]
async fn block_whose_header_does_not_commit_to_its_proof_and_dependencies_is_rejected() {
    let first = key(0x6e);
    let second = key(0x6f);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second).await;

    // A wrong `dependency_proof_hash`: the proof is the right one, the header
    // commits to another.
    let mut wrong_proof_hash = built.block.clone();
    wrong_proof_hash.header.dependency_proof_hash = Some(H256::repeat_byte(0xab));
    wrong_proof_hash.header.hash = Default::default();
    let results = import(&wrong_proof_hash, &built.bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidBody(InvalidBlockBodyError::DependencyProofHashNotMatch)
    );

    // A wrong `block_deps_hash`: here, the digest of the dependencies in another
    // order than `block_dependencies`.
    let mut reversed = block_dependencies(&built.block.body.transactions);
    reversed.reverse();
    let mut wrong_deps_hash = built.block.clone();
    wrong_deps_hash.header.block_deps_hash = Some(compute_deps_hash(&reversed, &NativeCrypto));
    wrong_deps_hash.header.hash = Default::default();
    let results = import(&wrong_deps_hash, &built.bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidBody(InvalidBlockBodyError::BlockDepsHashNotMatch)
    );

    // A header without one of the two commitments.
    let mut without_deps_hash = built.block.clone();
    without_deps_hash.header.block_deps_hash = None;
    without_deps_hash.header.hash = Default::default();
    let results = import(&without_deps_hash, &built.bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::BlockDepsHashNotPresent)
    );
    let mut without_proof_hash = built.block;
    without_proof_hash.header.dependency_proof_hash = None;
    without_proof_hash.header.hash = Default::default();
    let results = import(&without_proof_hash, &built.bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::DependencyProofHashNotPresent)
    );
}

#[tokio::test]
async fn block_with_a_proof_over_the_size_bound_is_rejected() {
    let first = key(0x70);
    let second = key(0x71);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second).await;
    let proof = built.block.body.dependency_proof.clone().unwrap();

    // The valid proof followed by padding up to one byte over the bound. The
    // size is checked before anything reads the proof.
    let mut oversized = proof.to_vec();
    oversized.resize(MAX_DEPENDENCY_PROOF_BYTES + 1, 0);
    let results = import(
        &with_proof(built.block.clone(), Bytes::from(oversized)),
        &built.bal,
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    for result in results {
        assert!(
            matches!(
                result,
                Err(ChainError::InvalidBlock(InvalidBlockError::InvalidBody(
                    InvalidBlockBodyError::DependencyProofTooLarge(len)
                ))) if len == MAX_DEPENDENCY_PROOF_BYTES + 1
            ),
            "got {result:?}"
        );
    }

    // At the bound the size is accepted, and the bytes are judged as a proof.
    let mut at_the_bound = proof.to_vec();
    at_the_bound.resize(MAX_DEPENDENCY_PROOF_BYTES, 0);
    let results = import(
        &with_proof(built.block, Bytes::from(at_the_bound)),
        &built.bal,
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(results, InvalidBlockError::InvalidDependencyProof(2));
}

#[tokio::test]
async fn lstar_block_with_a_witness_list_is_rejected() {
    let first = key(0x72);
    let second = key(0x73);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second).await;
    // The witnesses a Hegota block would carry for these transactions.
    let witnesses = vec![
        sign(&first, built.messages[0]),
        sign(&second, built.messages[1]),
    ];

    // The list beside the proof, without the header committing to it.
    let mut with_list = built.block.clone();
    with_list.body.dependency_witnesses = Some(witnesses.clone());
    let results = import(&with_list, &built.bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(results, InvalidBlockError::DependencyWitnessesPresent);

    // An empty list is still a list. A list a block carries is held to the
    // block's dependencies first, and this one has none of their witnesses.
    let mut with_empty_list = built.block;
    with_empty_list.body.dependency_witnesses = Some(Vec::new());
    let results = import(&with_empty_list, &built.bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::DependencyWitnessesCountMismatch {
            expected: 2,
            found: 0
        }
    );

    // The list with its commitment in the header, beside the proof.
    let mut committed = with_list;
    committed.header.dependency_witnesses_hash =
        Some(compute_dependency_witnesses_hash(&witnesses, &NativeCrypto));
    committed.header.hash = Default::default();
    let results = import(&committed, &built.bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::DependencyWitnessesHashPresent)
    );

    // The block as Hegota would have it: the list and its commitment, and no
    // proof.
    let mut hegota_shaped = committed;
    hegota_shaped.body.dependency_proof = None;
    hegota_shaped.header.dependency_proof_hash = None;
    hegota_shaped.header.block_deps_hash = None;
    hegota_shaped.header.hash = Default::default();
    let results = import(&hegota_shaped, &built.bal, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::DependencyWitnessesHashPresent)
    );
}

#[tokio::test]
async fn hegota_block_with_a_dependency_proof_is_rejected() {
    let first = key(0x74);
    let second = key(0x75);
    let keys = [&first, &second];
    // On this chain LStar never activates: the first block is a Hegota block.
    let (store, chain_id) = setup_store(&keys, None).await;
    let blockchain = blockchain(store.clone());
    let mut messages = Vec::new();
    for sender in keys {
        let tx = dependency_transaction(chain_id, sender, 0, &[]);
        messages.push(tx.compute_sig_hash());
        blockchain
            .add_dependency_transaction_to_pool(wrapper(tx, &[sender]))
            .await
            .expect("the dependency transaction enters the pool");
    }
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    let bal = built.block_access_list.expect("a Hegota block has a BAL");
    assert_eq!(block.body.transactions.len(), 2);
    assert_eq!(block.body.dependency_proof, None);
    assert_eq!(block.header.dependency_proof_hash, None);
    assert_eq!(block.header.block_deps_hash, None);
    for result in import(&block, &bal, &keys, None).await {
        result.expect("the Hegota block imports with its witnesses");
    }

    // A valid proof of the block's dependencies, beside its witnesses.
    let proof = proof_of(&[(&first, messages[0]), (&second, messages[1])]);
    let mut with_proof_only = block.clone();
    with_proof_only.body.dependency_proof = Some(proof.clone());
    let results = import(&with_proof_only, &bal, &keys, None).await;
    assert_rejected!(results, InvalidBlockError::DependencyProofPresent);

    // The proof with its commitments in the header.
    let mut committed = with_proof_only;
    committed.header.dependency_proof_hash =
        Some(compute_dependency_proof_hash(&proof, &NativeCrypto));
    committed.header.block_deps_hash = Some(compute_deps_hash(
        &block_dependencies(&block.body.transactions),
        &NativeCrypto,
    ));
    committed.header.hash = Default::default();
    let results = import(&committed, &bal, &keys, None).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::DependencyProofHashPresent)
    );

    // The block as LStar would have it: the proof in place of the witnesses.
    let mut lstar_shaped = committed;
    lstar_shaped.body.dependency_witnesses = None;
    lstar_shaped.header.dependency_witnesses_hash = None;
    lstar_shaped.header.hash = Default::default();
    let results = import(&lstar_shaped, &bal, &keys, None).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(
            InvalidBlockHeaderError::DependencyWitnessesHashNotPresent
        )
    );
}

#[tokio::test]
async fn account_keeps_authorizing_with_the_same_key_across_the_fork() {
    let key = key(0x76);
    let keys = [&key];
    // Hegota at genesis and for the first block; LStar from the second block on.
    let lstar_after = Some(2 * BLOCK_TIME);
    let (store, chain_id) = setup_store(&keys, lstar_after).await;
    let blockchain = blockchain(store.clone());
    let genesis = genesis_header(&store);
    let account_at_genesis = store
        .get_account_state(0, key.account)
        .await
        .unwrap()
        .expect("the account is in the genesis state");

    // The account authorizes a transaction in a Hegota block: the block carries
    // the signature.
    let first_wrapper = wrapper(dependency_transaction(chain_id, &key, 0, &[]), &keys);
    let first_witness = first_wrapper.witnesses[0].clone();
    blockchain
        .add_dependency_transaction_to_pool(first_wrapper)
        .await
        .expect("the dependency transaction enters the pool");
    let first_built = build_block(&store, &blockchain, &genesis);
    let first_block = first_built.payload;
    assert_eq!(first_block.body.transactions.len(), 1);
    assert!(first_built.receipts[0].succeeded);
    assert_eq!(
        first_block.body.dependency_witnesses,
        Some(vec![first_witness])
    );
    assert_eq!(first_block.body.dependency_proof, None);
    blockchain
        .add_block(first_block.clone())
        .expect("the Hegota block imports");
    apply_fork_choice(&store, first_block.hash(), H256::zero(), H256::zero(), None)
        .await
        .expect("the Hegota block becomes the head");
    blockchain
        .remove_block_transactions_from_pool(&first_block)
        .unwrap();

    // The next transaction of the account is signed and enters the pool while the
    // head is still a Hegota block.
    assert!(
        !store
            .get_chain_config()
            .is_lstar_activated(first_block.header.timestamp)
    );
    let second_tx = dependency_transaction(chain_id, &key, 1, &[]);
    let second_message = second_tx.compute_sig_hash();
    let second_wrapper = wrapper(second_tx, &keys);
    let second_witness = second_wrapper.witnesses[0].clone();
    let second_hash = blockchain
        .add_dependency_transaction_to_pool(second_wrapper)
        .await
        .expect("the dependency transaction enters the pool before LStar");

    // The chain crosses LStar. The block that includes the transaction proves the
    // signature the pool holds, the one made before the fork, and does not carry
    // it.
    assert_eq!(
        blockchain
            .mempool
            .get_dependency_witnesses(second_hash)
            .unwrap(),
        Some(vec![second_witness])
    );
    let second_built = build_block(&store, &blockchain, &first_block.header);
    let second_block = second_built.payload;
    assert!(
        store
            .get_chain_config()
            .is_lstar_activated(second_block.header.timestamp)
    );
    assert_eq!(second_block.body.transactions.len(), 1);
    assert_eq!(
        second_block.body.transactions[0].hash(&NativeCrypto),
        second_hash,
        "the block includes the transaction that was signed before the fork"
    );
    assert!(second_built.receipts[0].succeeded);
    assert_eq!(second_block.body.dependency_witnesses, None);
    let proof = second_block
        .body
        .dependency_proof
        .clone()
        .expect("the LStar block has a dependency proof");
    assert_eq!(
        block_dependencies(&second_block.body.transactions),
        vec![dependency(&key, second_message)]
    );
    assert!(!proof.is_empty());
    blockchain
        .add_block(second_block.clone())
        .expect("the LStar block imports");
    apply_fork_choice(
        &store,
        second_block.hash(),
        H256::zero(),
        H256::zero(),
        None,
    )
    .await
    .expect("the LStar block becomes the head");

    // The account is the one of the genesis state: same code, same storage. Only
    // its nonce and its balance moved.
    for (number, nonce) in [(1, 1), (2, 2)] {
        let account = store
            .get_account_state(number, key.account)
            .await
            .unwrap()
            .expect("the account is in the state");
        assert_eq!(account.code_hash, account_at_genesis.code_hash);
        assert_eq!(account.storage_root, account_at_genesis.storage_root);
        assert_eq!(account.nonce, nonce);
    }

    // Another node imports the two blocks, one of each fork, on both paths.
    let (other_store, _) = setup_store(&keys, lstar_after).await;
    let other = self::blockchain(other_store);
    other
        .add_block(first_block.clone())
        .expect("plain import of the Hegota block");
    other
        .add_block(second_block.clone())
        .expect("plain import of the LStar block");
    let (other_store, _) = setup_store(&keys, lstar_after).await;
    let other = self::blockchain(other_store);
    other
        .add_block_pipeline(
            first_block.clone(),
            first_built.block_access_list.map(Arc::new),
        )
        .expect("pipeline import of the Hegota block");
    other
        .add_block_pipeline(
            second_block.clone(),
            second_built.block_access_list.map(Arc::new),
        )
        .expect("pipeline import of the LStar block");

    // The two transactions do the same work. What they paid differs by the
    // charge for the dependency: a witness before LStar, a share of the proof
    // from it.
    let gas_before = first_built.receipts[0].cumulative_gas_used;
    let gas_after = second_built.receipts[0].cumulative_gas_used;
    println!(
        "Gas of the transaction in the Hegota block: {gas_before}. \
         Gas of the transaction in the LStar block: {gas_after}."
    );
    assert_eq!(first_block.header.gas_used, gas_before);
    assert_eq!(second_block.header.gas_used, gas_after);
    assert_eq!(
        gas_before - gas_after,
        LEANSPHINCS_WITNESS_GAS - LEANSTARK_VERIFICATION_GAS
    );
}

#[tokio::test]
async fn payload_build_loop_proves_the_payload_it_returns() {
    let first = key(0x77);
    let second = key(0x78);
    let keys = [&first, &second];
    let (store, chain_id) = setup_store(&keys, LSTAR_AT_FIRST_BLOCK).await;
    let blockchain = Arc::new(blockchain(store.clone()));
    for sender in keys {
        blockchain
            .add_dependency_transaction_to_pool(wrapper(
                dependency_transaction(chain_id, sender, 0, &[]),
                &[sender],
            ))
            .await
            .expect("the dependency transaction enters the pool");
    }

    // The loop's first build runs up to its first await. Cancelling it, as
    // `engine_getPayload` does, ends the loop with that build.
    let token = CancellationToken::new();
    let mut building = std::pin::pin!(
        blockchain
            .clone()
            .build_payload_loop(payload_on(&store, &genesis_header(&store)), token.clone())
    );
    std::future::poll_fn(|cx| {
        assert!(
            building.as_mut().poll(cx).is_pending(),
            "the build loop finished before it was cancelled"
        );
        std::task::Poll::Ready(())
    })
    .await;
    token.cancel();
    let built = building.await.expect("the payload is built and proven");

    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 2);
    assert!(built.unproven_dependencies.is_empty());
    let proof = block.body.dependency_proof.clone().unwrap();
    assert!(!proof.is_empty());
    assert_eq!(
        block.header.dependency_proof_hash,
        Some(compute_dependency_proof_hash(&proof, &NativeCrypto))
    );
    let bal = built.block_access_list.expect("an LStar block has a BAL");
    for result in import(&block, &bal, &keys, LSTAR_AT_FIRST_BLOCK).await {
        result.expect("the block imports");
    }
}

/// The parameters of `engine_newPayloadV5` for a block of these tests: no blobs,
/// a zero beacon root and no execution requests.
fn new_payload_params(payload: Value) -> Option<Vec<Value>> {
    Some(vec![payload, json!([]), json!(H256::zero()), json!([])])
}

async fn new_payload(payload: Value, keys: &[&Key], lstar_after: Option<u64>) -> Value {
    let (store, _) = setup_store(keys, lstar_after).await;
    let context = default_context_with_storage(store).await;
    NewPayloadV5Request::parse(&new_payload_params(payload))
        .expect("well-formed newPayloadV5 params")
        .handle(context.clone())
        .await
        .expect("newPayload returns a payload status")
}

fn payload_json(block: Block, bal: &BlockAccessList) -> Value {
    serde_json::to_value(ExecutionPayload::from_block(block, Some(bal.clone())))
        .expect("payload to json")
}

#[tokio::test]
async fn dependency_proof_travels_through_get_payload_and_new_payload() {
    const PAYLOAD_ID: u64 = 1;
    let first = key(0x79);
    let second = key(0x7a);
    let keys = [&first, &second];
    let (store, chain_id) = setup_store(&keys, LSTAR_AT_FIRST_BLOCK).await;
    let builder = default_context_with_storage(store.clone()).await;
    for sender in keys {
        builder
            .blockchain
            .add_dependency_transaction_to_pool(wrapper(
                dependency_transaction(chain_id, sender, 0, &[]),
                &[sender],
            ))
            .await
            .expect("the dependency transaction enters the pool");
    }

    // The builder starts a payload, as on a fork choice update with payload
    // attributes. Its first build runs before anything else on this thread.
    builder
        .blockchain
        .clone()
        .initiate_payload_build(payload_on(&store, &genesis_header(&store)), PAYLOAD_ID)
        .await;
    tokio::task::yield_now().await;

    // `engine_getPayloadV6` returns the proof, and no witnesses.
    let response = GetPayloadV6Request {
        payload_id: PAYLOAD_ID,
    }
    .handle(builder.clone())
    .await
    .expect("getPayload returns the payload");
    let payload = response["executionPayload"].clone();
    assert_eq!(payload["transactions"].as_array().map(Vec::len), Some(2));
    assert!(payload.get("dependencyWitnesses").is_none());
    let proof = payload["dependencyProof"]
        .as_str()
        .expect("the payload has a dependency proof");
    assert!(proof.starts_with("0x") && proof.len() > 2);
    // The header's two commitments are not payload fields: the receiver derives
    // them, and the block hash covers them.
    assert!(payload.get("dependencyProofHash").is_none());
    assert!(payload.get("blockDepsHash").is_none());

    // `engine_newPayloadV5` on another node accepts it.
    let status = new_payload(payload.clone(), &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_eq!(status["status"], "VALID", "got {status:?}");
    assert_eq!(status["latestValidHash"], payload["blockHash"]);

    // The same payload with another proof is not the block its hash names.
    let mut other_proof = payload.clone();
    other_proof["dependencyProof"] = json!(format!("{proof}00"));
    let status = new_payload(other_proof, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");

    // Nor is it with the witnesses a Hegota payload carries.
    let mut with_witnesses = payload;
    with_witnesses["dependencyWitnesses"] = json!(["0x01"]);
    let status = new_payload(with_witnesses, &keys, LSTAR_AT_FIRST_BLOCK).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
}

#[tokio::test]
async fn engine_answers_invalid_to_a_payload_whose_dependency_proof_does_not_hold() {
    let first = key(0x7b);
    let second = key(0x7c);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second).await;
    let proof = built.block.body.dependency_proof.clone().unwrap();

    // The block as built is valid.
    let status = new_payload(
        payload_json(built.block.clone(), &built.bal),
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_eq!(status["status"], "VALID", "got {status:?}");

    // In each of these the block hash is the hash of the block the payload
    // describes, so only the proof makes the payload invalid.
    let mut flipped = proof.to_vec();
    flipped[0] ^= 0x01;
    let mut oversized = proof.to_vec();
    oversized.resize(MAX_DEPENDENCY_PROOF_BYTES + 1, 0);
    let invalid_proofs = [
        // A proof of one dependency fewer.
        proof_of(&[(&first, built.messages[0])]),
        // Bytes that are not the proof.
        Bytes::from(flipped),
        proof.slice(..proof.len() / 2),
        // No proof for a block that has dependencies.
        Bytes::new(),
        // A proof over `MAX_DEPENDENCY_PROOF_BYTES`.
        Bytes::from(oversized),
    ];
    for invalid_proof in invalid_proofs {
        let block = with_proof(built.block.clone(), invalid_proof);
        let status =
            new_payload(payload_json(block, &built.bal), &keys, LSTAR_AT_FIRST_BLOCK).await;
        assert_eq!(status["status"], "INVALID", "got {status:?}");
    }

    // A proof in a block without dependencies.
    let (store, _) = setup_store(&keys, LSTAR_AT_FIRST_BLOCK).await;
    let empty = build_block(&store, &blockchain(store.clone()), &genesis_header(&store));
    let empty_bal = empty.block_access_list.expect("an LStar block has a BAL");
    let status = new_payload(
        payload_json(empty.payload.clone(), &empty_bal),
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_eq!(status["status"], "VALID", "got {status:?}");
    let status = new_payload(
        payload_json(with_proof(empty.payload, proof), &empty_bal),
        &keys,
        LSTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
}

#[tokio::test]
async fn engine_reads_the_dependency_fields_of_a_payload_under_its_fork() {
    let first = key(0x7d);
    let second = key(0x7e);
    let keys = [&first, &second];

    // A Hegota payload carries witnesses and no proof.
    let (store, chain_id) = setup_store(&keys, None).await;
    let blockchain = blockchain(store.clone());
    let mut messages = Vec::new();
    for sender in keys {
        let tx = dependency_transaction(chain_id, sender, 0, &[]);
        messages.push(tx.compute_sig_hash());
        blockchain
            .add_dependency_transaction_to_pool(wrapper(tx, &[sender]))
            .await
            .expect("the dependency transaction enters the pool");
    }
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let bal = built.block_access_list.expect("a Hegota block has a BAL");
    let payload = payload_json(built.payload, &bal);
    assert_eq!(
        payload["dependencyWitnesses"].as_array().map(Vec::len),
        Some(2)
    );
    assert!(payload.get("dependencyProof").is_none());
    let status = new_payload(payload.clone(), &keys, None).await;
    assert_eq!(status["status"], "VALID", "got {status:?}");

    // An empty proof is no proof before LStar; a proof makes the payload invalid.
    let mut with_empty_proof = payload.clone();
    with_empty_proof["dependencyProof"] = json!("0x");
    let status = new_payload(with_empty_proof, &keys, None).await;
    assert_eq!(status["status"], "VALID", "got {status:?}");
    let proof = proof_of(&[(&first, messages[0]), (&second, messages[1])]);
    let mut with_a_proof = payload;
    with_a_proof["dependencyProof"] = json!(format!("0x{}", hex::encode(&proof)));
    let status = new_payload(with_a_proof, &keys, None).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
}
