use crate::{
    eth::gas_tip_estimator::GasTipEstimator,
    rpc::{NodeData, RpcApiContext, start_api, start_block_executor},
};
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    Address, Bloom, H256, H512, U256,
    constants::DEFAULT_REQUESTS_HASH,
    types::{
        Block, BlockBody, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, Genesis,
        LegacyTransaction, Transaction, TxKind,
    },
};
use ethrex_p2p::{
    network::P2PContext,
    peer_handler::PeerHandler,
    peer_table::{PeerTable, TARGET_PEERS},
    rlpx::initiator::RLPxInitiator,
    sync::SyncMode,
    sync_manager::SyncManager,
    types::{Node, NodeRecord},
};
use ethrex_storage::{EngineType, Store};
use hex_literal::hex;
use secp256k1::SecretKey;
use spawned_concurrency::tasks::{GenServer, GenServerHandle};
use std::{net::SocketAddr, str::FromStr, sync::Arc};
use tokio::sync::Mutex as TokioMutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::info;
// Base price for each test transaction.
pub const BASE_PRICE_IN_WEI: u64 = 10_u64.pow(9);
pub const TEST_GENESIS: &str = include_str!("../../../fixtures/genesis/l1.json");

fn test_header(block_num: u64) -> BlockHeader {
    BlockHeader {
        parent_hash: H256::from_str(
            "0x1ac1bf1eef97dc6b03daba5af3b89881b7ae4bc1600dc434f450a9ec34d44999",
        )
        .unwrap(),
        ommers_hash: H256::from_str(
            "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        )
        .unwrap(),
        coinbase: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
        state_root: H256::from_str(
            "0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9",
        )
        .unwrap(),
        transactions_root: H256::from_str(
            "0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d",
        )
        .unwrap(),
        receipts_root: H256::from_str(
            "0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa",
        )
        .unwrap(),
        logs_bloom: Bloom::from([0; 256]),
        difficulty: U256::zero(),
        number: block_num,
        gas_limit: 0x016345785d8a0000,
        gas_used: 0xa8de,
        timestamp: 0x03e8,
        extra_data: Bytes::new(),
        prev_randao: H256::zero(),
        nonce: 0x0000000000000000,
        base_fee_per_gas: Some(BASE_PRICE_IN_WEI),
        withdrawals_root: Some(
            H256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap(),
        ),
        blob_gas_used: Some(0x00),
        excess_blob_gas: Some(0x00),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(*DEFAULT_REQUESTS_HASH),
        ..Default::default()
    }
}

async fn add_blocks_with_transactions(
    storage: &Store,
    block_count: u64,
    txs_per_block: Vec<Transaction>,
) {
    let mut new_canonical_blocks = vec![];
    for block_num in 1..=block_count {
        let block_body = BlockBody {
            transactions: txs_per_block.clone(),
            ommers: Default::default(),
            withdrawals: Default::default(),
        };
        let block_header = test_header(block_num);
        let block = Block::new(block_header.clone(), block_body);
        storage.add_block(block).await.unwrap();
        new_canonical_blocks.push((block_num, block_header.hash()));
    }
    let Some((last_number, last_hash)) = new_canonical_blocks.pop() else {
        return;
    };
    storage
        .forkchoice_update(new_canonical_blocks, last_number, last_hash, None, None)
        .await
        .unwrap();
}

fn legacy_tx_for_test(nonce: u64) -> Transaction {
    Transaction::LegacyTransaction(LegacyTransaction {
        nonce,
        gas_price: U256::from(nonce) * U256::from(BASE_PRICE_IN_WEI),
        gas: 10000,
        to: TxKind::Create,
        value: 100.into(),
        data: Default::default(),
        v: U256::from(0x1b),
        r: U256::from_big_endian(&hex!(
            "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
        )),
        s: U256::from_big_endian(&hex!(
            "5f6e3f188e3e6eab7d7d3b6568f5eac7d687b08d307d3154ccd8c87b4630509b"
        )),
        ..Default::default()
    })
}
fn eip1559_tx_for_test(nonce: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce,
        max_fee_per_gas: nonce * BASE_PRICE_IN_WEI,
        // This is less than gas_price in legacy txs because we should add base_fee to it
        // base_fee is 10^9, so (nonce - 1) * 10^9 + base_fee equals the legacy gas_price
        // for the same nonce. For consistency, we use the same value here.
        max_priority_fee_per_gas: (nonce - 1) * BASE_PRICE_IN_WEI,
        gas_limit: 10000,
        to: TxKind::Create,
        value: 100.into(),
        data: Default::default(),
        access_list: vec![],
        signature_y_parity: true,
        signature_r: U256::default(),
        signature_s: U256::default(),
        ..Default::default()
    })
}

