//! EIP-8141 declarative claims from submission to import.
//!
//! A frame transaction declares claims and travels with a witness for each: it
//! is submitted with them in its pooled representation, kept with them in the
//! pool, and placed in a block whose `claim_witnesses` carries one witness for
//! each distinct dependency of its transactions. Every node that imports the
//! block checks those witnesses against the block's pre-state before it executes
//! anything.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::{ChainError, MempoolError},
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    errors::InvalidBlockError,
    evm::calculate_create_address,
    types::{
        Block, Claim, DEFAULT_BUILDER_GAS_CEIL, Dependency, ELASTICITY_MULTIPLIER, Frame,
        FrameMode, FrameTransaction, Genesis, GenesisAccount, LEANSPHINCS_SIGNATURE_SIZE,
        LEANSPHINCS_VERIFIER, MAX_BLOCK_WITNESS_BYTES, MAX_CLAIM_WITNESS_BYTES, P2PTransaction,
        PooledFrameTransaction, SECP256K1_VERIFIER, Transaction,
        block_access_list::BlockAccessList, block_dependencies, compute_claim_witnesses_hash,
        decode_claim_witnesses, encode_claim_witnesses,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::{
    rpc::RpcHandler, test_utils::default_context_with_storage,
    types::transaction::SendRawTransactionRequest,
};
use ethrex_storage::{EngineType, Store};
use serde_json::json;

use crate::levm::eip8141_claims_tests::{claim_account_code, leansphincs_account_code, push32};

const RECIPIENT: Address = H160([0x86; 20]);
const GWEI: u64 = 1_000_000_000;

/// A leanSPHINCS key pair and the account it controls.
pub(crate) struct Key {
    secret: [u8; 32],
    pub(crate) public: H256,
    pub(crate) account: Address,
}

pub(crate) fn key(seed: u8) -> Key {
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

fn funded(code: Vec<u8>) -> GenesisAccount {
    GenesisAccount {
        code: Bytes::from(code),
        storage: BTreeMap::new(),
        balance: U256::from(10u64).pow(U256::from(18u64)),
        nonce: 0,
    }
}

/// `l1-hegota.json` (every fork through Hegota active at genesis) with each key's
/// account funded and running the validation code for that key, plus `accounts`.
/// With `hegota` unset the chain stops at the fork before it.
async fn setup_store_at(
    keys: &[&Key],
    accounts: &[(Address, GenesisAccount)],
    hegota: bool,
) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    if !hegota {
        genesis.config.hegota_time = None;
    }
    for key in keys {
        genesis
            .alloc
            .insert(key.account, funded(leansphincs_account_code(key.public)));
    }
    for (address, account) in accounts {
        genesis.alloc.insert(*address, account.clone());
    }
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

pub(crate) async fn setup_store(keys: &[&Key]) -> (Store, u64) {
    setup_store_at(keys, &[], true).await
}

pub(crate) fn blockchain(store: Store) -> Blockchain {
    Blockchain::new(
        store,
        BlockchainOptions {
            min_tip_wei: 0,
            ..Default::default()
        },
    )
}

/// A claim that the leanSPHINCS key signed `msg`, or the transaction's own
/// signature hash when `msg` is `None`.
fn leansphincs_claim(key: &Key, msg: Option<H256>) -> Claim {
    Claim {
        verifier: LEANSPHINCS_VERIFIER,
        key: Bytes::copy_from_slice(key.public.as_bytes()),
        msg: msg.map_or_else(Bytes::new, |msg| Bytes::copy_from_slice(msg.as_bytes())),
        ..Default::default()
    }
}

/// `[VERIFY, SENDER]` from `sender` with `claims`: the sender's validation frame
/// and a call to `RECIPIENT`.
fn frame_transaction(chain_id: u64, sender: Address, claims: Vec<Claim>) -> FrameTransaction {
    let mut tx = FrameTransaction {
        chain_id,
        sender,
        claims,
        frames: vec![
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0x03,
                target: Some(sender),
                gas_limit: 50_000,
                ..Default::default()
            },
            Frame {
                mode: u8::from(FrameMode::Sender),
                target: Some(RECIPIENT),
                gas_limit: 50_000,
                ..Default::default()
            },
        ],
        max_priority_fee_per_gas: U256::from(GWEI),
        ..Default::default()
    };
    tx.max_fee = U256::from(10 * GWEI) * U256::from(tx.max_gas());
    tx
}

/// A transaction from the key's account that claims the key's signature of the
/// transaction, followed by `more_claims`.
pub(crate) fn claim_transaction(
    chain_id: u64,
    key: &Key,
    more_claims: Vec<Claim>,
) -> FrameTransaction {
    let mut claims = vec![leansphincs_claim(key, None)];
    claims.extend(more_claims);
    frame_transaction(chain_id, key.account, claims)
}

/// The transaction in its pooled representation, with each leanSPHINCS claim
/// signed by the key it names. Signing is the slow part of these tests, so the
/// claims are signed on as many threads as the machine offers.
pub(crate) fn pooled(tx: FrameTransaction, keys: &[&Key]) -> PooledFrameTransaction {
    let dependencies = tx.claim_dependencies().expect("well-formed claims");
    let sign_claim = |dependency: &Dependency| {
        let key = keys
            .iter()
            .find(|key| key.public == dependency.key)
            .expect("a key for every claim");
        sign(key, dependency.msg)
    };
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let chunk_len = dependencies.len().div_ceil(threads).max(1);
    let witnesses = std::thread::scope(|scope| {
        let workers: Vec<_> = dependencies
            .chunks(chunk_len)
            .map(|chunk| scope.spawn(|| chunk.iter().map(sign_claim).collect::<Vec<_>>()))
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("signing does not panic"))
            .collect()
    });
    PooledFrameTransaction {
        tx,
        witnesses,
        blobs_bundle: None,
    }
}

fn tx_hash(tx: &FrameTransaction) -> H256 {
    Transaction::FrameTransaction(tx.clone()).hash(&NativeCrypto)
}

