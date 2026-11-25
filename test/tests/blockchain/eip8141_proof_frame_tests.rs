//! EIP-8141 proof frames from submission to import.
//!
//! An account controlled by a leanSPHINCS key sends `[PROOF, VERIFY, SENDER]`
//! transactions. The signature travels inside the transaction, as the proof of
//! its proof frame: it is submitted with it like any raw transaction, pooled and
//! gossiped as part of it, included in the block's transaction list, and checked
//! by every node that executes that block.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::MempoolError,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, FRAME_TX_LEANSPHINCS_VERIFIER, Frame,
        FrameMode, FrameTransaction, Genesis, GenesisAccount, LEANSPHINCS_VERIFIER_GAS,
        Transaction,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rpc::{
    rpc::RpcHandler, test_utils::default_context_with_storage,
    types::transaction::SendRawTransactionRequest,
};
use ethrex_storage::{EngineType, Store};
use serde_json::json;

const RECIPIENT: Address = H160([0x86; 20]);
const GWEI: u64 = 1_000_000_000;
/// Length in bytes of a leanSPHINCS signature.
const SIGNATURE_SIZE: usize = 7740;

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

/// Validation code of an account controlled by the leanSPHINCS key `public_key`:
/// approve execution and payment if frame 0 claims that key, through the
/// leanSPHINCS verifier, for the canonical signature hash. The protocol verified
/// the claim before this code runs, so reading it is enough.
fn account_code(public_key: H256) -> Vec<u8> {
    let mut code = vec![
        0x60, 0x01, 0x60, 0x00, 0xB4, // PROOFPARAM(0x01, 0): verifier
        0x60, 0x14, 0x14, // == LEANSPHINCS_VERIFIER
        0x60, 0x00, 0x60, 0x00, 0xB4, // PROOFPARAM(0x00, 0): key
        0x7F, // PUSH32 public_key
    ];
    code.extend_from_slice(public_key.as_bytes());
    code.extend_from_slice(&[
        0x14, 0x16, // EQ; AND
        0x60, 0x02, 0x60, 0x00, 0xB4, // PROOFPARAM(0x02, 0): msg
        0x15, 0x16, // ISZERO; AND
    ]);
    let approve = u8::try_from(code.len() + 8).expect("the code is shorter than 256 bytes");
    code.extend_from_slice(&[
        0x60, approve, 0x57, // PUSH1 approve; JUMPI
        0x60, 0x00, 0x60, 0x00, 0xFD, // REVERT(0, 0)
        0x5B, // JUMPDEST (approve)
        0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA, // APPROVE(0, 0, 3)
    ]);
    code
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `l1-hegota.json` (every fork through Hegota active at genesis) with each key's
/// account funded and running the validation code for that key.
async fn setup_store(keys: &[&Key]) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
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

/// `[PROOF, VERIFY, SENDER]` from the key's account, signed by `signer`: a proof
/// frame claiming the account's key for the canonical signature hash, the
/// account's validation frame, and a call to `RECIPIENT`.
fn proof_transaction(chain_id: u64, key: &Key, signer: &Key) -> FrameTransaction {
    let mut claim = key.public.as_bytes().to_vec();
    claim.extend_from_slice(&[0u8; 32]);
    let mut placeholder = claim.clone();
    placeholder.extend_from_slice(&[0u8; SIGNATURE_SIZE]);
    let mut tx = FrameTransaction {
        chain_id,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: key.account,
        frames: vec![
            Frame {
                mode: u8::from(FrameMode::Proof),
                flags: 0,
                target: Some(FRAME_TX_LEANSPHINCS_VERIFIER),
                // The verifier's cost plus the access of a warm precompile.
                gas_limit: LEANSPHINCS_VERIFIER_GAS + 100,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::from(placeholder),
            },
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0x03,
                target: Some(key.account),
                gas_limit: 50_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
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
        max_priority_fee_per_gas: U256::from(GWEI),
        blob_versioned_hashes: Vec::new(),
        ..Default::default()
    };
    tx.max_fee = U256::from(10 * GWEI) * U256::from(tx.max_gas());
    // The proof is elided from the signature hash, so it is signed last.
    let sig_hash = tx.compute_sig_hash();
    let mut data = claim;
    data.extend_from_slice(
        &lean_sphincs::sign(&signer.secret, sig_hash.as_fixed_bytes()).to_bytes(),
    );
    tx.frames[0].data = Bytes::from(data);
    tx
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

#[tokio::test]
async fn proof_frame_transaction_goes_from_the_pool_into_a_block_that_imports() {
    let key = key(0x51);
    let (store, chain_id) = setup_store(&[&key]).await;
    let blockchain = blockchain(store.clone());

    let frame_tx = proof_transaction(chain_id, &key, &key);
    let tx = Transaction::FrameTransaction(frame_tx.clone());
    // The transaction is its own pooled form: nothing travels beside it.
    assert!(tx.encode_canonical_to_vec().len() > SIGNATURE_SIZE);
    let tx_hash = blockchain
        .add_transaction_to_pool(tx.clone())
        .await
        .expect("the transaction enters the pool on its claim");
    assert_eq!(tx_hash, tx.hash(&NativeCrypto));
    assert!(blockchain.get_p2p_transaction_by_hash(&tx_hash).is_ok());

    // The block includes the transaction, proof and all, and nothing else.
    let built = build_block(&store, &blockchain);
    let block = built.payload;
    assert_eq!(block.body.transactions, vec![tx]);
    assert!(
        built.receipts.iter().all(|receipt| receipt.succeeded),
        "the transaction must succeed: {:?}",
        built.receipts
    );
    // The signature is priced as transaction data, at the calldata floor.
    assert_eq!(block.header.gas_used, frame_tx.calldata_floor_total());

    // It imports on every path.
    let bal = Arc::new(built.block_access_list.expect("a Hegota block has a BAL"));
    let (other_store, _) = setup_store(&[&key]).await;
    self::blockchain(other_store)
        .add_block_pipeline(block.clone(), Some(bal))
        .expect("pipeline import");
    let (other_store, _) = setup_store(&[&key]).await;
    self::blockchain(other_store)
        .add_block(block)
        .expect("plain import");
}

#[tokio::test]
async fn pool_admits_a_proof_frame_transaction_only_on_a_claim_that_holds() {
    let key = key(0x52);
    let other_key = self::key(0x53);
    let (store, chain_id) = setup_store(&[&key]).await;
    let blockchain = blockchain(store);

    // Signed by a key the claim does not name.
    let forged = Transaction::FrameTransaction(proof_transaction(chain_id, &key, &other_key));
    let result = blockchain.add_transaction_to_pool(forged).await;
    assert!(
        matches!(result, Err(MempoolError::InvalidFrameProof)),
        "got {result:?}"
    );

    // A proof frame that cannot pay for its verifier.
    let mut underfunded = proof_transaction(chain_id, &key, &key);
    underfunded.frames[0].gas_limit = LEANSPHINCS_VERIFIER_GAS + 99;
    let sig_hash = underfunded.compute_sig_hash();
    let mut data = underfunded.frames[0].proof_claim().to_vec();
    data.extend_from_slice(&lean_sphincs::sign(&key.secret, sig_hash.as_fixed_bytes()).to_bytes());
    underfunded.frames[0].data = Bytes::from(data);
    let result = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(underfunded))
        .await;
    assert!(
        matches!(result, Err(MempoolError::InvalidFrameProof)),
        "got {result:?}"
    );
}

#[tokio::test]
async fn proof_frame_transaction_is_submitted_like_any_raw_transaction() {
    let key = key(0x5a);
    let (store, chain_id) = setup_store(&[&key]).await;
    let ctx = default_context_with_storage(store).await;
    let tx = Transaction::FrameTransaction(proof_transaction(chain_id, &key, &key));

    let raw = tx.encode_canonical_to_vec();
    let returned_hash =
        SendRawTransactionRequest::parse(&Some(vec![json!(format!("0x{}", hex::encode(&raw)))]))
            .expect("a frame transaction decodes")
            .handle(ctx.clone())
            .await
            .expect("the transaction enters the pool");
    let tx_hash = tx.hash(&NativeCrypto);
    assert_eq!(
        returned_hash,
        json!(format!("{tx_hash:#x}")),
        "the transaction hash covers the proof, which is part of the transaction"
    );
}
