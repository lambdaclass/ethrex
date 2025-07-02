#![expect(clippy::expect_used)]
#![expect(clippy::panic)]

use std::cmp::min;

use ethrex_common::{Address, U256};
use ethrex_rpc::{EthClient, types::receipt::RpcLog};
use keccak_hash::keccak;

pub async fn get_logs(
    last_l1_block_fetched: &mut U256,
    on_chain_proposer_address: Address,
    log_signature: &str,
    eth_client: &EthClient,
) -> Vec<RpcLog> {
    let last_l1_block_number = eth_client
        .get_block_number()
        .await
        .expect("Failed to get latest L1 block");

    let mut batch_committed_logs = Vec::new();
    while *last_l1_block_fetched < last_l1_block_number {
        let new_last_l1_fetched_block = min(*last_l1_block_fetched + 50, last_l1_block_number);

        // Fetch logs from the L1 chain for the BatchCommitted event.
        let logs = eth_client
            .get_logs(
                *last_l1_block_fetched + 1,
                new_last_l1_fetched_block,
                on_chain_proposer_address,
                keccak(log_signature.as_bytes()),
            )
            .await
            .unwrap_or_else(|_| panic!("Failed to fetch {log_signature} logs"));

        // Update the last L1 block fetched.
        *last_l1_block_fetched = new_last_l1_fetched_block;

        batch_committed_logs.extend_from_slice(&logs);
    }

    batch_committed_logs
}
