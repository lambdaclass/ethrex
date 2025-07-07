use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ethrex_rpc::EthClient;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, StatefulWidget, Tabs, Widget};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use tui_logger::{TuiLoggerLevelOutput, TuiLoggerSmartWidget, TuiWidgetEvent, TuiWidgetState};

use crate::based::sequencer_state::SequencerState;
use crate::monitor::widget::{ETHREX_LOGO, LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS};
use crate::{
    SequencerConfig,
    monitor::widget::{
        BatchesTable, BlocksTable, GlobalChainStatusTable, L1ToL2MessagesTable,
        L2ToL1MessagesTable, MempoolTable, NodeStatusTable, tabs::TabsState,
    },
    sequencer::errors::MonitorError,
};

pub struct EthrexMonitor {
    pub title: String,
    pub should_quit: bool,
    pub tabs: TabsState,
    pub tick_rate: u64,

    pub logger: TuiWidgetState,
    pub node_status: NodeStatusTable,
    pub global_chain_status: GlobalChainStatusTable,
    pub mempool: MempoolTable,
    pub batches_table: BatchesTable,
    pub blocks_table: BlocksTable,
    pub l1_to_l2_messages: L1ToL2MessagesTable,
    pub l2_to_l1_messages: L2ToL1MessagesTable,

    pub eth_client: EthClient,
    pub rollup_client: EthClient,
    pub store: Store,
    pub rollup_store: StoreRollup,
}

impl EthrexMonitor {
    pub async fn new(
        sequencer_state: SequencerState,
        store: Store,
        rollup_store: StoreRollup,
        cfg: &SequencerConfig,
    ) -> Self {
        let eth_client = EthClient::new(cfg.eth.rpc_url.first().expect("No RPC URLs provided"))
            .expect("Failed to create EthClient");
        // TODO: De-hardcode the rollup client URL
        let rollup_client =
            EthClient::new("http://localhost:1729").expect("Failed to create RollupClient");

        EthrexMonitor {
            title: if cfg.based.based {
                "Based Ethrex Monitor".to_string()
            } else {
                "Ethrex Monitor".to_string()
            },
            should_quit: false,
            tabs: TabsState::default(),
            tick_rate: cfg.monitor.tick_rate,
            global_chain_status: GlobalChainStatusTable::new(
                &eth_client,
                cfg,
                &store,
                &rollup_store,
            )
            .await,
            logger: TuiWidgetState::new().set_default_display_level(tui_logger::LevelFilter::Info),
            node_status: NodeStatusTable::new(sequencer_state.clone(), &store).await,
            mempool: MempoolTable::new(&rollup_client).await,
            batches_table: BatchesTable::new(
                cfg.l1_committer.on_chain_proposer_address,
                &eth_client,
                &rollup_store,
            )
            .await,
            blocks_table: BlocksTable::new(&store).await,
            l1_to_l2_messages: L1ToL2MessagesTable::new(
                cfg.l1_watcher.bridge_address,
                &eth_client,
                &store,
            )
            .await,
            l2_to_l1_messages: L2ToL1MessagesTable::new(
                cfg.l1_watcher.bridge_address,
                &eth_client,
                &rollup_client,
            )
            .await,
            eth_client,
            rollup_client,
            store,
            rollup_store,
        }
    }

    pub async fn start(mut self) -> Result<(), MonitorError> {
        // setup terminal
        enable_raw_mode().map_err(MonitorError::Io)?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(MonitorError::Io)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).map_err(MonitorError::Io)?;

        let app_result = self.run(&mut terminal).await;

        // restore terminal
        disable_raw_mode().map_err(MonitorError::Io)?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .map_err(MonitorError::Io)?;
        terminal.show_cursor().map_err(MonitorError::Io)?;

        let _ = app_result.inspect_err(|err| {
            eprintln!("Monitor error: {err}");
        });

