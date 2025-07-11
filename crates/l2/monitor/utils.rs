use std::cmp::min;

use ethrex_common::{Address, U256};
use ethrex_rpc::{EthClient, types::receipt::RpcLog};
use keccak_hash::keccak;

pub async fn get_logs(
    last_block_fetched: &mut U256,
    emitter: Address,
    logs_signatures: Vec<&str>,
    client: &EthClient,
) -> Vec<RpcLog> {
    let last_block_number = client
        .get_block_number()
        .await
        .expect("Failed to get latest L1 block");

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
            .unwrap_or_else(|_| panic!("Failed to fetch {logs_signatures:?} logs from {emitter}"));

        // Update the last L1 block fetched.
        *last_block_fetched = new_last_l1_fetched_block;

        batch_committed_logs.extend_from_slice(&logs);
    }

    batch_committed_logs
}
