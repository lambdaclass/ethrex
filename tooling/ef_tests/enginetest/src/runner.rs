use std::sync::Arc;

use ethrex_blockchain::Blockchain;
use ethrex_common::{
    H256,
    constants::EMPTY_KECCACK_HASH,
    types::{Account as CoreAccount, DEFAULT_BUILDER_GAS_CEIL},
};
use ethrex_rpc::{
    ClientVersion, GasTipEstimator, NodeData, RpcApiContext,
    RpcErr, RpcErrorMetadata, RpcHandler,
    start_block_executor,
    test_utils::{
        dummy_peer_handler, dummy_sync_manager,
        example_local_node_record, example_p2p_node,
    },
};
use ethrex_rpc::engine::fork_choice::{
    ForkChoiceUpdatedV1, ForkChoiceUpdatedV2,
    ForkChoiceUpdatedV3, ForkChoiceUpdatedV4,
};
use ethrex_rpc::engine::payload::{
    NewPayloadV1Request, NewPayloadV2Request,
    NewPayloadV3Request, NewPayloadV4Request,
    NewPayloadV5Request,
};
use ethrex_rpc::types::fork_choice::ForkChoiceResponse;
use ethrex_rpc::types::payload::{
    PayloadStatus, PayloadValidationStatus,
};
use ethrex_storage::{EngineType, Store};
use serde::Serialize;
use tokio::sync::{Mutex as TokioMutex, OnceCell};

use crate::types::{EngineNewPayload, EngineTestUnit};

use bytes::Bytes;
use ethrex_p2p::peer_handler::PeerHandler;
use ethrex_p2p::sync_manager::SyncManager;

/// Shared dummy infrastructure (SyncManager + PeerHandler) that is
/// expensive to create.  Built once and reused across all tests so
/// we don't exhaust OS thread limits.
struct SharedTestInfra {
    syncer: Arc<SyncManager>,
    peer_handler: PeerHandler,
}

/// Lazily-initialised shared infrastructure.
static SHARED_INFRA: OnceCell<SharedTestInfra> =
    OnceCell::const_new();

/// Returns a reference to the shared infra, initializing it on the
/// first call (within the current async runtime).
async fn shared_infra() -> &'static SharedTestInfra {
    SHARED_INFRA
        .get_or_init(|| async {
            let dummy_store = Store::new(
                "",
                EngineType::InMemory,
            )
            .expect("Failed to create dummy store");
            SharedTestInfra {
                syncer: Arc::new(dummy_sync_manager().await),
                peer_handler: dummy_peer_handler(
                    dummy_store,
                )
                .await,
            }
        })
        .await
}

/// Build a lightweight `RpcApiContext` for a single test, reusing the
/// shared SyncManager and PeerHandler.
#[allow(unexpected_cfgs)]
async fn build_context(store: Store) -> RpcApiContext {
    let blockchain =
        Arc::new(Blockchain::default_with_store(store.clone()));
    let block_worker_channel =
        start_block_executor(blockchain.clone());
    let infra = shared_infra().await;

    RpcApiContext {
        storage: store,
        blockchain,
        active_filters: Default::default(),
        syncer: Some(infra.syncer.clone()),
        peer_handler: Some(infra.peer_handler.clone()),
        node_data: NodeData {
            jwt_secret: Default::default(),
            local_p2p_node: example_p2p_node(),
            local_node_record: example_local_node_record(),
            client_version: ClientVersion::new(
                "ethrex".to_string(),
                "0.1.0".to_string(),
                "test".to_string(),
                "abcd1234".to_string(),
                "x86_64".to_string(),
                "1.70.0".to_string(),
            ),
            extra_data: Bytes::new(),
        },
        gas_tip_estimator: Arc::new(
            TokioMutex::new(GasTipEstimator::new()),
        ),
        log_filter_handler: None,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        block_worker_channel,
        #[cfg(feature = "eip-8025")]
        proof_coordinator: None,
    }
}

