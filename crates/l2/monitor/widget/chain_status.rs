use ethrex_common::{Address, H256};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rpc::{EthClient, clients::Overrides};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::SequencerConfig;

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

    pub async fn on_tick(&mut self, eth_client: &EthClient, rollup_client: &EthClient) {
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
        let last_committed_block =
            if last_committed_batch == 0 {
                0
            } else {
                rollup_client
            .get_batch_by_number(last_committed_batch)
            .await
            .unwrap_or_else(|err| {
                panic!("Failed to get last committed batch ({last_committed_batch}) data: {err}")
            })
            .batch
            .last_block
            };
        let last_verified_block = if last_verified_batch == 0 {
            0
        } else {
            rollup_client
                .get_batch_by_number(last_verified_batch)
                .await
                .unwrap_or_else(|err| {
                    panic!("Failed to get last verified batch ({last_verified_batch}) data: {err}")
                })
                .batch
                .last_block
        };
        let current_block = rollup_client
            .get_block_number()
            .await
            .expect("Failed to get latest L2 block")
            + 1;
        let current_batch = if sequencer_registry_address.is_some() {
            "NaN".to_string() // TODO: Implement current batch retrieval (should be last known + 1)
        } else {
            (last_committed_batch + 1).to_string()
        };

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

impl StatefulWidget for &mut GlobalChainStatusTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let constraints = vec![Constraint::Percentage(50), Constraint::Percentage(50)];
        let rows = self.items.iter().map(|(key, value)| {
            Row::new(vec![
                Span::styled(key, Style::default()),
                Span::styled(value, Style::default()),
            ])
        });
        let global_chain_status_table = Table::new(rows, constraints).block(
            Block::bordered()
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    "Global Chain Status",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
        );

        global_chain_status_table.render(area, buf, state);
    }
}
