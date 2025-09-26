use std::cmp::min;

use ethrex_common::utils::keccak;
use ethrex_common::{Address, U256};
use ethrex_rpc::{EthClient, types::receipt::RpcLog};

use crate::sequencer::errors::MonitorError;

pub async fn get_logs(
    last_block_fetched: &mut U256,
    emitter: Address,
    logs_signatures: Vec<&str>,
    client: &EthClient,
) -> Result<Vec<RpcLog>, MonitorError> {
    let last_block_number = client
        .get_block_number()
        .await
        .map_err(|_| MonitorError::GetLatestBlock)?;

    let mut batch_committed_logs = Vec::new();
    while *last_block_fetched < last_block_number {
        let new_last_l1_fetched_block = min(*last_block_fetched + 50, last_block_number);

        // Fetch logs from the L1 chain for the BatchCommitted event.
        let logs = client
            .get_logs(
                *last_block_fetched + 1,
                new_last_l1_fetched_block,
                emitter,
                logs_signatures
                    .iter()
                    .map(|log_signature| keccak(log_signature.as_bytes()))
                    .collect(),
            )
            .await
            .map_err(|e| {
                MonitorError::LogsSignatures(
                    logs_signatures.iter().map(|s| s.to_string()).collect(),
                    emitter,
                    e,
                )
            })?;

        // Update the last L1 block fetched.
        *last_block_fetched = new_last_l1_fetched_block;

        batch_committed_logs.extend_from_slice(&logs);
    }

    Ok(batch_committed_logs)
}

pub trait SelectableScroller {
    fn selected(&mut self, is_selected: bool);
    fn scroll_up(&mut self);
    fn scroll_down(&mut self);
}
