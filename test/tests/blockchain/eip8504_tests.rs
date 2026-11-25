//! EIP-8504 dependency transactions from submission to import.
//!
//! An account controlled by a leanSPHINCS key sends `[VERIFY, SENDER]`
//! transactions whose signature list declares the dependency. The signature travels beside the transaction: it is submitted
//! with it, kept with it in the pool, placed in the block the transaction is
//! included in, and checked by every node that imports that block.

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
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Frame, FrameMode, FrameTransaction,
        Genesis, GenesisAccount, LEANSPHINCS_SIGNATURE_SIZE, ProofWrapper, Transaction,
        block_dependencies, compute_dependency_witnesses_hash,
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

use crate::levm::eip8504_tests::{account_code, dependency_entries};

const RECIPIENT: Address = H160([0x86; 20]);
const GWEI: u64 = 1_000_000_000;

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

/// `l1-hegota.json` (every fork through Hegota active at genesis, and EIP-8504
/// with it) with each key's account funded and running the validation code for
/// that key. With `hegota` unset the chain stops at the fork before it.
async fn setup_store_at(keys: &[&Key], hegota: bool) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    if !hegota {
        genesis.config.hegota_time = None;
    }
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

async fn setup_store(keys: &[&Key]) -> (Store, u64) {
    setup_store_at(keys, true).await
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

/// `[VERIFY, SENDER]` from the key's account: its validation frame and a call to
/// `RECIPIENT`. The signature list declares a signature of the transaction's own
/// signature hash under the key, followed by `more_declarations`.
fn dependency_transaction(
    chain_id: u64,
    key: &Key,
    more_declarations: &[(H256, H256)],
) -> FrameTransaction {
    let mut declarations = vec![(H256::zero(), key.public)];
    declarations.extend_from_slice(more_declarations);
    // The validation frame points at entry 0 of the signature list.
    let verify_data = [0u8; 32];
    let mut tx = FrameTransaction {
        chain_id,
        nonce_keys: vec![],
        nonce_seq: 0,
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

fn build_block(store: &Store, blockchain: &Blockchain) -> PayloadBuildResult {
    let parent = store.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(payload).unwrap()
}

/// The block with its witness list replaced and the header committing to the new
/// list, so only the witnesses themselves can make it invalid.
fn with_witnesses(mut block: Block, witnesses: Vec<Bytes>) -> Block {
    block.header.dependency_witnesses_hash =
        Some(compute_dependency_witnesses_hash(&witnesses, &NativeCrypto));
    block.body.dependency_witnesses = Some(witnesses);
    block
}

#[tokio::test]
async fn dependency_transaction_goes_from_the_pool_into_a_block_that_imports() {
    let key = key(0x51);
    let (store, chain_id) = setup_store(&[&key]).await;
    let blockchain = blockchain(store.clone());

    let wrapper = wrapper(dependency_transaction(chain_id, &key, &[]), &[&key]);
    let witness = wrapper.witnesses[0].clone();
    assert_eq!(witness.len(), LEANSPHINCS_SIGNATURE_SIZE);
    let tx_hash = blockchain
        .add_dependency_transaction_to_pool(wrapper.clone())
        .await
        .expect("the dependency transaction enters the pool with its witness");
    assert_eq!(
        tx_hash,
        Transaction::FrameTransaction(wrapper.tx.clone()).hash(&NativeCrypto)
    );

    // The pool keeps the witness with the transaction, and offers the two to
    // peers only as the wrapper.
    assert_eq!(
        blockchain
            .mempool
            .get_dependency_witnesses(tx_hash)
            .unwrap(),
        Some(vec![witness.clone()])
    );
    assert!(
        blockchain
            .mempool
            .get_txs_for_broadcast()
            .unwrap()
            .is_empty()
    );
    assert!(blockchain.get_p2p_transaction_by_hash(&tx_hash).is_err());
    let announced = blockchain
        .mempool
        .get_proof_wrappers_for_broadcast()
        .unwrap();
    assert_eq!(announced.len(), 1);
    assert_eq!(announced[0].tx_hash, tx_hash);
    assert_eq!(announced[0].wrapper_hash, wrapper.hash(&NativeCrypto));
    assert_eq!(announced[0].wrapper_len, wrapper.encode_to_vec().len());
    // The pool serves the wrapper it announced, byte for byte.
    assert_eq!(
        blockchain
            .mempool
            .get_proof_wrapper(announced[0].wrapper_hash)
            .unwrap()
            .map(|served| served.encode_to_vec()),
        Some(wrapper.encode_to_vec())
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
    assert_eq!(block.body.dependency_witnesses, Some(vec![witness.clone()]));
    assert_eq!(
        block.header.dependency_witnesses_hash,
        Some(compute_dependency_witnesses_hash(
            std::slice::from_ref(&witness),
            &NativeCrypto
        ))
    );

    // It imports on every path.
    let bal = Arc::new(built.block_access_list.expect("a Hegota block has a BAL"));
    let (other_store, _) = setup_store(&[&key]).await;
    self::blockchain(other_store)
        .add_block_pipeline(block.clone(), Some(bal.clone()))
        .expect("pipeline import");
    let (other_store, _) = setup_store(&[&key]).await;
    self::blockchain(other_store)
        .add_block(block.clone())
        .expect("plain import");

    // A block whose witness is not a signature of the dependency is rejected, on
    // every path, before anything executes.
    let mut tampered = witness.to_vec();
    tampered[100] ^= 0x01;
    let tampered_block = with_witnesses(block.clone(), vec![Bytes::from(tampered)]);
    let (other_store, _) = setup_store(&[&key]).await;
    let result = self::blockchain(other_store).add_block(tampered_block.clone());
    assert!(
        matches!(
            result,
            Err(ChainError::InvalidBlock(
                InvalidBlockError::DependencyNotSatisfied(0)
            ))
        ),
        "got {result:?}"
    );
    let (other_store, _) = setup_store(&[&key]).await;
    let result = self::blockchain(other_store).add_block_pipeline(tampered_block, Some(bal));
    assert!(
        matches!(
            result,
            Err(ChainError::InvalidBlock(
                InvalidBlockError::DependencyNotSatisfied(0)
            ))
        ),
        "got {result:?}"
    );

    // So is a block that leaves the witness out.
    let (other_store, _) = setup_store(&[&key]).await;
    let result = self::blockchain(other_store).add_block(with_witnesses(block, Vec::new()));
    assert!(
        matches!(
            result,
            Err(ChainError::InvalidBlock(
                InvalidBlockError::DependencyWitnessesCountMismatch {
                    expected: 1,
                    found: 0
                }
            ))
        ),
        "got {result:?}"
    );
}

#[tokio::test]
async fn block_carries_one_witness_for_a_dependency_two_transactions_share() {
    let first = key(0x52);
    let second = key(0x53);
    let shared = key(0x54);
    let (store, chain_id) = setup_store(&[&first, &second]).await;
    let blockchain = blockchain(store.clone());

    // Both transactions also depend on a signature of the same message under a
    // third key.
    let message = H256::repeat_byte(0x77);
    let mut all_witnesses = Vec::new();
    for sender in [&first, &second] {
        let tx = dependency_transaction(chain_id, sender, &[(message, shared.public)]);
        let wrapper = wrapper(tx, &[sender, &shared]);
        assert_eq!(wrapper.witnesses.len(), 2);
        all_witnesses.extend(
            wrapper
                .tx
                .dependencies()
                .into_iter()
                .zip(wrapper.witnesses.clone()),
        );
        blockchain
            .add_dependency_transaction_to_pool(wrapper)
            .await
            .expect("the dependency transaction enters the pool");
    }

    let built = build_block(&store, &blockchain);
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 2);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));

    // Three distinct dependencies, in the order of `block_dependencies`, each
    // with a witness one of the transactions was submitted with.
    let dependencies = block_dependencies(&block.body.transactions);
    assert_eq!(dependencies.len(), 3);
    let witnesses = block
        .body
        .dependency_witnesses
        .clone()
        .expect("the block has a witness list");
    assert_eq!(witnesses.len(), 3);
    for (dependency, witness) in dependencies.iter().zip(&witnesses) {
        assert!(
            all_witnesses.contains(&(*dependency, witness.clone())),
            "the witness of {dependency:?} comes from the pool"
        );
    }

    let (other_store, _) = setup_store(&[&first, &second]).await;
    self::blockchain(other_store)
        .add_block(block)
        .expect("the block imports");
}

#[tokio::test]
async fn pool_admits_a_dependency_transaction_only_with_valid_witnesses() {
    let key = key(0x55);
    let other_key = self::key(0x56);
    let (store, chain_id) = setup_store(&[&key]).await;
    let blockchain = blockchain(store.clone());
    let tx = dependency_transaction(chain_id, &key, &[]);

    // Alone, without its witnesses.
    let result = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
        .await;
    assert!(
        matches!(
            result,
            Err(MempoolError::DependencyTransactionWithoutWitnesses)
        ),
        "got {result:?}"
    );

    // With a signature made with another key.
    let forged = ProofWrapper::new(
        tx.clone(),
        None,
        vec![sign(&other_key, tx.compute_sig_hash())],
    );
    let result = blockchain.add_dependency_transaction_to_pool(forged).await;
    assert!(
        matches!(result, Err(MempoolError::DependencyNotSatisfied(0))),
        "got {result:?}"
    );

    // With a wrapper that states other dependencies than the transaction's.
    let mut misstated = wrapper(tx.clone(), &[&key]);
    misstated.deps[0] = Bytes::from(vec![0u8; 96]);
    let result = blockchain
        .add_dependency_transaction_to_pool(misstated)
        .await;
    assert!(
        matches!(result, Err(MempoolError::WrapperDependenciesMismatch)),
        "got {result:?}"
    );

    // With fewer witnesses than dependencies.
    let mut short = wrapper(tx.clone(), &[&key]);
    short.witnesses.clear();
    let result = blockchain.add_dependency_transaction_to_pool(short).await;
    assert!(
        matches!(result, Err(MempoolError::WrapperDependenciesMismatch)),
        "got {result:?}"
    );

    // A transaction that declares no dependency has nothing for witnesses to
    // satisfy.
    let mut plain = tx.clone();
    plain.signatures.clear();
    let result = blockchain
        .add_dependency_transaction_to_pool(ProofWrapper::new(plain, None, Vec::new()))
        .await;
    assert!(
        matches!(result, Err(MempoolError::WitnessesWithoutDependencies)),
        "got {result:?}"
    );

    assert_eq!(
        blockchain.mempool.dependency_witnesses_size().unwrap(),
        (0, 0)
    );

    // With its witness it is admitted, and a later wrapper for the same
    // transaction is ignored: the pool keeps the first one.
    let valid = wrapper(tx.clone(), &[&key]);
    let tx_hash = blockchain
        .add_dependency_transaction_to_pool(valid.clone())
        .await
        .expect("admitted");
    let later = ProofWrapper::new(
        tx.clone(),
        None,
        vec![sign(&other_key, tx.compute_sig_hash())],
    );
    assert_eq!(
        blockchain
            .add_dependency_transaction_to_pool(later)
            .await
            .expect("ignored"),
        tx_hash
    );
    assert_eq!(
        blockchain
            .mempool
            .get_dependency_witnesses(tx_hash)
            .unwrap(),
        Some(valid.witnesses.clone())
    );
    assert_eq!(
        blockchain.mempool.dependency_witnesses_size().unwrap(),
        (1, LEANSPHINCS_SIGNATURE_SIZE)
    );

    // The witnesses leave the pool with the transaction.
    blockchain.remove_transaction_from_pool(&tx_hash).unwrap();
    assert_eq!(
        blockchain.mempool.dependency_witnesses_size().unwrap(),
        (0, 0)
    );
    assert!(
        !blockchain
            .mempool
            .contains_proof_wrapper(valid.hash(&NativeCrypto))
            .unwrap()
    );
}

#[tokio::test]
async fn pool_limits_the_dependencies_of_a_transaction_to_max_sigs_per_tx() {
    let key = key(0x57);
    let second_key = self::key(0x58);
    let (store, chain_id) = setup_store(&[&key]).await;
    let tx = dependency_transaction(
        chain_id,
        &key,
        &[(H256::repeat_byte(0x01), second_key.public)],
    );
    let wrapper = wrapper(tx, &[&key, &second_key]);

    let strict = Blockchain::new(
        store.clone(),
        BlockchainOptions {
            min_tip_wei: 0,
            max_sigs_per_tx: 1,
            ..Default::default()
        },
    );
    let result = strict
        .add_dependency_transaction_to_pool(wrapper.clone())
        .await;
    assert!(
        matches!(
            result,
            Err(MempoolError::TooManyDependencies {
                actual: 2,
                limit: 1
            })
        ),
        "got {result:?}"
    );

    // The default limit admits it.
    blockchain(store)
        .add_dependency_transaction_to_pool(wrapper)
        .await
        .expect("two dependencies are within the default limit");
}

#[tokio::test]
async fn pool_rejects_a_dependency_transaction_before_hegota() {
    let key = key(0x59);
    let (store, chain_id) = setup_store_at(&[&key], false).await;
    let blockchain = blockchain(store);
    let wrapper = wrapper(dependency_transaction(chain_id, &key, &[]), &[&key]);

    let result = blockchain.add_dependency_transaction_to_pool(wrapper).await;
    assert!(
        matches!(result, Err(MempoolError::FrameTxPreFork)),
        "got {result:?}"
    );
}

#[tokio::test]
async fn dependency_transaction_is_submitted_with_its_witnesses_through_the_rpc() {
    let key = key(0x5a);
    let (store, chain_id) = setup_store(&[&key]).await;
    let ctx = default_context_with_storage(store).await;
    let wrapper = wrapper(dependency_transaction(chain_id, &key, &[]), &[&key]);

    // FRAME_TX_TYPE || rlp([tx_payload_body, witnesses])
    let mut raw = Vec::new();
    wrapper.encode_submission(&mut raw);
    let returned_hash =
        SendRawTransactionRequest::parse(&Some(vec![json!(format!("0x{}", hex::encode(&raw)))]))
            .expect("the submitted form decodes")
            .handle(ctx.clone())
            .await
            .expect("the dependency transaction enters the pool");
    let tx_hash = Transaction::FrameTransaction(wrapper.tx.clone()).hash(&NativeCrypto);
    assert_eq!(
        returned_hash,
        json!(format!("{tx_hash:#x}")),
        "the transaction hash covers the transaction alone, not its witnesses"
    );
    assert_eq!(
        ctx.blockchain
            .mempool
            .get_dependency_witnesses(tx_hash)
            .unwrap(),
        Some(wrapper.witnesses.clone())
    );

    // The same transaction submitted alone is rejected.
    let key = self::key(0x5b);
    let (store, chain_id) = setup_store(&[&key]).await;
    let ctx = default_context_with_storage(store).await;
    let bare = Transaction::FrameTransaction(dependency_transaction(chain_id, &key, &[]))
        .encode_canonical_to_vec();
    let result =
        SendRawTransactionRequest::parse(&Some(vec![json!(format!("0x{}", hex::encode(&bare)))]))
            .expect("a frame transaction decodes")
            .handle(ctx.clone())
            .await;
    assert!(result.is_err(), "got {result:?}");
}