fn build_block_on(
    store: &Store,
    blockchain: &Blockchain,
    parent_number: u64,
) -> PayloadBuildResult {
    let parent = store.get_block_header(parent_number).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(parent_number + 1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(payload).unwrap()
}

fn build_block(store: &Store, blockchain: &Blockchain) -> PayloadBuildResult {
    build_block_on(store, blockchain, 0)
}

/// The block with its claim witnesses replaced by `claim_witnesses` and the
/// header committing to them, so only the witnesses themselves can make it
/// invalid.
fn with_claim_witnesses(mut block: Block, claim_witnesses: Bytes) -> Block {
    block.header.claim_witnesses_hash = Some(compute_claim_witnesses_hash(
        &claim_witnesses,
        &NativeCrypto,
    ));
    block.body.claim_witnesses = Some(claim_witnesses);
    block
}

fn with_witnesses(block: Block, witnesses: &[Bytes]) -> Block {
    with_claim_witnesses(block, encode_claim_witnesses(witnesses))
}

/// Imports `block` on two fresh nodes built by `fresh_store`, one through each
/// import path, and returns what each of them answered: `add_block` first, then
/// `add_block_pipeline`.
async fn import_on_both_paths<F, Fut>(
    block: &Block,
    bal: &Arc<BlockAccessList>,
    fresh_store: F,
) -> [Result<(), ChainError>; 2]
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Store>,
{
    let plain = blockchain(fresh_store().await).add_block(block.clone());
    let pipeline =
        blockchain(fresh_store().await).add_block_pipeline(block.clone(), Some(bal.clone()));
    [plain, pipeline]
}

/// Asserts that both import paths reject `block` with an invalid-block error
/// that `expected` recognizes.
async fn assert_rejected_on_both_paths<F, Fut>(
    block: Block,
    bal: &Arc<BlockAccessList>,
    fresh_store: F,
    what: &str,
    expected: impl Fn(&InvalidBlockError) -> bool,
) where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Store>,
{
    for (path, result) in ["add_block", "add_block_pipeline"]
        .into_iter()
        .zip(import_on_both_paths(&block, bal, fresh_store).await)
    {
        assert!(
            matches!(&result, Err(ChainError::InvalidBlock(error)) if expected(error)),
            "{what}, {path}: got {result:?}"
        );
    }
}

#[tokio::test]
async fn a_frame_transaction_with_a_claim_goes_from_the_pool_into_a_block_that_imports() {
    let key = key(0x51);
    let other_key = self::key(0x52);
    let (store, chain_id) = setup_store(&[&key]).await;
    let blockchain = blockchain(store.clone());

    let pooled_tx = pooled(claim_transaction(chain_id, &key, Vec::new()), &[&key]);
    let witness = pooled_tx.witnesses[0].clone();
    assert_eq!(witness.len(), LEANSPHINCS_SIGNATURE_SIZE);
    let hash = blockchain
        .add_frame_transaction_to_pool(pooled_tx.clone())
        .await
        .expect("the transaction enters the pool with its witness");
    assert_eq!(hash, tx_hash(&pooled_tx.tx));

    // The pool keeps the witness with the transaction. It is announced by hash
    // like any other pooled transaction, and served in its pooled representation.
    assert_eq!(
        blockchain.mempool.get_claim_witnesses(hash).unwrap(),
        Some(vec![witness.clone()])
    );
    assert_eq!(
        blockchain
            .mempool
            .get_txs_for_broadcast()
            .unwrap()
            .iter()
            .map(|tx| tx.hash(&NativeCrypto))
            .collect::<Vec<_>>(),
        vec![hash]
    );
    assert_eq!(
        blockchain
            .get_p2p_transaction_by_hash(&hash)
            .unwrap()
            .encode_canonical_to_vec(),
        P2PTransaction::FrameTransaction(pooled_tx.clone()).encode_canonical_to_vec()
    );

    // The block includes the transaction and carries its witness.
    let built = build_block(&store, &blockchain);
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 1);
    assert!(
        built.receipts.iter().all(|receipt| receipt.succeeded),
        "the transaction must succeed: {:?}",
        built.receipts
    );
    let claim_witnesses = block
        .body
        .claim_witnesses
        .clone()
        .expect("a Hegota block has claim witnesses");
    assert_eq!(
        claim_witnesses,
        encode_claim_witnesses(std::slice::from_ref(&witness))
    );
    assert_eq!(
        block.header.claim_witnesses_hash,
        Some(H256(ethrex_crypto::keccak::keccak_hash(&claim_witnesses)))
    );
    // The transaction in the block is the bare body: its hash and the block's
    // transactions root say nothing about the witness.
    assert_eq!(block.body.transactions[0].hash(&NativeCrypto), hash);

    // It imports on every path.
    let bal = Arc::new(built.block_access_list.expect("a Hegota block has a BAL"));
    let fresh_store = || async { setup_store(&[&key]).await.0 };
    for (path, result) in ["add_block", "add_block_pipeline"]
        .into_iter()
        .zip(import_on_both_paths(&block, &bal, fresh_store).await)
    {
        result.unwrap_or_else(|error| panic!("{path}: {error}"));
    }

    // A block whose witness is not a signature of the dependency is rejected, on
    // every path, before anything executes.
    let mut tampered = witness.to_vec();
    tampered[100] ^= 0x01;
    assert_rejected_on_both_paths(
        with_witnesses(block.clone(), &[Bytes::from(tampered)]),
        &bal,
        fresh_store,
        "a tampered witness",
        |error| matches!(error, InvalidBlockError::ClaimNotSatisfied(0)),
    )
    .await;

    // So is one whose witness is a valid signature of the same message under
    // another key.
    let dependency = pooled_tx.tx.dependencies().unwrap()[0];
    assert_rejected_on_both_paths(
        with_witnesses(block.clone(), &[sign(&other_key, dependency.msg)]),
        &bal,
        fresh_store,
        "the witness of another key",
        |error| matches!(error, InvalidBlockError::ClaimNotSatisfied(0)),
    )
    .await;

    // A witness that is not exactly as long as its claim fixes.
    assert_rejected_on_both_paths(
        with_witnesses(block.clone(), &[witness.slice(..witness.len() - 1)]),
        &bal,
        fresh_store,
        "a witness one byte short",
        |error| matches!(error, InvalidBlockError::ClaimNotSatisfied(0)),
    )
    .await;

    // A block that leaves the witness out, and one that carries one too many.
    assert_rejected_on_both_paths(
        with_witnesses(block.clone(), &[]),
        &bal,
        fresh_store,
        "a missing witness",
        |error| {
            matches!(
                error,
                InvalidBlockError::ClaimWitnessesCountMismatch {
                    expected: 1,
                    found: 0
                }
            )
        },
    )
    .await;
    assert_rejected_on_both_paths(
        with_witnesses(block.clone(), &[witness.clone(), witness.clone()]),
        &bal,
        fresh_store,
        "an extra witness",
        |error| {
            matches!(
                error,
                InvalidBlockError::ClaimWitnessesCountMismatch {
                    expected: 1,
                    found: 2
                }
            )
        },
    )
    .await;

    // A header that commits to other bytes than the block carries.
    let mut other_commitment = block.clone();
    other_commitment.header.claim_witnesses_hash = Some(compute_claim_witnesses_hash(
        &encode_claim_witnesses(&[sign(&other_key, dependency.msg)]),
        &NativeCrypto,
    ));
    assert_rejected_on_both_paths(
        other_commitment,
        &bal,
        fresh_store,
        "a wrong header commitment",
        |error| matches!(error, InvalidBlockError::ClaimWitnessesHashMismatch),
    )
    .await;

    // Claim witnesses of a version this node does not know, an encoding that is
    // not the version byte and a list, and the list with bytes after it.
    let mut other_version = claim_witnesses.to_vec();
    other_version[0] = 0x01;
    let mut trailing = claim_witnesses.to_vec();
    trailing.push(0x00);
    for (what, malformed) in [
        ("an unknown version", other_version),
        ("no list", vec![0x00]),
        ("an empty list", vec![0x00, 0xc0]),
        ("trailing bytes", trailing),
    ] {
        assert_rejected_on_both_paths(
            with_claim_witnesses(block.clone(), Bytes::from(malformed)),
            &bal,
            fresh_store,
            what,
            |error| matches!(error, InvalidBlockError::InvalidClaimWitnesses(_)),
        )
        .await;
    }

    // Claim witnesses longer than a block may carry, whatever they hold.
    let oversized = encode_claim_witnesses(&[Bytes::from(vec![0u8; MAX_BLOCK_WITNESS_BYTES])]);
    assert!(oversized.len() > MAX_BLOCK_WITNESS_BYTES);
    assert_rejected_on_both_paths(
        with_claim_witnesses(block.clone(), oversized),
        &bal,
        fresh_store,
        "claim witnesses over the size limit",
        |error| {
            matches!(error, InvalidBlockError::InvalidClaimWitnesses(reason)
                if reason.contains("MAX_BLOCK_WITNESS_BYTES"))
        },
    )
    .await;
}

