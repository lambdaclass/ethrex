//! Shared setup helpers for the rpc integration tests. Each test file builds
//! its own in-memory `Store`, funds a known sender, and uses [`rpc_call`] to
//! drive the dispatcher just like a live request would. Individual test files
//! only need a subset of the helpers, so the module is permissive about dead
//! code rather than gating each item separately.
#![allow(dead_code)]

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    evm::calculate_create_address,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER,
        GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_rpc::rpc::map_http_requests;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::utils::{RpcErr, RpcRequest};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use serde_json::{Value, json};

pub const TEST_PRIVATE_KEY: &str =
    "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
pub const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
pub const TEST_GAS_LIMIT: u64 = 100_000;

pub fn test_secret_key() -> SecretKey {
    SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn sender_from_key(sk: &SecretKey) -> Address {
    LocalSigner::new(*sk).address
}

pub async fn setup_store(sender: Address) -> (Store, u64) {
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

pub async fn build_block(
    store: &Store,
    blockchain: &Blockchain,
    parent_header: &BlockHeader,
) -> Block {
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

pub async fn create_transfer_tx(
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

pub async fn create_deploy_tx(
    chain_id: u64,
    nonce: u64,
    init_code: Bytes,
    signer: &Signer,
) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: 1_000_000,
        to: TxKind::Create,
        value: U256::zero(),
        data: init_code,
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

pub async fn build_and_execute_block(
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

pub async fn rpc_call(store: &Store, method: &str, params: Vec<Value>) -> Value {
    let request = build_rpc_request(method, params);
    let context = default_context_with_storage(store.clone()).await;
    map_http_requests(&request, context)
        .await
        .expect("RPC call should succeed")
}

pub async fn rpc_call_expect_err(store: &Store, method: &str, params: Vec<Value>) -> RpcErr {
    let request = build_rpc_request(method, params);
    let context = default_context_with_storage(store.clone()).await;
    map_http_requests(&request, context)
        .await
        .expect_err("RPC call should fail")
}

fn build_rpc_request(method: &str, params: Vec<Value>) -> RpcRequest {
    let body = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    serde_json::from_value(body).expect("valid RPC request")
}

pub struct TestEnv {
    pub store: Store,
    pub block: Block,
    pub tx_hash: H256,
    pub sender: Address,
}

pub async fn setup_single_transfer_block() -> TestEnv {
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
        sender,
    }
}

pub struct DeployedEnv {
    pub store: Store,
    pub block: Block,
    pub sender: Address,
    /// Address of the contract deployed by the single tx in `block`.
    pub contract: Address,
}

/// Deploys a tiny contract whose constructor writes three storage slots:
/// slot 0 = 0x11, slot 1 = 0x22, slot 2 = 0x33. Used by storage-range tests
/// to verify both content and pagination against real trie data.
pub async fn setup_block_with_storage_contract() -> DeployedEnv {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();
    let (store, chain_id) = setup_store(sender).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    // PUSH1 0x11 PUSH1 0x00 SSTORE   ; slot 0 = 0x11
    // PUSH1 0x22 PUSH1 0x01 SSTORE   ; slot 1 = 0x22
    // PUSH1 0x33 PUSH1 0x02 SSTORE   ; slot 2 = 0x33
    // PUSH1 0x00 PUSH1 0x00 RETURN   ; deploy empty runtime
    let init_code = Bytes::from_static(&[
        0x60, 0x11, 0x60, 0x00, 0x55, 0x60, 0x22, 0x60, 0x01, 0x55, 0x60, 0x33, 0x60, 0x02, 0x55,
        0x60, 0x00, 0x60, 0x00, 0xF3,
    ]);
    let tx = create_deploy_tx(chain_id, 0, init_code, &signer).await;
    let block = build_and_execute_block(&store, &blockchain, &genesis_header, vec![tx]).await;
    let contract = calculate_create_address(sender, 0);
    DeployedEnv {
        store,
        block,
        sender,
        contract,
    }
}
