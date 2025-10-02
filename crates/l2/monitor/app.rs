use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ethrex_rpc::EthClient;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use futures::StreamExt;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, StatefulWidget, Tabs, Widget};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use spawned_concurrency::{
    messages::Unused,
    tasks::{
        CastResponse, GenServer, GenServerHandle, InitResult, Success, send_interval,
        spawn_listener,
    },
};
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tui_logger::{TuiLoggerLevelOutput, TuiLoggerSmartWidget, TuiWidgetEvent, TuiWidgetState};

use crate::monitor::utils::SelectableScroller;
use crate::monitor::widget::{ETHREX_LOGO, LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS};
use crate::sequencer::configs::MonitorConfig;
use crate::{
    SequencerConfig,
    monitor::widget::{
        BatchesTable, BlocksTable, GlobalChainStatusTable, L1ToL2MessagesTable,
        L2ToL1MessagesTable, MempoolTable, NodeStatusTable, tabs::TabsState,
    },
    sequencer::errors::MonitorError,
};
use crate::{
    based::sequencer_state::SequencerState, monitor::widget::rich_accounts::RichAccountsTable,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

const SCROLL_DEBOUNCE_DURATION: Duration = Duration::from_millis(700); // 700ms

const SCROLLABLE_WIDGETS: usize = 5;
pub struct EthrexMonitorWidget {
    pub title: String,
    pub should_quit: bool,
    pub tabs: TabsState,
    pub cfg: MonitorConfig,

    pub logger: TuiWidgetState,
    pub node_status: NodeStatusTable,
    pub global_chain_status: GlobalChainStatusTable,
    pub mempool: MempoolTable,
    pub batches_table: BatchesTable,
    pub blocks_table: BlocksTable,
    pub l1_to_l2_messages: L1ToL2MessagesTable,
    pub l2_to_l1_messages: L2ToL1MessagesTable,
    pub rich_accounts: RichAccountsTable,

    pub eth_client: EthClient,
    pub rollup_client: EthClient,
    pub store: Store,
    pub rollup_store: StoreRollup,
    pub last_scroll: Instant,
    pub overview_selected_widget: usize,
}

#[derive(Clone, Debug)]
pub enum CastInMessage {
    Tick,
    Event(Event),
}

#[derive(Clone, PartialEq, Debug)]
pub enum OutMessage {
    Done,
}

pub struct EthrexMonitor {
    widget: EthrexMonitorWidget,
    terminal: Arc<Mutex<Terminal<CrosstermBackend<io::Stdout>>>>,
    cancellation_token: CancellationToken,
}

impl EthrexMonitor {
    pub async fn spawn(
        sequencer_state: SequencerState,
        store: Store,
        rollup_store: StoreRollup,
        cfg: &SequencerConfig,
        cancellation_token: CancellationToken,
    ) -> Result<GenServerHandle<EthrexMonitor>, MonitorError> {
        let widget = EthrexMonitorWidget::new(sequencer_state, store, rollup_store, cfg).await?;
        let ethrex_monitor = EthrexMonitor {
            widget,
            terminal: Arc::new(Mutex::new(setup_terminal()?)),
            cancellation_token,
        };
        Ok(ethrex_monitor.start())
    }
}

impl GenServer for EthrexMonitor {
    type CallMsg = Unused;
    type CastMsg = CastInMessage;
    type OutMsg = OutMessage;
    type Error = MonitorError;

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        // Tick handling
        send_interval(
            Duration::from_millis(self.widget.cfg.tick_rate),
            handle.clone(),
            Self::CastMsg::Tick,
        );
        // Event handling
        spawn_listener(
            handle.clone(),
            EventStream::new()
                .filter_map(|result| async move { result.ok().map(Self::CastMsg::Event) }),
        );
        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            // On event
            CastInMessage::Event(event) => {
                let widget = &mut self.widget;
                if let Some(key) = event.as_key_press_event() {
                    widget.on_key_event(key.code);
                }
                if let Some(mouse) = event.as_mouse_event() {
                    widget.on_mouse_event(mouse.kind);
                }
            }
            // Tick received
            CastInMessage::Tick => {
                let _ = self
                    .widget
                    .on_tick()
                    .await
                    .inspect_err(|err| error!("Monitor error: {err}"));
            }
        }

        if !self.widget.should_quit {
            let _ = self
                .widget
                .draw(&mut *self.terminal.lock().await)
                .inspect_err(|err| error!("Render error: {err}"));
            CastResponse::NoReply
        } else {
            CastResponse::Stop
        }
    }

    async fn teardown(self, _handle: &GenServerHandle<Self>) -> Result<(), Self::Error> {
        let mut terminal = self.terminal.lock().await;
        let _ = restore_terminal(&mut terminal).inspect_err(|err| {
            error!("Error restoring terminal: {err}");
        });
        info!("Monitor has been cancelled");
        self.cancellation_token.cancel();
        Ok(())
    }
}

