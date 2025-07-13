use std::{cmp::min, net::IpAddr};

use ethrex_common::H256;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect, Size},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, Widget},
};
use tui_widgets::scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::discv4::Kademlia;

pub const MAX_ROWS: u16 = 100;

// node id | ip:port
pub type ContactsRow = (H256, IpAddr);

#[derive(Clone)]
pub struct ContactsTable {
    pub state: ScrollViewState,
    rows: Vec<ContactsRow>,
    kademlia: Kademlia,
}

impl ContactsTable {
    pub fn new(kademlia: Kademlia) -> Self {
        Self {
            state: ScrollViewState::default(),
            rows: Vec::default(),
            kademlia,
        }
    }

    pub async fn on_tick(&mut self) {
        self.refresh_items().await;
    }

    pub async fn refresh_items(&mut self) {
        self.rows.clear();
        for (node_id, node) in self.kademlia.lock().await.iter().take(MAX_ROWS.into()) {
            self.rows.push((*node_id, node.ip));
        }
    }
}

impl StatefulWidget for &mut ContactsTable {
    type State = ScrollViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let constraints = [Constraint::Fill(1), Constraint::Fill(1)];

        let rows = self.rows.iter().map(|(node_id, node_ip)| {
            Row::new(vec![
                Span::styled(format!("{node_id:#x}"), Style::default()),
                Span::styled(node_ip.to_string(), Style::default()),
            ])
        });

        let mut extensible_area = area;
        extensible_area.height = min(self.rows.len() as u16, MAX_ROWS);

        let mut scroll_view =
            ScrollView::new(Size::new(extensible_area.width, extensible_area.height))
                .horizontal_scrollbar_visibility(ScrollbarVisibility::Never);

        let table = Table::new(rows, constraints).header(Row::new(vec![
            Span::styled("Node ID", Style::default()),
            Span::styled("IP", Style::default()),
        ]));

        let table_block = Block::bordered()
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                "Contacts",
                Style::default().add_modifier(Modifier::BOLD),
            ));

        let table_block_inner = table_block.inner(area);

        table_block.render(area, buf);

        scroll_view.render_widget(table, extensible_area);

        scroll_view.render(table_block_inner, buf, state);
    }
}
