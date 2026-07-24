use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER,
        GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_rpc::rpc::map_http_requests;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::utils::RpcRequest;
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use serde_json::{Value, json};

const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 100_000;

fn test_secret_key() -> SecretKey {
    SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn sender_from_key(sk: &SecretKey) -> Address {
    LocalSigner::new(*sk).address
}

async fn setup_store(sender: Address) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    let chain_id = genesis.config.chain_id;
    genesis.alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10).pow(U256::from(20)),
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );
    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    (store, chain_id)
}

async fn build_block(store: &Store, blockchain: &Blockchain, parent_header: &BlockHeader) -> Block {
    let args = BuildPayloadArgs {
        parent: parent_header.hash(),
        timestamp: parent_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(block).unwrap();
    result.payload
}

async fn create_transfer_tx(
    chain_id: u64,
    nonce: u64,
    to: Address,
    value: U256,
    signer: &Signer,
) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to: TxKind::Call(to),
        value,
        data: Bytes::new(),
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

async fn build_and_execute_block(
    store: &Store,
    blockchain: &Blockchain,
    parent_header: &BlockHeader,
    transactions: Vec<Transaction>,
) -> Block {
    for tx in &transactions {
        blockchain
            .add_transaction_to_pool(tx.clone())
            .await
            .expect("tx should enter pool");
    }
    let block = build_block(store, blockchain, parent_header).await;
    assert_eq!(block.body.transactions.len(), transactions.len());
    blockchain
        .add_block(block.clone())
        .expect("block should be valid");
    store
        .forkchoice_update(vec![], block.header.number, block.hash(), None, None)
        .await
        .unwrap();
    block
}

async fn rpc_call(store: &Store, method: &str, params: Vec<Value>) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });
    let request: RpcRequest = serde_json::from_value(body).expect("valid RPC request");
    let context = default_context_with_storage(store.clone()).await;
    map_http_requests(&request, context)
        .await
        .expect("RPC call should succeed")
}

struct TestEnv {
    store: Store,
    block: Block,
    tx_hash: H256,
}

async fn setup_single_transfer_block() -> TestEnv {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();
    let (store, chain_id) = setup_store(sender).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let recipient = Address::from_low_u64_be(0xAA);
    let value = U256::from(1_000_000_000_000_000_000u64);
    let tx = create_transfer_tx(chain_id, 0, recipient, value, &signer).await;
    let tx_hash = tx.hash(&NativeCrypto);
    let block = build_and_execute_block(&store, &blockchain, &genesis_header, vec![tx]).await;
    TestEnv {
        store,
        block,
        tx_hash,
    }
}

/// Runtime bytecode `PUSH1 0x00 PUSH1 0x00 REVERT`: reverts on any call.
const REVERT_BYTECODE: [u8; 5] = [0x60, 0x00, 0x60, 0x00, 0xfd];

/// Builds a block with a single tx that calls a contract which always reverts.
/// The tx is genuinely executed and reverts; it is still included in the block.
async fn setup_reverting_call_block() -> TestEnv {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    let chain_id = genesis.config.chain_id;
    genesis.alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10).pow(U256::from(20)),
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );
    let contract = Address::from_low_u64_be(0xC0);
    genesis.alloc.insert(
        contract,
        GenesisAccount {
            balance: U256::zero(),
            code: Bytes::copy_from_slice(&REVERT_BYTECODE),
            storage: Default::default(),
            nonce: 1,
        },
    );
    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");

    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let tx = create_transfer_tx(chain_id, 0, contract, U256::zero(), &signer).await;
    let tx_hash = tx.hash(&NativeCrypto);
    let block = build_and_execute_block(&store, &blockchain, &genesis_header, vec![tx]).await;
    TestEnv {
        store,
        block,
        tx_hash,
    }
}

#[tokio::test]
async fn trace_tx_noop_tracer() {
    let env = setup_single_transfer_block().await;

    let result = rpc_call(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", env.tx_hash)),
            json!({"tracer": "noopTracer"}),
        ],
    )
    .await;

    // noopTracer returns an empty object {}.
    let obj = result.as_object().expect("response should be an object");
    assert!(obj.is_empty(), "noopTracer should return empty object");
}

#[tokio::test]
async fn trace_tx_noop_tracer_unknown_hash_errors() {
    let env = setup_single_transfer_block().await;

    let body = json!({
        "jsonrpc": "2.0",
        "method": "debug_traceTransaction",
        "params": [
            json!(format!("{:#x}", H256::from_low_u64_be(0xdeadbeef))),
            json!({"tracer": "noopTracer"}),
        ],
        "id": 1,
    });
    let request: RpcRequest = serde_json::from_value(body).expect("valid RPC request");
    let context = default_context_with_storage(env.store.clone()).await;
    let err = map_http_requests(&request, context)
        .await
        .expect_err("missing tx should error, not return {}");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Transaction not Found"),
        "expected tx-not-found error, got: {msg}"
    );
}

#[tokio::test]
async fn trace_block_noop_tracer() {
    let env = setup_single_transfer_block().await;

    let result = rpc_call(
        &env.store,
        "debug_traceBlockByNumber",
        vec![
            json!(format!("{:#x}", env.block.header.number)),
            json!({"tracer": "noopTracer"}),
        ],
    )
    .await;

    let arr = result.as_array().expect("response should be an array");
    assert_eq!(arr.len(), 1, "one tx in block");
    let entry = arr[0].as_object().expect("entry should be an object");
    assert_eq!(
        entry["txHash"].as_str().unwrap().to_lowercase(),
        format!("{:#x}", env.tx_hash).to_lowercase()
    );
    assert!(
        entry["result"]
            .as_object()
            .expect("result should be an object")
            .is_empty(),
        "noopTracer should return empty object per tx"
    );
}

/// geth's `noopTracer` returns `{}` regardless of how the tx ended. A reverting
/// tx must therefore trace as an empty object and must NOT surface as an RPC
/// error — guarding against `trace_tx_noop` propagating the tx outcome through
/// `vm.execute()?` (reverts are reported in the execution result, not as `Err`).
#[tokio::test]
async fn trace_tx_noop_tracer_reverting_tx() {
    let env = setup_reverting_call_block().await;

    let result = rpc_call(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", env.tx_hash)),
            json!({"tracer": "noopTracer"}),
        ],
    )
    .await;

    let obj = result
        .as_object()
        .expect("reverting tx should trace as {}, not error");
    assert!(
        obj.is_empty(),
        "noopTracer should return empty object even for a reverting tx"
    );
}