#[derive(Serialize)]
pub struct TestResult {
    pub name: String,
    pub pass: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Run a single engine test. Returns `Ok(())` on pass, `Err(msg)` on
/// failure.
pub async fn run_engine_test(
    test_key: &str,
    test: &EngineTestUnit,
) -> Result<(), String> {
    let store = build_store(test).await;
    let context = build_context(store.clone()).await;

    // Track the current head hash for fork-choice updates.
    #[allow(unused_assignments)]
    let mut head_hash = test.genesis_block_header.hash;

    for (i, payload_entry) in
        test.engine_new_payloads.iter().enumerate()
    {
        let expects_error = payload_entry.expects_error();
        let expected_rpc_code = payload_entry.error_code;

        let version = payload_entry.new_payload_version;
        let params = Some(payload_entry.params.clone());

        // ---- 1. Dispatch to the real RPC handler ----
        //
        // RpcHandler::parse() validates parameter count and
        // deserializes the payload, and handle() runs the full
        // engine pipeline: payload validation, block construction,
        // block hash check, blob versioned hash check, execution
        // requests validation, block execution, and storage.
        let handler_result: Result<serde_json::Value, RpcErr> =
            dispatch_new_payload(version, &params, &context).await;

        // ---- 2. Check for RPC-level errors ----
        match handler_result {
            Err(rpc_err) => {
                let meta: RpcErrorMetadata = rpc_err.into();
                if let Some(expected_code) = expected_rpc_code {
                    if meta.code == expected_code as i32 {
                        // Expected RPC error, skip this payload.
                        continue;
                    }
                    return Err(format!(
                        "payload[{i}]: expected RPC error code \
                         {expected_code}, got {} ({})",
                        meta.code, meta.message
                    ));
                }
                if expects_error {
                    // The fixture expected an INVALID status but
                    // we got an RPC error instead. Treat it as a
                    // matching error.
                    continue;
                }
                return Err(format!(
                    "payload[{i}]: unexpected RPC error: \
                     code={}, msg={}",
                    meta.code, meta.message
                ));
            }
            Ok(ref _val) if expected_rpc_code.is_some() => {
                return Err(format!(
                    "payload[{i}]: expected RPC error code {:?} \
                     but handler succeeded",
                    expected_rpc_code
                ));
            }
            Ok(response_value) => {
                // ---- 3. Inspect PayloadStatus ----
                let status: PayloadStatus =
                    serde_json::from_value(response_value.clone())
                        .map_err(|e| {
                            format!(
                                "payload[{i}]: failed to \
                                 deserialize PayloadStatus: {e} \
                                 (raw: {response_value})"
                            )
                        })?;

                match status.status {
                    PayloadValidationStatus::Valid => {
                        if expects_error {
                            return Err(format!(
                                "payload[{i}]: expected error \
                                 ({:?}) but got VALID",
                                payload_entry.validation_error
                            ));
                        }
                    }
                    PayloadValidationStatus::Invalid => {
                        if !expects_error {
                            return Err(format!(
                                "payload[{i}]: got INVALID \
                                 unexpectedly: {:?}",
                                status.validation_error
                            ));
                        }
                        // Expected error -- do NOT advance fork
                        // choice, but continue processing
                        // subsequent payloads.
                        continue;
                    }
                    PayloadValidationStatus::Syncing
                    | PayloadValidationStatus::Accepted => {
                        // Syncing/Accepted in a test context is
                        // unexpected; the dummy SyncManager runs
                        // in Full mode so this should not happen.
                        return Err(format!(
                            "payload[{i}]: unexpected status {:?}",
                            status.status
                        ));
                    }
                }
            }
        }

        // ---- 4. Apply fork choice (advance the canonical
        //         head) via the real handler ----
        head_hash = payload_entry
            .block_hash_from_params()
            .unwrap_or(head_hash);

        let fcu_version = payload_entry.forkchoice_updated_version;
        dispatch_fork_choice(
            fcu_version,
            head_hash,
            &context,
        )
        .await
        .map_err(|e| {
            format!("payload[{i}]: fork choice update failed: {e}")
        })?;
    }

    // ---- 5. Verify post-state ----
    verify_post_state(test_key, test, &store).await?;

    Ok(())
}

// ---- RPC dispatch helpers ----

/// Dispatch to the version-appropriate `engine_newPayload` handler.
async fn dispatch_new_payload(
    version: u8,
    params: &Option<Vec<serde_json::Value>>,
    context: &RpcApiContext,
) -> Result<serde_json::Value, RpcErr> {
    match version {
        1 => {
            let req = NewPayloadV1Request::parse(params)?;
            req.handle(context.clone()).await
        }
        2 => {
            let req = NewPayloadV2Request::parse(params)?;
            req.handle(context.clone()).await
        }
        3 => {
            let req = NewPayloadV3Request::parse(params)?;
            req.handle(context.clone()).await
        }
        4 => {
            let req = NewPayloadV4Request::parse(params)?;
            req.handle(context.clone()).await
        }
        5 => {
            let req = NewPayloadV5Request::parse(params)?;
            req.handle(context.clone()).await
        }
        _ => Err(RpcErr::BadParams(format!(
            "Unsupported newPayload version: {version}"
        ))),
    }
}

/// Dispatch to the version-appropriate `engine_forkchoiceUpdated`
/// handler. We only pass the fork-choice state (no payload
/// attributes) since the test runner does not build payloads.
async fn dispatch_fork_choice(
    version: u8,
    head_hash: H256,
    context: &RpcApiContext,
) -> Result<(), String> {
    let fcu_state = serde_json::json!({
        "headBlockHash": head_hash,
        "safeBlockHash": head_hash,
        "finalizedBlockHash": head_hash,
    });
    let params = Some(vec![fcu_state]);

    let result = match version {
        1 => {
            let req = ForkChoiceUpdatedV1::parse(&params)
                .map_err(|e| format!("FCU parse: {e}"))?;
            req.handle(context.clone())
                .await
                .map_err(|e| format!("FCU handle: {e}"))
        }
        2 => {
            let req = ForkChoiceUpdatedV2::parse(&params)
                .map_err(|e| format!("FCU parse: {e}"))?;
            req.handle(context.clone())
                .await
                .map_err(|e| format!("FCU handle: {e}"))
        }
        3 => {
            let req = ForkChoiceUpdatedV3::parse(&params)
                .map_err(|e| format!("FCU parse: {e}"))?;
            req.handle(context.clone())
                .await
                .map_err(|e| format!("FCU handle: {e}"))
        }
        4 => {
            let req = ForkChoiceUpdatedV4::parse(&params)
                .map_err(|e| format!("FCU parse: {e}"))?;
            req.handle(context.clone())
                .await
                .map_err(|e| format!("FCU handle: {e}"))
        }
        _ => {
            return Err(format!(
                "Unsupported forkchoiceUpdated version: {version}"
            ));
        }
    }?;

    // Verify the FCU response indicates VALID (not SYNCING or
    // INVALID).
    let response: ForkChoiceResponse =
        serde_json::from_value(result).map_err(|e| {
            format!("Failed to parse ForkChoiceResponse: {e}")
        })?;

    match response.payload_status.status {
        PayloadValidationStatus::Valid => Ok(()),
        other => Err(format!(
            "Fork choice returned {:?}: {:?}",
            other, response.payload_status.validation_error
        )),
    }
}

// ---- Store setup ----

async fn build_store(test: &EngineTestUnit) -> Store {
    let mut store = Store::new("store.db", EngineType::InMemory)
        .expect("Failed to build DB for engine test");
    let genesis = test.get_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    store
}

// ---- Post-state verification ----

async fn verify_post_state(
    test_key: &str,
    test: &EngineTestUnit,
    store: &Store,
) -> Result<(), String> {
    let latest_block_number =
        store.get_latest_block_number().await.map_err(|e| {
            format!("Failed to get latest block number: {e}")
        })?;

    // Verify account state
    if let Some(post_state) = &test.post_state {
        for (addr, account) in post_state {
            let expected: CoreAccount = account.clone().into();

            let db_info = store
                .get_account_info(latest_block_number, *addr)
                .await
                .map_err(|e| format!("DB read error: {e}"))?
                .ok_or_else(|| {
                    format!(
                        "{test_key}: account {addr} not found \
                         in post-state"
                    )
                })?;

            if db_info != expected.info {
                return Err(format!(
                    "{test_key}: account {addr} info mismatch: \
                     expected {:?}, got {:?}",
                    expected.info, db_info
                ));
            }

            let code_hash = expected.info.code_hash;
            if code_hash != *EMPTY_KECCACK_HASH {
                let db_code = store
                    .get_account_code(code_hash)
                    .map_err(|e| format!("DB read error: {e}"))?
                    .ok_or_else(|| {
                        format!(
                            "{test_key}: code {code_hash} \
                             not found"
                        )
                    })?;
                if db_code != expected.code {
                    return Err(format!(
                        "{test_key}: code mismatch for {addr}"
                    ));
                }
            }

            for (key, value) in &expected.storage {
                let db_val = store
                    .get_storage_at(
                        latest_block_number,
                        *addr,
                        *key,
                    )
                    .map_err(|e| format!("DB read error: {e}"))?
                    .ok_or_else(|| {
                        format!(
                            "{test_key}: storage {key} for \
                             {addr} not found"
                        )
                    })?;
                if db_val != *value {
                    return Err(format!(
                        "{test_key}: storage mismatch for \
                         {addr} key {key}: expected {value}, \
                         got {db_val}"
                    ));
                }
            }
        }
    }

    // Verify lastblockhash
    let last_header = store
        .get_block_header(latest_block_number)
        .map_err(|e| format!("DB read error: {e}"))?
        .ok_or_else(|| {
            format!("{test_key}: last block header not found")
        })?;
    let last_hash = last_header.hash();

    if test.lastblockhash != last_hash {
        return Err(format!(
            "{test_key}: lastblockhash mismatch: expected \
             {:#x}, got {last_hash:#x}",
            test.lastblockhash
        ));
    }

    Ok(())
}

impl EngineNewPayload {
    /// Returns true if this payload entry expects an error (INVALID
    /// status or validation error).
    pub fn expects_error(&self) -> bool {
        self.validation_error
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }

    /// Extract the block hash from params[0] of the fixture.
    pub fn block_hash_from_params(&self) -> Option<H256> {
        self.params.first().and_then(|p| {
            p.get("blockHash")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
        })
    }
}
