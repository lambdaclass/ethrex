use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
};
use tui_logger::{TuiLoggerLevelOutput, TuiLoggerSmartWidget};

use crate::monitor::EthrexMonitor;

pub fn draw(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let chunks = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(area);
    draw_logger(frame, app, chunks[0]);
    draw_text(frame, chunks[1]);
}

fn draw_logger(frame: &mut Frame, app: &mut EthrexMonitor, area: Rect) {
    let log_widget = TuiLoggerSmartWidget::default()
        .style_error(Style::default().fg(Color::Red))
        .style_debug(Style::default().fg(Color::LightBlue))
        .style_warn(Style::default().fg(Color::Yellow))
        .style_trace(Style::default().fg(Color::Magenta))
        .style_info(Style::default().fg(Color::White))
        .border_style(Style::default().fg(Color::Cyan))
        .output_separator(' ')
        .output_timestamp(Some("%F %H:%M:%S%.3f".to_string()))
        .output_level(Some(TuiLoggerLevelOutput::Long))
        .output_target(true)
        .output_file(false)
        .output_line(false)
        .state(&app.logger);
    frame.render_widget(log_widget, area);
}

fn draw_text(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Line::raw("tab: switch tab |  Q: quit | ↑/↓: select target | f: focus target | ←/→: display level | +/-: filter level | h: hide target selector").centered(),
        area,
    )
}
