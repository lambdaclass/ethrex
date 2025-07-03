#![expect(clippy::expect_used)]

use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ethrex_rpc::EthClient;
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use tui_logger::{TuiWidgetEvent, TuiWidgetState};

use crate::{
    SequencerConfig,
    monitor::{
        ui,
        widget::{
            BatchesTable, BlocksTable, GlobalChainStatusTable, L1ToL2MessagesTable,
            L2ToL1MessagesTable, MempoolTable, NodeStatusTable,
        },
    },
    sequencer::errors::MonitorError,
};

pub struct TabsState<'a> {
    pub titles: Vec<&'a str>,
    pub index: usize,
}

impl<'a> TabsState<'a> {
    pub const fn new(titles: Vec<&'a str>) -> Self {
        Self { titles, index: 0 }
    }
    pub fn next(&mut self) {
        self.index = (self.index + 1) % self.titles.len();
    }

    pub fn previous(&mut self) {
        if self.index > 0 {
            self.index -= 1;
        } else {
            self.index = self.titles.len() - 1;
        }
    }
}

pub struct EthrexMonitor<'a> {
    pub title: &'a str,
    pub should_quit: bool,
    pub tabs: TabsState<'a>,
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
}

impl<'a> EthrexMonitor<'a> {
    pub async fn new(cfg: &SequencerConfig) -> Self {
        let eth_client = EthClient::new(cfg.eth.rpc_url.first().expect("No RPC URLs provided"))
            .expect("Failed to create EthClient");
        // TODO: De-hardcode the rollup client URL
        let rollup_client =
            EthClient::new("http://localhost:1729").expect("Failed to create RollupClient");

        EthrexMonitor {
            title: if cfg.based.based {
                "Based Ethrex Monitor"
            } else {
                "Ethrex Monitor"
            },
            should_quit: false,
            tabs: TabsState::new(vec!["Overview", "Logs"]),
            tick_rate: cfg.monitor.tick_rate,
            global_chain_status: GlobalChainStatusTable::new(&eth_client, &rollup_client, cfg)
                .await,
            logger: TuiWidgetState::new().set_default_display_level(tui_logger::LevelFilter::Info),
            node_status: NodeStatusTable::new(&rollup_client).await,
            mempool: MempoolTable::new(&rollup_client).await,
            batches_table: BatchesTable::new(
                cfg.l1_committer.on_chain_proposer_address,
                &eth_client,
                &rollup_client,
            )
            .await,
            blocks_table: BlocksTable::new(&rollup_client).await,
            l1_to_l2_messages: L1ToL2MessagesTable::new(cfg.l1_watcher.bridge_address, &eth_client)
                .await,
            l2_to_l1_messages: L2ToL1MessagesTable::new(
                cfg.l1_watcher.bridge_address,
                &eth_client,
                &rollup_client,
            )
            .await,
            eth_client,
            rollup_client,
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
            terminal.draw(|frame| ui::render(frame, self))?;

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

    pub fn on_key_event(&mut self, code: KeyCode) {
        match code {
            KeyCode::Left => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::LeftKey),
                _ => {}
            },
            KeyCode::Down => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::DownKey),
                _ => {}
            },
            KeyCode::Up => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::UpKey),
                _ => {}
            },
            KeyCode::Right => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::RightKey),
                _ => {}
            },
            KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Char('h') => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::HideKey),
                _ => {}
            },
            KeyCode::Char('f') => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::FocusKey),
                _ => {}
            },
            KeyCode::Char('+') => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::PlusKey),
                _ => {}
            },
            KeyCode::Char('-') => match self.tabs.index {
                0 => {}
                1 => self.logger.transition(TuiWidgetEvent::MinusKey),
                _ => {}
            },
            KeyCode::Tab => self.tabs.next(),
            _ => {}
        }
    }

    pub fn on_mouse_event(&mut self, kind: MouseEventKind) {
        match kind {
            MouseEventKind::ScrollDown => self.logger.transition(TuiWidgetEvent::NextPageKey),
            MouseEventKind::ScrollUp => self.logger.transition(TuiWidgetEvent::PrevPageKey),
            _ => {}
        }
    }

    pub async fn on_tick(&mut self) {
        self.node_status.on_tick(&self.rollup_client).await;
        self.global_chain_status
            .on_tick(&self.eth_client, &self.rollup_client)
            .await;
        self.mempool.on_tick(&self.rollup_client).await;
        self.batches_table
            .on_tick(&self.eth_client, &self.rollup_client)
            .await;
        self.blocks_table.on_tick(&self.rollup_client).await;
        self.l1_to_l2_messages.on_tick(&self.eth_client).await;
        self.l2_to_l1_messages
            .on_tick(&self.eth_client, &self.rollup_client)
            .await;
    }
}
