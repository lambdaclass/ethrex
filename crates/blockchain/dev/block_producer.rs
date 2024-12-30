use crate::utils::engine_client::{errors::EngineClientError, EngineClient};
use bytes::Bytes;
use ethereum_types::{Address, H256};
use ethrex_rpc::types::fork_choice::{ForkChoiceState, PayloadAttributesV3};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn start_block_producer(
    execution_client_auth_url: String,
    jwt_secret: Bytes,
    head_block_hash: H256,
    max_tries: u32,
    block_production_interval_ms: u64,
    coinbase_address: Address,
) -> Result<(), EngineClientError> {
    let engine_client = EngineClient::new(&execution_client_auth_url, jwt_secret);

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
                tracing::debug!("ForkChoiceV3 response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending engine_forkchoiceUpdateV3 with PayloadAttributes: {error}"
                );
                tries += 1;
                continue;
            }
        };
        let payload_id = fork_choice_response
            .payload_id
            .expect("Failed to produce block: payload_id is None in ForkChoiceResponse");
        let execution_payload_response = match engine_client.engine_get_payload_v3(payload_id).await
        {
            Ok(response) => {
                tracing::debug!("GetPayloadV3 response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending engine_getPayloadV3: {error}"
                );
                tries += 1;
                continue;
            }
        };
        let payload_status = match engine_client
            .engine_new_payload_v3(
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
                tracing::debug!("NewPayloadV3 response: {response:?}");
                response
            }
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending engine_newPayloadV3: {error}"
                );
                tries += 1;
                continue;
            }
        };
        let produced_block_hash = if let Some(latest_valid_hash) = payload_status.latest_valid_hash
        {
            latest_valid_hash
        } else {
            tracing::error!("Failed to produce block: latest_valid_hash is None in PayloadStatus: {payload_status:?}");
            tries += 1;
            continue;
        };
        tracing::info!("Produced block {produced_block_hash:#x}");

        head_block_hash = produced_block_hash;

        tokio::time::sleep(tokio::time::Duration::from_millis(
            block_production_interval_ms,
        ))
        .await;
    }
    Err(EngineClientError::SystemFailed(format!("{}", max_tries)))
}