impl EthrexMonitorWidget {
    pub async fn new(
        sequencer_state: SequencerState,
        store: Store,
        rollup_store: StoreRollup,
        cfg: &SequencerConfig,
    ) -> Result<Self, MonitorError> {
        let eth_client = EthClient::new(cfg.eth.rpc_url.first().ok_or(MonitorError::RPCListEmpty)?)
            .map_err(MonitorError::EthClientError)?;
        // TODO: De-hardcode the rollup client URL
        let rollup_client =
            EthClient::new("http://localhost:1729").map_err(MonitorError::EthClientError)?;

        let mut monitor_widget = EthrexMonitorWidget {
            title: if cfg.based.enabled {
                "Based Ethrex Monitor".to_string()
            } else {
                "Ethrex Monitor".to_string()
            },
            should_quit: false,
            tabs: TabsState::default(),
            cfg: cfg.monitor.clone(),
            global_chain_status: GlobalChainStatusTable::new(cfg),
            logger: TuiWidgetState::new().set_default_display_level(tui_logger::LevelFilter::Info),
            node_status: NodeStatusTable::new(sequencer_state.clone(), cfg.based.enabled),
            mempool: MempoolTable::new(),
            batches_table: BatchesTable::new(cfg.l1_committer.on_chain_proposer_address),
            blocks_table: BlocksTable::new(),
            l1_to_l2_messages: L1ToL2MessagesTable::new(cfg.l1_watcher.bridge_address),
            l2_to_l1_messages: L2ToL1MessagesTable::new(cfg.l1_watcher.bridge_address),
            rich_accounts: RichAccountsTable::new(&rollup_client).await?,
            eth_client,
            rollup_client,
            store,
            rollup_store,
            last_scroll: Instant::now(),
            overview_selected_widget: 0,
        };
        monitor_widget.selected_table().selected(true);
        monitor_widget.on_tick().await?;
        Ok(monitor_widget)
    }

    fn draw(&mut self, terminal: &mut Terminal<impl Backend>) -> Result<(), MonitorError> {
        terminal.draw(|frame| {
            frame.render_widget(self, frame.area());
        })?;
        Ok(())
    }

