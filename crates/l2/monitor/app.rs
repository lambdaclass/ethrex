use std::io;
use std::sync::{Arc};
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
use spawned_concurrency::messages::Unused;
use spawned_concurrency::tasks::{send_after, CastResponse, GenServer, GenServerHandle};
use tokio::sync::Mutex;
use tui_logger::{TuiLoggerLevelOutput, TuiLoggerSmartWidget, TuiWidgetEvent, TuiWidgetState};

use crate::based::sequencer_state::SequencerState;
use crate::monitor::widget::{self, ETHREX_LOGO, LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS};
use crate::{
    SequencerConfig,
    monitor::widget::{
        BatchesTable, BlocksTable, GlobalChainStatusTable, L1ToL2MessagesTable,
        L2ToL1MessagesTable, MempoolTable, NodeStatusTable, tabs::TabsState,
    },
    sequencer::errors::MonitorError,
};
use tracing::{debug, error, info, warn};
#[derive(Clone)]
pub struct EthrexMonitorState {
    pub title: String,
    pub should_quit: bool,
    pub tabs: TabsState,
    pub tick_rate: u64,

    pub logger: Arc<TuiWidgetState>,
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
    pub last_tick: Instant,
}

pub struct EthrexMonitor {
    terminal: Arc<Mutex<Terminal<CrosstermBackend<io::Stdout>>>>,
}

#[derive(Clone)]
pub enum InMessage {
    Monitor,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
}

impl EthrexMonitor {
    pub async fn spawn(
        sequencer_state: SequencerState,
        store: Store,
        rollup_store: StoreRollup,
        cfg: &SequencerConfig,
    ) -> Result<(), MonitorError> {
        let state = EthrexMonitorState::new(sequencer_state, store, rollup_store, cfg).await?;
        let mut ethrex_monitor = EthrexMonitor::start(state);
        ethrex_monitor
            .cast(InMessage::Monitor)
            .await
            .map_err(MonitorError::GenServerError)
    }

    pub async fn monitor(&self, state: &mut EthrexMonitorState) -> Result<(), MonitorError> {
        let mut terminal = self.terminal.lock().await;

        draw(&mut terminal, state)?;

        let timeout = Duration::from_millis(state.tick_rate).saturating_sub(state.last_tick.elapsed());
        if !event::poll(timeout)? {
            on_tick(state).await?;
            state.last_tick = Instant::now();
            return Ok(());
        }
        let event = event::read()?;
        if let Some(key) = event.as_key_press_event() {
            on_key_event(key.code,state);
        }
        if let Some(mouse) = event.as_mouse_event() {
            on_mouse_event(mouse.kind,state);
        }


        Ok(())

        
    }
}


impl GenServer for EthrexMonitor {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = EthrexMonitorState;
    type Error = MonitorError;

    fn new() -> Self {
        // setup terminal
        enable_raw_mode().map_err(MonitorError::Io).unwrap();
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(MonitorError::Io).unwrap();
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).map_err(MonitorError::Io).unwrap();
        Self {
            terminal: Arc::new(Mutex::new(terminal)),
        }
    }

    async fn handle_cast(
        &mut self,
        _message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        let _ = self.monitor(&mut state)
            .await
            .inspect_err(|err| error!("Monitor Error: {err}"));
        if !state.should_quit {
            send_after(
                Duration::from_millis(1),
                handle.clone(),
                Self::CastMsg::Monitor,
            );
        } else {
            let mut terminal = self.terminal.lock().await;
            // restore terminal
            disable_raw_mode().map_err(MonitorError::Io).unwrap();
            execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture
            )
            .map_err(MonitorError::Io).unwrap();
            terminal.show_cursor().map_err(MonitorError::Io).unwrap();

        }
        CastResponse::NoReply(state)
    }
}

