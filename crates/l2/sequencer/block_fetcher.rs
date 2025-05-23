use std::{cmp::min, ops::Deref, sync::Arc, time::Duration};

use ethrex_blockchain::Blockchain;
use ethrex_common::{types::Block, Address, U256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rpc::EthClient;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use keccak_hash::keccak;
use tokio::{sync::Mutex, time::sleep};
use tracing::{error, info};

use crate::SequencerConfig;

use super::{
    errors::{BlockFetcherError, SequencerError},
    execution_cache::ExecutionCache,
    SequencerState,
};

pub struct BlockFetcher {
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    bridge_address: Address,
    store: Store,
    rollup_store: StoreRollup,
    blockchain: Arc<Blockchain>,
    execution_cache: Arc<ExecutionCache>,
    sequencer_state: Arc<Mutex<SequencerState>>,
    fetch_interval_ms: u64,
    last_l1_block_fetched: U256,
    max_block_step: U256,
}

pub async fn start_block_fetcher(
    store: Store,
    blockchain: Arc<Blockchain>,
    execution_cache: Arc<ExecutionCache>,
    sequencer_state: Arc<Mutex<SequencerState>>,
    rollup_store: StoreRollup,
    cfg: SequencerConfig,
) -> Result<(), SequencerError> {
    let mut block_fetcher = BlockFetcher::new(
        &cfg,
        store.clone(),
        rollup_store,
        blockchain,
        execution_cache,
        sequencer_state,
    )?;
    block_fetcher.run().await;
    Ok(())
}

impl BlockFetcher {
    pub fn new(
        cfg: &SequencerConfig,
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
        execution_cache: Arc<ExecutionCache>,
        sequencer_state: Arc<Mutex<SequencerState>>,
    ) -> Result<Self, BlockFetcherError> {
        Ok(Self {
            eth_client: EthClient::new_with_multiple_urls(cfg.eth.rpc_url.clone())?,
            on_chain_proposer_address: cfg.l1_committer.on_chain_proposer_address,
            bridge_address: cfg.l1_watcher.bridge_address,
            store,
            rollup_store,
            blockchain,
            execution_cache,
            sequencer_state,
            fetch_interval_ms: cfg.block_producer.block_time_ms,
            last_l1_block_fetched: U256::zero(),
            max_block_step: cfg.l1_watcher.max_block_step, // TODO: block fetcher config
        })
    }

    pub async fn run(&mut self) {
        loop {
            if let Err(err) = self.main_logic().await {
                error!("Block Producer Error: {}", err);
            }

            sleep(Duration::from_millis(self.fetch_interval_ms)).await;
        }
    }

    pub async fn main_logic(&mut self) -> Result<(), BlockFetcherError> {
        let sequencer_state_clone = self.sequencer_state.clone();
        let sequencer_state_mutex_guard = sequencer_state_clone.lock().await;
        match sequencer_state_mutex_guard.deref() {
            SequencerState::Sequencing => Ok(()),
            SequencerState::Following => self.fetch().await,
        }
    }

    async fn fetch(&mut self) -> Result<(), BlockFetcherError> {
        while !self.node_is_up_to_date().await? {
            info!("Node is not up to date, waiting for it to sync...");

            let last_l2_block_number_known = self.store.get_latest_block_number().await?;

            let last_l2_batch_number_known = self
                .rollup_store
                .get_batch_number_by_block(last_l2_block_number_known)
                .await?
                .ok_or(BlockFetcherError::InternalError(format!(
                    "Failed to get last batch number known for block {last_l2_block_number_known}"
                )))?;

            let last_l2_committed_batch_number = self
                .eth_client
                .get_last_committed_batch(self.on_chain_proposer_address)
                .await?;

            let l2_batches_behind =
                last_l2_committed_batch_number.checked_sub(last_l2_batch_number_known).ok_or(
                    BlockFetcherError::InternalError(
                        "Failed to calculate batches behind. Last batch number known is greater than last committed batch number.".to_string(),
                    ),
                )?;

            info!(
                "Node is {l2_batches_behind} batches behind. Last committed batch number: {last_l2_committed_batch_number}, last batch number known: {last_l2_batch_number_known}",
            );

            if self.last_l1_block_fetched.is_zero() {
                self.last_l1_block_fetched = self
                    .eth_client
                    .get_last_fetched_l1_block(self.bridge_address)
                    .await?
                    .into();
            }

            let last_l1_block_number = self.eth_client.get_block_number().await?;

            let mut missing_batches_logs = Vec::new();

            while self.last_l1_block_fetched < last_l1_block_number {
                let new_last_l1_fetched_block = min(
                    self.last_l1_block_fetched + self.max_block_step,
                    last_l1_block_number,
                );

                info!(
                    "Fetching logs from block {} to {}",
                    self.last_l1_block_fetched + 1,
                    new_last_l1_fetched_block
                );

                let logs = self
                    .eth_client
                    .get_logs(
                        self.last_l1_block_fetched + 1,
                        new_last_l1_fetched_block,
                        self.on_chain_proposer_address,
                        keccak(b"BatchCommitted(uint256,bytes32)"),
                    )
                    .await?;

                let last_block_number_known = self.store.get_latest_block_number().await?;

                let last_batch_number_known = self
                    .rollup_store
                    .get_batch_number_by_block(last_block_number_known)
                    .await?
                    .ok_or(BlockFetcherError::InternalError(format!(
                        "Failed to get last batch number known for block {last_block_number_known}"
                    )))?;

                for batch_committed_log in logs.into_iter() {
                    let committed_batch_number = U256::from_big_endian(
                        batch_committed_log
                            .log
                            .topics
                            .get(1)
                            .ok_or(BlockFetcherError::InternalError(
                                "Failed to get committed batch number from BatchCommitted log"
                                    .to_string(),
                            ))?
                            .as_bytes(),
                    );

                    if committed_batch_number > last_batch_number_known.into() {
                        missing_batches_logs.push(batch_committed_log);
                    }
                }

                self.last_l1_block_fetched = new_last_l1_fetched_block;

                sleep(Duration::from_millis(self.fetch_interval_ms)).await;
            }

            for batch_committed_log in missing_batches_logs {
                let tx = self
                    .eth_client
                    .get_transaction_by_hash(batch_committed_log.transaction_hash)
                    .await?
                    .ok_or(BlockFetcherError::InternalError(format!(
                        "Failed to get transaction receipt for transaction {:x}",
                        batch_committed_log.transaction_hash
                    )))?;
                // dbg!(tx.data.len());
                // dbg!(hex::encode(&tx.data));
                // dbg!(hex::encode(tx.data.clone()));
                // let a = tx.data.strip_prefix(b"0x").unwrap_or(&tx.data);

                // let (block, _) = BlockBody::decode_unfinished(a).unwrap();
                // dbg!(block);
                decode(&tx.data);

                // TODO: Get from calldata
                let batch_withdrawal_hashes = Vec::new();
                // TODO: Get from calldata or log
                let batch_number = u64::default();
                // TODO: Get from calldata
                let batch = Vec::new();

                for block in batch.iter() {
                    self.blockchain.add_block(block).await?;

                    info!(
                        "Fetched new block {:#x} from transaction {:#x}",
                        block.hash(),
                        batch_committed_log.transaction_hash
                    );
                }

                self.rollup_store
                    .store_batch(
                        batch_number,
                        batch.first().unwrap().header.number,
                        batch.last().unwrap().header.number,
                        batch_withdrawal_hashes,
                    )
                    .await?;

                info!(
                    "Stored batch {} from transaction {:#x}",
                    batch_number, batch_committed_log.transaction_hash
                );
            }

            sleep(Duration::from_millis(self.fetch_interval_ms)).await;
        }
        info!("Node is up to date");
        Ok(())
    }

    async fn node_is_up_to_date(&self) -> Result<bool, BlockFetcherError> {
        let last_committed_batch_number = self
            .eth_client
            .get_last_committed_batch(self.on_chain_proposer_address)
            .await?;

        self.rollup_store
            .contains_batch(&last_committed_batch_number)
            .await
            .map_err(BlockFetcherError::StoreError)
    }

    // async fn yyy(&self) -> Result<(), BlockFetcherError> {
    // let last_block_number_known = self.store.get_latest_block_number().await?;

    // let version = 3;

    // let head_hash = self
    //     .store
    //     .get_block_header(last_block_number_known)?
    //     .ok_or(BlockFetcherError::InternalError(
    //         "Failed to get last block known header".to_string(),
    //     ))?
    //     .compute_block_hash();

    // // Assumed to be the Sequencer that committed the block.
    // // The fee recipient is the sender of the transaction that committed the block.
    // // To get the transaction that committed the block in L1, we need to watch for
    // // BatchCommitted events.
    // let fee_recipient

    // let args = BuildPayloadArgs {
    //     parent: head_hash,
    //     timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
    //     fee_recipient: self.coinbase_address,
    //     random: H256::zero(),
    //     withdrawals: Default::default(),
    //     beacon_root: Some(H256::zero()),
    //     version,
    //     elasticity_multiplier: ELASTICITY_MULTIPLIER,
    // };

    //     Ok(())
    // }
}

// Necesitamos block, withdrawal hash y los batches
#[allow(clippy::indexing_slicing)]
#[allow(clippy::unwrap_used)]
#[allow(clippy::as_conversions)]
fn decode(data: &[u8]) {
    // function commitBatch(
    //     uint256 batchNumber,
    //     bytes32 newStateRoot,
    //     bytes32 stateDiffKZGVersionedHash,
    //     bytes32 withdrawalsLogsMerkleRoot,
    //     bytes32 processedDepositLogsRollingHash,
    //     bytes[] calldata _hexEncodedBlocks
    // ) external;

    // data =   4 bytes (function selector) 0..4
    //          || 8 bytes (batch number)   4..36
    //          || 32 bytes (new state root) 36..68
    //          || 32 bytes (state diff KZG versioned hash) 68..100
    //          || 32 bytes (withdrawals logs merkle root) 100..132
    //          || 32 bytes (processed deposit logs rolling hash) 132..164
    // println!("offset array: {:?}", hex::encode(&data[164..196])); // offset array
    // println!("length array: {:?}", hex::encode(&data[196..228])); // length array
    let a = U256::from_big_endian(&data[196..228]).as_u64();
    let base = 228;
    for i in 0..a {
        let b: usize = base + i as usize * 32;
        let string_offset = U256::from_big_endian(&data[b..b + 32]).as_usize();
        // println!("string offset: {:?}", hex::encode(&data[b..b + 32])); // string offset
        let string_len =
            U256::from_big_endian(&data[base + string_offset..base + string_offset + 32])
                .as_usize();
        // dbg!(string_len);
        // println!(
        //     "string len: {:?}",
        //     hex::encode(&data[base + string_offset..base + string_offset + 32])
        // ); // string len
        // println!(
        //     "string: {:?}",
        //     hex::encode(&data[base + string_offset + 32..base + string_offset + 32 + string_len])
        // ); // string
        let (block, _) = Block::decode_unfinished(
            &data[base + string_offset + 32..base + string_offset + 32 + string_len],
        )
        .unwrap();
        // dbg!(block);
    }
}

/*

..  || offset array || length_array (n)
    --
    || bytes_offset_0 || bytes_offset_1 || ... || bytes_offset_n ||
    || bytes_length_0 || bytes_0
    || bytes_length_1 || bytes_1
    ...
    || bytes_length_n || bytes_n ||

*/