        Ok(())
    }

    async fn run<B>(&mut self, terminal: &mut Terminal<B>) -> Result<(), MonitorError>
    where
        B: Backend,
    {
        let mut last_tick = Instant::now();
        loop {
            self.draw(terminal)?;

            let timeout = Duration::from_millis(self.tick_rate).saturating_sub(last_tick.elapsed());
            if !event::poll(timeout)? {
                self.on_tick().await;
                last_tick = Instant::now();
                continue;
            }
            let event = event::read()?;
            if let Some(key) = event.as_key_press_event() {
                self.on_key_event(key.code);
            }
            if let Some(mouse) = event.as_mouse_event() {
                self.on_mouse_event(mouse.kind);
            }
            if self.should_quit {
                return Ok(());
            }
        }
    }

    fn draw(&mut self, terminal: &mut Terminal<impl Backend>) -> Result<(), MonitorError> {
        terminal.draw(|frame| {
            frame.render_widget(self, frame.area());
        })?;
        Ok(())
    }

    pub fn on_key_event(&mut self, code: KeyCode) {
        match (&self.tabs, code) {
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
            (TabsState::Overview, KeyCode::Char('Q')) => self.should_quit = true,
            (TabsState::Overview | TabsState::Logs, KeyCode::Tab) => self.tabs.next(),
            _ => {}
        }
    }

    pub fn on_mouse_event(&mut self, kind: MouseEventKind) {
        match (&self.tabs, kind) {
            (TabsState::Logs, MouseEventKind::ScrollDown) => {
                self.logger.transition(TuiWidgetEvent::NextPageKey)
            }
            (TabsState::Logs, MouseEventKind::ScrollUp) => {
                self.logger.transition(TuiWidgetEvent::PrevPageKey)
            }
            _ => {}
        }
    }

    pub async fn on_tick(&mut self) {
        self.node_status.on_tick(&self.store).await;
        self.global_chain_status
            .on_tick(&self.eth_client, &self.store, &self.rollup_store)
            .await;
        self.mempool.on_tick(&self.rollup_client).await;
        self.batches_table
            .on_tick(&self.eth_client, &self.rollup_store)
            .await;
        self.blocks_table.on_tick(&self.store).await;
        self.l1_to_l2_messages
            .on_tick(&self.eth_client, &self.store)
            .await;
        self.l2_to_l1_messages
            .on_tick(&self.eth_client, &self.rollup_client)
            .await;
    }
}

impl Widget for &mut EthrexMonitor {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);
        let tabs = Tabs::default()
            .titles([TabsState::Overview.to_string(), TabsState::Logs.to_string()])
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        self.title.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            )
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .select(self.tabs.clone());

        tabs.render(chunks[0], buf);

        match self.tabs {
            TabsState::Overview => {
                let chunks = Layout::vertical([
                    Constraint::Length(10),
                    Constraint::Fill(1),
                    Constraint::Fill(1),
                    Constraint::Fill(1),
                    Constraint::Fill(1),
                    Constraint::Fill(1),
                    Constraint::Length(1),
                ])
                .split(chunks[1]);
                {
                    let constraints = vec![
                        Constraint::Fill(1),
                        Constraint::Length(LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS),
                    ];

                    let chunks = Layout::horizontal(constraints).split(chunks[0]);

                    let logo = Paragraph::new(ETHREX_LOGO)
                        .centered()
                        .style(Style::default())
                        .block(Block::bordered().border_style(Style::default().fg(Color::Cyan)));

                    logo.render(chunks[0], buf);

                    {
                        let constraints = vec![Constraint::Fill(1), Constraint::Fill(1)];

                        let chunks = Layout::horizontal(constraints).split(chunks[1]);

                        let mut node_status_state = self.node_status.state.clone();
                        self.node_status
                            .render(chunks[0], buf, &mut node_status_state);

                        let mut global_chain_status_state = self.global_chain_status.state.clone();
                        self.global_chain_status.render(
                            chunks[1],
                            buf,
                            &mut global_chain_status_state,
                        );
                    }
                }
                let mut batches_table_state = self.batches_table.state.clone();
                self.batches_table
                    .render(chunks[1], buf, &mut batches_table_state);

                let mut blocks_table_state = self.blocks_table.state.clone();
                self.blocks_table
                    .render(chunks[2], buf, &mut blocks_table_state);

                let mut mempool_state = self.mempool.state.clone();
                self.mempool.render(chunks[3], buf, &mut mempool_state);

                let mut l1_to_l2_messages_state = self.l1_to_l2_messages.state.clone();
                self.l1_to_l2_messages
                    .render(chunks[4], buf, &mut l1_to_l2_messages_state);

                let mut l2_to_l1_messages_state = self.l2_to_l1_messages.state.clone();
                self.l2_to_l1_messages
                    .render(chunks[5], buf, &mut l2_to_l1_messages_state);

                let help = Line::raw("tab: switch tab |  Q: quit").centered();

                help.render(chunks[6], buf);
            }
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
        };
    }
}
