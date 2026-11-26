//! EIP-8141 §Aggregation, from the pool to a block and its import.
//!
//! Accounts controlled by leanSPHINCS keys send frame transactions whose
//! signatures are the proofs of their proof frames, as before IStar: each one is
//! submitted, pooled and gossiped with its signature inside it. From IStar the
//! builder of a block strips every proof the protocol may strip from the
//! transactions it includes and aggregates the stripped signatures into one
//! proof. The block carries that proof, its header commits to it, and every node
//! that imports the block checks it against the claims of the block's stripped
//! proof frames.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    aggregate_proof::aggregate_signatures,
    error::{ChainError, MempoolError},
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    H160, H256, U256,
    errors::InvalidBlockError,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER,
        FRAME_TX_SECP256K1_VERIFIER, Fork, FrameTransaction, Genesis, GenesisAccount,
        InvalidBlockBodyError, InvalidBlockHeaderError, LEANSPHINCS_VERIFIER_PROOF_LENGTH,
        MAX_AGGREGATE_PROOF_BYTES, PROOF_COMMIT_FLAG, PROOF_READ_FLAG, Transaction,
        block_access_list::BlockAccessList, compute_aggregate_proof_hash,
        compute_transactions_root, validate_block_body,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_p2p::rlpx::{
    eth::blocks::{BlockBodies, GetBlockBodies},
    message::RLPxMessage,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_rpc::{
    engine::payload::{GetPayloadV6Request, NewPayloadV5Request},
    rpc::RpcHandler,
    test_utils::default_context_with_storage,
    types::{payload::ExecutionPayload, transaction::SendRawTransactionRequest},
};
use ethrex_storage::{EngineType, Store};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::levm::eip8141_stripped_proof_tests::{
    Key, SIGNATURE_SIZE, account_code, key, leansphincs_proof_frame, proof_transaction, sign,
    sign_proof_frame, stripped, transaction_with_proof_frames,
};

/// Seconds between a block and its parent on the chains of these tests.
const BLOCK_TIME: u64 = 12;
/// IStar from the first block on: the genesis block is the only Hegota block.
const ISTAR_AT_FIRST_BLOCK: Option<u64> = Some(BLOCK_TIME);

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `l1-hegota.json` (every fork through Hegota active at genesis) with each
/// key's account funded and running the validation code for that key. IStar
/// activates `istar_after` seconds after genesis, or never when it is `None`.
async fn setup_store(keys: &[&Key], istar_after: Option<u64>) -> (Store, u64) {
    setup_store_with_lstar(keys, istar_after, None).await
}

/// The store of `setup_store` for a chain that also schedules LStar,
/// `lstar_after` seconds after genesis.
async fn setup_store_with_lstar(
    keys: &[&Key],
    istar_after: Option<u64>,
    lstar_after: Option<u64>,
) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    genesis.config.istar_time = istar_after.map(|seconds| genesis.timestamp + seconds);
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

fn blockchain(store: Store) -> Blockchain {
    Blockchain::new(
        store,
        BlockchainOptions {
            min_tip_wei: 0,
            ..Default::default()
        },
    )
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

fn frame_tx(tx: &Transaction) -> &FrameTransaction {
    match tx {
        Transaction::FrameTransaction(frame_tx) => frame_tx,
        other => panic!("expected a frame transaction, got {other:?}"),
    }
}

/// The canonical encoding of the transaction: what the pool or a block holds of
/// it. A transaction caches its encoding and its hash, so two of them are
/// compared through this and not through `==`.
fn encoded(tx: &FrameTransaction) -> Vec<u8> {
    let mut fresh = tx.clone();
    fresh.inner_hash = Default::default();
    fresh.cached_canonical = Default::default();
    Transaction::FrameTransaction(fresh).encode_canonical_to_vec()
}

/// The hash of the transaction as it is encoded here.
fn tx_hash(tx: &FrameTransaction) -> H256 {
    ethrex_common::utils::keccak(encoded(tx))
}

/// The transaction of the block that `sender` sent.
fn tx_from<'a>(block: &'a Block, sender: &Key) -> &'a FrameTransaction {
    block
        .body
        .transactions
        .iter()
        .map(frame_tx)
        .find(|tx| tx.sender == sender.account)
        .expect("the block includes a transaction of the sender")
}

/// The block's transactions as the pool held them, with their proofs, in the
/// block's order.
fn pooled_form(block: &Block, pooled: &[FrameTransaction]) -> Vec<Transaction> {
    block
        .body
        .transactions
        .iter()
        .map(|included| {
            let pooled = pooled
                .iter()
                .find(|tx| tx.sender == frame_tx(included).sender)
                .expect("every transaction of the block came from the pool");
            Transaction::FrameTransaction(pooled.clone())
        })
        .collect()
}

/// An aggregate proof of the given `(key, message)` claims, each signed here.
fn aggregate(claims: &[(&Key, H256)]) -> Bytes {
    let signatures: Vec<_> = claims
        .iter()
        .map(|(key, message)| ((key.public, *message), Bytes::from(sign(key, *message))))
        .collect();
    aggregate_signatures(&signatures).expect("valid signatures aggregate")
}

/// The block with its aggregate proof replaced and the header committing to the
/// new proof, so only the proof itself can make it invalid.
fn with_proof(mut block: Block, proof: Bytes) -> Block {
    block.header.aggregate_proof_hash = Some(compute_aggregate_proof_hash(&proof, &NativeCrypto));
    block.body.aggregate_proof = Some(proof);
    block.header.hash = Default::default();
    block
}

/// The block with its transactions replaced and the header committing to them,
/// so only the transactions themselves can make it invalid.
fn with_transactions(mut block: Block, transactions: Vec<Transaction>) -> Block {
    block.header.transactions_root = compute_transactions_root(&transactions, &NativeCrypto);
    block.body.transactions = transactions;
    block.header.hash = Default::default();
    block
}

/// Imports the block into fresh nodes, one for each path a block takes, and
/// returns what each answered: the plain import, the pipeline with the block's
/// access list, and the pipeline that rebuilds the access list.
async fn import(
    block: &Block,
    bal: &BlockAccessList,
    keys: &[&Key],
    istar_after: Option<u64>,
) -> [Result<(), ChainError>; 3] {
    import_after(None, block, bal, keys, istar_after, None).await
}

/// `import` on a chain that also schedules LStar, `lstar_after` seconds after
/// genesis, and into nodes that first import `parent`, a valid block with its
/// access list, when one is given.
async fn import_after(
    parent: Option<(&Block, &BlockAccessList)>,
    block: &Block,
    bal: &BlockAccessList,
    keys: &[&Key],
    istar_after: Option<u64>,
    lstar_after: Option<u64>,
) -> [Result<(), ChainError>; 3] {
    let (store, _) = setup_store_with_lstar(keys, istar_after, lstar_after).await;
    let node = blockchain(store);
    if let Some((parent, _)) = parent {
        node.add_block(parent.clone())
            .expect("plain import of the parent");
    }
    let plain = node.add_block(block.clone());

    let (store, _) = setup_store_with_lstar(keys, istar_after, lstar_after).await;
    let node = blockchain(store);
    if let Some((parent, parent_bal)) = parent {
        node.add_block_pipeline(parent.clone(), Some(Arc::new(parent_bal.clone())))
            .expect("pipeline import of the parent");
    }
    let pipeline = node.add_block_pipeline(block.clone(), Some(Arc::new(bal.clone())));

    let (store, _) = setup_store_with_lstar(keys, istar_after, lstar_after).await;
    let node = blockchain(store);
    if let Some((parent, _)) = parent {
        node.add_block_pipeline(parent.clone(), None)
            .expect("pipeline import of the parent, rebuilding its access list");
    }
    let pipeline_without_bal = node.add_block_pipeline(block.clone(), None);
    [plain, pipeline, pipeline_without_bal]
}

/// Asserts that every import path rejects the block with an error matching the
/// pattern.
macro_rules! assert_rejected {
    ($results:expr, $pattern:pat $(if $guard:expr)?) => {
        for result in $results {
            assert!(
                matches!(&result, Err(ChainError::InvalidBlock($pattern)) $(if $guard)?),
                "got {result:?}"
            );
        }
    };
}

/// An IStar block built from two transactions in the pool, one from each key:
/// two stripped proof frames, one claim for each transaction's signature hash.
struct IStarBlock {
    block: Block,
    bal: BlockAccessList,
    /// The two transactions as the pool held them, in the order of the keys.
    pooled: [FrameTransaction; 2],
    /// Their signature hashes, in the order of the keys.
    sig_hashes: [H256; 2],
}

async fn istar_block(first: &Key, second: &Key) -> IStarBlock {
    let (store, chain_id) = setup_store(&[first, second], ISTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());
    let pooled = [first, second].map(|sender| proof_transaction(chain_id, sender, 0, 0));
    for tx in &pooled {
        blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
            .await
            .expect("the transaction enters the pool with its proof");
    }
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    assert_eq!(built.payload.body.transactions.len(), 2);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));
    let sig_hashes = [pooled[0].compute_sig_hash(), pooled[1].compute_sig_hash()];
    IStarBlock {
        block: built.payload,
        bal: built.block_access_list.expect("an IStar block has a BAL"),
        pooled,
        sig_hashes,
    }
}

