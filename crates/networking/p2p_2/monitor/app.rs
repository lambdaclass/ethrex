use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent};
use ratatui::{
    Terminal,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    prelude::CrosstermBackend,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Tabs, Widget},
};
use tokio::sync::mpsc;
use tui_logger::{TuiLoggerLevelOutput, TuiLoggerSmartWidget, TuiWidgetEvent, TuiWidgetState};

use crate::monitor::widgets::tabs::TabsState;

pub async fn input_thread(tx_event: mpsc::UnboundedSender<Event>) -> color_eyre::Result<()> {
    while let Ok(event) = event::read() {
        tx_event.send(event)?;
    }
    Ok(())
}

pub struct Monitor<'title> {
    title: &'title str,
    tabs: TabsState,
    logger: TuiWidgetState,
    should_exit: bool,
}

impl<'title> Monitor<'title> {
    pub fn new(title: &'title str) -> Self {
        Self {
            title,
            tabs: TabsState::default(),
            logger: TuiWidgetState::new().set_default_display_level(tui_logger::LevelFilter::Info),
            should_exit: false,
        }
    }

    pub async fn start(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
        // Use an mpsc::channel to combine stdin events with app events
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = tx.clone();

        let input_task = tokio::spawn(async move { input_thread(event_tx).await });

        tokio::select! {
            _ = self.run(terminal, rx) => {
                println!("Aborting tasks...");
                input_task.abort();
                println!("Tasks aborted successfully.");
            },
        }
    }

    async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        mut rx: mpsc::UnboundedReceiver<Event>,
    ) {
        loop {
            self.draw(terminal);

            let Some(event) = rx.recv().await else {
                continue;
            };

            self.handle_event(event);

            if self.should_exit {
                println!("Exiting application...");
                break;
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key_event) => self.handle_key_event(key_event),
            Event::Mouse(mouse_event) => self.handle_mouse_event(mouse_event),
            _ => {}
        }
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match (&self.tabs, key_event.code) {
            (TabsState::Logs, KeyCode::Left) => self.logger.transition(TuiWidgetEvent::LeftKey),
            (TabsState::Logs, KeyCode::Down) => self.logger.transition(TuiWidgetEvent::DownKey),
            (TabsState::Logs, KeyCode::Up) => self.logger.transition(TuiWidgetEvent::UpKey),
            (TabsState::Logs, KeyCode::Right) => self.logger.transition(TuiWidgetEvent::RightKey),
            (TabsState::Logs, KeyCode::Char('h')) => {
                self.logger.transition(TuiWidgetEvent::HideKey)
            }
            (TabsState::Logs, KeyCode::Char('f')) => {
                self.logger.transition(TuiWidgetEvent::FocusKey)
            }
            (TabsState::Logs, KeyCode::Char('+')) => {
                self.logger.transition(TuiWidgetEvent::PlusKey)
            }
            (TabsState::Logs, KeyCode::Char('-')) => {
                self.logger.transition(TuiWidgetEvent::MinusKey)
            }
            (TabsState::Overview | TabsState::Logs, KeyCode::Char('Q')) => self.should_exit = true,
            (TabsState::Overview | TabsState::Logs, KeyCode::Tab) => self.tabs.next(),
            _ => {}
        }
    }

    fn handle_mouse_event(&mut self, mouse_event: crossterm::event::MouseEvent) {
        #[expect(clippy::match_single_binding)]
        match mouse_event.kind {
            _ => {}
        }
    }

    fn draw(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
        terminal
            .draw(|frame| {
                frame.render_widget(self, frame.area());
            })
            .expect("Failed to draw terminal");
    }
}

impl<'title> Widget for &mut Monitor<'title> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);
        let tabs = Tabs::default()
            .titles([TabsState::Overview.to_string(), TabsState::Logs.to_string()])
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        self.title.to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            )
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .select(self.tabs.clone());

        tabs.render(chunks[0], buf);

        match self.tabs {
            TabsState::Overview => {}
            TabsState::Logs => {
                let chunks =
                    Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(chunks[1]);
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
                    .state(&self.logger);

                log_widget.render(chunks[0], buf);

                let help = Line::raw("tab: switch tab |  Q: quit | ↑/↓: select target | f: focus target | ←/→: display level | +/-: filter level | h: hide target selector").centered();

                help.render(chunks[1], buf);
            }
        }
    }
}
