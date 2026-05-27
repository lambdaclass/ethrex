use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{GenesisAccount},
};
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_rpc::rpc::map_http_requests;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::utils::RpcRequest;
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use serde_json::{Value, json};

const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";

fn test_secret_key() -> SecretKey {
    SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn sender_from_key(sk: &SecretKey) -> Address {
    LocalSigner::new(*sk).address
}

async fn setup_store(sender: Address) -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
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
    store
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

#[tokio::test]
async fn preimage() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let store = setup_store(sender).await;

    let result = rpc_call(
        &store,
        "debug_preimage",
        vec![json!(format!("{:#x}", H256::zero()))],
    )
    .await;

    // ethrex does not maintain a preimage store, so always returns null.
    assert!(result.is_null(), "should return null");
}
