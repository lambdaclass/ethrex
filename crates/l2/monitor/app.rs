#![expect(clippy::expect_used)]
#![expect(clippy::panic)]

use std::cmp::min;
use std::fmt::Display;

use crossterm::event::{KeyCode, MouseEventKind};
use ethrex_common::{Address, H256, U256};
use ethrex_l2_sdk::COMMON_BRIDGE_L2_ADDRESS;
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rpc::EthClient;
use ethrex_rpc::clients::Overrides;
use ethrex_rpc::clients::eth::{BlockByNumber, RpcBatch};
use ethrex_rpc::types::block::{BlockBodyWrapper, RpcBlock};
use ethrex_rpc::types::receipt::RpcLog;
use ratatui::widgets::TableState;
use tui_logger::{TuiWidgetEvent, TuiWidgetState};

use crate::{DepositData, SequencerConfig, monitor};

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
        cfg: &SequencerConfig,
    ) -> Self {
        let sequencer_registry_address =
            if cfg.based.state_updater.sequencer_registry == Address::default() {
                None
            } else {
                Some(cfg.based.state_updater.sequencer_registry)
            };
        Self {
            state: TableState::default(),
            items: Self::refresh_items(
                eth_client,
                rollup_client,
                cfg.l1_committer.on_chain_proposer_address,
                sequencer_registry_address,
            )
            .await,
            on_chain_proposer_address: cfg.l1_committer.on_chain_proposer_address,
            sequencer_registry_address,
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

pub struct BatchesTable {
    pub state: TableState,
    // batch number | # blocks | # messages | commit tx hash | verify tx hash
    #[expect(clippy::type_complexity)]
    pub items: Vec<(u64, u64, usize, Option<H256>, Option<H256>)>,
    last_l1_block_fetched: u64,
    on_chain_proposer_address: Address,
}

impl BatchesTable {
    pub async fn new(
        on_chain_proposer_address: Address,
        eth_client: &EthClient,
        rollup_client: &EthClient,
    ) -> Self {
        let mut last_l1_block_fetched = 0;
        let items = Self::refresh_items(
            &mut last_l1_block_fetched,
            on_chain_proposer_address,
            eth_client,
            rollup_client,
        )
        .await;
        Self {
            state: TableState::default(),
            items,
            last_l1_block_fetched,
            on_chain_proposer_address,
        }
    }

    async fn on_tick(&mut self, eth_client: &EthClient, rollup_client: &EthClient) {
        let mut new_latest_batches = Self::refresh_items(
            &mut self.last_l1_block_fetched,
            self.on_chain_proposer_address,
            eth_client,
            rollup_client,
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
        last_l2_batch_fetched: &mut u64,
        on_chain_proposer_address: Address,
        eth_client: &EthClient,
        rollup_client: &EthClient,
    ) -> Vec<(u64, u64, usize, Option<H256>, Option<H256>)> {
        let new_batches = Self::get_batches(
            last_l2_batch_fetched,
            on_chain_proposer_address,
            eth_client,
            rollup_client,
        )
        .await;

        Self::process_batches(new_batches).await
    }

    async fn get_batches(
        last_l2_batch_known: &mut u64,
        on_chain_proposer_address: Address,
        eth_client: &EthClient,
        rollup_client: &EthClient,
    ) -> Vec<RpcBatch> {
        let last_l2_batch_number = eth_client
            .get_last_committed_batch(on_chain_proposer_address)
            .await
            .expect("Failed to get latest L2 batch");

        let mut new_batches = Vec::new();
        while *last_l2_batch_known < last_l2_batch_number {
            let new_last_l2_fetched_batch = min(*last_l2_batch_known + 1, last_l2_batch_number);

            let new_batch = rollup_client
                .get_batch_by_number(new_last_l2_fetched_batch)
                .await
                .unwrap_or_else(|err| {
                    panic!("Failed to get batch by number ({new_last_l2_fetched_batch}): {err}")
                });

            // Update the last L1 block fetched.
            *last_l2_batch_known = new_last_l2_fetched_batch;

            new_batches.push(new_batch);
        }

        new_batches
    }

    async fn process_batches(
        new_batches: Vec<RpcBatch>,
    ) -> Vec<(u64, u64, usize, Option<H256>, Option<H256>)> {
        let mut new_blocks_processed = new_batches
            .iter()
            .map(|batch| {
                (
                    batch.batch.number,
                    batch.batch.last_block - batch.batch.first_block + 1,
                    batch.batch.message_hashes.len(),
                    batch.batch.commit_tx,
                    batch.batch.verify_tx,
                )
            })
            .collect::<Vec<_>>();

        new_blocks_processed
            .sort_by(|(number_a, _, _, _, _), (number_b, _, _, _, _)| number_b.cmp(number_a));

        new_blocks_processed
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

pub struct L1ToL2MessagesTable {
    pub state: TableState,
    // Status | Kind | L1 tx hash | L2 tx hash | amount
    pub items: Vec<(L1ToL2MessageStatus, L1ToL2MessageKind, H256, H256, U256)>,
    last_l1_block_fetched: U256,
    common_bridge_address: Address,
}

#[derive(Debug, Clone)]
pub enum L1ToL2MessageStatus {
    Pending,
    Processed,
}

impl Display for L1ToL2MessageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            L1ToL2MessageStatus::Pending => write!(f, "Pending"),
            L1ToL2MessageStatus::Processed => write!(f, "Processed"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum L1ToL2MessageKind {
    Deposit,
    Message,
}

impl From<&DepositData> for L1ToL2MessageKind {
    fn from(data: &DepositData) -> Self {
        if data.from == COMMON_BRIDGE_L2_ADDRESS && data.to_address == COMMON_BRIDGE_L2_ADDRESS {
            Self::Deposit
        } else {
            Self::Message
        }
    }
}

impl Display for L1ToL2MessageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            L1ToL2MessageKind::Deposit => write!(f, "Deposit"),
            L1ToL2MessageKind::Message => write!(f, "Message"),
        }
    }
}

impl L1ToL2MessagesTable {
    pub async fn new(common_bridge_address: Address, eth_client: &EthClient) -> Self {
        let mut last_l1_block_fetched = eth_client
            .get_last_fetched_l1_block(common_bridge_address)
            .await
            .expect("Failed to get last fetched L1 block")
            .into();
        let items = Self::refresh_items(
            &mut last_l1_block_fetched,
            common_bridge_address,
            eth_client,
        )
        .await;
        Self {
            state: TableState::default(),
            items,
            last_l1_block_fetched,
            common_bridge_address,
        }
    }

    async fn on_tick(&mut self, eth_client: &EthClient) {
        let mut new_l1_to_l2_messages = Self::refresh_items(
            &mut self.last_l1_block_fetched,
            self.common_bridge_address,
            eth_client,
        )
        .await;

        let n_new_latest_batches = new_l1_to_l2_messages.len();

        if n_new_latest_batches > 50 {
            new_l1_to_l2_messages.truncate(50);
            self.items.extend_from_slice(&new_l1_to_l2_messages);
        } else {
            self.items.truncate(50 - n_new_latest_batches);
            self.items.extend_from_slice(&new_l1_to_l2_messages);
            self.items.rotate_right(n_new_latest_batches);
        }
    }

    async fn refresh_items(
        last_l1_block_fetched: &mut U256,
        common_bridge_address: Address,
        eth_client: &EthClient,
    ) -> Vec<(L1ToL2MessageStatus, L1ToL2MessageKind, H256, H256, U256)> {
        let logs = Self::get_logs(last_l1_block_fetched, common_bridge_address, eth_client).await;
        Self::process_logs(&logs, common_bridge_address, eth_client).await
    }

    async fn get_logs(
        last_l1_block_fetched: &mut U256,
        common_bridge_address: Address,
        eth_client: &EthClient,
    ) -> Vec<RpcLog> {
        monitor::utils::get_logs(
            last_l1_block_fetched,
            common_bridge_address,
            "L1ToL2Message(uint256,address,uint256,address,uint256,bytes,bytes32)",
            eth_client,
        )
        .await
    }

    async fn process_logs(
        logs: &[RpcLog],
        common_bridge_address: Address,
        eth_client: &EthClient,
    ) -> Vec<(L1ToL2MessageStatus, L1ToL2MessageKind, H256, H256, U256)> {
        let mut processed_logs = Vec::new();

        let pending_l1_to_l2_messages = eth_client
            .get_pending_deposit_logs(common_bridge_address)
            .await
            .expect("Failed to get pending L1 to L2 messages");

        for log in logs {
            let l1_to_l2_message =
                DepositData::from_log(log.log.clone()).expect("Failed to parse L1ToL2Message log");

            processed_logs.push((
                if pending_l1_to_l2_messages.contains(&log.transaction_hash) {
                    L1ToL2MessageStatus::Pending
                } else {
                    L1ToL2MessageStatus::Processed
                },
                L1ToL2MessageKind::from(&l1_to_l2_message),
                log.transaction_hash,
                l1_to_l2_message.deposit_tx_hash,
                l1_to_l2_message.value,
            ));
        }

        processed_logs
    }
}

pub struct EthrexMonitor<'a> {
    pub title: &'a str,
    pub should_quit: bool,
    pub tabs: TabsState<'a>,

    pub logger: TuiWidgetState,
    pub node_status: NodeStatusTable,
    pub global_chain_status: GlobalChainStatusTable,
    pub mempool: MempoolTable,
    pub batches_table: BatchesTable,
    pub blocks_table: BlocksTable,
    pub l1_to_l2_messages: L1ToL2MessagesTable,

    pub eth_client: EthClient,
    pub rollup_client: EthClient,
}

impl<'a> EthrexMonitor<'a> {
    pub async fn new(cfg: &SequencerConfig) -> Self {
        let eth_client = EthClient::new(cfg.eth.rpc_url.first().expect("No RPC URLs provided"))
            .expect("Failed to create EthClient");
        // TODO: De-hardcode the rollup client URL
        let rollup_client =
            EthClient::new("http://localhost:1729").expect("Failed to create RollupClient");

        EthrexMonitor {
            title: if cfg.based.based {
                "Based Ethrex Monitor"
            } else {
                "Ethrex Monitor"
            },
            should_quit: false,
            tabs: TabsState::new(vec!["Overview", "Logs"]),
            global_chain_status: GlobalChainStatusTable::new(&eth_client, &rollup_client, cfg)
                .await,
            logger: TuiWidgetState::new().set_default_display_level(tui_logger::LevelFilter::Info),
            node_status: NodeStatusTable::new(&rollup_client).await,
            mempool: MempoolTable::new(&rollup_client).await,
            batches_table: BatchesTable::new(
                cfg.l1_committer.on_chain_proposer_address,
                &eth_client,
                &rollup_client,
            )
            .await,
            blocks_table: BlocksTable::new(&rollup_client).await,
            l1_to_l2_messages: L1ToL2MessagesTable::new(cfg.l1_watcher.bridge_address, &eth_client)
                .await,
            eth_client,
            rollup_client,
        }
    }

    pub fn on_key_event(&mut self, code: KeyCode) {
        match code {
            KeyCode::Left => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::LeftKey),
                _ => {}
            },
            KeyCode::Down => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::DownKey),
                _ => {}
            },
            KeyCode::Up => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::UpKey),
                _ => {}
            },
            KeyCode::Right => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::RightKey),
                _ => {}
            },
            KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Char('h') => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::HideKey),
                _ => {}
            },
            KeyCode::Char('f') => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::FocusKey),
                _ => {}
            },
            KeyCode::Char('+') => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::PlusKey),
                _ => {}
            },
            KeyCode::Char('-') => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::MinusKey),
                _ => {}
            },
            KeyCode::Tab => self.tabs.next(),
            _ => {}
        }
    }

    pub fn on_mouse_event(&mut self, kind: MouseEventKind) {
        match kind {
            MouseEventKind::ScrollDown => self.logger.transition(TuiWidgetEvent::NextPageKey),
            MouseEventKind::ScrollUp => self.logger.transition(TuiWidgetEvent::PrevPageKey),
            _ => {}
        }
    }

    pub async fn on_tick(&mut self) {
        self.node_status.on_tick(&self.rollup_client).await;
        self.global_chain_status
            .on_tick(&self.eth_client, &self.rollup_client)
            .await;
        self.mempool.on_tick(&self.rollup_client).await;
        self.batches_table
            .on_tick(&self.eth_client, &self.rollup_client)
            .await;
        self.blocks_table.on_tick(&self.rollup_client).await;
        self.l1_to_l2_messages.on_tick(&self.eth_client).await;
    }
}