#[tokio::test]
async fn a_block_without_claims_carries_empty_claim_witnesses() {
    let key = key(0x5c);
    let (store, _) = setup_store(&[&key]).await;
    let blockchain = blockchain(store.clone());

    let built = build_block(&store, &blockchain);
    let block = built.payload;
    assert!(block.body.transactions.is_empty());
    assert_eq!(block.body.claim_witnesses, Some(Bytes::new()));
    assert_eq!(
        block.header.claim_witnesses_hash,
        Some(H256(hex_literal::hex!(
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        )))
    );
    let bal = Arc::new(built.block_access_list.expect("a Hegota block has a BAL"));
    let fresh_store = || async { setup_store(&[&key]).await.0 };
    for result in import_on_both_paths(&block, &bal, fresh_store).await {
        result.expect("the block imports");
    }

    // A witness nobody claimed makes it invalid.
    let stray = with_witnesses(block, &[sign(&key, H256::repeat_byte(1))]);
    assert_rejected_on_both_paths(stray, &bal, fresh_store, "a stray witness", |error| {
        matches!(
            error,
            InvalidBlockError::ClaimWitnessesCountMismatch {
                expected: 0,
                found: 1
            }
        )
    })
    .await;
}

#[tokio::test]
async fn a_dependency_two_transactions_share_is_carried_once() {
    let first = key(0x52);
    let second = key(0x53);
    let shared = key(0x54);
    let (store, chain_id) = setup_store(&[&first, &second]).await;
    let blockchain = blockchain(store.clone());

    // Both transactions also claim a signature of the same message under a third
    // key.
    let message = H256::repeat_byte(0x77);
    let mut all_witnesses: Vec<(Dependency, Bytes)> = Vec::new();
    for sender in [&first, &second] {
        let tx = claim_transaction(
            chain_id,
            sender,
            vec![leansphincs_claim(&shared, Some(message))],
        );
        let pooled_tx = pooled(tx, &[sender, &shared]);
        assert_eq!(pooled_tx.witnesses.len(), 2);
        all_witnesses.extend(
            pooled_tx
                .tx
                .claim_dependencies()
                .unwrap()
                .into_iter()
                .zip(pooled_tx.witnesses.clone()),
        );
        blockchain
            .add_frame_transaction_to_pool(pooled_tx)
            .await
            .expect("the transaction enters the pool");
    }

    let built = build_block(&store, &blockchain);
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 2);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));

    // Four claims and three distinct dependencies, in the order of
    // `block_dependencies`, each with a witness one of the transactions was
    // submitted with.
    let dependencies = block_dependencies(&block.body.transactions).unwrap();
    assert_eq!(dependencies.len(), 3);
    let witnesses = decode_claim_witnesses(
        block
            .body
            .claim_witnesses
            .as_ref()
            .expect("the block has claim witnesses"),
    )
    .expect("they decode");
    assert_eq!(witnesses.len(), 3);
    for (dependency, witness) in dependencies.iter().zip(&witnesses) {
        assert!(
            all_witnesses.contains(&(*dependency, witness.clone())),
            "the witness of {dependency:?} comes from the pool"
        );
    }
    assert_eq!(
        dependencies
            .iter()
            .filter(|dependency| dependency.msg == message)
            .count(),
        1,
        "the shared dependency appears once"
    );

    let bal = Arc::new(built.block_access_list.expect("a Hegota block has a BAL"));
    let fresh_store = || async { setup_store(&[&first, &second]).await.0 };
    for result in import_on_both_paths(&block, &bal, fresh_store).await {
        result.expect("the block imports");
    }

    // The witnesses in any other order do not satisfy their dependencies.
    let mut swapped = witnesses.clone();
    swapped.swap(0, 2);
    assert_rejected_on_both_paths(
        with_witnesses(block, &swapped),
        &bal,
        fresh_store,
        "witnesses out of order",
        |error| matches!(error, InvalidBlockError::ClaimNotSatisfied(_)),
    )
    .await;
}

#[tokio::test]
async fn the_pool_admits_a_frame_transaction_only_with_a_valid_witness_for_each_claim() {
    let key = key(0x55);
    let other_key = self::key(0x56);
    let (store, chain_id) = setup_store(&[&key]).await;
    let blockchain = blockchain(store.clone());
    let tx = claim_transaction(chain_id, &key, Vec::new());
    let valid = pooled(tx.clone(), &[&key]);
    let with_witnesses = |witnesses: Vec<Bytes>| PooledFrameTransaction {
        tx: tx.clone(),
        witnesses,
        blobs_bundle: None,
    };

    // Alone, without its witnesses.
    let result = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
        .await;
    assert!(
        matches!(result, Err(MempoolError::FrameTxWithoutWitnesses)),
        "got {result:?}"
    );

    // With a signature made with another key.
    let forged = with_witnesses(vec![sign(&other_key, tx.compute_sig_hash())]);
    let result = blockchain.add_frame_transaction_to_pool(forged).await;
    assert!(
        matches!(result, Err(MempoolError::ClaimNotSatisfied(0))),
        "got {result:?}"
    );

    // With the key's signature of another message.
    let other_message = with_witnesses(vec![sign(&key, H256::repeat_byte(0x01))]);
    let result = blockchain
        .add_frame_transaction_to_pool(other_message)
        .await;
    assert!(
        matches!(result, Err(MempoolError::ClaimNotSatisfied(0))),
        "got {result:?}"
    );

    // With fewer witnesses than claims, and with more.
    let result = blockchain
        .add_frame_transaction_to_pool(with_witnesses(Vec::new()))
        .await;
    assert!(
        matches!(
            result,
            Err(MempoolError::ClaimWitnessCountMismatch {
                expected: 1,
                found: 0
            })
        ),
        "got {result:?}"
    );
    let two = vec![valid.witnesses[0].clone(), valid.witnesses[0].clone()];
    let result = blockchain
        .add_frame_transaction_to_pool(with_witnesses(two))
        .await;
    assert!(
        matches!(
            result,
            Err(MempoolError::ClaimWitnessCountMismatch {
                expected: 1,
                found: 2
            })
        ),
        "got {result:?}"
    );

    // With a witness that is not as long as its claim fixes.
    let short = valid.witnesses[0].slice(..LEANSPHINCS_SIGNATURE_SIZE - 1);
    let result = blockchain
        .add_frame_transaction_to_pool(with_witnesses(vec![short]))
        .await;
    assert!(
        matches!(
            result,
            Err(MempoolError::ClaimWitnessLengthMismatch {
                index: 0,
                expected: 7740,
                found: 7739
            })
        ),
        "got {result:?}"
    );

    // A transaction without claims has nothing for a witness to satisfy.
    let plain = frame_transaction(chain_id, key.account, Vec::new());
    let result = blockchain
        .add_frame_transaction_to_pool(PooledFrameTransaction {
            tx: plain,
            witnesses: vec![valid.witnesses[0].clone()],
            blobs_bundle: None,
        })
        .await;
    assert!(
        matches!(
            result,
            Err(MempoolError::ClaimWitnessCountMismatch {
                expected: 0,
                found: 1
            })
        ),
        "got {result:?}"
    );

    assert_eq!(blockchain.mempool.claim_witnesses_size().unwrap(), (0, 0));

    // With its witness it is admitted, and a later copy of the same transaction
    // with another witness is ignored: the pool keeps the first one.
    let hash = blockchain
        .add_frame_transaction_to_pool(valid.clone())
        .await
        .expect("admitted");
    let later = with_witnesses(vec![sign(&other_key, tx.compute_sig_hash())]);
    assert_eq!(
        blockchain
            .add_frame_transaction_to_pool(later)
            .await
            .expect("ignored"),
        hash
    );
    assert_eq!(
        blockchain.mempool.get_claim_witnesses(hash).unwrap(),
        Some(valid.witnesses.clone())
    );
    assert_eq!(
        blockchain.mempool.claim_witnesses_size().unwrap(),
        (1, LEANSPHINCS_SIGNATURE_SIZE)
    );

    // The witnesses leave the pool with the transaction.
    blockchain.remove_transaction_from_pool(&hash).unwrap();
    assert_eq!(blockchain.mempool.claim_witnesses_size().unwrap(), (0, 0));
    assert_eq!(blockchain.mempool.get_claim_witnesses(hash).unwrap(), None);
}