#[tokio::test]
async fn istar_block_strips_every_eligible_proof_and_carries_one_aggregate_proof() {
    let first = key(0x61);
    let second = key(0x62);
    let readable = key(0x63);
    let sponsored = key(0x64);
    // Signs an explicit message for the last transaction. It controls no account.
    let sponsor = key(0x65);
    let keys = [&first, &second, &readable, &sponsored];
    let (store, chain_id) = setup_store(&keys, ISTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());

    // Two transactions whose only proof is the account's signature.
    let first_tx = proof_transaction(chain_id, &first, 0, 0);
    let second_tx = proof_transaction(chain_id, &second, 0, 0);
    // One whose signature `VERIFY` code may read: it cannot leave the transaction.
    let readable_tx = proof_transaction(chain_id, &readable, 0, PROOF_READ_FLAG);
    // One with two proofs: the account's signature, and a sponsor's signature of
    // an explicit message that the signature hash commits to. The committed proof
    // is signed first, since the account signs over its digest.
    let sponsored_message = H256::repeat_byte(0x77);
    let mut sponsored_tx = transaction_with_proof_frames(
        chain_id,
        &sponsored,
        0,
        vec![
            leansphincs_proof_frame(sponsored.public, H256::zero(), 0),
            leansphincs_proof_frame(sponsor.public, sponsored_message, PROOF_COMMIT_FLAG),
        ],
    );
    sign_proof_frame(&mut sponsored_tx, 1, &sponsor);
    sign_proof_frame(&mut sponsored_tx, 0, &sponsored);

    let pooled = [&first_tx, &second_tx, &readable_tx, &sponsored_tx];
    for tx in pooled {
        let hash = blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
            .await
            .expect("the transaction enters the pool with its proofs");
        assert_eq!(hash, tx_hash(tx));
    }

    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 4);
    assert!(
        built.receipts.iter().all(|receipt| receipt.succeeded),
        "the transactions must succeed: {:?}",
        built.receipts
    );

    // The proofs the protocol may strip left their transactions.
    for (sender, pooled_tx) in [(&first, &first_tx), (&second, &second_tx)] {
        let included = tx_from(&block, sender);
        assert_eq!(
            included.stripped_proofs,
            BTreeMap::from([(0, LEANSPHINCS_VERIFIER_PROOF_LENGTH)])
        );
        assert_eq!(
            included.frames[0].data,
            pooled_tx.frames[0].data.slice(..64)
        );
        assert_eq!(encoded(included), encoded(&stripped(pooled_tx)));
        assert_ne!(tx_hash(included), tx_hash(pooled_tx));
        assert_eq!(included.compute_sig_hash(), pooled_tx.compute_sig_hash());
    }
    // A readable proof stays, and so does the transaction as it was pooled.
    let included = tx_from(&block, &readable);
    assert!(!included.has_stripped_proofs());
    assert_eq!(encoded(included), encoded(&readable_tx));
    // A committed proof stays, beside a proof of the same transaction that left.
    let included = tx_from(&block, &sponsored);
    assert_eq!(
        included.stripped_proofs,
        BTreeMap::from([(0, LEANSPHINCS_VERIFIER_PROOF_LENGTH)])
    );
    assert_eq!(included.frames[1], sponsored_tx.frames[1]);
    assert_eq!(
        included.frames[1].proof_bytes(),
        sign(&sponsor, sponsored_message).as_slice()
    );
    assert_eq!(included.compute_sig_hash(), sponsored_tx.compute_sig_hash());

    // The block carries one proof for the three claims of its stripped frames,
    // and the header commits to it and to the transactions as the block has them.
    let proof = block
        .body
        .aggregate_proof
        .clone()
        .expect("an IStar block has an aggregate proof");
    assert!(!proof.is_empty());
    assert!(proof.len() <= MAX_AGGREGATE_PROOF_BYTES);
    assert_eq!(
        block.header.aggregate_proof_hash,
        Some(compute_aggregate_proof_hash(&proof, &NativeCrypto))
    );
    let claims: Vec<_> = block
        .body
        .transactions
        .iter()
        .flat_map(|tx| frame_tx(tx).stripped_claims())
        .collect();
    assert_eq!(claims.len(), 3);
    for (key, pooled_tx) in [
        (&first, &first_tx),
        (&second, &second_tx),
        (&sponsored, &sponsored_tx),
    ] {
        assert!(claims.contains(&(key.public, pooled_tx.compute_sig_hash())));
    }
    validate_block_body(&block.header, &block.body, &NativeCrypto)
        .expect("the header commits to the block's transactions and proof");

    // Stripping changed no gas: each transaction is charged the calldata floor of
    // the transaction that was signed, proof included.
    assert_eq!(
        block.header.gas_used,
        pooled
            .iter()
            .map(|tx| tx.calldata_floor_total())
            .sum::<u64>()
    );

    // The transactions stay in the pool with their proofs until the block is
    // imported: the block did not take the signatures, it proved them.
    for tx in pooled {
        let pending = blockchain
            .mempool
            .get_transaction_by_hash(tx_hash(tx))
            .unwrap()
            .expect("the transaction is still pending");
        assert_eq!(encoded(frame_tx(&pending)), encoded(tx));
    }

    // It imports on every path.
    let bal = built.block_access_list.expect("an IStar block has a BAL");
    for result in import(&block, &bal, &keys, ISTAR_AT_FIRST_BLOCK).await {
        result.expect("the block imports");
    }

    // A proof that stayed in its transaction is still checked by its frame: with
    // another signature in it the block is invalid, whatever the aggregate says.
    let mut forged = readable_tx.clone();
    let mut data = forged.frames[0].proof_claim().to_vec();
    data.extend_from_slice(&sign(&first, forged.compute_sig_hash()));
    forged.frames[0].data = Bytes::from(data);
    let transactions = block
        .body
        .transactions
        .iter()
        .map(|tx| {
            if frame_tx(tx).sender == readable.account {
                Transaction::FrameTransaction(forged.clone())
            } else {
                tx.clone()
            }
        })
        .collect();
    let results = import(
        &with_transactions(block.clone(), transactions),
        &bal,
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    for result in results {
        let error = result.expect_err("a forged proof makes the block invalid");
        assert!(
            error
                .to_string()
                .contains("the claim of a proof frame does not hold"),
            "got {error:?}"
        );
    }

    // The same transactions in a Hegota block, on a chain without IStar: every
    // proof stays in its transaction, and the block has no aggregate proof.
    let (hegota_store, _) = setup_store(&keys, None).await;
    let hegota_blockchain = self::blockchain(hegota_store.clone());
    for tx in pooled {
        hegota_blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
            .await
            .expect("the transaction enters the pool with its proofs");
    }
    let hegota_built = build_block(
        &hegota_store,
        &hegota_blockchain,
        &genesis_header(&hegota_store),
    );
    let hegota_block = hegota_built.payload;
    assert_eq!(hegota_block.body.transactions.len(), 4);
    for tx in pooled {
        assert!(
            hegota_block
                .body
                .transactions
                .iter()
                .any(|included| encoded(frame_tx(included)) == encoded(tx))
        );
    }
    assert_eq!(hegota_block.body.aggregate_proof, None);
    assert_eq!(hegota_block.header.aggregate_proof_hash, None);
    assert_eq!(hegota_block.header.gas_used, block.header.gas_used);
    println!(
        "IStar block: {} bytes of RLP block and {} bytes of aggregate proof, {} gas used. \
         Hegota block with the same transactions: {} bytes of RLP block, {} gas used.",
        block.length_without_aggregate_proof(),
        proof.len(),
        block.header.gas_used,
        hegota_block.length(),
        hegota_block.header.gas_used,
    );
    // Three signatures left the RLP block, which gained one header field: the
    // commitment to the proof.
    let saved = hegota_block.length() - block.length_without_aggregate_proof();
    assert!(saved > 3 * SIGNATURE_SIZE - 128 && saved < 3 * SIGNATURE_SIZE);
    // The proof travels with the block, after the body's other items.
    assert!(block.length() > block.length_without_aggregate_proof() + proof.len());
}

