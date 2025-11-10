use std::collections::{BTreeMap, BTreeSet};

use ethrex_common::{Address, H256, U256, types::BYTES_PER_BLOB};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, net::TcpListener};
use tracing::{debug, error, info, warn};

use crate::sequencer::{configs::SuperBuilderConfig, errors::SuperBuilderError};

#[derive(Deserialize, Serialize)]
pub enum Message {
    CommitBatch {
        chain_id: u64,
        #[serde(flatten)]
        batch_info: BatchInfo,
    },
    CrossChainTransaction {
        source_chain_id: u64,
        from: Address,
        to: Address,
        value: U256,
        gas_limit: U256,
        data: Vec<u8>,
    },
}

struct BatchInfo {
    batch_number: u64,
    new_state_root: H256,
    withdrawals_logs_merkle_root: H256,
    processed_privileged_transactions_rolling_hash: H256,
    last_block_hash: H256,
}

struct SuperBuilder {
    chains: BTreeSet<u64>,
}

pub async fn start_superbuilder(opts: SuperBuilderConfig) -> Result<(), SuperBuilderError> {
    let mut chains = BTreeSet::new();
    chains.insert(65536899u64);
    chains.insert(65536999);
    let mut batches = BTreeMap::new();
    batches.insert(
        65536899u64,
        BTreeMap::from_iter(vec![(
            0u64,
            BatchInfo {
                batch_number: 0,
                new_state_root: H256::zero(),
                withdrawals_logs_merkle_root: H256::zero(),
                processed_privileged_transactions_rolling_hash: H256::zero(),
                last_block_hash: H256::zero(),
            },
        )]),
    );
    batches.insert(65536999, 0);

    let listener =
        TcpListener::bind(format!("{}:{}", opts.listen_address, opts.listen_port)).await?;

    info!(
        "Starting TCP server at {}:{}.",
        opts.listen_address, opts.listen_port
    );
    loop {
        let res = listener.accept().await;
        match res {
            Ok((mut stream, addr)) => {
                let mut buffer = Vec::new();
                stream.read_to_end(&mut buffer).await?;
                let data: Result<Message, _> = serde_json::from_slice(&buffer);
                match data {
                    Ok(Message::CommitBatch {
                        chain_id,
                        batch_info:
                            BatchInfo {
                                batch_number,
                                new_state_root,
                                withdrawals_logs_merkle_root,
                                processed_privileged_transactions_rolling_hash,
                                last_block_hash,
                            },
                    }) => {
                        debug!(
                            chain_id,
                            batch_number,
                            ?new_state_root,
                            ?withdrawals_logs_merkle_root,
                            ?processed_privileged_transactions_rolling_hash,
                            ?last_block_hash,
                            "Received new CommitBatch message"
                        );
                        if !super_builder.chains.contains(&chain_id) {
                            error!(chain = chain_id, "Chain ID not registered");
                            continue;
                        }
                        batches
                    }
                    Ok(Message::CrossChainTransaction {
                        source_chain_id,
                        from,
                        to,
                        value,
                        gas_limit,
                        data,
                    }) => {
                        debug!(
                            source_chain_id,
                            ?from,
                            ?to,
                            ?value,
                            ?gas_limit,
                            ?data,
                            "Received new CrossChainTransaction message"
                        );
                        if super_builder.chains.contains(&source_chain_id) {
                            error!(chain = source_chain_id, "Chain ID not registered");
                            continue;
                        }
                    }
                    Err(_) => warn!("Error decoding data"),
                }
            }
            Err(e) => {
                error!("Failed to accept connection: {e}");
            }
        }

        debug!("Connection closed");
    }
    Ok(())
}