#[tokio::test]
async fn the_pool_counts_every_claim_toward_the_validation_gas_budget() {
    let key = key(0x57);
    let second_key = self::key(0x58);
    let (store, chain_id) = setup_store(&[&key]).await;

    // Two leanSPHINCS claims cost 26,400 gas to verify, and the validation frame
    // declares 50,000 more.
    let tx = claim_transaction(
        chain_id,
        &key,
        vec![leansphincs_claim(
            &second_key,
            Some(H256::repeat_byte(0x01)),
        )],
    );
    let pooled_tx = pooled(tx, &[&key, &second_key]);

    let strict = Blockchain::new(
        store.clone(),
        BlockchainOptions {
            min_tip_wei: 0,
            max_verify_gas: 76_399,
            ..Default::default()
        },
    );
    let result = strict
        .add_frame_transaction_to_pool(pooled_tx.clone())
        .await;
    assert!(
        matches!(result, Err(MempoolError::FrameTxVerifyGasBudgetExceeded)),
        "got {result:?}"
    );

    // One gas more is enough, and so is the default budget.
    Blockchain::new(
        store.clone(),
        BlockchainOptions {
            min_tip_wei: 0,
            max_verify_gas: 76_400,
            ..Default::default()
        },
    )
    .add_frame_transaction_to_pool(pooled_tx.clone())
    .await
    .expect("exactly at the budget");
    blockchain(store)
        .add_frame_transaction_to_pool(pooled_tx)
        .await
        .expect("two claims are within the default budget");
}

// A block carries at most MAX_BLOCK_WITNESS_BYTES of claim witnesses, so the pool
// does not take a transaction that needs more than that for itself: no block
// could include it. Claims that resolve to one dependency need one witness.
#[tokio::test]
async fn the_pool_rejects_a_frame_transaction_whose_witnesses_cannot_fit_a_block() {
    let verifier = H160([0x77; 20]);
    let (store, chain_id) = setup_store_at(&[], &[], true).await;
    let blockchain = blockchain(store);
    let witness_len = MAX_CLAIM_WITNESS_BYTES;
    let witness = Bytes::from(vec![0u8; usize::try_from(witness_len).unwrap()]);
    let claim = |key: u8| Claim {
        verifier,
        key: Bytes::from(vec![key; 32]),
        msg: Bytes::new(),
        gas_limit: CUSTOM_CLAIM_GAS,
        witness_len,
    };
    let pooled_with = |claims: Vec<Claim>| PooledFrameTransaction {
        witnesses: vec![witness.clone(); claims.len()],
        tx: frame_transaction(chain_id, CUSTOM_ACCOUNT, claims),
        blobs_bundle: None,
    };

    // Three witnesses of the largest size a claim may declare are over the limit.
    let result = blockchain
        .add_frame_transaction_to_pool(pooled_with(vec![claim(1), claim(2), claim(3)]))
        .await;
    assert!(
        matches!(
            result,
            Err(MempoolError::ClaimWitnessesTooLarge { actual, limit })
                if limit == MAX_BLOCK_WITNESS_BYTES && actual > limit
        ),
        "got {result:?}"
    );

    // The same three claims on one dependency need one witness in a block, so
    // this limit does not reject them. The transaction fails a later check.
    let result = blockchain
        .add_frame_transaction_to_pool(pooled_with(vec![claim(1), claim(1), claim(1)]))
        .await;
    assert!(
        matches!(&result, Err(error) if !matches!(error, MempoolError::ClaimWitnessesTooLarge { .. })),
        "got {result:?}"
    );
}

#[tokio::test]
async fn the_pool_rejects_a_frame_transaction_before_hegota() {
    let key = key(0x59);
    let (store, chain_id) = setup_store_at(&[&key], &[], false).await;
    let blockchain = blockchain(store);
    let pooled_tx = pooled(claim_transaction(chain_id, &key, Vec::new()), &[&key]);

    let result = blockchain.add_frame_transaction_to_pool(pooled_tx).await;
    assert!(
        matches!(result, Err(MempoolError::FrameTxPreFork)),
        "got {result:?}"
    );
}

#[tokio::test]
async fn a_frame_transaction_is_submitted_with_its_witnesses_through_the_rpc() {
    let key = key(0x5a);
    let (store, chain_id) = setup_store(&[&key]).await;
    let ctx = default_context_with_storage(store).await;
    let pooled_tx = pooled(claim_transaction(chain_id, &key, Vec::new()), &[&key]);

    // FRAME_TX_TYPE || rlp([tx_body, wrapper_version, witnesses, blob_sidecar])
    let mut raw = vec![0x06];
    pooled_tx.encode(&mut raw);
    let returned_hash =
        SendRawTransactionRequest::parse(&Some(vec![json!(format!("0x{}", hex::encode(&raw)))]))
            .expect("the pooled representation decodes")
            .handle(ctx.clone())
            .await
            .expect("the transaction enters the pool");
    let hash = tx_hash(&pooled_tx.tx);
    assert_eq!(
        returned_hash,
        json!(format!("{hash:#x}")),
        "the transaction hash covers the transaction alone, not its witnesses"
    );
    assert_eq!(
        ctx.blockchain.mempool.get_claim_witnesses(hash).unwrap(),
        Some(pooled_tx.witnesses.clone())
    );

    // The bare transaction, the form it has inside a block, is not a submission.
    let key = self::key(0x5b);
    let (store, chain_id) = setup_store(&[&key]).await;
    let ctx = default_context_with_storage(store).await;
    let bare = Transaction::FrameTransaction(claim_transaction(chain_id, &key, Vec::new()))
        .encode_canonical_to_vec();
    let result =
        SendRawTransactionRequest::parse(&Some(vec![json!(format!("0x{}", hex::encode(&bare)))]));
    assert!(result.is_err(), "got {result:?}");

    // A pooled representation whose witness does not satisfy its claim is
    // rejected by the pool.
    let mut forged = pooled(claim_transaction(chain_id, &key, Vec::new()), &[&key]);
    forged.witnesses[0] = Bytes::from(vec![0x5a; LEANSPHINCS_SIGNATURE_SIZE]);
    let mut raw = vec![0x06];
    forged.encode(&mut raw);
    let result =
        SendRawTransactionRequest::parse(&Some(vec![json!(format!("0x{}", hex::encode(&raw)))]))
            .expect("the pooled representation decodes")
            .handle(ctx.clone())
            .await;
    assert!(result.is_err(), "got {result:?}");
    assert_eq!(
        ctx.blockchain.mempool.claim_witnesses_size().unwrap(),
        (0, 0)
    );
}