#[tokio::test]
async fn claim_that_two_transactions_share_is_proven_once() {
    let first = key(0x8a);
    let second = key(0x8b);
    // Signs the same explicit message for both transactions.
    let shared = key(0x8c);
    let keys = [&first, &second];
    let (store, chain_id) = setup_store(&keys, ISTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());
    let message = H256::repeat_byte(0x99);
    let mut sig_hashes = Vec::new();
    for sender in keys {
        let mut tx = transaction_with_proof_frames(
            chain_id,
            sender,
            0,
            vec![
                leansphincs_proof_frame(sender.public, H256::zero(), 0),
                leansphincs_proof_frame(shared.public, message, 0),
            ],
        );
        sign_proof_frame(&mut tx, 1, &shared);
        sign_proof_frame(&mut tx, 0, sender);
        sig_hashes.push(tx.compute_sig_hash());
        blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(tx))
            .await
            .expect("the transaction enters the pool with its proofs");
    }

    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 2);
    assert!(
        built.receipts.iter().all(|receipt| receipt.succeeded),
        "the transactions must succeed: {:?}",
        built.receipts
    );

    // Four stripped frames make three distinct claims.
    let claims: Vec<_> = block
        .body
        .transactions
        .iter()
        .flat_map(|tx| frame_tx(tx).stripped_claims())
        .collect();
    assert_eq!(claims.len(), 4);
    assert_eq!(
        claims
            .iter()
            .filter(|claim| **claim == (shared.public, message))
            .count(),
        2
    );
    let bal = built.block_access_list.expect("an IStar block has a BAL");
    for result in import(&block, &bal, &keys, ISTAR_AT_FIRST_BLOCK).await {
        result.expect("the block imports");
    }

    // A proof of the three claims made here, each once, stands for the same
    // block. Without the shared claim it does not.
    let same_claims = aggregate(&[
        (&first, sig_hashes[0]),
        (&second, sig_hashes[1]),
        (&shared, message),
    ]);
    for result in import(
        &with_proof(block.clone(), same_claims),
        &bal,
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await
    {
        result.expect("a proof of the block's distinct claims holds");
    }
    let without_shared = aggregate(&[(&first, sig_hashes[0]), (&second, sig_hashes[1])]);
    let results = import(
        &with_proof(block, without_shared),
        &bal,
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(results, InvalidBlockError::InvalidAggregateProof(4));
}

#[tokio::test]
async fn aggregate_proof_is_stored_and_served_with_the_block_body() {
    let first = key(0x66);
    let second = key(0x67);
    let keys = [&first, &second];
    let built = istar_block(&first, &second).await;
    let proof = built.block.body.aggregate_proof.clone().unwrap();

    // A node that imports the block keeps the proof with the block's body, and
    // the transactions as the block carries them.
    let (store, _) = setup_store(&keys, ISTAR_AT_FIRST_BLOCK).await;
    blockchain(store.clone())
        .add_block(built.block.clone())
        .expect("the block imports");
    apply_fork_choice(&store, built.block.hash(), H256::zero(), H256::zero(), None)
        .await
        .expect("the block becomes the head");
    let stored = store
        .get_block_body_by_hash(built.block.hash())
        .await
        .unwrap()
        .expect("the body is stored");
    assert_eq!(stored.encode_to_vec(), built.block.body.encode_to_vec());
    assert_eq!(stored.aggregate_proof, Some(proof.clone()));
    assert!(
        stored
            .transactions
            .iter()
            .all(|tx| frame_tx(tx).stripped_proof_len(0).is_some())
    );

    // A transaction is indexed by its hash in the block. The hash it had in the
    // pool, with its proof, names nothing on the chain.
    for (included, pooled) in built.block.body.transactions.iter().zip(&built.pooled) {
        let included_hash = included.hash(&NativeCrypto);
        let indexed = store
            .get_transaction_by_hash(included_hash)
            .await
            .unwrap()
            .expect("the transaction is indexed by its hash in the block");
        assert_eq!(indexed.hash(&NativeCrypto), included_hash);
        assert_eq!(
            store
                .get_transaction_by_hash(tx_hash(pooled))
                .await
                .unwrap(),
            None
        );
    }

    // It serves the body with the proof to a peer that asks for it, and the peer
    // checks what it receives against the header it already has.
    let request = GetBlockBodies::new(1, vec![built.block.hash()]);
    let response = BlockBodies::new(1, request.fetch_blocks(&store).await);
    let mut encoded = Vec::new();
    response.encode(&mut encoded).unwrap();
    let received = BlockBodies::decode(&encoded).unwrap().block_bodies;
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].encode_to_vec(),
        built.block.body.encode_to_vec()
    );
    assert_eq!(received[0].aggregate_proof, Some(proof));
    validate_block_body(&built.block.header, &received[0], &NativeCrypto)
        .expect("the body matches the header");

    // A body served with another proof is not the body of that header.
    let mut tampered = received[0].clone();
    tampered.aggregate_proof = Some(Bytes::from(vec![0xEE; 10]));
    let result = validate_block_body(&built.block.header, &tampered, &NativeCrypto);
    assert!(
        matches!(
            result,
            Err(InvalidBlockBodyError::AggregateProofHashNotMatch)
        ),
        "got {result:?}"
    );
    // Nor is a body served with the transactions as they were pooled.
    let mut unstripped = received[0].clone();
    unstripped.transactions = pooled_form(&built.block, &built.pooled);
    let result = validate_block_body(&built.block.header, &unstripped, &NativeCrypto);
    assert!(
        matches!(result, Err(InvalidBlockBodyError::TransactionsRootNotMatch)),
        "got {result:?}"
    );
}

