use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Row, Table},
};

use crate::monitor::EthrexMonitor;

pub const H256_LENGTH_IN_DIGITS: u16 = 66; // 64 hex characters + 2 for "0x" prefix
pub const ADDRESS_LENGTH_IN_DIGITS: u16 = 42; // 40 hex characters + 2 for "0x" prefix
pub const BLOCK_NUMBER_LENGTH_IN_DIGITS: u16 = 9; // 1e8
pub const BATCH_NUMBER_LENGTH_IN_DIGITS: u16 = 9; // 1e8
pub const TX_NUMBER_LENGTH_IN_DIGITS: u16 = 4;
pub const GAS_USED_LENGTH_IN_DIGITS: u16 = 8; // 1e7
pub const BLOCK_SIZE_LENGTH_IN_DIGITS: u16 = 6; // 1e6

pub const LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS: u16 = BLOCK_NUMBER_LENGTH_IN_DIGITS
    + TX_NUMBER_LENGTH_IN_DIGITS
    + H256_LENGTH_IN_DIGITS
    + ADDRESS_LENGTH_IN_DIGITS
    + GAS_USED_LENGTH_IN_DIGITS
    + GAS_USED_LENGTH_IN_DIGITS
    + BLOCK_SIZE_LENGTH_IN_DIGITS;

pub fn draw(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(10),
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .split(area);
    draw_status(frame, app, chunks[0]);
    draw_latest_batches_and_blocks(frame, app, chunks[1]);
    draw_mempool(frame, app, chunks[2]);
    draw_text(frame, chunks[3]);
}

fn draw_status(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![Constraint::Percentage(50), Constraint::Percentage(50)];
    let chunks = Layout::horizontal(constraints).split(area);
    draw_node_status(frame, app, chunks[0]);
    draw_global_chain_status(frame, app, chunks[1]);
}

fn draw_node_status(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![Constraint::Percentage(50), Constraint::Percentage(50)];
    let rows = app.node_status.items.iter().map(|(key, value)| {
        Row::new(vec![
            Span::styled(key, Style::default()),
            Span::styled(value, Style::default()),
        ])
    });
    let node_status_table = Table::new(rows, constraints).block(Block::bordered().title(
        Span::styled("Node Status", Style::default().add_modifier(Modifier::BOLD)),
    ));
    frame.render_stateful_widget(node_status_table, area, &mut app.node_status.state);
}

fn draw_global_chain_status(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![Constraint::Percentage(50), Constraint::Percentage(50)];
    let rows = app.global_chain_status.items.iter().map(|(key, value)| {
        Row::new(vec![
            Span::styled(key, Style::default()),
            Span::styled(value, Style::default()),
        ])
    });
    let global_chain_status_table =
        Table::new(rows, constraints).block(Block::bordered().title(Span::styled(
            "Global Chain Status",
            Style::default().add_modifier(Modifier::BOLD),
        )));

    frame.render_stateful_widget(
        global_chain_status_table,
        area,
        &mut app.global_chain_status.state,
    );
}

fn draw_latest_batches_and_blocks(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![
        Constraint::Fill(1), // The committed batches table will continue growing, there's no reason to limit its length now.
        Constraint::Length(LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS),
    ];
    let chunks = Layout::horizontal(constraints).split(area);
    {
        let constraints = vec![
            Constraint::Length(BATCH_NUMBER_LENGTH_IN_DIGITS),
            Constraint::Fill(1),
        ];
        let rows = app
            .committed_batches
            .items
            .iter()
            .map(|(number, commit_tx_hash)| {
                Row::new(vec![
                    Span::styled(number, Style::default()),
                    Span::styled(commit_tx_hash, Style::default()),
                ])
            });
        let committed_batches_table = Table::new(rows, constraints)
            .header(Row::new(vec!["Number", "Commit Tx Hash"]).style(Style::default()))
            .block(Block::bordered().title(Span::styled(
                "Committed Batches",
                Style::default().add_modifier(Modifier::BOLD),
            )));
        frame.render_stateful_widget(
            committed_batches_table,
            chunks[0],
            &mut app.committed_batches.state,
        );

        let constraints = vec![
            Constraint::Length(BLOCK_NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(TX_NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(H256_LENGTH_IN_DIGITS),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(GAS_USED_LENGTH_IN_DIGITS),
            Constraint::Length(GAS_USED_LENGTH_IN_DIGITS),
            Constraint::Length(BLOCK_SIZE_LENGTH_IN_DIGITS),
        ];
        let rows = app.blocks_table.items.iter().map(
            |(number, n_txs, hash, coinbase, gas, blob_bas, size)| {
                Row::new(vec![
                    Span::styled(number, Style::default()),
                    Span::styled(n_txs.to_string(), Style::default()),
                    Span::styled(hash, Style::default()),
                    Span::styled(coinbase, Style::default()),
                    Span::styled(gas.to_string(), Style::default()),
                    Span::styled(blob_bas.to_string(), Style::default()),
                    Span::styled(size.to_string(), Style::default()),
                ])
            },
        );
        let latest_blocks_table = Table::new(rows, constraints)
            .header(
                Row::new(vec![
                    "Number", "#Txs", "Hash", "Coinbase", "Gas", "Blob Gas", "Size",
                ])
                .style(Style::default()),
            )
            .block(Block::bordered().title(Span::styled(
                "Latest Blocks",
                Style::default().add_modifier(Modifier::BOLD),
            )));
        frame.render_stateful_widget(latest_blocks_table, chunks[1], &mut app.blocks_table.state);
    }
}

fn draw_mempool(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Fill(1),
    ];
    let rows = app.mempool.items.iter().map(|(hash, sender, nonce)| {
        Row::new(vec![
            Span::styled(hash, Style::default()),
            Span::styled(sender, Style::default()),
            Span::styled(nonce, Style::default()),
        ])
    });
    let mempool_table = Table::new(rows, constraints)
        .header(Row::new(vec!["Hash", "Sender", "Nonce"]).style(Style::default()))
        .block(Block::bordered().title(Span::styled(
            "Mempool",
            Style::default().add_modifier(Modifier::BOLD),
        )));
    frame.render_stateful_widget(mempool_table, area, &mut app.mempool.state);
}

fn draw_text(frame: &mut Frame, area: Rect) {
    frame.render_widget(Line::raw("◄ ► change tab |  Q to quit").centered(), area)
}
