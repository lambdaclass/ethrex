use ethrex_common::{Address, H256};
use ethrex_l2_sdk::{calldata::encode_calldata, get_last_committed_batch, get_last_verified_batch};
use ethrex_rpc::{EthClient, clients::Overrides};
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::{SequencerConfig, sequencer::errors::MonitorError};

#[derive(Clone, Default)]
pub struct GlobalChainStatusTable {
    pub state: TableState,
    pub items: Vec<(String, String)>,
    pub on_chain_proposer_address: Address,
    pub sequencer_registry_address: Option<Address>,
}

impl GlobalChainStatusTable {
    pub fn new(cfg: &SequencerConfig) -> Self {
        let sequencer_registry_address =
            if cfg.based.state_updater.sequencer_registry == Address::default() {
                None
            } else {
                Some(cfg.based.state_updater.sequencer_registry)
            };
        Self {
            on_chain_proposer_address: cfg.l1_committer.on_chain_proposer_address,
            sequencer_registry_address,
            ..Default::default()
        }
    }

    pub async fn on_tick(
        &mut self,
        eth_client: &EthClient,
        store: &Store,
        rollup_store: &StoreRollup,
    ) -> Result<(), MonitorError> {
        self.items = Self::refresh_items(
            eth_client,
            self.on_chain_proposer_address,
            self.sequencer_registry_address,
            store,
            rollup_store,
        )
        .await?;
        Ok(())
    }

    async fn refresh_items(
        eth_client: &EthClient,
        on_chain_proposer_address: Address,
        sequencer_registry_address: Option<Address>,
        store: &Store,
        rollup_store: &StoreRollup,
    ) -> Result<Vec<(String, String)>, MonitorError> {
        let last_update = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let lead_sequencer = if let Some(sequencer_registry_address) = sequencer_registry_address {
            let calldata = encode_calldata("leaderSequencer()", &[])
                .map_err(MonitorError::CalldataEncodeError)?;

            let raw_lead_sequencer: H256 = eth_client
                .call(
                    sequencer_registry_address,
                    calldata.into(),
                    Overrides::default(),
                )
                .await
                .map_err(MonitorError::EthClientError)?
                .parse()
                .unwrap_or_default();

            Address::from_slice(&raw_lead_sequencer.as_fixed_bytes()[12..])
        } else {
            Address::default()
        };

        let last_committed_batch = get_last_committed_batch(eth_client, on_chain_proposer_address)
            .await
            .map_err(|_| MonitorError::GetLatestCommittedBatch)?;

        let last_verified_batch = get_last_verified_batch(eth_client, on_chain_proposer_address)
            .await
            .map_err(|_| MonitorError::GetLatestVerifiedBatch)?;

        let last_committed_block = if last_committed_batch == 0 {
            0
        } else {
            match rollup_store
                .get_block_numbers_by_batch(last_committed_batch)
                .await
                .map_err(|e| MonitorError::GetBlocksByBatch(last_committed_batch, e))?
            {
                Some(block_numbers) => block_numbers.last().copied().unwrap_or(0),
                None => 0,
            }
        };

        let last_verified_block = if last_verified_batch == 0 {
            0
        } else {
            match rollup_store
                .get_block_numbers_by_batch(last_verified_batch)
                .await
                .map_err(|e| MonitorError::GetBlocksByBatch(last_verified_batch, e))?
            {
                Some(block_numbers) => block_numbers.last().copied().unwrap_or(0),
                None => 0,
            }
        };

        let current_block = store
            .get_latest_block_number()
            .await
            .map_err(|_| MonitorError::GetLatestBlock)?
            + 1;

        let current_batch = if sequencer_registry_address.is_some() {
            "NaN".to_string() // TODO: Implement current batch retrieval (should be last known + 1)
        } else {
            (last_committed_batch + 1).to_string()
        };

        let items = if sequencer_registry_address.is_some() {
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
        };
        Ok(items)
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