#[tokio::test]
async fn block_whose_aggregate_proof_misses_a_claim_is_rejected() {
    let first = key(0x68);
    let second = key(0x69);
    let keys = [&first, &second];
    let built = istar_block(&first, &second).await;

    // A proof that the first transaction was signed says nothing of the second.
    let partial = aggregate(&[(&first, built.sig_hashes[0])]);
    let results = import(
        &with_proof(built.block, partial),
        &built.bal,
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(results, InvalidBlockError::InvalidAggregateProof(2));
}

#[tokio::test]
async fn block_whose_aggregate_proof_is_for_other_claims_is_rejected() {
    let first = key(0x6a);
    let second = key(0x6b);
    let other = key(0x6c);
    let keys = [&first, &second];
    let built = istar_block(&first, &second).await;

    // The first key signed another message than the transaction's signature hash.
    let another_message = aggregate(&[
        (&first, H256::repeat_byte(0x11)),
        (&second, built.sig_hashes[1]),
    ]);
    // Another key signed the first transaction's signature hash.
    let another_key = aggregate(&[
        (&other, built.sig_hashes[0]),
        (&second, built.sig_hashes[1]),
    ]);
    // A proof of the block's claims and of one more is not a proof of exactly
    // the block's claims.
    let one_more = aggregate(&[
        (&first, built.sig_hashes[0]),
        (&second, built.sig_hashes[1]),
        (&other, H256::repeat_byte(0x22)),
    ]);
    for proof in [another_message, another_key, one_more] {
        let results = import(
            &with_proof(built.block.clone(), proof),
            &built.bal,
            &keys,
            ISTAR_AT_FIRST_BLOCK,
        )
        .await;
        assert_rejected!(results, InvalidBlockError::InvalidAggregateProof(2));
    }

    // The proof of the block's own claims, made again here, is accepted.
    let same_claims = aggregate(&[
        (&second, built.sig_hashes[1]),
        (&first, built.sig_hashes[0]),
    ]);
    let results = import(
        &with_proof(built.block, same_claims),
        &built.bal,
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    for result in results {
        result.expect("a proof of exactly the block's claims holds");
    }
}

#[tokio::test]
async fn block_with_a_damaged_or_missing_aggregate_proof_is_rejected() {
    let first = key(0x6d);
    let second = key(0x6e);
    let keys = [&first, &second];
    let built = istar_block(&first, &second).await;
    let proof = built.block.body.aggregate_proof.clone().unwrap();

    // One byte flipped.
    let mut flipped = proof.to_vec();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0x01;
    // Cut short.
    let truncated = proof.slice(..proof.len() - 1);
    let half = proof.slice(..proof.len() / 2);
    // The empty proof, which only a block without stripped frames carries.
    let empty = Bytes::new();
    for damaged in [Bytes::from(flipped), truncated, half, empty] {
        let results = import(
            &with_proof(built.block.clone(), damaged),
            &built.bal,
            &keys,
            ISTAR_AT_FIRST_BLOCK,
        )
        .await;
        assert_rejected!(results, InvalidBlockError::InvalidAggregateProof(2));
    }

    // No proof at all, with the header still committing to one.
    let mut proofless = built.block.clone();
    proofless.body.aggregate_proof = None;
    let results = import(&proofless, &built.bal, &keys, ISTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(results, InvalidBlockError::AggregateProofNotPresent);

    // Neither the proof nor the commitment, as a block before IStar.
    let mut hegota_shaped = proofless;
    hegota_shaped.header.aggregate_proof_hash = None;
    hegota_shaped.header.hash = Default::default();
    let results = import(&hegota_shaped, &built.bal, &keys, ISTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::AggregateProofHashNotPresent)
    );
}

#[tokio::test]
async fn block_without_stripped_frames_must_carry_an_empty_aggregate_proof() {
    let first = key(0x6f);
    let second = key(0x70);
    let keys = [&first, &second];
    // A valid proof, of the claims of another block.
    let other = istar_block(&first, &second).await;
    let proof = other.block.body.aggregate_proof.clone().unwrap();
    let proof_len = proof.len();

    // An IStar block without transactions: its proof is empty, and the header
    // commits to the empty proof.
    let (store, _) = setup_store(&keys, ISTAR_AT_FIRST_BLOCK).await;
    let built = build_block(&store, &blockchain(store.clone()), &genesis_header(&store));
    let empty_block = built.payload;
    let empty_bal = built.block_access_list.expect("an IStar block has a BAL");
    assert!(empty_block.body.transactions.is_empty());
    assert_eq!(empty_block.body.aggregate_proof, Some(Bytes::new()));
    assert_eq!(
        empty_block.header.aggregate_proof_hash,
        Some(compute_aggregate_proof_hash(&[], &NativeCrypto))
    );
    for result in import(&empty_block, &empty_bal, &keys, ISTAR_AT_FIRST_BLOCK).await {
        result.expect("the block imports");
    }
    // With a proof it has nothing to prove, it is invalid, however valid the
    // proof is for other claims.
    let results = import(
        &with_proof(empty_block, proof.clone()),
        &empty_bal,
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(
        results,
        InvalidBlockError::AggregateProofNotEmpty(len) if *len == proof_len
    );

    // An IStar block whose transactions all keep their proofs: the builder may
    // strip them, the protocol does not require it. Its aggregate proof is empty.
    let unstripped = with_proof(
        with_transactions(
            other.block.clone(),
            pooled_form(&other.block, &other.pooled),
        ),
        Bytes::new(),
    );
    for result in import(&unstripped, &other.bal, &keys, ISTAR_AT_FIRST_BLOCK).await {
        result.expect("a block whose proofs stayed in their transactions imports");
    }
    // The proof of the same signatures has nothing to stand for in it.
    let results = import(
        &with_proof(unstripped.clone(), proof),
        &other.bal,
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(
        results,
        InvalidBlockError::AggregateProofNotEmpty(len) if *len == proof_len
    );

    // A block may mix the two: one proof stripped, the other in its transaction.
    let stripped_tx = frame_tx(&other.block.body.transactions[0]);
    let signer = keys
        .iter()
        .find(|key| key.account == stripped_tx.sender)
        .unwrap();
    let mixed = with_proof(
        with_transactions(
            other.block.clone(),
            vec![
                other.block.body.transactions[0].clone(),
                unstripped.body.transactions[1].clone(),
            ],
        ),
        aggregate(&[(signer, stripped_tx.compute_sig_hash())]),
    );
    for result in import(&mixed, &other.bal, &keys, ISTAR_AT_FIRST_BLOCK).await {
        result.expect("a block with stripped and unstripped proof frames imports");
    }
}

#[tokio::test]
async fn block_whose_header_does_not_commit_to_its_aggregate_proof_is_rejected() {
    let first = key(0x71);
    let second = key(0x72);
    let keys = [&first, &second];
    let built = istar_block(&first, &second).await;

    // The proof is the right one, the header commits to another.
    let mut wrong_hash = built.block.clone();
    wrong_hash.header.aggregate_proof_hash = Some(H256::repeat_byte(0xab));
    wrong_hash.header.hash = Default::default();
    let results = import(&wrong_hash, &built.bal, &keys, ISTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidBody(InvalidBlockBodyError::AggregateProofHashNotMatch)
    );

    // The header commits to the empty proof, which the block had before its
    // proofs were stripped.
    let mut stale_hash = built.block;
    stale_hash.header.aggregate_proof_hash = Some(compute_aggregate_proof_hash(&[], &NativeCrypto));
    stale_hash.header.hash = Default::default();
    let results = import(&stale_hash, &built.bal, &keys, ISTAR_AT_FIRST_BLOCK).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidBody(InvalidBlockBodyError::AggregateProofHashNotMatch)
    );
}

#[tokio::test]
async fn block_with_an_aggregate_proof_over_the_size_bound_is_rejected() {
    let first = key(0x73);
    let second = key(0x74);
    let keys = [&first, &second];
    let built = istar_block(&first, &second).await;
    let proof = built.block.body.aggregate_proof.clone().unwrap();

    // The valid proof followed by padding up to one byte over the bound. The
    // size is checked before anything reads the proof.
    let mut oversized = proof.to_vec();
    oversized.resize(MAX_AGGREGATE_PROOF_BYTES + 1, 0);
    let results = import(
        &with_proof(built.block.clone(), Bytes::from(oversized)),
        &built.bal,
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidBody(InvalidBlockBodyError::AggregateProofTooLarge(len))
            if *len == MAX_AGGREGATE_PROOF_BYTES + 1
    );

    // At the bound the size is accepted, and the bytes are judged as a proof.
    let mut at_the_bound = proof.to_vec();
    at_the_bound.resize(MAX_AGGREGATE_PROOF_BYTES, 0);
    let results = import(
        &with_proof(built.block, Bytes::from(at_the_bound)),
        &built.bal,
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_rejected!(results, InvalidBlockError::InvalidAggregateProof(2));
}

/// The block with its first transaction replaced by `replacement`, and an
/// aggregate proof that holds for the claims of the stripped frames it then has:
/// the replacement's first proof frame, signed again by `signer`, and the second
/// transaction's. Only the stripped frame itself can make the block invalid.
fn with_first_transaction(
    built: &IStarBlock,
    replacement: FrameTransaction,
    signer: &Key,
    second: &Key,
) -> Block {
    let (key, message) = replacement.stripped_claims()[0];
    assert_eq!(key, signer.public);
    let proof = aggregate(&[(signer, message), (second, built.sig_hashes[1])]);
    let mut transactions = built.block.body.transactions.clone();
    let index = transactions
        .iter()
        .position(|tx| frame_tx(tx).sender == replacement.sender)
        .unwrap();
    transactions[index] = Transaction::FrameTransaction(replacement);
    with_proof(with_transactions(built.block.clone(), transactions), proof)
}

#[tokio::test]
async fn block_with_a_stripped_frame_the_protocol_may_not_strip_is_rejected() {
    let first = key(0x75);
    let second = key(0x76);
    let keys = [&first, &second];
    let built = istar_block(&first, &second).await;
    let stripped_tx = stripped(&built.pooled[0]);

    // The verifier is not the leanSPHINCS verifier.
    let mut another_verifier = stripped_tx.clone();
    another_verifier.frames[0].target = Some(FRAME_TX_SECP256K1_VERIFIER);
    // A contract verifier, the account itself here.
    let mut contract_verifier = stripped_tx.clone();
    contract_verifier.frames[0].target = Some(first.account);
    // `VERIFY` code may read the proof.
    let mut readable = stripped_tx.clone();
    readable.frames[0].flags = PROOF_READ_FLAG;
    // The signature hash commits to the proof's digest, for an explicit message.
    let mut committed = stripped_tx.clone();
    committed.frames[0].flags = PROOF_COMMIT_FLAG;
    let mut claim = committed.frames[0].data.to_vec();
    claim[32..].fill(0x55);
    committed.frames[0].data = Bytes::from(claim);
    // The proof that left was not one signature long.
    let mut shorter = stripped_tx.clone();
    shorter.stripped_proofs.insert(0, 7739);
    let mut longer = stripped_tx.clone();
    longer.stripped_proofs.insert(0, 7741);
    let mut empty = stripped_tx.clone();
    empty.stripped_proofs.insert(0, 0);

    let cases = [
        (another_verifier, "LEANSPHINCS_VERIFIER"),
        (contract_verifier, "LEANSPHINCS_VERIFIER"),
        (readable, "PROOF_READ_FLAG"),
        (committed, "PROOF_COMMIT_FLAG"),
        (shorter, "7740 bytes"),
        (longer, "7740 bytes"),
        (empty, "7740 bytes"),
    ];
    for (invalid, expected) in cases {
        let block = with_first_transaction(&built, invalid, &first, &second);
        let index = block
            .body
            .transactions
            .iter()
            .position(|tx| frame_tx(tx).sender == first.account)
            .unwrap();
        let results = import(&block, &built.bal, &keys, ISTAR_AT_FIRST_BLOCK).await;
        assert_rejected!(
            results,
            InvalidBlockError::InvalidStrippedProofFrame(at, reason)
                if *at == index && reason.contains(expected)
        );
    }

    // The same replacement with an untouched stripped frame is a valid block:
    // the aggregate made here stands for the proofs as the builder's does.
    let block = with_first_transaction(&built, stripped_tx, &first, &second);
    for result in import(&block, &built.bal, &keys, ISTAR_AT_FIRST_BLOCK).await {
        result.expect("the block imports");
    }
}

#[tokio::test]
async fn stripped_transaction_in_a_hegota_block_is_rejected() {
    let first = key(0x77);
    let second = key(0x78);
    let keys = [&first, &second];
    // On this chain IStar never activates: the first block is a Hegota block.
    let (store, chain_id) = setup_store(&keys, None).await;
    let blockchain = blockchain(store.clone());
    let pooled = keys.map(|sender| proof_transaction(chain_id, sender, 0, 0));
    for tx in &pooled {
        blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
            .await
            .expect("the transaction enters the pool");
    }
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    let bal = built.block_access_list.expect("a Hegota block has a BAL");
    assert_eq!(block.body.transactions.len(), 2);
    // Before IStar the builder strips nothing, and the block has no proof.
    assert!(
        block
            .body
            .transactions
            .iter()
            .all(|tx| !frame_tx(tx).has_stripped_proofs())
    );
    assert_eq!(block.body.aggregate_proof, None);
    assert_eq!(block.header.aggregate_proof_hash, None);
    for result in import(&block, &bal, &keys, None).await {
        result.expect("the Hegota block imports with its proofs");
    }

    // The same block with one transaction stripped.
    let mut transactions = block.body.transactions.clone();
    transactions[1] = Transaction::FrameTransaction(stripped(frame_tx(&transactions[1])));
    let with_stripped = with_transactions(block.clone(), transactions.clone());
    let results = import(&with_stripped, &bal, &keys, None).await;
    assert_rejected!(results, InvalidBlockError::StrippedProofFramePresent(1));

    // With an aggregate proof of the stripped claim beside it.
    let stripped_tx = frame_tx(&transactions[1]);
    let signer = keys
        .iter()
        .find(|key| key.account == stripped_tx.sender)
        .unwrap();
    let proof = aggregate(&[(signer, stripped_tx.compute_sig_hash())]);
    let mut with_proof_only = with_stripped.clone();
    with_proof_only.body.aggregate_proof = Some(proof.clone());
    let results = import(&with_proof_only, &bal, &keys, None).await;
    assert_rejected!(results, InvalidBlockError::AggregateProofPresent);

    // The block as IStar would have it: the proof and the header's commitment.
    let istar_shaped = with_proof(with_stripped, proof.clone());
    let results = import(&istar_shaped, &bal, &keys, None).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::AggregateProofHashPresent)
    );

    // An aggregate proof in a Hegota block whose proofs all stayed in their
    // transactions.
    let results = import(&with_proof(block, proof), &bal, &keys, None).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::AggregateProofHashPresent)
    );
}

