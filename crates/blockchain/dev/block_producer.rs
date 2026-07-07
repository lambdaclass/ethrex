use bytes::Bytes;
use ethereum_types::{Address, H256};
use ethrex_rpc::clients::{EngineClient, EngineClientError};
use ethrex_rpc::types::fork_choice::{ForkChoiceState, PayloadAttributesV3, PayloadAttributesV4};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

#[allow(clippy::too_many_arguments)]
pub async fn start_block_producer(
    execution_client_auth_url: String,
    jwt_secret: Bytes,
    head_block_hash: H256,
    max_tries: u32,
    block_production_interval_ms: u64,
    coinbase_address: Address,
    // Amsterdam activation timestamp (from the chain config). `Some(t)` with
    // `t <= block_timestamp` means the payload must be built with the V4 engine
    // methods (V3 forkchoice is rejected at Amsterdam+). `None` → pre-Amsterdam.
    amsterdam_time: Option<u64>,
    // CL-supplied target gas limit for V4 payload building (execution-apis#796).
    target_gas_limit: u64,
) -> Result<(), EngineClientError> {
    let engine_client = EngineClient::new(&execution_client_auth_url, jwt_secret);

    // Sleep for one slot to avoid timestamp collision with the genesis block.
    sleep(Duration::from_millis(block_production_interval_ms)).await;

    let mut head_block_hash: H256 = head_block_hash;
    let parent_beacon_block_root = H256::zero();
    let mut slot_number: u64 = 0;
    let mut tries = 0;
    while tries < max_tries {
        tracing::info!("Producing block");
        tracing::debug!("Head block hash: {head_block_hash:#x}");
        let fork_choice_state = ForkChoiceState {
            head_block_hash,
            safe_block_hash: head_block_hash,
            finalized_block_hash: head_block_hash,
        };

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let is_amsterdam = amsterdam_time.is_some_and(|t| t <= timestamp);

        // Amsterdam+ payloads MUST be built via forkchoiceUpdatedV4 (V3 is
        // rejected at Amsterdam+, e.g. for a native-rollup L1 at LStar); earlier
        // forks use V3, which rejects the V4-only slot_number/target_gas_limit.
        let (fork_choice_result, fcu_version) = if is_amsterdam {
            slot_number += 1;
            let payload_attributes = PayloadAttributesV4 {
                timestamp,
                prev_randao: H256::zero(),
                suggested_fee_recipient: coinbase_address,
                parent_beacon_block_root: Some(parent_beacon_block_root),
                withdrawals: Some(Vec::new()),
                slot_number,
                target_gas_limit,
            };
            (
                engine_client
                    .engine_forkchoice_updated_v4(fork_choice_state, Some(payload_attributes))
                    .await,
                "engine_forkchoiceUpdatedV4",
            )
        } else {
            let payload_attributes = PayloadAttributesV3 {
                timestamp,
                prev_randao: H256::zero(),
                suggested_fee_recipient: coinbase_address,
                parent_beacon_block_root: Some(parent_beacon_block_root),
                withdrawals: Some(Vec::new()),
            };
            (
                engine_client
                    .engine_forkchoice_updated_v3(fork_choice_state, Some(payload_attributes))
                    .await,
                "engine_forkchoiceUpdatedV3",
            )
        };
        let fork_choice_response = match fork_choice_result {
            Ok(response) => {
                tracing::debug!("{fcu_version} response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending {fcu_version} with PayloadAttributes: {error}"
                );
                sleep(Duration::from_millis(300)).await;
                tries += 1;
                continue;
            }
        };
        let Some(payload_id) = fork_choice_response.payload_id else {
            tracing::error!("Failed to produce block: payload_id is None in ForkChoiceResponse");
            sleep(Duration::from_millis(300)).await;
            tries += 1;
            continue;
        };

        // Wait to retrieve the payload.
        // Note that this makes getPayload failures result in skipped blocks.
        sleep(Duration::from_millis(block_production_interval_ms)).await;

        // Amsterdam+ payloads are retrieved with getPayloadV6 (V5 is Osaka-only,
        // capped below Amsterdam); earlier forks use V5.
        let (get_payload_result, get_payload_version) = if is_amsterdam {
            (
                engine_client.engine_get_payload_v6(payload_id).await,
                "engine_getPayloadV6",
            )
        } else {
            (
                engine_client.engine_get_payload_v5(payload_id).await,
                "engine_getPayloadV5",
            )
        };
        let execution_payload_response = match get_payload_result {
            Ok(response) => {
                tracing::debug!("{get_payload_version} response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending {get_payload_version}: {error}"
                );
                sleep(Duration::from_millis(300)).await;
                tries += 1;
                continue;
            }
        };
        let execution_payload = execution_payload_response.execution_payload;
        let versioned_hashes: Vec<H256> = execution_payload_response
            .blobs_bundle
            .unwrap_or_default()
            .commitments
            .iter()
            .map(|commitment| {
                let mut hasher = Sha256::new();
                hasher.update(commitment);
                let mut hash = hasher.finalize();
                // https://eips.ethereum.org/EIPS/eip-4844 -> kzg_to_versioned_hash
                hash[0] = 0x01;
                H256::from_slice(&hash)
            })
            .collect();

        // Amsterdam+ payloads carry a Block Access List and MUST use newPayloadV5;
        // earlier forks use V4, which rejects the BAL field.
        let is_amsterdam = execution_payload.block_access_list.is_some();
        let endpoint = if is_amsterdam {
            "engine_newPayloadV5"
        } else {
            "engine_newPayloadV4"
        };
        let new_payload_result = if is_amsterdam {
            engine_client
                .engine_new_payload_v5(
                    execution_payload,
                    versioned_hashes,
                    parent_beacon_block_root,
                )
                .await
        } else {
            engine_client
                .engine_new_payload_v4(
                    execution_payload,
                    versioned_hashes,
                    parent_beacon_block_root,
                )
                .await
        };
        let payload_status = match new_payload_result {
            Ok(response) => {
                tracing::debug!("{endpoint} response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!("Failed to produce block: error sending {endpoint}: {error}");
                sleep(Duration::from_millis(300)).await;
                tries += 1;
                continue;
            }
        };
        let produced_block_hash = if let Some(latest_valid_hash) = payload_status.latest_valid_hash
        {
            latest_valid_hash
        } else {
            tracing::error!(
                "Failed to produce block: latest_valid_hash is None in PayloadStatus: {payload_status:?}"
            );
            sleep(Duration::from_millis(300)).await;
            tries += 1;
            continue;
        };
        tracing::info!("Produced block {produced_block_hash:#x}");

        head_block_hash = produced_block_hash;
    }
    Err(EngineClientError::SystemFailed(format!("{max_tries}")))
}
