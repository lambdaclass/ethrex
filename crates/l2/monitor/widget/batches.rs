use ethrex_common::{Address, H256, types::batch::Batch};
use ethrex_rpc::EthClient;
use ethrex_storage_rollup::StoreRollup;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::{
    monitor::{
        utils::SelectableScroller,
        widget::{HASH_LENGTH_IN_DIGITS, NUMBER_LENGTH_IN_DIGITS},
    },
    sequencer::errors::MonitorError,
};

const BATCH_WINDOW_SIZE: usize = 50;

#[derive(Clone)]
pub struct BatchLine {
    number: u64,
    block_count: u64,
    message_count: usize,
    commit_tx: Option<H256>,
    verify_tx: Option<H256>,
}

#[derive(Clone, Default)]
pub struct BatchesTable {
    pub state: TableState,
    pub items: Vec<BatchLine>,
    last_l1_block_fetched: u64,
    on_chain_proposer_address: Address,
    selected: bool,
}

impl BatchesTable {
    pub fn new(on_chain_proposer_address: Address) -> Self {
        Self {
            on_chain_proposer_address,
            ..Default::default()
        }
    }

    pub async fn on_tick(
        &mut self,
        eth_client: &EthClient,
        rollup_store: &StoreRollup,
    ) -> Result<(), MonitorError> {
        let mut new_latest_batches = Self::fetch_new_items(
            &mut self.last_l1_block_fetched,
            self.on_chain_proposer_address,
            eth_client,
            rollup_store,
        )
        .await?;
        new_latest_batches.truncate(BATCH_WINDOW_SIZE);

        let n_new_latest_batches = new_latest_batches.len();
        self.items
            .truncate(BATCH_WINDOW_SIZE - n_new_latest_batches);
        self.refresh_items(rollup_store).await?;
        self.items.extend_from_slice(&new_latest_batches);
        self.items.rotate_right(n_new_latest_batches);

        Ok(())
    }

    async fn refresh_items(&mut self, rollup_store: &StoreRollup) -> Result<(), MonitorError> {
        if self.items.is_empty() {
            return Ok(());
        }

        let mut refreshed_batches = Vec::new();

        for batch in self.items.iter() {
            if batch.commit_tx.is_some() && batch.verify_tx.is_some() {
                refreshed_batches.push(batch.clone());
            } else {
                let batch_number = batch.number;
                let new_batch = rollup_store
                    .get_batch(batch_number)
                    .await
                    .map_err(|e| MonitorError::GetBatchByNumber(batch_number, e))?
                    .ok_or(MonitorError::BatchNotFound(batch_number))?;

                refreshed_batches.push(Self::process_batch(&new_batch));
            }
        }

        Self::reorder_batches(&mut refreshed_batches);

        self.items = refreshed_batches;

        Ok(())
    }

    async fn fetch_new_items(
        last_l2_batch_fetched: &mut u64,
        on_chain_proposer_address: Address,
        eth_client: &EthClient,
        rollup_store: &StoreRollup,
    ) -> Result<Vec<BatchLine>, MonitorError> {
        let last_l2_batch_number = eth_client
            .get_last_committed_batch(on_chain_proposer_address)
            .await
            .map_err(|_| MonitorError::GetLatestBatch)?;

        *last_l2_batch_fetched = (*last_l2_batch_fetched).max(
            last_l2_batch_number.saturating_sub(
                BATCH_WINDOW_SIZE
                    .try_into()
                    .map_err(|_| MonitorError::BatchWindow)?,
            ),
        );

        let new_batches =
            Self::get_batches(last_l2_batch_fetched, last_l2_batch_number, rollup_store).await?;

        Ok(Self::process_batches(new_batches))
    }

    async fn get_batches(
        from: &mut u64,
        to: u64,
        rollup_store: &StoreRollup,
    ) -> Result<Vec<Batch>, MonitorError> {
        let mut new_batches = Vec::new();

        for batch_number in *from + 1..=to {
            let batch = rollup_store
                .get_batch(batch_number)
                .await
                .map_err(|e| MonitorError::GetBatchByNumber(batch_number, e))?
                .ok_or(MonitorError::BatchNotFound(batch_number))?;

            *from = batch_number;

            new_batches.push(batch);
        }

        Ok(new_batches)
    }

    fn process_batch(batch: &Batch) -> BatchLine {
        BatchLine {
            number: batch.number,
            block_count: batch.last_block - batch.first_block + 1,
            message_count: batch.message_hashes.len(),
            commit_tx: batch.commit_tx,
            verify_tx: batch.verify_tx,
        }
    }

    fn reorder_batches(new_blocks_processed: &mut [BatchLine]) {
        new_blocks_processed.sort_by(|a, b| b.number.cmp(&a.number));
    }

    fn process_batches(new_batches: Vec<Batch>) -> Vec<BatchLine> {
        let mut new_blocks_processed = new_batches
            .iter()
            .map(Self::process_batch)
            .collect::<Vec<_>>();

        Self::reorder_batches(&mut new_blocks_processed);

        new_blocks_processed
    }
}

impl StatefulWidget for &mut BatchesTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let constraints = vec![
            Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(17),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
        ];
        let rows = self.items.iter().map(|batch| {
            Row::new(vec![
                Span::styled(batch.number.to_string(), Style::default()),
                Span::styled(batch.block_count.to_string(), Style::default()),
                Span::styled(batch.message_count.to_string(), Style::default()),
                Span::styled(
                    batch
                        .commit_tx
                        .map_or_else(|| "Uncommitted".to_string(), |hash| format!("{hash:#x}")),
                    Style::default(),
                ),
                Span::styled(
                    batch
                        .verify_tx
                        .map_or_else(|| "Unverified".to_string(), |hash| format!("{hash:#x}")),
                    Style::default(),
                ),
            ])
        });
        let committed_batches_table = Table::new(rows, constraints)
            .header(
                Row::new(vec![
                    "Number",
                    "# Blocks",
                    "# L2 to L1 Messages",
                    "Commit Tx Hash",
                    "Verify Tx Hash",
                ])
                .style(Style::default()),
            )
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(if self.selected {
                        Color::Magenta
                    } else {
                        Color::Cyan
                    }))
                    .title(Span::styled(
                        "L2 Batches",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        committed_batches_table.render(area, buf, state);
    }
}

impl SelectableScroller for BatchesTable {
    fn selected(&mut self, is_selected: bool) {
        self.selected = is_selected;
    }
    fn scroll_up(&mut self) {
        let selected = self.state.selected_mut();
        *selected = Some(selected.unwrap_or(0).saturating_sub(1))
    }
    fn scroll_down(&mut self) {
        let selected = self.state.selected_mut();
        *selected = Some(
            selected
                .unwrap_or(0)
                .saturating_add(1)
                .min(self.items.len().saturating_sub(1)),
        )
    }
}