#[tokio::test]
async fn stripped_transaction_is_rejected_by_the_pool() {
    let key = key(0x79);
    // The pool holds transactions with their proofs at every fork.
    for istar_after in [None, ISTAR_AT_FIRST_BLOCK, Some(0)] {
        let (store, chain_id) = setup_store(&[&key], istar_after).await;
        let ctx = default_context_with_storage(store).await;
        let tx = proof_transaction(chain_id, &key, 0, 0);
        let stripped = Transaction::FrameTransaction(stripped(&tx));

        // Received from a peer.
        let result = ctx
            .blockchain
            .add_transaction_to_pool(stripped.clone())
            .await;
        assert!(
            matches!(result, Err(MempoolError::FrameTxStrippedProof)),
            "got {result:?}"
        );
        // Submitted by a user.
        let result = ctx
            .blockchain
            .add_local_transaction_to_pool(stripped.clone())
            .await;
        assert!(
            matches!(result, Err(MempoolError::FrameTxStrippedProof)),
            "got {result:?}"
        );
        let raw = stripped.encode_canonical_to_vec();
        let result =
            SendRawTransactionRequest::parse(&Some(vec![json!(format!("0x{}", hex::encode(raw)))]))
                .expect("a stripped frame transaction decodes")
                .handle(ctx.clone())
                .await;
        let error = result.expect_err("eth_sendRawTransaction rejects a stripped transaction");
        assert!(format!("{error:?}").contains("stripped"), "got {error:?}");
        assert!(
            ctx.blockchain
                .mempool
                .get_transaction_by_hash(stripped.hash(&NativeCrypto))
                .unwrap()
                .is_none()
        );

        // The transaction with its proof is admitted.
        ctx.blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(tx))
            .await
            .expect("the transaction enters the pool with its proof");
    }
}

#[tokio::test]
async fn account_keeps_authorizing_with_the_same_key_across_the_fork() {
    let key = key(0x7a);
    let keys = [&key];
    // Hegota at genesis and for the first block; IStar from the second block on.
    let istar_after = Some(2 * BLOCK_TIME);
    let (store, chain_id) = setup_store(&keys, istar_after).await;
    let blockchain = blockchain(store.clone());
    let genesis = genesis_header(&store);
    let account_at_genesis = store
        .get_account_state(0, key.account)
        .await
        .unwrap()
        .expect("the account is in the genesis state");

    // The account authorizes a transaction in a Hegota block: the signature is
    // in the transaction, and the block carries the transaction whole.
    let first_tx = proof_transaction(chain_id, &key, 0, 0);
    let first_hash = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(first_tx.clone()))
        .await
        .expect("the transaction enters the pool");
    let first_built = build_block(&store, &blockchain, &genesis);
    let first_block = first_built.payload;
    assert_eq!(first_block.body.transactions.len(), 1);
    assert_eq!(
        encoded(frame_tx(&first_block.body.transactions[0])),
        encoded(&first_tx)
    );
    assert_eq!(
        first_block.body.transactions[0].hash(&NativeCrypto),
        first_hash
    );
    assert!(first_built.receipts[0].succeeded);
    assert_eq!(first_block.body.aggregate_proof, None);
    assert_eq!(first_block.header.aggregate_proof_hash, None);
    blockchain
        .add_block(first_block.clone())
        .expect("the Hegota block imports");
    apply_fork_choice(&store, first_block.hash(), H256::zero(), H256::zero(), None)
        .await
        .expect("the Hegota block becomes the head");
    blockchain
        .remove_block_transactions_from_pool(&first_block)
        .unwrap();
    assert!(
        blockchain
            .mempool
            .get_transaction_by_hash(first_hash)
            .unwrap()
            .is_none()
    );

    // The next transaction of the account is signed and enters the pool while the
    // head is still a Hegota block.
    assert!(
        !store
            .get_chain_config()
            .is_istar_activated(first_block.header.timestamp)
    );
    let second_tx = proof_transaction(chain_id, &key, 1, 0);
    let pool_hash = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(second_tx.clone()))
        .await
        .expect("the transaction enters the pool before IStar");
    assert_eq!(pool_hash, tx_hash(&second_tx));

    // The chain crosses IStar. The block that includes the transaction strips
    // the signature made before the fork and proves it.
    let second_built = build_block(&store, &blockchain, &first_block.header);
    let second_block = second_built.payload;
    assert!(
        store
            .get_chain_config()
            .is_istar_activated(second_block.header.timestamp)
    );
    assert_eq!(second_block.body.transactions.len(), 1);
    assert!(second_built.receipts[0].succeeded);
    let included = frame_tx(&second_block.body.transactions[0]);
    assert_eq!(encoded(included), encoded(&stripped(&second_tx)));
    let block_hash_of_tx = tx_hash(included);
    assert_ne!(block_hash_of_tx, pool_hash);
    assert_eq!(included.compute_sig_hash(), second_tx.compute_sig_hash());
    let proof = second_block
        .body
        .aggregate_proof
        .clone()
        .expect("the IStar block has an aggregate proof");
    assert!(!proof.is_empty());
    // The proof stands for the signature the pool held, the one made before the
    // fork: the builder had no other, and the claim the proof covers is the one
    // that signature was made for.
    assert_eq!(
        included.stripped_claims(),
        vec![(key.public, second_tx.compute_sig_hash())]
    );
    let bal = second_built
        .block_access_list
        .clone()
        .expect("an IStar block has a BAL");

    // Until the block is imported the pool holds the transaction with its proof.
    assert!(
        blockchain
            .mempool
            .get_transaction_by_hash(pool_hash)
            .unwrap()
            .is_some()
    );
    blockchain
        .add_block(second_block.clone())
        .expect("the IStar block imports");
    apply_fork_choice(
        &store,
        second_block.hash(),
        H256::zero(),
        H256::zero(),
        None,
    )
    .await
    .expect("the IStar block becomes the head");
    // The included transaction leaves the pool, where it had another hash.
    blockchain
        .remove_block_transactions_from_pool(&second_block)
        .unwrap();
    assert!(
        blockchain
            .mempool
            .get_transaction_by_hash(pool_hash)
            .unwrap()
            .is_none()
    );
    // The chain knows the transaction by the hash of what the block carries.
    let indexed = store
        .get_transaction_by_hash(block_hash_of_tx)
        .await
        .unwrap()
        .expect("the transaction is indexed by its hash in the block");
    assert_eq!(encoded(frame_tx(&indexed)), encoded(included));
    assert_eq!(
        store.get_transaction_by_hash(pool_hash).await.unwrap(),
        None
    );

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

    // Other nodes import the two blocks, one of each fork, on every path.
    let (other_store, _) = setup_store(&keys, istar_after).await;
    let other = self::blockchain(other_store);
    other
        .add_block(first_block.clone())
        .expect("plain import of the Hegota block");
    other
        .add_block(second_block.clone())
        .expect("plain import of the IStar block");
    let (other_store, _) = setup_store(&keys, istar_after).await;
    let other = self::blockchain(other_store);
    other
        .add_block_pipeline(
            first_block.clone(),
            first_built.block_access_list.clone().map(Arc::new),
        )
        .expect("pipeline import of the Hegota block");
    other
        .add_block_pipeline(second_block.clone(), Some(Arc::new(bal)))
        .expect("pipeline import of the IStar block");
    let (other_store, _) = setup_store(&keys, istar_after).await;
    let other = self::blockchain(other_store);
    other
        .add_block_pipeline(first_block.clone(), None)
        .expect("pipeline import of the Hegota block, rebuilding its access list");
    other
        .add_block_pipeline(second_block.clone(), None)
        .expect("pipeline import of the IStar block, rebuilding its access list");

    // The two transactions do the same work and pay the same: the charge follows
    // the signature hash, which does not see whether the proof was stripped.
    let gas_before = first_built.receipts[0].cumulative_gas_used;
    let gas_after = second_built.receipts[0].cumulative_gas_used;
    assert_eq!(first_block.header.gas_used, gas_before);
    assert_eq!(second_block.header.gas_used, gas_after);
    assert_eq!(gas_before, gas_after);
    assert_eq!(gas_before, first_tx.calldata_floor_total());
    println!(
        "Gas of the transaction in the Hegota block: {gas_before}. \
         Gas of the transaction in the IStar block: {gas_after}."
    );
    println!(
        "Hash of the Hegota transaction, in the pool and in the block: {first_hash:#x}. \
         Hash of the IStar transaction in the pool: {pool_hash:#x}; \
         in the block: {block_hash_of_tx:#x}."
    );
    println!(
        "Hegota block: {} bytes of RLP block, no aggregate proof. \
         IStar block: {} bytes of RLP block and {} bytes of aggregate proof.",
        first_block.length(),
        second_block.length_without_aggregate_proof(),
        proof.len(),
    );
    // The signature left the RLP block, which gained one header field: the
    // commitment to the proof.
    let saved = first_block.length() - second_block.length_without_aggregate_proof();
    assert!(saved > SIGNATURE_SIZE - 64 && saved < SIGNATURE_SIZE);
}

