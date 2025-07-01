#![expect(clippy::indexing_slicing)]

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{self, Span},
    widgets::{Block, Tabs},
};

use crate::monitor::EthrexMonitor;

mod tabs;

pub fn render(frame: &mut Frame, app: &mut EthrexMonitor) {
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(frame.area());
    let tabs = app
        .tabs
        .titles
        .iter()
        .map(|t| text::Line::from(Span::styled(*t, Style::default())))
        .collect::<Tabs>()
        .block(
            Block::bordered()
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    app.title,
                    Style::default().add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .select(app.tabs.index);
    frame.render_widget(tabs, chunks[0]);
    // #[expect(clippy::single_match)]
    match app.tabs.index {
        0 => tabs::overview::draw(frame, app, chunks[1]),
        1 => tabs::logs::draw(frame, app, chunks[1]),
        _ => {}
    };
}
