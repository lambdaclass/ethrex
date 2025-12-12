use bytes::Bytes;
use ethereum_types::{Address, H256};
use ethrex_rpc::clients::{EngineClient, EngineClientError};
use ethrex_rpc::types::fork_choice::{ForkChoiceState, PayloadAttributesV3};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

pub async fn start_block_producer(
    execution_client_auth_url: String,
    jwt_secret: Bytes,
    head_block_hash: H256,
    max_tries: u32,
    block_production_interval_ms: u64,
    coinbase_address: Address,
) -> Result<(), EngineClientError> {
    let engine_client = EngineClient::new(&execution_client_auth_url, jwt_secret);

    // Sleep for one slot to avoid timestamp collision with the genesis block.
    sleep(Duration::from_millis(block_production_interval_ms)).await;

    let mut head_block_hash: H256 = head_block_hash;
    let parent_beacon_block_root = H256::zero();
    let mut tries = 0;
    while tries < max_tries {
        tracing::info!("Producing block");
        tracing::debug!("Head block hash: {head_block_hash:#x}");
        let fork_choice_state = ForkChoiceState {
            head_block_hash,
            safe_block_hash: head_block_hash,
            finalized_block_hash: head_block_hash,
        };

        let payload_attributes = PayloadAttributesV3 {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            prev_randao: H256::zero(),
            suggested_fee_recipient: coinbase_address,
            parent_beacon_block_root: Some(parent_beacon_block_root),
            withdrawals: Some(Vec::new()),
        };
        let fork_choice_response = match engine_client
            .engine_forkchoice_updated_v3(fork_choice_state, Some(payload_attributes))
            .await
        {
            Ok(response) => {
                tracing::debug!("engine_forkchoiceUpdatedV3 response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending engine_forkchoiceUpdatedV3 with PayloadAttributes: {error}"
                );
                sleep(Duration::from_millis(300)).await;
                tries += 1;
                continue;
            }
        };
        let payload_id = fork_choice_response
            .payload_id
            .expect("Failed to produce block: payload_id is None in ForkChoiceResponse");

        // Wait to retrieve the payload.
        // Note that this makes getPayload failures result in skipped blocks.
        sleep(Duration::from_millis(block_production_interval_ms)).await;

        // Fork-aware payload retrieval: try V5 (Osaka) first, fall back to V4 (Prague) if unsupported
        let execution_payload_response = match engine_client.engine_get_payload_v5(payload_id).await
        {
            Ok(response) => {
                tracing::debug!("engine_getPayloadV5 response: {response:?}");
                response
            }
            Err(EngineClientError::FailedDuringGetPayload(
                ethrex_rpc::clients::auth::errors::GetPayloadError::UnsupportedFork(msg),
            )) => {
                tracing::debug!("V5 not supported for current fork, falling back to V4: {msg}");
                match engine_client.engine_get_payload_v4(payload_id).await {
                    Ok(response) => {
                        tracing::debug!("engine_getPayloadV4 response: {response:?}");
                        response
                    }
                    Err(v4_error) => {
                        tracing::error!(
                            "Failed to produce block: error sending engine_getPayloadV4: {v4_error}"
                        );
                        sleep(Duration::from_millis(300)).await;
                        tries += 1;
                        continue;
                    }
                }
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending engine_getPayloadV5: {error}"
                );
                sleep(Duration::from_millis(300)).await;
                tries += 1;
                continue;
            }
        };
        let payload_status = match engine_client
            .engine_new_payload_v4(
                execution_payload_response.execution_payload,
                execution_payload_response
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
                    .collect(),
                parent_beacon_block_root,
            )
            .await
        {
            Ok(response) => {
                tracing::debug!("engine_newPayloadV4 response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending engine_newPayloadV4: {error}"
                );
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

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_rpc::clients::auth::errors::GetPayloadError;

    #[test]
    fn test_unsupported_fork_error_matching() {
        let error = EngineClientError::FailedDuringGetPayload(GetPayloadError::UnsupportedFork(
            "Unsupported fork: Prague".to_string(),
        ));

        match error {
            EngineClientError::FailedDuringGetPayload(GetPayloadError::UnsupportedFork(_msg)) => {
                // Correctly matched UnsupportedFork error
            }
            _ => panic!("Failed to match UnsupportedFork error variant"),
        }
    }

    #[test]
    fn test_non_fork_error_not_matched() {
        let error = EngineClientError::FailedDuringGetPayload(GetPayloadError::RPCError(
            "Network error".to_string(),
        ));

        match error {
            EngineClientError::FailedDuringGetPayload(GetPayloadError::UnsupportedFork(_)) => {
                panic!("Should not match non-fork errors");
            }
            EngineClientError::FailedDuringGetPayload(GetPayloadError::RPCError(_)) => {
                // Correctly identified non-fork error
            }
            _ => panic!("Unexpected error variant"),
        }
    }

    #[test]
    fn test_error_variant_enumeration() {
        let unsupported_fork = GetPayloadError::UnsupportedFork("test".to_string());
        let rpc_error = GetPayloadError::RPCError("test".to_string());

        match unsupported_fork {
            GetPayloadError::UnsupportedFork(_) => {}
            _ => panic!("UnsupportedFork variant should exist"),
        }

        match rpc_error {
            GetPayloadError::RPCError(_) => {}
            _ => panic!("RPCError variant should exist"),
        }
    }
}