#[tokio::test]
async fn chain_that_starts_in_istar_carries_the_proof_from_its_first_block() {
    let key = key(0x8d);
    let keys = [&key];
    // IStar is active at genesis.
    let istar_at_genesis = Some(0);
    let (store, chain_id) = setup_store(&keys, istar_at_genesis).await;
    let blockchain = blockchain(store.clone());
    let genesis = genesis_header(&store);
    assert!(
        store
            .get_chain_config()
            .is_istar_activated(genesis.timestamp)
    );
    // The genesis block carries the empty aggregate proof, and its header
    // commits to it.
    assert_eq!(
        genesis.aggregate_proof_hash,
        Some(compute_aggregate_proof_hash(&[], &NativeCrypto))
    );
    let genesis_body = store
        .get_block_body_by_hash(genesis.hash())
        .await
        .unwrap()
        .expect("the genesis body is stored");
    assert_eq!(genesis_body.aggregate_proof, Some(Bytes::new()));
    validate_block_body(&genesis, &genesis_body, &NativeCrypto)
        .expect("the genesis header commits to its body");

    let tx = proof_transaction(chain_id, &key, 0, 0);
    blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
        .await
        .expect("the transaction enters the pool with its proof");
    let built = build_block(&store, &blockchain, &genesis);
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 1);
    assert!(built.receipts[0].succeeded);
    assert_eq!(
        encoded(frame_tx(&block.body.transactions[0])),
        encoded(&stripped(&tx))
    );
    let proof = block
        .body
        .aggregate_proof
        .clone()
        .expect("the first block has an aggregate proof");
    assert!(!proof.is_empty());
    assert_eq!(
        block.header.aggregate_proof_hash,
        Some(compute_aggregate_proof_hash(&proof, &NativeCrypto))
    );
    let bal = built.block_access_list.expect("an IStar block has a BAL");
    for result in import(&block, &bal, &keys, istar_at_genesis).await {
        result.expect("the block imports");
    }
}

#[tokio::test]
async fn istar_block_carries_the_aggregate_proof_and_no_burned_fees() {
    let first = key(0x8e);
    let second = key(0x8f);
    // On this chain LStar never activates.
    let built = istar_block(&first, &second).await;
    let header = &built.block.header;
    assert!(built.block.body.aggregate_proof.is_some());
    assert!(header.aggregate_proof_hash.is_some());
    // The fees burned are a header field of LStar (EIP-8079), not of IStar.
    assert_eq!(header.burned_fees, None);

    // The header reads back as it was written: the commitment is its last field.
    let decoded = BlockHeader::decode(&header.encode_to_vec()).expect("the header decodes");
    assert_eq!(&decoded, header);
    assert_eq!(decoded.aggregate_proof_hash, header.aggregate_proof_hash);
    assert_eq!(decoded.burned_fees, None);

    // The payload of the block carries the proof and no fees burned.
    let payload = payload_json(built.block, &built.bal);
    assert!(payload.get("aggregateProof").is_some());
    assert!(payload.get("burnedFees").is_none());
}

/// Builds a block on `parent` from one transaction of `sender` with this nonce,
/// which enters the pool with its proof, and makes the block the head.
async fn build_next_block(
    store: &Store,
    blockchain: &Blockchain,
    parent: &BlockHeader,
    sender: &Key,
    nonce: u64,
    chain_id: u64,
) -> PayloadBuildResult {
    blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(proof_transaction(
            chain_id, sender, nonce, 0,
        )))
        .await
        .expect("the transaction enters the pool with its proof");
    let built = build_block(store, blockchain, parent);
    assert_eq!(built.payload.body.transactions.len(), 1);
    assert!(built.receipts[0].succeeded);
    blockchain
        .add_block(built.payload.clone())
        .expect("the block imports");
    apply_fork_choice(
        store,
        built.payload.hash(),
        H256::zero(),
        H256::zero(),
        None,
    )
    .await
    .expect("the block becomes the head");
    blockchain
        .remove_block_transactions_from_pool(&built.payload)
        .unwrap();
    built
}

#[tokio::test]
async fn lstar_block_carries_the_aggregate_proof_and_the_burned_fees() {
    let sender = key(0x90);
    let other = key(0x91);
    let keys = [&sender];
    // Hegota at genesis, IStar from the first block on and LStar from the second.
    let istar_after = Some(BLOCK_TIME);
    let lstar_after = Some(2 * BLOCK_TIME);
    let (store, chain_id) = setup_store_with_lstar(&keys, istar_after, lstar_after).await;
    let blockchain = blockchain(store.clone());
    let config = store.get_chain_config();
    let genesis = genesis_header(&store);
    assert_eq!(config.fork(genesis.timestamp), Fork::Hegota);
    assert_eq!(genesis.aggregate_proof_hash, None);
    assert_eq!(genesis.burned_fees, None);

    let first = build_next_block(&store, &blockchain, &genesis, &sender, 0, chain_id).await;
    let in_istar = &first.payload;
    let second =
        build_next_block(&store, &blockchain, &in_istar.header, &sender, 1, chain_id).await;
    let in_lstar = &second.payload;
    assert_eq!(config.fork(in_istar.header.timestamp), Fork::IStar);
    assert_eq!(config.fork(in_lstar.header.timestamp), Fork::LStar);

    // In both forks the builder strips the proof of the transaction, and the
    // block carries an aggregate proof its header commits to.
    for block in [in_istar, in_lstar] {
        assert!(
            frame_tx(&block.body.transactions[0])
                .stripped_proof_len(0)
                .is_some()
        );
        let proof = block
            .body
            .aggregate_proof
            .clone()
            .expect("the block has an aggregate proof");
        assert!(!proof.is_empty());
        assert_eq!(
            block.header.aggregate_proof_hash,
            Some(compute_aggregate_proof_hash(&proof, &NativeCrypto))
        );
        validate_block_body(&block.header, &block.body, &NativeCrypto)
            .expect("the header commits to the block's transactions and proof");
    }
    // Only the LStar block has the fees burned (EIP-8079): the base fee times the
    // gas its transaction paid for, since the block has no blobs.
    assert_eq!(in_istar.header.burned_fees, None);
    let base_fee = in_lstar.header.base_fee_per_gas.expect("a base fee");
    let gas = second.receipts[0].cumulative_gas_used;
    assert_eq!(in_lstar.header.blob_gas_used, Some(0));
    assert_eq!(in_lstar.header.burned_fees, Some(base_fee * gas));
    // The transaction pays the same in both forks.
    assert_eq!(first.receipts[0].cumulative_gas_used, gas);

    // The header of each fork reads back as it was written: the Hegota header
    // with neither field, the IStar header with the commitment, the LStar header
    // with the commitment and then the fees burned.
    for header in [&genesis, &in_istar.header, &in_lstar.header] {
        let decoded = BlockHeader::decode(&header.encode_to_vec()).expect("the header decodes");
        assert_eq!(&decoded, header);
        assert_eq!(decoded.aggregate_proof_hash, header.aggregate_proof_hash);
        assert_eq!(decoded.burned_fees, header.burned_fees);
    }
    println!(
        "Header: {} bytes in Hegota, {} in IStar, {} in LStar.",
        genesis.encode_to_vec().len(),
        in_istar.header.encode_to_vec().len(),
        in_lstar.header.encode_to_vec().len(),
    );

    // Other nodes import the two blocks on every path.
    let first_bal = first
        .block_access_list
        .clone()
        .expect("an IStar block has a BAL");
    let bal = second
        .block_access_list
        .clone()
        .expect("an LStar block has a BAL");
    let parent = Some((in_istar, &first_bal));
    for result in import_after(parent, in_lstar, &bal, &keys, istar_after, lstar_after).await {
        result.expect("the LStar block imports");
    }

    // The aggregate proof is checked in LStar as it is in IStar. A proof of
    // another key's signature does not stand for the block's stripped frame.
    let sig_hash = frame_tx(&in_lstar.body.transactions[0]).compute_sig_hash();
    let another_key = with_proof(in_lstar.clone(), aggregate(&[(&other, sig_hash)]));
    let results = import_after(parent, &another_key, &bal, &keys, istar_after, lstar_after).await;
    assert_rejected!(results, InvalidBlockError::InvalidAggregateProof(1));
    // Nor is the block valid without its proof,
    let mut proofless = in_lstar.clone();
    proofless.body.aggregate_proof = None;
    let results = import_after(parent, &proofless, &bal, &keys, istar_after, lstar_after).await;
    assert_rejected!(results, InvalidBlockError::AggregateProofNotPresent);
    // or with the fees burned and no commitment, the header of a chain that
    // schedules LStar without IStar.
    let mut uncommitted = proofless;
    uncommitted.header.aggregate_proof_hash = None;
    uncommitted.header.hash = Default::default();
    let results = import_after(parent, &uncommitted, &bal, &keys, istar_after, lstar_after).await;
    assert_rejected!(
        results,
        InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::AggregateProofHashNotPresent)
    );
}