    fn selected_table(&mut self) -> &mut dyn SelectableScroller {
        let widgets: [&mut dyn SelectableScroller; SCROLLABLE_WIDGETS] = [
            &mut self.batches_table,
            &mut self.blocks_table,
            &mut self.mempool,
            &mut self.l1_to_l2_messages,
            &mut self.l2_to_l1_messages,
        ];
        // index always within bounds
        #[expect(clippy::indexing_slicing)]
        widgets[self.overview_selected_widget % SCROLLABLE_WIDGETS]
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
            (TabsState::Overview, KeyCode::Up) => {
                self.selected_table().selected(false);
                self.overview_selected_widget = self
                    .overview_selected_widget
                    .wrapping_add(SCROLLABLE_WIDGETS - 1)
                    % SCROLLABLE_WIDGETS;
                self.selected_table().selected(true);
            }
            (TabsState::Overview, KeyCode::Down) => {
                self.selected_table().selected(false);
                self.overview_selected_widget =
                    self.overview_selected_widget.wrapping_add(1) % SCROLLABLE_WIDGETS;
                self.selected_table().selected(true);
            }
            (TabsState::Overview, KeyCode::Char('w')) => {
                self.selected_table().scroll_up();
            }
            (TabsState::Overview, KeyCode::Char('s')) => {
                self.selected_table().scroll_down();
            }
            (
                TabsState::Overview | TabsState::Logs | TabsState::RichAccounts,
                KeyCode::Char('Q'),
            ) => self.should_quit = true,
            (TabsState::Overview | TabsState::Logs | TabsState::RichAccounts, KeyCode::Tab) => {
                self.tabs.next()
            }
            (TabsState::RichAccounts, KeyCode::Char('w')) => {
                self.rich_accounts.scroll_up();
            }
            (TabsState::RichAccounts, KeyCode::Char('s')) => {
                self.rich_accounts.scroll_down();
            }
            _ => {}
        }
    }

    pub fn on_mouse_event(&mut self, kind: MouseEventKind) {
        let now = Instant::now();
        if now.duration_since(self.last_scroll) < SCROLL_DEBOUNCE_DURATION {
            return; // Ignore the scroll — too soon
        }

        self.last_scroll = now;

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

    pub async fn on_tick(&mut self) -> Result<(), MonitorError> {
        self.node_status
            .on_tick(&self.store, &self.rollup_client)
            .await?;
        self.global_chain_status
            .on_tick(&self.eth_client, &self.store, &self.rollup_store)
            .await?;
        self.mempool.on_tick(&self.rollup_client).await?;
        self.batches_table
            .on_tick(&self.eth_client, &self.rollup_store)
            .await?;
        self.blocks_table.on_tick(&self.store).await?;
        self.l1_to_l2_messages
            .on_tick(&self.eth_client, &self.store)
            .await?;
        self.l2_to_l1_messages
            .on_tick(&self.eth_client, &self.rollup_client)
            .await?;
        self.rich_accounts.on_tick(&self.rollup_client).await?;

        Ok(())
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) -> Result<(), MonitorError>
    where
        Self: Sized,
    {
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);
        let tabs = Tabs::default()
            .titles([
                TabsState::Overview.to_string(),
                TabsState::Logs.to_string(),
                TabsState::RichAccounts.to_string(),
            ])
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

        tabs.render(*chunks.first().ok_or(MonitorError::Chunks)?, buf);

        match self.tabs {
            TabsState::Overview => {
                let chunks = Layout::vertical([
                    Constraint::Length(10),
                    if let Some(height) = self.cfg.batch_widget_height {
                        Constraint::Length(height)
                    } else {
                        Constraint::Fill(1)
                    },
                    Constraint::Fill(1),
                    Constraint::Fill(1),
                    Constraint::Fill(1),
                    Constraint::Fill(1),
                    Constraint::Length(1),
                ])
                .split(*chunks.get(1).ok_or(MonitorError::Chunks)?);
                {
                    let constraints = vec![
                        Constraint::Fill(1),
                        Constraint::Length(LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS),
                    ];

                    let chunks = Layout::horizontal(constraints)
                        .split(*chunks.first().ok_or(MonitorError::Chunks)?);

                    let logo = Paragraph::new(ETHREX_LOGO)
                        .centered()
                        .style(Style::default())
                        .block(Block::bordered().border_style(Style::default().fg(Color::Cyan)));

                    logo.render(*chunks.first().ok_or(MonitorError::Chunks)?, buf);

                    {
                        let constraints = vec![Constraint::Fill(1), Constraint::Fill(1)];

                        let chunks = Layout::horizontal(constraints)
                            .split(*chunks.get(1).ok_or(MonitorError::Chunks)?);

                        let mut node_status_state = self.node_status.state.clone();
                        self.node_status.render(
                            *chunks.first().ok_or(MonitorError::Chunks)?,
                            buf,
                            &mut node_status_state,
                        );

                        let mut global_chain_status_state = self.global_chain_status.state.clone();
                        self.global_chain_status.render(
                            *chunks.get(1).ok_or(MonitorError::Chunks)?,
                            buf,
                            &mut global_chain_status_state,
                        );
                    }
                }
                let mut batches_table_state = self.batches_table.state.clone();
                self.batches_table.render(
                    *chunks.get(1).ok_or(MonitorError::Chunks)?,
                    buf,
                    &mut batches_table_state,
                );

                let mut blocks_table_state = self.blocks_table.state.clone();
                self.blocks_table.render(
                    *chunks.get(2).ok_or(MonitorError::Chunks)?,
                    buf,
                    &mut blocks_table_state,
                );

                let mut mempool_state = self.mempool.state.clone();
                self.mempool.render(
                    *chunks.get(3).ok_or(MonitorError::Chunks)?,
                    buf,
                    &mut mempool_state,
                );

                let mut l1_to_l2_messages_state = self.l1_to_l2_messages.state.clone();
                self.l1_to_l2_messages.render(
                    *chunks.get(4).ok_or(MonitorError::Chunks)?,
                    buf,
                    &mut l1_to_l2_messages_state,
                );

                let mut l2_to_l1_messages_state = self.l2_to_l1_messages.state.clone();
                self.l2_to_l1_messages.render(
                    *chunks.get(5).ok_or(MonitorError::Chunks)?,
                    buf,
                    &mut l2_to_l1_messages_state,
                );

                let help =
                    Line::raw("↑/↓: select table | w/s: scroll table | tab: switch tab | Q: quit")
                        .centered();

                help.render(*chunks.get(6).ok_or(MonitorError::Chunks)?, buf);
            }
            TabsState::Logs => {
                let chunks = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)])
                    .split(*chunks.get(1).ok_or(MonitorError::Chunks)?);
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

                log_widget.render(*chunks.first().ok_or(MonitorError::Chunks)?, buf);

                let help = Line::raw("↑/↓: select target | f: focus target | ←/→: display level | +/-: filter level | h: hide target selector | tab: switch tab | Q: quit").centered();

                help.render(*chunks.get(1).ok_or(MonitorError::Chunks)?, buf);
            }
            TabsState::RichAccounts => {
                let chunks = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)])
                    .split(*chunks.get(1).ok_or(MonitorError::Chunks)?);
                let mut accounts = self.rich_accounts.state.clone();
                self.rich_accounts.render(
                    *chunks.first().ok_or(MonitorError::Chunks)?,
                    buf,
                    &mut accounts,
                );
                let help = Line::raw("w/s: scroll table | tab: switch tab | Q: quit").centered();
                help.render(*chunks.get(1).ok_or(MonitorError::Chunks)?, buf);
            }
        };
        Ok(())
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, MonitorError> {
    enable_raw_mode().map_err(MonitorError::Io)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(MonitorError::Io)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).map_err(MonitorError::Io)?;
    Ok(terminal)
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), MonitorError> {
    disable_raw_mode().map_err(MonitorError::Io)?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .map_err(MonitorError::Io)?;
    terminal.show_cursor().map_err(MonitorError::Io)?;
    Ok(())
}

impl Widget for &mut EthrexMonitorWidget {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let result = self.render(area, buf);
        match result {
            Ok(_) => {}
            Err(e) => {
                buf.reset();
                let error = Line::raw(format!("Error: {e}")).centered();

                error.render(area, buf);
            }
        }
    }
}
