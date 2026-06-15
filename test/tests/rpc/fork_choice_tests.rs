use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER},
};
use ethrex_rpc::engine::fork_choice::ForkChoiceUpdatedV3;
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::utils::RpcRequest;
use ethrex_storage::{EngineType, Store};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn test_store() -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis = serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    store
}

async fn new_block(store: &Store, parent: &BlockHeader) -> Block {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::random(),
        random: H256::random(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::random()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let blockchain = Blockchain::default_with_store(store.clone());
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(block).unwrap().payload
}

// Regression test for execution-apis PR #786: when engine_forkchoiceUpdatedV3
// receives a head that is a VALID canonical ancestor of the latest known
// finalized block, the response MUST be {payloadStatus: VALID, payloadId: null}
// and the client MUST NOT begin a payload build process — even when
// payloadAttributes is non-null.
#[tokio::test]
async fn test_fcu_v3_finalized_ancestor_returns_valid_with_null_payload_id() {
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());

    let block_1 = new_block(&store, &genesis_header).await;
    let hash_1 = block_1.hash();
    blockchain.add_block(block_1.clone()).unwrap();

    let block_2 = new_block(&store, &block_1.header).await;
    let hash_2 = block_2.hash();
    blockchain.add_block(block_2.clone()).unwrap();

    // head = block_2 (latest tip), safe = finalized = block_1.
    // After this, block_1 is canonical, finalized number == 1, latest == 2.
    apply_fork_choice(&store, hash_2, hash_1, hash_1)
        .await
        .expect("apply_fork_choice failed");

    // Now drive engine_forkchoiceUpdatedV3 with head = block_1 (finalized ancestor)
    // and non-null payloadAttributes. The guard in apply_fork_choice should
    // return InvalidForkChoice::NewHeadAlreadyCanonical, which the RPC layer
    // must translate into VALID + null payloadId without calling build_payload.
    let attrs_timestamp = block_1.header.timestamp + 12;
    let body = format!(
        r#"{{
            "jsonrpc": "2.0",
            "method": "engine_forkchoiceUpdatedV3",
            "params": [
                {{
                    "headBlockHash": "{hash_1:#x}",
                    "safeBlockHash": "{hash_1:#x}",
                    "finalizedBlockHash": "{hash_1:#x}"
                }},
                {{
                    "timestamp": "{attrs_timestamp:#x}",
                    "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "suggestedFeeRecipient": "0x0000000000000000000000000000000000000000",
                    "withdrawals": [],
                    "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000002"
                }}
            ],
            "id": 1
        }}"#
    );
    let request: RpcRequest = serde_json::from_str(&body).expect("valid FCU request");

    let context = default_context_with_storage(store).await;
    let response = ForkChoiceUpdatedV3::call(&request, context)
        .await
        .expect("FCU V3 call should succeed");

    assert_eq!(
        response["payloadStatus"]["status"], "VALID",
        "payloadStatus.status must be VALID per execution-apis PR #786"
    );
    assert_eq!(
        response["payloadStatus"]["latestValidHash"],
        format!("{hash_1:#x}"),
        "latestValidHash must echo the head hash"
    );
    assert!(
        response["payloadId"].is_null(),
        "payloadId must be null when head is a finalized ancestor; got {:?}",
        response["payloadId"]
    );
}

// At the Amsterdam activation timestamp, engine_forkchoiceUpdatedV3 must reject
// otherwise-valid V3 payload attributes with UnsupportedFork; payload building
// from that timestamp onward requires engine_forkchoiceUpdatedV4.
#[tokio::test]
async fn forkchoice_updated_v3_rejects_amsterdam_payload_attributes() {
    let mut store = test_store().await;
    let mut chain_config = store.get_chain_config();
    let amsterdam_time = 24;
    chain_config.amsterdam_time = Some(amsterdam_time);
    store.set_chain_config(&chain_config).await.unwrap();

    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_hash = genesis_header.hash();
    let block = new_block(&store, &genesis_header).await;
    let block_hash = block.hash();
    Blockchain::default_with_store(store.clone())
        .add_block(block)
        .unwrap();

    let body = format!(
        r#"{{
            "jsonrpc": "2.0",
            "method": "engine_forkchoiceUpdatedV3",
            "params": [
                {{
                    "headBlockHash": "{block_hash:#x}",
                    "safeBlockHash": "{genesis_hash:#x}",
                    "finalizedBlockHash": "{genesis_hash:#x}"
                }},
                {{
                    "timestamp": "{amsterdam_time:#x}",
                    "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "suggestedFeeRecipient": "0x0000000000000000000000000000000000000000",
                    "withdrawals": [],
                    "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000002"
                }}
            ],
            "id": 1
        }}"#
    );
    let request: RpcRequest = serde_json::from_str(&body).expect("valid FCU request");
    let context = default_context_with_storage(store).await;

    let err = ForkChoiceUpdatedV3::call(&request, context)
        .await
        .unwrap_err();

    assert!(matches!(err, ethrex_rpc::utils::RpcErr::UnsupportedFork(_)));
}