#[tokio::test]
async fn chain_that_starts_in_lstar_carries_the_proof_and_the_burned_fees_from_its_first_block() {
    let sender = key(0x92);
    let keys = [&sender];
    // IStar and LStar are both active at genesis.
    let at_genesis = Some(0);
    let (store, chain_id) = setup_store_with_lstar(&keys, at_genesis, at_genesis).await;
    let blockchain = blockchain(store.clone());
    let genesis = genesis_header(&store);
    assert_eq!(
        store.get_chain_config().fork(genesis.timestamp),
        Fork::LStar
    );
    // The genesis block commits to the empty aggregate proof. No block came
    // before it, so its header has no fees burned.
    assert_eq!(
        genesis.aggregate_proof_hash,
        Some(compute_aggregate_proof_hash(&[], &NativeCrypto))
    );
    assert_eq!(genesis.burned_fees, None);

    let tx = proof_transaction(chain_id, &sender, 0, 0);
    blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
        .await
        .expect("the transaction enters the pool with its proof");
    let built = build_block(&store, &blockchain, &genesis);
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 1);
    assert!(built.receipts[0].succeeded);
    assert_eq!(
        encoded(frame_tx(&block.body.transactions[0])),
        encoded(&stripped(&tx))
    );
    let proof = block
        .body
        .aggregate_proof
        .clone()
        .expect("the first block has an aggregate proof");
    assert!(!proof.is_empty());
    assert_eq!(
        block.header.aggregate_proof_hash,
        Some(compute_aggregate_proof_hash(&proof, &NativeCrypto))
    );
    let base_fee = block.header.base_fee_per_gas.expect("a base fee");
    assert_eq!(
        block.header.burned_fees,
        Some(base_fee * built.receipts[0].cumulative_gas_used)
    );
    let bal = built.block_access_list.expect("an LStar block has a BAL");
    for result in import_after(None, &block, &bal, &keys, at_genesis, at_genesis).await {
        result.expect("the block imports");
    }

    // Through the Engine API the payload carries both fields, and the block is
    // not the one its hash names without either.
    let payload = payload_json(block, &bal);
    assert!(payload.get("aggregateProof").is_some());
    assert!(payload.get("burnedFees").is_some());
    let (other_store, _) = setup_store_with_lstar(&keys, at_genesis, at_genesis).await;
    let status = new_payload_on(other_store, payload.clone()).await;
    assert_eq!(status["status"], "VALID", "got {status:?}");
    for field in ["aggregateProof", "burnedFees"] {
        let mut without = payload.clone();
        without.as_object_mut().unwrap().remove(field);
        let (other_store, _) = setup_store_with_lstar(&keys, at_genesis, at_genesis).await;
        let status = new_payload_on(other_store, without).await;
        assert_eq!(
            status["status"], "INVALID",
            "without {field}: got {status:?}"
        );
    }
}

#[tokio::test]
async fn chain_that_schedules_lstar_without_istar_has_no_aggregate_proof() {
    let first = key(0x93);
    let second = key(0x94);
    let keys = [&first, &second];
    // LStar from the first block on, and IStar never: the chain goes from Hegota
    // to LStar.
    let lstar_after = Some(BLOCK_TIME);
    let (store, chain_id) = setup_store_with_lstar(&keys, None, lstar_after).await;
    let blockchain = blockchain(store.clone());
    for sender in keys {
        blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(proof_transaction(
                chain_id, sender, 0, 0,
            )))
            .await
            .expect("the transaction enters the pool");
    }
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    let bal = built.block_access_list.expect("an LStar block has a BAL");
    assert_eq!(
        store.get_chain_config().fork(block.header.timestamp),
        Fork::LStar
    );
    assert_eq!(block.body.transactions.len(), 2);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));

    // The builder strips nothing. The block has the fees burned of LStar
    // (EIP-8079) and no aggregate proof: its header is the one EIP-8079 defines.
    assert!(
        block
            .body
            .transactions
            .iter()
            .all(|tx| !frame_tx(tx).has_stripped_proofs())
    );
    assert_eq!(block.body.aggregate_proof, None);
    assert_eq!(block.header.aggregate_proof_hash, None);
    assert!(block.header.burned_fees.is_some());
    let decoded = BlockHeader::decode(&block.header.encode_to_vec()).expect("the header decodes");
    assert_eq!(decoded, block.header);
    assert_eq!(decoded.aggregate_proof_hash, None);
    assert_eq!(decoded.burned_fees, block.header.burned_fees);
    for result in import_after(None, &block, &bal, &keys, None, lstar_after).await {
        result.expect("the LStar block imports with its proofs");
    }

    // A stripped transaction is invalid in it, as in a Hegota block,
    let mut transactions = block.body.transactions.clone();
    transactions[1] = Transaction::FrameTransaction(stripped(frame_tx(&transactions[1])));
    let stripped_tx = frame_tx(&transactions[1]);
    let signer = keys
        .iter()
        .find(|key| key.account == stripped_tx.sender)
        .unwrap();
    let proof = aggregate(&[(signer, stripped_tx.compute_sig_hash())]);
    let with_stripped = with_transactions(block.clone(), transactions.clone());
    let results = import_after(None, &with_stripped, &bal, &keys, None, lstar_after).await;
    assert_rejected!(results, InvalidBlockError::StrippedProofFramePresent(1));
    // and so is an aggregate proof, with the stripped claim it proves or without.
    for invalid in [
        with_proof(with_stripped, proof.clone()),
        with_proof(block, proof),
    ] {
        let results = import_after(None, &invalid, &bal, &keys, None, lstar_after).await;
        assert_rejected!(
            results,
            InvalidBlockError::InvalidHeader(InvalidBlockHeaderError::AggregateProofHashPresent)
        );
    }
}

#[tokio::test]
async fn included_transaction_leaves_the_pool_by_its_signature_hash() {
    let first = key(0x84);
    let second = key(0x85);
    let keys = [&first, &second];
    let built = istar_block(&first, &second).await;

    // Another node holds the first transaction as it was signed, and another
    // transaction of the second sender for the same nonce.
    let (store, chain_id) = setup_store(&keys, ISTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());
    let same = built.pooled[0].clone();
    let mut other = transaction_with_proof_frames(
        chain_id,
        &second,
        0,
        vec![leansphincs_proof_frame(second.public, H256::zero(), 0)],
    );
    other.max_priority_fee_per_gas = U256::from(2_000_000_000u64);
    sign_proof_frame(&mut other, 0, &second);
    assert_ne!(other.compute_sig_hash(), built.sig_hashes[1]);
    for tx in [&same, &other] {
        blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
            .await
            .expect("the transaction enters the pool");
    }

    blockchain
        .add_block(built.block.clone())
        .expect("the block imports");
    apply_fork_choice(&store, built.block.hash(), H256::zero(), H256::zero(), None)
        .await
        .expect("the block becomes the head");
    blockchain
        .remove_block_transactions_from_pool(&built.block)
        .unwrap();

    // The block carries neither transaction under the hash the pool has for it.
    // The included one is found by its signature hash, which stripping keeps.
    let pending = |tx: &FrameTransaction| {
        blockchain
            .mempool
            .get_transaction_by_hash(tx_hash(tx))
            .unwrap()
            .is_some()
    };
    assert!(!pending(&same));
    // The other one is not the transaction the block included.
    assert!(pending(&other));
}

#[tokio::test]
async fn payload_build_loop_strips_and_proves_the_payload_it_returns() {
    let first = key(0x7b);
    let second = key(0x7c);
    let keys = [&first, &second];
    let (store, chain_id) = setup_store(&keys, ISTAR_AT_FIRST_BLOCK).await;
    let blockchain = Arc::new(blockchain(store.clone()));
    for sender in keys {
        blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(proof_transaction(
                chain_id, sender, 0, 0,
            )))
            .await
            .expect("the transaction enters the pool");
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
    let built = building
        .await
        .expect("the payload is built, stripped and proven");

    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 2);
    assert!(
        block
            .body
            .transactions
            .iter()
            .all(|tx| frame_tx(tx).stripped_proof_len(0).is_some())
    );
    let proof = block.body.aggregate_proof.clone().unwrap();
    assert!(!proof.is_empty());
    assert_eq!(
        block.header.aggregate_proof_hash,
        Some(compute_aggregate_proof_hash(&proof, &NativeCrypto))
    );
    let bal = built.block_access_list.expect("an IStar block has a BAL");
    for result in import(&block, &bal, &keys, ISTAR_AT_FIRST_BLOCK).await {
        result.expect("the block imports");
    }
}

