use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Row, Table},
};

use crate::monitor::EthrexMonitor;

pub const ETHREX_LOGO: &str = r#"
███████╗████████╗██╗░░██╗██████╗░███████╗██╗░░██╗
██╔════╝╚══██╔══╝██║░░██║██╔══██╗██╔════╝╚██╗██╔╝
█████╗░░░░░██║░░░███████║██████╔╝█████╗░░░╚███╔╝░
██╔══╝░░░░░██║░░░██╔══██║██╔══██╗██╔══╝░░░██╔██╗░
███████╗░░░██║░░░██║░░██║██║░░██║███████╗██╔╝╚██╗
╚══════╝░░░╚═╝░░░╚═╝░░╚═╝╚═╝░░╚═╝╚══════╝╚═╝░░╚═╝"#;

pub const HASH_LENGTH_IN_DIGITS: u16 = 66; // 64 hex characters + 2 for "0x" prefix
pub const ADDRESS_LENGTH_IN_DIGITS: u16 = 42; // 40 hex characters + 2 for "0x" prefix
pub const NUMBER_LENGTH_IN_DIGITS: u16 = 9; // 1e8
pub const TX_NUMBER_LENGTH_IN_DIGITS: u16 = 4;
pub const GAS_USED_LENGTH_IN_DIGITS: u16 = 8; // 1e7
pub const BLOCK_SIZE_LENGTH_IN_DIGITS: u16 = 6; // 1e6

pub const LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS: u16 = NUMBER_LENGTH_IN_DIGITS
    + TX_NUMBER_LENGTH_IN_DIGITS
    + HASH_LENGTH_IN_DIGITS
    + ADDRESS_LENGTH_IN_DIGITS
    + GAS_USED_LENGTH_IN_DIGITS
    + GAS_USED_LENGTH_IN_DIGITS
    + BLOCK_SIZE_LENGTH_IN_DIGITS;

pub fn draw(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(10),
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .split(area);
    {
        let constraints = vec![
            Constraint::Fill(1),
            Constraint::Length(LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS),
        ];
        let chunks = Layout::horizontal(constraints).split(chunks[0]);
        draw_ethrex_logo(frame, chunks[0]);
        {
            let constraints = vec![Constraint::Fill(1), Constraint::Fill(1)];
            let chunks = Layout::horizontal(constraints).split(chunks[1]);
            draw_node_status(frame, app, chunks[0]);
            draw_global_chain_status(frame, app, chunks[1]);
        }
    }
    draw_batches(frame, app, chunks[1]);
    draw_blocks(frame, app, chunks[2]);
    draw_mempool(frame, app, chunks[3]);
    draw_l1_to_l2_messages(frame, app, chunks[4]);
    draw_text(frame, chunks[5]);
}

fn draw_ethrex_logo(frame: &mut Frame, area: Rect) {
    let logo = Paragraph::new(ETHREX_LOGO)
        .centered()
        .style(Style::default())
        .block(Block::bordered().border_style(Style::default().fg(Color::Cyan)));
    frame.render_widget(logo, area);
}

fn draw_node_status(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![Constraint::Percentage(50), Constraint::Percentage(50)];
    let rows = app.node_status.items.iter().map(|(key, value)| {
        Row::new(vec![
            Span::styled(key, Style::default()),
            Span::styled(value, Style::default()),
        ])
    });
    let node_status_table = Table::new(rows, constraints).block(
        Block::bordered()
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                "Node Status",
                Style::default().add_modifier(Modifier::BOLD),
            )),
    );
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
    let global_chain_status_table = Table::new(rows, constraints).block(
        Block::bordered()
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                "Global Chain Status",
                Style::default().add_modifier(Modifier::BOLD),
            )),
    );

    frame.render_stateful_widget(
        global_chain_status_table,
        area,
        &mut app.global_chain_status.state,
    );
}

fn draw_batches(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![
        Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
        Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
        Constraint::Length(17),
        Constraint::Length(HASH_LENGTH_IN_DIGITS),
        Constraint::Length(HASH_LENGTH_IN_DIGITS),
    ];
    let rows = app.batches_table.items.iter().map(
        |(number, n_blocks, n_messages, commit_tx_hash, verify_tx_hash)| {
            Row::new(vec![
                Span::styled(number.to_string(), Style::default()),
                Span::styled(n_blocks.to_string(), Style::default()),
                Span::styled(n_messages.to_string(), Style::default()),
                Span::styled(
                    commit_tx_hash
                        .map_or_else(|| "Uncommitted".to_string(), |hash| format!("{hash:#x}")),
                    Style::default(),
                ),
                Span::styled(
                    verify_tx_hash
                        .map_or_else(|| "Unverified".to_string(), |hash| format!("{hash:#x}")),
                    Style::default(),
                ),
            ])
        },
    );
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
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    "L2 Batches",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
        );
    frame.render_stateful_widget(committed_batches_table, area, &mut app.batches_table.state);
}

fn draw_blocks(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![
        Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
        Constraint::Length(TX_NUMBER_LENGTH_IN_DIGITS),
        Constraint::Length(HASH_LENGTH_IN_DIGITS),
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
        .block(
            Block::bordered()
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    "L2 Blocks",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
        );
    frame.render_stateful_widget(latest_blocks_table, area, &mut app.blocks_table.state);
}

fn draw_mempool(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![
        Constraint::Length(HASH_LENGTH_IN_DIGITS),
        Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
        Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
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
        .block(
            Block::bordered()
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    "Mempool",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
        );
    frame.render_stateful_widget(mempool_table, area, &mut app.mempool.state);
}

fn draw_l1_to_l2_messages(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let constraints = vec![
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(HASH_LENGTH_IN_DIGITS),
        Constraint::Length(HASH_LENGTH_IN_DIGITS),
        Constraint::Fill(1),
    ];

    let rows =
        app.l1_to_l2_messages
            .items
            .iter()
            .map(|(status, kind, l1_tx_hash, l2_tx_hash, amount)| {
                Row::new(vec![
                    Span::styled(format!("{status}"), Style::default()),
                    Span::styled(format!("{kind}"), Style::default()),
                    Span::styled(format!("{l1_tx_hash:#x}"), Style::default()),
                    Span::styled(format!("{l2_tx_hash:#x}"), Style::default()),
                    Span::styled(amount.to_string(), Style::default()),
                ])
            });

    let l1_to_l2_messages_table = Table::new(rows, constraints)
        .header(
            Row::new(vec!["Status", "Kind", "L1 Tx Hash", "L2 Tx Hash", "Value"])
                .style(Style::default()),
        )
        .block(
            Block::bordered()
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    "L1 to L2 Messages",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
        );

    frame.render_stateful_widget(
        l1_to_l2_messages_table,
        area,
        &mut app.l1_to_l2_messages.state,
    );
}

fn draw_text(frame: &mut Frame, area: Rect) {
    frame.render_widget(Line::raw("tab: switch tab |  Q: quit").centered(), area)
}
