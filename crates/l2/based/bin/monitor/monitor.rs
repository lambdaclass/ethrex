#![expect(clippy::expect_used)]
#![expect(clippy::panic)]
#![expect(clippy::indexing_slicing)]

use std::cmp::min;

use ethrex_common::{Address, H256, U256};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rpc::EthClient;
use ethrex_rpc::clients::Overrides;
use ethrex_rpc::clients::eth::BlockByNumber;
use ethrex_rpc::types::block::{BlockBodyWrapper, RpcBlock};
use ethrex_rpc::types::receipt::RpcLog;
use keccak_hash::keccak;
use ratatui::widgets::TableState;

use crate::MonitorOptions;

pub struct TabsState<'a> {
    pub titles: Vec<&'a str>,
    pub index: usize,
}

impl<'a> TabsState<'a> {
    pub const fn new(titles: Vec<&'a str>) -> Self {
        Self { titles, index: 0 }
    }
    pub fn next(&mut self) {
        self.index = (self.index + 1) % self.titles.len();
    }

    pub fn previous(&mut self) {
        if self.index > 0 {
            self.index -= 1;
        } else {
            self.index = self.titles.len() - 1;
        }
    }
}

pub struct GlobalChainStatusTable {
    pub state: TableState,
    pub items: Vec<(String, String)>,
    pub on_chain_proposer_address: Address,
    pub sequencer_registry_address: Option<Address>,
}

impl GlobalChainStatusTable {
    pub async fn new(
        eth_client: &EthClient,
        rollup_client: &EthClient,
        opts: &MonitorOptions,
    ) -> Self {
        Self {
            state: TableState::default(),
            items: Self::refresh_items(
                eth_client,
                rollup_client,
                opts.on_chain_proposer_address,
                opts.sequencer_registry_address,
            )
            .await,
            on_chain_proposer_address: opts.on_chain_proposer_address,
            sequencer_registry_address: opts.sequencer_registry_address,
        }
    }

    async fn on_tick(&mut self, eth_client: &EthClient, rollup_client: &EthClient) {
        self.items = Self::refresh_items(
            eth_client,
            rollup_client,
            self.on_chain_proposer_address,
            self.sequencer_registry_address,
        )
        .await;
    }

    async fn refresh_items(
        eth_client: &EthClient,
        rollup_client: &EthClient,
        on_chain_proposer_address: Address,
        sequencer_registry_address: Option<Address>,
    ) -> Vec<(String, String)> {
        let last_update = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let lead_sequencer = if let Some(sequencer_registry_address) = sequencer_registry_address {
            let calldata = encode_calldata("leaderSequencer()", &[])
                .expect("Failed to encode leadSequencer calldata");

            let raw_lead_sequencer: H256 = eth_client
                .call(
                    sequencer_registry_address,
                    calldata.into(),
                    Overrides::default(),
                )
                .await
                .expect("Failed to call leaderSequencer")
                .parse()
                .unwrap_or_default();

            Address::from_slice(&raw_lead_sequencer.as_fixed_bytes()[12..])
        } else {
            Address::default()
        };
        let last_committed_batch = eth_client
            .get_last_committed_batch(on_chain_proposer_address)
            .await
            .expect("Failed to get last committed batch");
        let last_verified_batch = eth_client
            .get_last_verified_batch(on_chain_proposer_address)
            .await
            .expect("Failed to get last verified batch");
        let current_batch = if sequencer_registry_address.is_some() {
            "NaN".to_string() // TODO: Implement current batch retrieval (should be last known + 1)
        } else {
            (last_committed_batch + 1).to_string()
        };
        let last_committed_block = "NaN"; // TODO: Implement committed block retrieval
        let last_verified_block = "NaN"; // TODO: Implement verified block retrieval
        let current_block = rollup_client
            .get_block_number()
            .await
            .expect("Failed to get latest L2 block")
            + 1;

        if sequencer_registry_address.is_some() {
            vec![
                ("Last Update:".to_string(), last_update),
                (
                    "Lead Sequencer:".to_string(),
                    format!("{lead_sequencer:#x}"),
                ),
                ("Current Batch:".to_string(), current_batch.to_string()),
                ("Current Block:".to_string(), current_block.to_string()),
                (
                    "Last Committed Batch:".to_string(),
                    last_committed_batch.to_string(),
                ),
                (
                    "Last Committed Block:".to_string(),
                    last_committed_block.to_string(),
                ),
                (
                    "Last Verified Batch:".to_string(),
                    last_verified_batch.to_string(),
                ),
                (
                    "Last Verified Block:".to_string(),
                    last_verified_block.to_string(),
                ),
            ]
        } else {
            vec![
                ("Last Update:".to_string(), last_update),
                ("Current Batch:".to_string(), current_batch.to_string()),
                ("Current Block:".to_string(), current_block.to_string()),
                (
                    "Last Committed Batch:".to_string(),
                    last_committed_batch.to_string(),
                ),
                (
                    "Last Committed Block:".to_string(),
                    last_committed_block.to_string(),
                ),
                (
                    "Last Verified Batch:".to_string(),
                    last_verified_batch.to_string(),
                ),
                (
                    "Last Verified Block:".to_string(),
                    last_verified_block.to_string(),
                ),
            ]
        }
    }
}