/// The parameters of `engine_newPayloadV5` for a block of these tests: no blobs,
/// a zero beacon root and no execution requests.
fn new_payload_params(payload: Value) -> Option<Vec<Value>> {
    Some(vec![payload, json!([]), json!(H256::zero()), json!([])])
}

async fn new_payload(payload: Value, keys: &[&Key], istar_after: Option<u64>) -> Value {
    let (store, _) = setup_store(keys, istar_after).await;
    new_payload_on(store, payload).await
}

/// What `engine_newPayloadV5` answers for the payload on a node with this store.
async fn new_payload_on(store: Store, payload: Value) -> Value {
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
async fn aggregate_proof_travels_through_get_payload_and_new_payload() {
    const PAYLOAD_ID: u64 = 1;
    let first = key(0x7d);
    let second = key(0x7e);
    let keys = [&first, &second];
    let (store, chain_id) = setup_store(&keys, ISTAR_AT_FIRST_BLOCK).await;
    let builder = default_context_with_storage(store.clone()).await;
    let mut pooled = Vec::new();
    for sender in keys {
        let tx = Transaction::FrameTransaction(proof_transaction(chain_id, sender, 0, 0));
        builder
            .blockchain
            .add_transaction_to_pool(tx.clone())
            .await
            .expect("the transaction enters the pool");
        pooled.push(format!("0x{}", hex::encode(tx.encode_canonical_to_vec())));
    }

    // The builder starts a payload, as on a fork choice update with payload
    // attributes. Its first build runs before anything else on this thread.
    builder
        .blockchain
        .clone()
        .initiate_payload_build(payload_on(&store, &genesis_header(&store)), PAYLOAD_ID)
        .await;
    tokio::task::yield_now().await;

    // `engine_getPayloadV6` returns the aggregate proof and the transactions
    // without their signatures.
    let response = GetPayloadV6Request {
        payload_id: PAYLOAD_ID,
    }
    .handle(builder.clone())
    .await
    .expect("getPayload returns the payload");
    let payload = response["executionPayload"].clone();
    let transactions = payload["transactions"]
        .as_array()
        .expect("the payload lists its transactions");
    assert_eq!(transactions.len(), 2);
    for encoded in transactions {
        let encoded = encoded.as_str().unwrap();
        assert!(!pooled.iter().any(|tx| tx == encoded));
        let raw = hex::decode(encoded.trim_start_matches("0x")).unwrap();
        assert!(raw.len() < SIGNATURE_SIZE);
        let tx = Transaction::decode_canonical(&raw).expect("the payload's transaction decodes");
        assert_eq!(
            frame_tx(&tx).stripped_proof_len(0),
            Some(LEANSPHINCS_VERIFIER_PROOF_LENGTH)
        );
    }
    let proof = payload["aggregateProof"]
        .as_str()
        .expect("the payload has an aggregate proof");
    assert!(proof.starts_with("0x") && proof.len() > 2);
    // The header's commitment is not a payload field: the receiver derives it,
    // and the block hash covers it.
    assert!(payload.get("aggregateProofHash").is_none());

    // `engine_newPayloadV5` on another node accepts it.
    let status = new_payload(payload.clone(), &keys, ISTAR_AT_FIRST_BLOCK).await;
    assert_eq!(status["status"], "VALID", "got {status:?}");
    assert_eq!(status["latestValidHash"], payload["blockHash"]);

    // The same payload with another proof is not the block its hash names.
    let mut other_proof = payload.clone();
    other_proof["aggregateProof"] = json!(format!("{proof}00"));
    let status = new_payload(other_proof, &keys, ISTAR_AT_FIRST_BLOCK).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
    // Nor is it without the proof.
    let mut without_proof = payload.clone();
    without_proof
        .as_object_mut()
        .unwrap()
        .remove("aggregateProof");
    let status = new_payload(without_proof, &keys, ISTAR_AT_FIRST_BLOCK).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
    // Nor with the transactions as they were pooled.
    let mut with_pooled_transactions = payload;
    with_pooled_transactions["transactions"] = json!(pooled);
    let status = new_payload(with_pooled_transactions, &keys, ISTAR_AT_FIRST_BLOCK).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
}

#[tokio::test]
async fn engine_answers_invalid_to_a_payload_whose_aggregate_proof_does_not_hold() {
    let first = key(0x7f);
    let second = key(0x80);
    let other = key(0x81);
    let keys = [&first, &second];
    let built = istar_block(&first, &second).await;
    let proof = built.block.body.aggregate_proof.clone().unwrap();

    // The block as built is valid.
    let status = new_payload(
        payload_json(built.block.clone(), &built.bal),
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_eq!(status["status"], "VALID", "got {status:?}");

    // In each of these the block hash is the hash of the block the payload
    // describes, so only the proof makes the payload invalid.
    let mut flipped = proof.to_vec();
    flipped[0] ^= 0x01;
    let mut oversized = proof.to_vec();
    oversized.resize(MAX_AGGREGATE_PROOF_BYTES + 1, 0);
    let invalid_proofs = [
        // A proof of one claim fewer.
        aggregate(&[(&first, built.sig_hashes[0])]),
        // A proof of one claim more.
        aggregate(&[
            (&first, built.sig_hashes[0]),
            (&second, built.sig_hashes[1]),
            (&other, H256::repeat_byte(0x22)),
        ]),
        // A proof for another key.
        aggregate(&[
            (&other, built.sig_hashes[0]),
            (&second, built.sig_hashes[1]),
        ]),
        // Bytes that are not the proof.
        Bytes::from(flipped),
        proof.slice(..proof.len() / 2),
        // No proof for a block that has stripped frames.
        Bytes::new(),
        // A proof over `MAX_AGGREGATE_PROOF_BYTES`.
        Bytes::from(oversized),
    ];
    for invalid_proof in invalid_proofs {
        let block = with_proof(built.block.clone(), invalid_proof);
        let status =
            new_payload(payload_json(block, &built.bal), &keys, ISTAR_AT_FIRST_BLOCK).await;
        assert_eq!(status["status"], "INVALID", "got {status:?}");
    }

    // A stripped frame the protocol may not strip, under an aggregate that holds
    // for its claim.
    let mut readable = stripped(&built.pooled[0]);
    readable.frames[0].flags = PROOF_READ_FLAG;
    let block = with_first_transaction(&built, readable, &first, &second);
    let status = new_payload(payload_json(block, &built.bal), &keys, ISTAR_AT_FIRST_BLOCK).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");

    // A proof in a block without stripped frames.
    let (store, _) = setup_store(&keys, ISTAR_AT_FIRST_BLOCK).await;
    let empty = build_block(&store, &blockchain(store.clone()), &genesis_header(&store));
    let empty_bal = empty.block_access_list.expect("an IStar block has a BAL");
    let status = new_payload(
        payload_json(empty.payload.clone(), &empty_bal),
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_eq!(status["status"], "VALID", "got {status:?}");
    let status = new_payload(
        payload_json(with_proof(empty.payload, proof), &empty_bal),
        &keys,
        ISTAR_AT_FIRST_BLOCK,
    )
    .await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
}

#[tokio::test]
async fn engine_reads_the_aggregate_proof_of_a_payload_under_its_fork() {
    let first = key(0x82);
    let second = key(0x83);
    let keys = [&first, &second];

    // A Hegota payload carries its transactions whole and no aggregate proof.
    let (store, chain_id) = setup_store(&keys, None).await;
    let blockchain = blockchain(store.clone());
    let pooled = keys.map(|sender| proof_transaction(chain_id, sender, 0, 0));
    for tx in &pooled {
        blockchain
            .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
            .await
            .expect("the transaction enters the pool");
    }
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let bal = built.block_access_list.expect("a Hegota block has a BAL");
    let payload = payload_json(built.payload.clone(), &bal);
    assert!(payload.get("aggregateProof").is_none());
    let status = new_payload(payload.clone(), &keys, None).await;
    assert_eq!(status["status"], "VALID", "got {status:?}");

    // With an aggregate proof it is another block than its hash names, and a
    // block with one before IStar is invalid under its own hash too.
    let mut with_empty_proof = payload.clone();
    with_empty_proof["aggregateProof"] = json!("0x");
    let status = new_payload(with_empty_proof, &keys, None).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
    let proof = aggregate(&[
        (&first, pooled[0].compute_sig_hash()),
        (&second, pooled[1].compute_sig_hash()),
    ]);
    let status = new_payload(
        payload_json(with_proof(built.payload.clone(), proof), &bal),
        &keys,
        None,
    )
    .await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");

    // A stripped transaction in a Hegota payload.
    let mut transactions = built.payload.body.transactions.clone();
    transactions[0] = Transaction::FrameTransaction(stripped(frame_tx(&transactions[0])));
    let status = new_payload(
        payload_json(with_transactions(built.payload, transactions), &bal),
        &keys,
        None,
    )
    .await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
}