pub async fn setup_store() -> Store {
    let genesis: &str = include_str!("../../../fixtures/genesis/l1.json");
    let genesis: Genesis = serde_json::from_str(genesis).expect("Fatal: test config is invalid");
    let mut store =
        Store::new("test-store", EngineType::InMemory).expect("Fail to create in-memory db test");
    store.add_initial_state(genesis).await.unwrap();
    store
}

pub async fn add_legacy_tx_blocks(storage: &Store, block_count: u64, tx_count: u64) {
    for block_num in 1..=block_count {
        let mut txs = vec![];
        for nonce in 1..=tx_count {
            txs.push(legacy_tx_for_test(nonce));
        }
        add_blocks_with_transactions(storage, block_num, txs).await;
    }
}

pub async fn add_eip1559_tx_blocks(storage: &Store, block_count: u64, tx_count: u64) {
    for block_num in 1..=block_count {
        let mut txs = vec![];
        for nonce in 1..=tx_count {
            txs.push(eip1559_tx_for_test(nonce));
        }
        add_blocks_with_transactions(storage, block_num, txs).await;
    }
}

pub async fn add_mixed_tx_blocks(storage: &Store, block_count: u64, tx_count: u64) {
    for block_num in 1..=block_count {
        let mut txs = vec![];
        for nonce in 1..=tx_count {
            if nonce % 2 == 0 {
                txs.push(legacy_tx_for_test(nonce));
            } else {
                txs.push(eip1559_tx_for_test(nonce));
            }
        }
        add_blocks_with_transactions(storage, block_num, txs).await;
    }
}

pub async fn add_empty_blocks(storage: &Store, block_count: u64) {
    for block_num in 1..=block_count {
        add_blocks_with_transactions(storage, block_num, vec![]).await;
    }
}

pub fn example_p2p_node() -> Node {
    let public_key_1 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
    Node::new("127.0.0.1".parse().unwrap(), 30303, 30303, public_key_1)
}

pub fn example_local_node_record() -> NodeRecord {
    let public_key_1 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
    let node = Node::new("127.0.0.1".parse().unwrap(), 30303, 30303, public_key_1);
    let signer = SecretKey::new(&mut rand::rngs::OsRng);

    NodeRecord::from_node(&node, 1, &signer).unwrap()
}

