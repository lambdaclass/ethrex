use bytes::Bytes;
use ethereum_types::{Address, H256};
use ethrex_common::types::ChainConfig;
use ethrex_rpc::clients::{EngineClient, EngineClientError};
use ethrex_rpc::types::fork_choice::{ForkChoiceState, PayloadAttributesV3, PayloadAttributesV4};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

/// What the dev producer needs to stand in for a consensus layer.
pub struct BlockProducerConfig {
    pub execution_client_auth_url: String,
    pub jwt_secret: Bytes,
    pub head_block_hash: H256,
    pub max_tries: u32,
    pub block_production_interval_ms: u64,
    pub coinbase_address: Address,
    pub chain_config: ChainConfig,
    /// EIP-7843 slot to continue the sequence from, i.e. the head's slot.
    pub head_slot_number: u64,
    /// execution-apis#796 `targetGasLimit`, required on V4 payload attributes.
    pub target_gas_limit: u64,
}

pub async fn start_block_producer(config: BlockProducerConfig) -> Result<(), EngineClientError> {
    let BlockProducerConfig {
        execution_client_auth_url,
        jwt_secret,
        head_block_hash,
        max_tries,
        block_production_interval_ms,
        coinbase_address,
        chain_config,
        head_slot_number,
        target_gas_limit,
    } = config;
    let engine_client = EngineClient::new(&execution_client_auth_url, jwt_secret);

    // Sleep for one slot to avoid timestamp collision with the genesis block.
    sleep(Duration::from_millis(block_production_interval_ms)).await;

    let mut head_block_hash: H256 = head_block_hash;
    let parent_beacon_block_root = H256::zero();
    // EIP-7843: every Amsterdam+ header carries a beacon slot. There is no beacon
    // chain in dev mode, so the producer stands in for the slot clock and advances
    // one slot per production attempt, continuing from the head's slot so the
    // sequence stays monotonic across restarts.
    let mut slot_number = head_slot_number;
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
        slot_number += 1;

        // Amsterdam+ payload attributes carry `slotNumber` and `targetGasLimit`
        // and MUST use forkchoiceUpdatedV4; V3 attributes are rejected for an
        // Amsterdam timestamp.
        let is_amsterdam = chain_config.is_amsterdam_activated(timestamp);
        let fork_choice_result = if is_amsterdam {
            engine_client
                .engine_forkchoice_updated_v4(
                    fork_choice_state,
                    Some(PayloadAttributesV4 {
                        timestamp,
                        prev_randao: H256::zero(),
                        suggested_fee_recipient: coinbase_address,
                        parent_beacon_block_root: Some(parent_beacon_block_root),
                        withdrawals: Some(Vec::new()),
                        slot_number,
                        target_gas_limit,
                    }),
                )
                .await
        } else {
            engine_client
                .engine_forkchoice_updated_v3(
                    fork_choice_state,
                    Some(PayloadAttributesV3 {
                        timestamp,
                        prev_randao: H256::zero(),
                        suggested_fee_recipient: coinbase_address,
                        parent_beacon_block_root: Some(parent_beacon_block_root),
                        withdrawals: Some(Vec::new()),
                    }),
                )
                .await
        };
        let fcu_endpoint = if is_amsterdam {
            "engine_forkchoiceUpdatedV4"
        } else {
            "engine_forkchoiceUpdatedV3"
        };
        let fork_choice_response = match fork_choice_result {
            Ok(response) => {
                tracing::debug!("{fcu_endpoint} response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending {fcu_endpoint} with PayloadAttributes: {error}"
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

        // V5 serves Osaka only; Amsterdam+ payloads must be fetched with V6.
        let get_payload_result = if is_amsterdam {
            engine_client.engine_get_payload_v6(payload_id).await
        } else {
            engine_client.engine_get_payload_v5(payload_id).await
        };
        let get_payload_endpoint = if is_amsterdam {
            "engine_getPayloadV6"
        } else {
            "engine_getPayloadV5"
        };
        let execution_payload_response = match get_payload_result {
            Ok(response) => {
                tracing::debug!("{get_payload_endpoint} response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending {get_payload_endpoint}: {error}"
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
        // Reset the failure counter on success so `max_tries` bounds CONSECUTIVE failures,
        // not cumulative ones over the node's lifetime (otherwise a long-lived dev node
        // with occasional transient hiccups would eventually abort).
        tries = 0;
    }
    Err(EngineClientError::SystemFailed(format!("{max_tries}")))
}