pub struct NodeStatusTable {
    pub state: TableState,
    pub items: [(String, String); 5],
}

impl NodeStatusTable {
    pub async fn new(rollup_client: &EthClient) -> Self {
        Self {
            state: TableState::default(),
            items: Self::refresh_items(rollup_client).await,
        }
    }

    async fn on_tick(&mut self, rollup_client: &EthClient) {
        self.items = Self::refresh_items(rollup_client).await;
    }

    async fn refresh_items(rollup_client: &EthClient) -> [(String, String); 5] {
        let last_update = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let status = rollup_client
            .node_status()
            .await
            .expect("Failed to get node status");
        let last_known_batch = "NaN"; // TODO: Implement last known batch retrieval
        let last_known_block = rollup_client
            .get_block_number()
            .await
            .expect("Failed to get latest known L2 block");
        let follower_nodes = "NaN"; // TODO: Implement follower nodes retrieval

        [
            ("Last Update:".to_string(), last_update),
            ("Status:".to_string(), status.to_string()),
            (
                "Last Known Batch:".to_string(),
                last_known_batch.to_string(),
            ),
            (
                "Last Known Block:".to_string(),
                last_known_block.to_string(),
            ),
            ("Peers:".to_string(), follower_nodes.to_string()),
        ]
    }
}

pub struct CommittedBatchesTable {
    pub state: TableState,
    // batch number | commit tx hash
    pub items: Vec<(String, String)>,
    last_l1_block_fetched: U256,
    on_chain_proposer_address: Address,
}

impl CommittedBatchesTable {
    pub async fn new(eth_client: &EthClient, opts: &MonitorOptions) -> Self {
        let mut last_l1_block_fetched = eth_client
            .get_last_fetched_l1_block(opts.common_bridge_address)
            .await
            .expect("Failed to get last fetched L1 block")
            .into();
        let items = Self::refresh_items(
            &mut last_l1_block_fetched,
            opts.on_chain_proposer_address,
            eth_client,
        )
        .await;
        Self {
            state: TableState::default(),
            items,
            last_l1_block_fetched,
            on_chain_proposer_address: opts.on_chain_proposer_address,
        }
    }

    async fn on_tick(&mut self, eth_client: &EthClient) {
        let mut new_latest_batches = Self::refresh_items(
            &mut self.last_l1_block_fetched,
            self.on_chain_proposer_address,
            eth_client,
        )
        .await;

        let n_new_latest_batches = new_latest_batches.len();

        if n_new_latest_batches > 50 {
            new_latest_batches.truncate(50);
            self.items.extend_from_slice(&new_latest_batches);
        } else {
            self.items.truncate(50 - n_new_latest_batches);
            self.items.extend_from_slice(&new_latest_batches);
            self.items.rotate_right(n_new_latest_batches);
        }
    }

    async fn refresh_items(
        last_l1_block_fetched: &mut U256,
        on_chain_proposer_address: Address,
        eth_client: &EthClient,
    ) -> Vec<(String, String)> {
        let logs =
            Self::get_logs(last_l1_block_fetched, on_chain_proposer_address, eth_client).await;

        let processed_logs = Self::process_logs(&logs, eth_client).await;

        processed_logs
            .iter()
            .map(|(_log, batch_number, tx_hash)| {
                (format!("{batch_number}"), format!("{tx_hash:#x}"))
            })
            .collect()
    }