// Util to start an api for testing on ports 8500 and 8501,
// mostly for when hive is missing some endpoints to test
// like eth_uninstallFilter.
// Here's how you would use it:
// ```
// let server_handle = start_stest_api().await;
// ...
// assert!(something_that_needs_the_server);
// ...
// server_handle.abort();
// ```
pub async fn start_test_api() -> tokio::task::JoinHandle<()> {
    let http_addr: SocketAddr = "127.0.0.1:8500".parse().unwrap();
    let ws_addr: SocketAddr = "127.0.0.1:8546".parse().unwrap();
    let authrpc_addr: SocketAddr = "127.0.0.1:8501".parse().unwrap();
    let mut storage =
        Store::new("", EngineType::InMemory).expect("Failed to create in-memory storage");
    storage
        .add_initial_state(serde_json::from_str(TEST_GENESIS).unwrap())
        .await
        .expect("Failed to build test genesis");
    let blockchain = Arc::new(Blockchain::default_with_store(storage.clone()));
    let jwt_secret = Default::default();
    let local_p2p_node = example_p2p_node();
    let local_node_record = example_local_node_record();
    tokio::spawn(async move {
        start_api(
            http_addr,
            Some(ws_addr),
            Some(authrpc_addr),
            storage,
            blockchain,
            jwt_secret,
            local_p2p_node,
            local_node_record,
            dummy_sync_manager().await,
            dummy_peer_handler().await,
            "ethrex/test".to_string(),
            None,
            DEFAULT_BUILDER_GAS_CEIL,
            String::new(),
            None,
        )
        .await
        .unwrap()
    })
}

pub async fn default_context_with_storage(storage: Store) -> RpcApiContext {
    let blockchain = Arc::new(Blockchain::default_with_store(storage.clone()));
    let local_node_record = example_local_node_record();
    let block_worker_channel = start_block_executor(blockchain.clone());
    RpcApiContext {
        storage,
        blockchain,
        active_filters: Default::default(),
        syncer: Some(Arc::new(dummy_sync_manager().await)),
        peer_handler: Some(dummy_peer_handler().await),
        node_data: NodeData {
            jwt_secret: Default::default(),
            local_p2p_node: example_p2p_node(),
            local_node_record,
            client_version: "ethrex/test".to_string(),
            extra_data: Bytes::new(),
        },
        gas_tip_estimator: Arc::new(TokioMutex::new(GasTipEstimator::new())),
        log_filter_handler: None,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        block_worker_channel,
        dev_tx_sender: None,
    }
}

/// Creates a dummy SyncManager for tests where syncing is not needed
/// This should only be used in tests as it won't be able to connect to the p2p network
pub async fn dummy_sync_manager() -> SyncManager {
    SyncManager::new(
        dummy_peer_handler().await,
        &SyncMode::Full,
        CancellationToken::new(),
        Arc::new(Blockchain::default_with_store(
            Store::new("", EngineType::InMemory).expect("Failed to start Store Engine"),
        )),
        Store::new("temp.db", ethrex_storage::EngineType::InMemory)
            .expect("Failed to start Storage Engine"),
        ".".into(),
    )
    .await
}

/// Creates a dummy PeerHandler for tests where interacting with peers is not needed
/// This should only be used in tests as it won't be able to interact with the node's connected peers
pub async fn dummy_peer_handler() -> PeerHandler {
    let peer_table = PeerTable::spawn(TARGET_PEERS);
    PeerHandler::new(peer_table.clone(), dummy_gen_server(peer_table).await)
}

/// Creates a dummy GenServer for tests
/// This should only be used in tests
pub async fn dummy_gen_server(peer_table: PeerTable) -> GenServerHandle<RLPxInitiator> {
    info!("Starting RLPx Initiator");
    let state = RLPxInitiator::new(dummy_p2p_context(peer_table).await);
    RLPxInitiator::start_on_thread(state)
}

/// Creates a dummy P2PContext for tests
/// This should only be used in tests as it won't be able to connect to the p2p network
pub async fn dummy_p2p_context(peer_table: PeerTable) -> P2PContext {
    let local_node = Node::from_enode_url(
        "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
    ).expect("Bad enode url");
    let storage = Store::new("./temp", EngineType::InMemory).expect("Failed to create Store");

    P2PContext::new(
        local_node,
        TaskTracker::default(),
        SecretKey::from_byte_array(&[0xcd; 32]).expect("32 bytes, within curve order"),
        peer_table,
        storage.clone(),
        Arc::new(Blockchain::default_with_store(storage)),
        "".to_string(),
        None,
        1000,
        100.0,
    )
    .unwrap()
}