// ==================== Custom contract verifiers ====================

/// A contract that deploys the init code it is called with.
const FACTORY: Address = H160([0xFA; 20]);
/// `CALLDATACOPY(0, 0, CALLDATASIZE); CREATE(0, 0, CALLDATASIZE); STOP`
const FACTORY_CODE: &[u8] = &[0x36, 0x5f, 0x5f, 0x37, 0x36, 0x5f, 0x5f, 0xf0, 0x00];
/// An account that approves on a claim its custom verifier checks.
const CUSTOM_ACCOUNT: Address = H160([0xC5; 20]);
/// The key of that account's claim, as its verifier receives it.
const CUSTOM_KEY: H256 = H256([0x0b; 32]);
/// The only witness the custom verifier accepts.
const CUSTOM_WITNESS: H256 = H256([0xD7; 32]);
const CUSTOM_CLAIM_GAS: u64 = 5_000;

/// Runtime code of a verifier that succeeds if and only if its witness, the 32
/// bytes after `key || msg`, is `CUSTOM_WITNESS`.
fn custom_verifier_code() -> Vec<u8> {
    // PUSH1 0x40, CALLDATALOAD, PUSH32 witness, EQ
    let mut code = vec![0x60, 0x40, 0x35];
    code.extend_from_slice(&push32(CUSTOM_WITNESS));
    code.push(0x14);
    // JUMPI to STOP, 6 bytes ahead: PUSH1 dest, JUMPI, PUSH0, PUSH0, REVERT.
    let accept_at = u8::try_from(code.len() + 6).unwrap();
    code.extend_from_slice(&[0x60, accept_at, 0x57, 0x5f, 0x5f, 0xfd, 0x5b, 0x00]);
    code
}

/// Init code that deploys `runtime`.
fn init_code(runtime: &[u8]) -> Vec<u8> {
    let len = u8::try_from(runtime.len()).expect("short runtime code");
    // PUSH1 len, PUSH1 10, PUSH0, CODECOPY, PUSH1 len, PUSH0, RETURN
    let mut code = vec![0x60, len, 0x60, 0x0a, 0x5f, 0x39, 0x60, len, 0x5f, 0xf3];
    code.extend_from_slice(runtime);
    code
}

fn custom_claim(verifier: Address) -> Claim {
    Claim {
        verifier,
        key: Bytes::copy_from_slice(CUSTOM_KEY.as_bytes()),
        msg: Bytes::new(),
        gas_limit: CUSTOM_CLAIM_GAS,
        witness_len: 32,
    }
}

fn custom_pooled(tx: FrameTransaction, witness: H256) -> PooledFrameTransaction {
    PooledFrameTransaction {
        tx,
        witnesses: vec![Bytes::copy_from_slice(witness.as_bytes())],
        blobs_bundle: None,
    }
}

/// Makes `block` the canonical head, as a fork choice update does.
async fn make_head(store: &Store, block: &Block) {
    store
        .forkchoice_update(
            vec![],
            block.header.number,
            block.hash(),
            Some(block.header.number),
            Some(block.header.number),
        )
        .await
        .expect("fork choice update");
}