    async fn get_logs(
        last_l1_block_fetched: &mut U256,
        on_chain_proposer_address: Address,
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
                    keccak(b"BatchCommitted(bytes32)"),
                )
                .await
                .expect("Failed to fetch BatchCommitted logs");

            // Update the last L1 block fetched.
            *last_l1_block_fetched = new_last_l1_fetched_block;

            batch_committed_logs.extend_from_slice(&logs);
        }

        batch_committed_logs
    }

    async fn process_logs(logs: &[RpcLog], eth_client: &EthClient) -> Vec<(RpcLog, U256, H256)> {
        let mut log_txs = Vec::new();

        for log in logs {
            if let Some(tx) = eth_client
                .get_transaction_by_hash(log.transaction_hash)
                .await
                .unwrap_or_else(|_| {
                    panic!("Failed to get transaction by hash {}", log.transaction_hash)
                })
            {
                let calldata_derived_batch_number = U256::from_big_endian(&tx.data[4..36]);

                log_txs.push((log.clone(), calldata_derived_batch_number, tx.hash));
            }
        }

        log_txs.sort_by(|(_, batch_number_a, _), (_, batch_number_b, _)| {
            batch_number_b.cmp(batch_number_a)
        });

        log_txs
    }
}

pub struct BlocksTable {
    pub state: TableState,
    // block number | #transactions | hash | coinbase | gas | blob gas | size
    pub items: Vec<(String, String, String, String, String, String, String)>,
    last_l2_block_known: U256,
}

impl BlocksTable {
    pub async fn new(rollup_client: &EthClient) -> Self {
        let mut last_l2_block_known = U256::zero();
        let items = Self::refresh_items(&mut last_l2_block_known, rollup_client).await;
        Self {
            state: TableState::default(),
            items,
            last_l2_block_known,
        }
    }

    async fn on_tick(&mut self, rollup_client: &EthClient) {
        let mut new_blocks =
            Self::refresh_items(&mut self.last_l2_block_known, rollup_client).await;

        let n_new_blocks = new_blocks.len();

        if n_new_blocks > 50 {
            new_blocks.truncate(50);
            self.items.extend_from_slice(&new_blocks);
        } else {
            self.items.truncate(50 - n_new_blocks);
            self.items.extend_from_slice(&new_blocks);
            self.items.rotate_right(n_new_blocks);
        }
    }

    async fn refresh_items(
        last_l2_block_known: &mut U256,
        rollup_client: &EthClient,
    ) -> Vec<(String, String, String, String, String, String, String)> {
        let new_blocks = Self::get_blocks(last_l2_block_known, rollup_client).await;

        let new_blocks_processed = Self::process_blocks(new_blocks).await;

        new_blocks_processed
            .iter()
            .map(|(number, n_txs, hash, coinbase, gas, blob_gas, size)| {
                (
                    number.to_string(),
                    n_txs.to_string(),
                    format!("{hash:#x}"),
                    format!("{coinbase:#x}"),
                    gas.to_string(),
                    blob_gas.map_or("0".to_string(), |bg| bg.to_string()),
                    size.to_string(),
                )
            })
            .collect()
    }

    async fn get_blocks(
        last_l2_block_known: &mut U256,
        rollup_client: &EthClient,
    ) -> Vec<RpcBlock> {
        let last_l2_block_number = rollup_client
            .get_block_number()
            .await
            .expect("Failed to get latest L2 block");

        let mut new_blocks = Vec::new();
        while *last_l2_block_known < last_l2_block_number {
            let new_last_l1_fetched_block = min(*last_l2_block_known + 1, last_l2_block_number);

            let new_block = rollup_client
                .get_block_by_number(BlockByNumber::Number(new_last_l1_fetched_block.as_u64()))
                .await
                .unwrap_or_else(|_| {
                    panic!("Failed to get block  by number ({new_last_l1_fetched_block})")
                });

            // Update the last L1 block fetched.
            *last_l2_block_known = new_last_l1_fetched_block;

            new_blocks.push(new_block);
        }

        new_blocks
    }

