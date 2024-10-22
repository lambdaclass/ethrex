use crate::utils::engine_client::EngineClient;
use bytes::Bytes;
use ethereum_rust_rpc::types::fork_choice::{ForkChoiceState, PayloadAttributesV3};
use ethereum_types::H256;
use std::{
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

pub async fn start_block_producer(
    execution_client_auth_url: SocketAddr,
    jwt_secret: Bytes,
    head_block_hash: H256,
    max_tries: u32,
) {
    let engine_client =
        EngineClient::new(&format!("http://{execution_client_auth_url}"), jwt_secret);

    let mut head_block_hash: H256 = head_block_hash;
    let mut tries = 0;
    while tries < max_tries {
        tracing::info!("Producing block");
        let fork_choice_state = ForkChoiceState {
            head_block_hash,
            safe_block_hash: head_block_hash,
            finalized_block_hash: head_block_hash,
        };
        let payload_attributes = PayloadAttributesV3 {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Failed to produce block: error getting current timestamp")
                .as_secs(),
            ..Default::default()
        };
        let fork_choice_response = match engine_client
            .engine_forkchoice_updated_v3(fork_choice_state, Some(payload_attributes))
            .await
        {
            Ok(response) => response,
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
            Ok(response) => response,
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
                Default::default(),
                Default::default(),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::error!(
                    "Failed to produce block: error sending engine_newPayloadV3: {error}"
                );
                tries += 1;
                continue;
            }
        };
        let produced_block_hash = payload_status
            .latest_valid_hash
            .expect("Failed to produce block: latest_valid_hash is None in PayloadStatus");
        tracing::info!("Produced block {produced_block_hash:#x}");

        head_block_hash = produced_block_hash;

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