// A custom verifier is an ordinary contract, run in the pure context against the
// state before the block: a claim it checks holds only once the contract is in
// that state, not in the block that deploys it.
#[tokio::test]
async fn a_custom_verifier_must_be_deployed_before_the_block_that_relies_on_it() {
    let deployer = key(0x61);
    // The factory starts at nonce 1, so the verifier is its first creation.
    let verifier = calculate_create_address(FACTORY, 1);
    let accounts = [
        (
            FACTORY,
            GenesisAccount {
                nonce: 1,
                ..funded(FACTORY_CODE.to_vec())
            },
        ),
        (
            CUSTOM_ACCOUNT,
            funded(claim_account_code(verifier, CUSTOM_KEY)),
        ),
    ];
    let fresh_store = || async { setup_store_at(&[&deployer], &accounts, true).await.0 };
    let (store, chain_id) = setup_store_at(&[&deployer], &accounts, true).await;
    let blockchain = blockchain(store.clone());

    // The transaction that deploys the verifier through the factory.
    let mut deploy = claim_transaction(chain_id, &deployer, Vec::new());
    deploy.frames[1] = Frame {
        mode: u8::from(FrameMode::Sender),
        target: Some(FACTORY),
        gas_limit: 200_000,
        state_gas_limit: 1_000_000,
        data: Bytes::from(init_code(&custom_verifier_code())),
        ..Default::default()
    };
    deploy.max_fee = U256::from(10 * GWEI) * U256::from(deploy.max_gas());
    let deploy = pooled(deploy, &[&deployer]);

    // The transaction that relies on the verifier.
    let custom_tx = frame_transaction(chain_id, CUSTOM_ACCOUNT, vec![custom_claim(verifier)]);

    // Before the verifier exists, the pool cannot verify the claim.
    let result = blockchain
        .add_frame_transaction_to_pool(custom_pooled(custom_tx.clone(), CUSTOM_WITNESS))
        .await;
    assert!(
        matches!(result, Err(MempoolError::ClaimNotSatisfied(0))),
        "got {result:?}"
    );

    // Block 1 deploys it.
    blockchain
        .add_frame_transaction_to_pool(deploy.clone())
        .await
        .expect("the deployment enters the pool");
    let built_1 = build_block(&store, &blockchain);
    let block_1 = built_1.payload;
    assert_eq!(block_1.body.transactions.len(), 1);
    assert!(built_1.receipts.iter().all(|receipt| receipt.succeeded));
    let bal_1 = Arc::new(built_1.block_access_list.expect("a Hegota block has a BAL"));
    blockchain.add_block(block_1.clone()).expect("block 1");
    make_head(&store, &block_1).await;
    blockchain
        .remove_transaction_from_pool(&tx_hash(&deploy.tx))
        .unwrap();
    assert_eq!(
        store
            .get_account_info(1, verifier)
            .await
            .unwrap()
            .expect("the verifier exists")
            .code_hash,
        H256(ethrex_crypto::keccak::keccak_hash(custom_verifier_code()))
    );

    // With the verifier in the state, the pool admits the claim with the witness
    // the verifier accepts and with no other.
    let result = blockchain
        .add_frame_transaction_to_pool(custom_pooled(custom_tx.clone(), H256::repeat_byte(0xD8)))
        .await;
    assert!(
        matches!(result, Err(MempoolError::ClaimNotSatisfied(0))),
        "got {result:?}"
    );
    blockchain
        .add_frame_transaction_to_pool(custom_pooled(custom_tx.clone(), CUSTOM_WITNESS))
        .await
        .expect("the claim holds against the current state");

    // Block 2 includes it and carries its witness.
    let built_2 = build_block_on(&store, &blockchain, 1);
    let block_2 = built_2.payload;
    assert_eq!(block_2.body.transactions.len(), 1);
    assert!(built_2.receipts.iter().all(|receipt| receipt.succeeded));
    assert_eq!(
        block_2.body.claim_witnesses,
        Some(encode_claim_witnesses(&[Bytes::copy_from_slice(
            CUSTOM_WITNESS.as_bytes()
        )]))
    );
    let bal_2 = Arc::new(built_2.block_access_list.expect("a Hegota block has a BAL"));

    // The two blocks import on both paths.
    let plain = self::blockchain(fresh_store().await);
    plain.add_block(block_1.clone()).expect("block 1");
    plain.add_block(block_2.clone()).expect("block 2");
    let pipeline = self::blockchain(fresh_store().await);
    pipeline
        .add_block_pipeline(block_1.clone(), Some(bal_1.clone()))
        .expect("block 1 through the pipeline");
    pipeline
        .add_block_pipeline(block_2.clone(), Some(bal_2.clone()))
        .expect("block 2 through the pipeline");

    // Block 2 with any other witness is invalid.
    let wrong_witness = with_witnesses(block_2.clone(), &[Bytes::from(vec![0xD8; 32])]);
    let plain = self::blockchain(fresh_store().await);
    plain.add_block(block_1.clone()).expect("block 1");
    let result = plain.add_block(wrong_witness.clone());
    assert!(
        matches!(
            result,
            Err(ChainError::InvalidBlock(
                InvalidBlockError::ClaimNotSatisfied(0)
            ))
        ),
        "got {result:?}"
    );
    let pipeline = self::blockchain(fresh_store().await);
    pipeline
        .add_block_pipeline(block_1.clone(), Some(bal_1.clone()))
        .expect("block 1 through the pipeline");
    let result = pipeline.add_block_pipeline(wrong_witness, Some(bal_2));
    assert!(
        matches!(
            result,
            Err(ChainError::InvalidBlock(
                InvalidBlockError::ClaimNotSatisfied(0)
            ))
        ),
        "got {result:?}"
    );

    // A block that deploys the verifier and relies on it at once is invalid: its
    // claims are verified against the state before it, where the verifier is not.
    let mut both = block_1.clone();
    both.body
        .transactions
        .push(Transaction::FrameTransaction(custom_tx.clone()));
    let dependencies = block_dependencies(&both.body.transactions).unwrap();
    let custom_index = dependencies
        .iter()
        .position(|dependency| dependency.verifier == verifier)
        .expect("the custom dependency");
    let witnesses: Vec<Bytes> = dependencies
        .iter()
        .map(|dependency| {
            if dependency.verifier == verifier {
                Bytes::copy_from_slice(CUSTOM_WITNESS.as_bytes())
            } else {
                deploy.witnesses[0].clone()
            }
        })
        .collect();
    assert_rejected_on_both_paths(
        with_witnesses(both, &witnesses),
        &bal_1,
        fresh_store,
        "a verifier deployed in the same block",
        |error| matches!(error, InvalidBlockError::ClaimNotSatisfied(index) if *index == custom_index),
    )
    .await;
}

// The builder verifies a custom claim against the pre-state of the block it
// builds, which need not be the state the pool admitted the transaction on. A
// transaction whose claim does not hold there is left out and leaves the pool.
#[tokio::test]
async fn the_builder_leaves_out_a_transaction_whose_custom_claim_fails_in_the_pre_state() {
    let deployer = key(0x62);
    let verifier = calculate_create_address(FACTORY, 1);
    let accounts = [
        (
            FACTORY,
            GenesisAccount {
                nonce: 1,
                ..funded(FACTORY_CODE.to_vec())
            },
        ),
        (
            CUSTOM_ACCOUNT,
            funded(claim_account_code(verifier, CUSTOM_KEY)),
        ),
    ];
    let (store, chain_id) = setup_store_at(&[&deployer], &accounts, true).await;
    let blockchain = blockchain(store.clone());

    // Block 1 deploys the verifier and becomes the head.
    let mut deploy = claim_transaction(chain_id, &deployer, Vec::new());
    deploy.frames[1] = Frame {
        mode: u8::from(FrameMode::Sender),
        target: Some(FACTORY),
        gas_limit: 200_000,
        state_gas_limit: 1_000_000,
        data: Bytes::from(init_code(&custom_verifier_code())),
        ..Default::default()
    };
    deploy.max_fee = U256::from(10 * GWEI) * U256::from(deploy.max_gas());
    let deploy = pooled(deploy, &[&deployer]);
    blockchain
        .add_frame_transaction_to_pool(deploy.clone())
        .await
        .expect("the deployment enters the pool");
    let block_1 = build_block(&store, &blockchain).payload;
    assert_eq!(block_1.body.transactions.len(), 1);
    blockchain.add_block(block_1.clone()).expect("block 1");
    make_head(&store, &block_1).await;
    blockchain
        .remove_transaction_from_pool(&tx_hash(&deploy.tx))
        .unwrap();

    // The pool admits the claim against the head, where the verifier exists.
    let custom_tx = frame_transaction(chain_id, CUSTOM_ACCOUNT, vec![custom_claim(verifier)]);
    let hash = blockchain
        .add_frame_transaction_to_pool(custom_pooled(custom_tx, CUSTOM_WITNESS))
        .await
        .expect("the claim holds against the head");
    assert!(blockchain.mempool.contains_tx(hash).unwrap());

    // A block built on the genesis block has a pre-state without the verifier.
    let sibling = build_block_on(&store, &blockchain, 0).payload;
    assert!(sibling.body.transactions.is_empty());
    assert_eq!(sibling.body.claim_witnesses, Some(Bytes::new()));
    assert!(!blockchain.mempool.contains_tx(hash).unwrap());
    assert_eq!(blockchain.mempool.get_claim_witnesses(hash).unwrap(), None);
}