impl EthrexMonitorState {
    pub async fn new(
        sequencer_state: SequencerState,
        store: Store,
        rollup_store: StoreRollup,
        cfg: &SequencerConfig,
    ) -> Result<Self, MonitorError> {
        let eth_client = EthClient::new(cfg.eth.rpc_url.first().expect("No RPC URLs provided"))
            .expect("Failed to create EthClient");
        // TODO: De-hardcode the rollup client URL
        let rollup_client =
            EthClient::new("http://localhost:1729").expect("Failed to create RollupClient");

        Ok(EthrexMonitorState {
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
            logger: Arc::new(TuiWidgetState::new().set_default_display_level(tui_logger::LevelFilter::Info)),
            node_status: NodeStatusTable::new(sequencer_state.clone(), &store).await,
            mempool: MempoolTable::new(&rollup_client).await,
            batches_table: BatchesTable::new(
                cfg.l1_committer.on_chain_proposer_address,
                &eth_client,
                &rollup_store,
            )
            .await?,
            blocks_table: BlocksTable::new(&store).await?,
            l1_to_l2_messages: L1ToL2MessagesTable::new(
                cfg.l1_watcher.bridge_address,
                &eth_client,
                &store,
            )
            .await?,
            l2_to_l1_messages: L2ToL1MessagesTable::new(
                cfg.l1_watcher.bridge_address,
                &eth_client,
                &rollup_client,
            )
            .await?,
            eth_client,
            rollup_client,
            store,
            rollup_store,
            last_tick: Instant::now(),
        })
    }
}

fn draw(terminal: &mut Terminal<impl Backend>, state: &mut EthrexMonitorState) -> Result<(), MonitorError> {
    terminal.draw(|frame| {
        frame.render_widget(state, frame.area());
    })?;
    Ok(())
}

pub fn on_key_event(code: KeyCode, state: &mut EthrexMonitorState) { // todo remove unwraps
    let logger = &state.logger;
    match (&state.tabs, code) {
        (TabsState::Logs, KeyCode::Left) => logger.transition(TuiWidgetEvent::LeftKey),
        (TabsState::Logs, KeyCode::Down) => logger.transition(TuiWidgetEvent::DownKey),
        (TabsState::Logs, KeyCode::Up) => logger.transition(TuiWidgetEvent::UpKey),
        (TabsState::Logs, KeyCode::Right) => logger.transition(TuiWidgetEvent::RightKey),
        (TabsState::Logs, KeyCode::Char('h')) => {
            logger.transition(TuiWidgetEvent::HideKey)
        }
        (TabsState::Logs, KeyCode::Char('f')) => {
            logger.transition(TuiWidgetEvent::FocusKey)
        }
        (TabsState::Logs, KeyCode::Char('+')) => {
            logger.transition(TuiWidgetEvent::PlusKey)
        }
        (TabsState::Logs, KeyCode::Char('-')) => {
            logger.transition(TuiWidgetEvent::MinusKey)
        }
        (TabsState::Overview | TabsState::Logs, KeyCode::Char('Q')) => state.should_quit = true,
        (TabsState::Overview | TabsState::Logs, KeyCode::Tab) => state.tabs.next(),
        _ => {}
    }
}

pub fn on_mouse_event(kind: MouseEventKind, state: &mut EthrexMonitorState) {
    let logger = &state.logger;
    match (&state.tabs, kind) {
        (TabsState::Logs, MouseEventKind::ScrollDown) => {
            logger.transition(TuiWidgetEvent::NextPageKey)
        }
        (TabsState::Logs, MouseEventKind::ScrollUp) => {
            logger.transition(TuiWidgetEvent::PrevPageKey)
        }
        _ => {}
    }
}

pub async fn on_tick(state: &mut EthrexMonitorState) -> Result<(), MonitorError> {
    state.node_status.on_tick(&state.store).await;
    state.global_chain_status
        .on_tick(&state.eth_client, &state.store, &state.rollup_store)
        .await;
    state.mempool.on_tick(&state.rollup_client).await;
    state.batches_table
        .on_tick(&state.eth_client, &state.rollup_store)
        .await?;
    state.blocks_table.on_tick(&state.store).await?;
    state.l1_to_l2_messages
        .on_tick(&state.eth_client, &state.store)
        .await?;
    state.l2_to_l1_messages
        .on_tick(&state.eth_client, &state.rollup_client)
        .await?;

    Ok(())
}


impl Widget for &mut EthrexMonitorState {
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