    async fn process_blocks(
        new_blocks: Vec<RpcBlock>,
    ) -> Vec<(u64, usize, H256, Address, u64, Option<u64>, u64)> {
        let mut new_blocks_processed = new_blocks
            .iter()
            .map(|block| {
                let n_txs = match &block.body {
                    BlockBodyWrapper::Full(full_block_body) => full_block_body.transactions.len(),
                    BlockBodyWrapper::OnlyHashes(only_hashes_block_body) => {
                        only_hashes_block_body.transactions.len()
                    }
                };
                (
                    block.header.number,
                    n_txs,
                    block.header.hash(),
                    block.header.coinbase,
                    block.header.gas_used,
                    block.header.blob_gas_used,
                    block.size,
                )
            })
            .collect::<Vec<_>>();

        new_blocks_processed.sort_by(
            |(number_a, _, _, _, _, _, _), (number_b, _, _, _, _, _, _)| number_b.cmp(number_a),
        );

        new_blocks_processed
    }
}

pub struct MempoolTable {
    pub state: TableState,
    // hash | sender | nonce
    pub items: Vec<(String, String, String)>,
}

impl MempoolTable {
    pub async fn new(rollup_client: &EthClient) -> Self {
        Self {
            state: TableState::default(),
            items: Self::refresh_items(rollup_client).await,
        }
    }

    async fn on_tick(&mut self, rollup_client: &EthClient) {
        self.items = Self::refresh_items(rollup_client).await;
    }

    async fn refresh_items(rollup_client: &EthClient) -> Vec<(String, String, String)> {
        let mempool = rollup_client
            .tx_pool_content()
            .await
            .expect("Failed to get mempool content");

        let mut pending_txs = mempool
            .pending
            .iter()
            .flat_map(|(sender, txs_sorted_by_nonce)| {
                txs_sorted_by_nonce.iter().map(|(nonce, tx)| {
                    (
                        format!("{:#x}", tx.hash),
                        format!("{:#x}", *sender),
                        format!("{nonce}"),
                    )
                })
            })
            .collect::<Vec<_>>();

        pending_txs.sort_by(|(_, sender_a, nonce_a), (_, sender_b, nonce_b)| {
            sender_a.cmp(sender_b).then(nonce_a.cmp(nonce_b))
        });

        pending_txs
    }
}

pub struct EthrexMonitor<'a> {
    pub title: &'a str,
    pub should_quit: bool,
    pub tabs: TabsState<'a>,

    pub node_status: NodeStatusTable,
    pub global_chain_status: GlobalChainStatusTable,
    pub mempool: MempoolTable,
    pub committed_batches: CommittedBatchesTable,
    pub blocks_table: BlocksTable,

    pub eth_client: EthClient,
    pub rollup_client: EthClient,
}

impl<'a> EthrexMonitor<'a> {
    pub async fn new(opts: &MonitorOptions) -> Self {
        let eth_client = EthClient::new(&opts.l1_rpc_url).expect("Failed to create EthClient");
        let rollup_client =
            EthClient::new(&opts.l2_rpc_url).expect("Failed to create RollupClient");

        EthrexMonitor {
            title: if opts.based {
                "Based Ethrex Monitor"
            } else {
                "Ethrex Monitor"
            },
            should_quit: false,
            tabs: TabsState::new(vec!["Overview"]),
            global_chain_status: GlobalChainStatusTable::new(&eth_client, &rollup_client, opts)
                .await,
            node_status: NodeStatusTable::new(&rollup_client).await,
            mempool: MempoolTable::new(&rollup_client).await,
            committed_batches: CommittedBatchesTable::new(&eth_client, opts).await,
            blocks_table: BlocksTable::new(&rollup_client).await,
            eth_client,
            rollup_client,
        }
    }

    pub fn on_up(&mut self) {}

    pub fn on_down(&mut self) {}

    pub fn on_right(&mut self) {
        self.tabs.next();
    }

    pub fn on_left(&mut self) {
        self.tabs.previous();
    }

    pub fn on_key(&mut self, c: char) {
        #[expect(clippy::single_match)]
        match c {
            'Q' => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    pub async fn on_tick(&mut self) {
        self.node_status.on_tick(&self.rollup_client).await;
        self.global_chain_status
            .on_tick(&self.eth_client, &self.rollup_client)
            .await;
        self.mempool.on_tick(&self.rollup_client).await;
        self.committed_batches.on_tick(&self.eth_client).await;
        self.blocks_table.on_tick(&self.rollup_client).await;
    }
}
