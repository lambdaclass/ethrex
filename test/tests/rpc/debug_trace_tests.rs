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
    let tx_hash = tx.hash();
    let block = build_and_execute_block(&store, &blockchain, &genesis_header, vec![tx]).await;
    TestEnv {
        store,
        block,
        tx_hash,
    }
}

#[tokio::test]
async fn trace_tx_flat_call_tracer() {
    let env = setup_single_transfer_block().await;

    let result = rpc_call(
        &env.store,
        "debug_traceTransaction",
        vec![
            json!(format!("{:#x}", env.tx_hash)),
            json!({"tracer": "flatCallTracer"}),
        ],
    )
    .await;

    // flatCallTracer returns a flat array of call frames.
    let arr = result.as_array().expect("response should be an array");
    assert!(!arr.is_empty(), "should have at least one frame");

    let frame = arr[0].as_object().expect("frame should be an object");
    assert!(frame.contains_key("action"), "frame should have 'action'");
    assert!(
        frame.contains_key("subtraces"),
        "frame should have 'subtraces'"
    );
    assert!(
        frame.contains_key("traceAddress"),
        "frame should have 'traceAddress'"
    );
    assert!(frame.contains_key("type"), "frame should have 'type'");
    assert_eq!(frame["type"].as_str().unwrap(), "call");
}