// A claim's gas is part of its transaction's intrinsic gas, which the block's gas
// limit bounds. A block's claims are verified before it executes, so that bound
// is checked first: a block whose claims take more gas than its limit is
// rejected without a verifier running for it.
#[tokio::test]
async fn a_block_whose_claims_take_more_gas_than_its_limit_is_rejected_before_they_run() {
    /// A verifier that never returns: `JUMPDEST; PUSH0; JUMP`.
    const LOOPING_VERIFIER: Address = H160([0x1F; 20]);
    let accounts = [
        (LOOPING_VERIFIER, funded(vec![0x5b, 0x5f, 0x56])),
        (
            CUSTOM_ACCOUNT,
            funded(claim_account_code(LOOPING_VERIFIER, CUSTOM_KEY)),
        ),
    ];
    let fresh_store = || async { setup_store_at(&[], &accounts, true).await.0 };
    let (store, chain_id) = setup_store_at(&[], &accounts, true).await;
    let blockchain = blockchain(store.clone());
    let built = build_block(&store, &blockchain);
    let empty = built.payload;
    let bal = Arc::new(built.block_access_list.expect("a Hegota block has a BAL"));
    let gas_limit = empty.header.gas_limit;

    // The empty block with one more transaction for each entry of `claim_gas`,
    // each with one claim of that gas on the looping verifier.
    let block_claiming = |claim_gas: &[u64]| {
        let mut block = empty.clone();
        for (nonce_seq, gas_limit) in claim_gas.iter().enumerate() {
            let mut tx = frame_transaction(
                chain_id,
                CUSTOM_ACCOUNT,
                vec![Claim {
                    gas_limit: *gas_limit,
                    ..custom_claim(LOOPING_VERIFIER)
                }],
            );
            tx.nonce_seq = nonce_seq as u64;
            block
                .body
                .transactions
                .push(Transaction::FrameTransaction(tx));
        }
        let dependencies = block_dependencies(&block.body.transactions).unwrap();
        with_witnesses(block, &vec![Bytes::from(vec![0u8; 32]); dependencies.len()])
    };

    assert_rejected_on_both_paths(
        block_claiming(&[gas_limit + 1]),
        &bal,
        fresh_store,
        "one claim over the block's gas limit",
        |error| {
            matches!(
                error,
                InvalidBlockError::ClaimGasExceedsGasLimit { claim_gas, gas_limit: limit }
                    if *claim_gas == gas_limit + 1 && *limit == gas_limit
            )
        },
    )
    .await;

    // The bound is on the block's claims together.
    let half = gas_limit / 2 + 1;
    assert_rejected_on_both_paths(
        block_claiming(&[half, half]),
        &bal,
        fresh_store,
        "two transactions whose claims together are over the block's gas limit",
        |error| {
            matches!(
                error,
                InvalidBlockError::ClaimGasExceedsGasLimit { claim_gas, .. }
                    if *claim_gas == 2 * half
            )
        },
    )
    .await;

    // At the limit the verifier runs, spends the claim's gas and fails.
    assert_rejected_on_both_paths(
        block_claiming(&[gas_limit]),
        &bal,
        fresh_store,
        "one claim at the block's gas limit",
        |error| matches!(error, InvalidBlockError::ClaimNotSatisfied(0)),
    )
    .await;
}

// A claim whose `key` or `msg` has a length the EIP does not define has no
// dependency. Its transaction is invalid, so a block that includes it is
// rejected by the claim check, before anything executes.
#[tokio::test]
async fn a_block_with_a_malformed_claim_is_rejected() {
    let signer = key(0x71);
    let fresh_store = || async { setup_store(&[&signer]).await.0 };
    let (store, chain_id) = setup_store(&[&signer]).await;
    let blockchain = blockchain(store.clone());
    let built = build_block(&store, &blockchain);
    let empty = built.payload;
    let bal = Arc::new(built.block_access_list.expect("a Hegota block has a BAL"));

    for (what, key, msg) in [
        ("a 5-byte key", Bytes::from(vec![1u8; 5]), Bytes::new()),
        ("a 31-byte msg", Bytes::new(), Bytes::from(vec![1u8; 31])),
    ] {
        let mut block = empty.clone();
        let tx = frame_transaction(
            chain_id,
            signer.account,
            vec![Claim {
                verifier: LEANSPHINCS_VERIFIER,
                key,
                msg,
                gas_limit: 0,
                witness_len: 0,
            }],
        );
        block
            .body
            .transactions
            .push(Transaction::FrameTransaction(tx));
        assert_rejected_on_both_paths(
            with_witnesses(block, &[Bytes::from(vec![0u8; 32])]),
            &bal,
            fresh_store,
            what,
            |error| matches!(error, InvalidBlockError::MalformedClaim),
        )
        .await;
    }
}

// A block carries at most MAX_BLOCK_WITNESS_BYTES of claim witnesses: the builder
// leaves out the transaction whose witnesses would take it over the limit.
#[tokio::test]
async fn the_builder_keeps_the_claim_witnesses_within_the_block_limit() {
    // Sixty-four leanSPHINCS claims a transaction: just under half a megabyte of
    // witnesses, so four transactions fit a block and the fifth does not.
    const CLAIMS: usize = 64;
    const SENDERS: u8 = 5;
    let keys: Vec<Key> = (0..SENDERS).map(|i| key(0x70 + i)).collect();
    let key_refs: Vec<&Key> = keys.iter().collect();
    let (store, chain_id) = setup_store(&key_refs).await;
    // The public mempool's validation budget admits seven leanSPHINCS claims a
    // transaction; this node is told to verify more.
    let blockchain = Blockchain::new(
        store.clone(),
        BlockchainOptions {
            min_tip_wei: 0,
            max_verify_gas: 2_000_000,
            ..Default::default()
        },
    );

    let mut pooled_hashes = Vec::new();
    for key in &keys {
        let more_claims = (1..CLAIMS)
            .map(|i| leansphincs_claim(key, Some(H256::from_low_u64_be(i as u64))))
            .collect();
        let pooled_tx = pooled(claim_transaction(chain_id, key, more_claims), &[key]);
        assert_eq!(pooled_tx.witnesses.len(), CLAIMS);
        pooled_hashes.push(
            blockchain
                .add_frame_transaction_to_pool(pooled_tx)
                .await
                .expect("the transaction enters the pool"),
        );
    }
    assert_eq!(
        blockchain.mempool.claim_witnesses_size().unwrap(),
        (
            usize::from(SENDERS),
            usize::from(SENDERS) * CLAIMS * LEANSPHINCS_SIGNATURE_SIZE
        )
    );

    let built = build_block(&store, &blockchain);
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 4);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));
    let claim_witnesses = block.body.claim_witnesses.clone().unwrap();
    assert!(claim_witnesses.len() <= MAX_BLOCK_WITNESS_BYTES);
    assert_eq!(
        decode_claim_witnesses(&claim_witnesses).unwrap().len(),
        4 * CLAIMS
    );
    // One more transaction's witnesses would not have fit.
    assert!(claim_witnesses.len() + CLAIMS * LEANSPHINCS_SIGNATURE_SIZE > MAX_BLOCK_WITNESS_BYTES);

    // The block imports, and the transaction left out stays pooled for the next.
    let bal = Arc::new(built.block_access_list.expect("a Hegota block has a BAL"));
    let fresh_store = || async { setup_store(&key_refs).await.0 };
    for result in import_on_both_paths(&block, &bal, fresh_store).await {
        result.expect("the block imports");
    }
    let included: Vec<H256> = block
        .body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();
    let left_out: Vec<&H256> = pooled_hashes
        .iter()
        .filter(|hash| !included.contains(hash))
        .collect();
    assert_eq!(left_out.len(), 1);
    assert!(blockchain.mempool.contains_tx(*left_out[0]).unwrap());
}

// ==================== The default code ====================

// An account without code sends a frame transaction on a secp256k1 claim for its
// own address: the witness is its signature of the transaction's signature hash.
#[tokio::test]
async fn an_account_without_code_sends_a_frame_transaction_on_its_secp256k1_claim() {
    let signing_key = k256::ecdsa::SigningKey::from_bytes(&[0x42; 32].into()).unwrap();
    let uncompressed = signing_key.verifying_key().to_encoded_point(false);
    let public_key_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
    let sender = Address::from_slice(&public_key_hash[12..]);
    let accounts = [(sender, funded(Vec::new()))];
    let fresh_store = || async { setup_store_at(&[], &accounts, true).await.0 };
    let (store, chain_id) = setup_store_at(&[], &accounts, true).await;
    let blockchain = blockchain(store.clone());

    let tx = frame_transaction(
        chain_id,
        sender,
        vec![Claim {
            verifier: SECP256K1_VERIFIER,
            ..Default::default()
        }],
    );
    let sign_secp256k1 = |key: &k256::ecdsa::SigningKey, msg: H256| {
        let (signature, recovery_id) = key.sign_prehash_recoverable(msg.as_bytes()).unwrap();
        let mut witness = vec![recovery_id.to_byte()];
        witness.extend_from_slice(&signature.to_bytes());
        Bytes::from(witness)
    };
    let witness = sign_secp256k1(&signing_key, tx.compute_sig_hash());
    let pooled_tx = |witness: Bytes| PooledFrameTransaction {
        tx: tx.clone(),
        witnesses: vec![witness],
        blobs_bundle: None,
    };

    // Another key's signature does not satisfy the account's claim.
    let other_key = k256::ecdsa::SigningKey::from_bytes(&[0x43; 32].into()).unwrap();
    let result = blockchain
        .add_frame_transaction_to_pool(pooled_tx(sign_secp256k1(&other_key, tx.compute_sig_hash())))
        .await;
    assert!(
        matches!(result, Err(MempoolError::ClaimNotSatisfied(0))),
        "got {result:?}"
    );

    blockchain
        .add_frame_transaction_to_pool(pooled_tx(witness.clone()))
        .await
        .expect("the default code approves on the account's claim");
    let built = build_block(&store, &blockchain);
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 1);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));
    assert_eq!(
        block.body.claim_witnesses,
        Some(encode_claim_witnesses(std::slice::from_ref(&witness)))
    );
    let bal = Arc::new(built.block_access_list.expect("a Hegota block has a BAL"));
    for result in import_on_both_paths(&block, &bal, fresh_store).await {
        result.expect("the block imports");
    }
}

// ==================== Stateless execution ====================

// The witness a node collects while it imports a block is enough to check the
// block's claims again without the state: it holds the code of every custom
// verifier. Stateless execution repeats the check and refuses a block whose
// witnesses do not satisfy its claims.
#[tokio::test]
async fn stateless_execution_checks_the_claim_witnesses_of_a_block() {
    let key = key(0x65);
    let verifier = H160([0x7E; 20]);
    let accounts = [
        (
            verifier,
            GenesisAccount {
                balance: U256::zero(),
                ..funded(custom_verifier_code())
            },
        ),
        (
            CUSTOM_ACCOUNT,
            funded(claim_account_code(verifier, CUSTOM_KEY)),
        ),
    ];
    let (store, chain_id) = setup_store_at(&[&key], &accounts, true).await;
    let blockchain = blockchain(store.clone());

    blockchain
        .add_frame_transaction_to_pool(pooled(
            claim_transaction(chain_id, &key, Vec::new()),
            &[&key],
        ))
        .await
        .expect("the leanSPHINCS transaction enters the pool");
    let custom_tx = frame_transaction(chain_id, CUSTOM_ACCOUNT, vec![custom_claim(verifier)]);
    blockchain
        .add_frame_transaction_to_pool(custom_pooled(custom_tx, CUSTOM_WITNESS))
        .await
        .expect("the custom claim holds");
    let built = build_block(&store, &blockchain);
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 2);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));

    let (other_store, _) = setup_store_at(&[&key], &accounts, true).await;
    let execution_witness = self::blockchain(other_store)
        .add_block_pipeline_with_witness(block.clone(), None)
        .expect("import the block and collect its witness");
    ethrex_guest_program::l1::validate_blocks_statelessly(
        std::slice::from_ref(&block),
        execution_witness.clone(),
        Arc::new(NativeCrypto),
    )
    .expect("the block's claims are verified statelessly");

    // The same block with a witness its custom verifier rejects.
    let dependencies = block_dependencies(&block.body.transactions).unwrap();
    let mut witnesses = decode_claim_witnesses(block.body.claim_witnesses.as_ref().unwrap())
        .expect("the claim witnesses decode");
    let custom_index = dependencies
        .iter()
        .position(|dependency| dependency.verifier == verifier)
        .expect("the custom dependency");
    witnesses[custom_index] = Bytes::from(vec![0xD8; 32]);
    let invalid = with_witnesses(block, &witnesses);
    let error = ethrex_guest_program::l1::validate_blocks_statelessly(
        &[invalid],
        execution_witness,
        Arc::new(NativeCrypto),
    )
    .expect_err("stateless execution rejects the block");
    assert!(
        error
            .to_string()
            .contains(&format!("Claim witness {custom_index} does not satisfy")),
        "got {error}"
    );
}

// A build of stateless execution without the leanSPHINCS verifier cannot check a
// leanSPHINCS claim. It refuses the block rather than execute frames that would
// rely on a signature nobody checked, and still verifies every other claim.
#[tokio::test]
async fn stateless_execution_refuses_leansphincs_claims_it_cannot_verify() {
    use ethrex_blockchain::vm::StoreVmDatabase;
    use ethrex_guest_program::common::{ExecutionError, verify_claim_witnesses};
    use ethrex_vm::Evm;

    let key = key(0x66);
    let (store, chain_id) = setup_store(&[&key]).await;
    let blockchain = blockchain(store.clone());
    blockchain
        .add_frame_transaction_to_pool(pooled(
            claim_transaction(chain_id, &key, Vec::new()),
            &[&key],
        ))
        .await
        .expect("the transaction enters the pool");
    let block = build_block(&store, &blockchain).payload;
    assert_eq!(block.body.transactions.len(), 1);

    let parent = store.get_block_header(0).unwrap().unwrap();
    let evm = Evm::new_for_l1(
        StoreVmDatabase::new(store.clone(), parent).unwrap(),
        Arc::new(NativeCrypto),
    );
    // This build links the verifier, so the claim is checked and holds.
    const { assert!(ethrex_vm::LEANSPHINCS_VERIFIER_LINKED) };
    verify_claim_witnesses(&block, &evm, true).expect("the claim is verified");
    // A build that does not link it refuses the block.
    let result = verify_claim_witnesses(&block, &evm, false);
    assert!(
        matches!(result, Err(ExecutionError::UnverifiedClaimWitnesses(1))),
        "got {result:?}"
    );

    // A block without leanSPHINCS claims is checked the same either way.
    let empty = with_witnesses(block, &[]);
    let empty = Block::new(
        empty.header,
        ethrex_common::types::BlockBody {
            transactions: Vec::new(),
            ..empty.body
        },
    );
    verify_claim_witnesses(&empty, &evm, false).expect("no claims, nothing to verify");
}
