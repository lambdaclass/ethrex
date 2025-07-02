use std::error::Error;
use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

pub mod app;
pub mod ui;
pub(crate) mod utils;

pub use app::EthrexMonitor;
pub use ui::render;

use crate::SequencerConfig;
use crate::sequencer::errors::{MonitorError, SequencerError};

pub async fn start_monitor(cfg: SequencerConfig) -> Result<(), SequencerError> {
    // setup terminal
    enable_raw_mode().map_err(MonitorError::Io)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(MonitorError::Io)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(MonitorError::Io)?;

    // create app and run it
    let app = EthrexMonitor::new(&cfg).await;
    let app_result = run_app(&mut terminal, app, &cfg).await;

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
        if let Some(monitor_err) = err.downcast_ref::<MonitorError>() {
            eprintln!("Monitor error: {monitor_err}");
        } else {
            eprintln!("Unexpected error: {err}");
        }
    });

    Ok(())
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: EthrexMonitor<'_>,
    cfg: &SequencerConfig,
) -> Result<(), Box<dyn Error>> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        let timeout =
            Duration::from_millis(cfg.monitor.tick_rate).saturating_sub(last_tick.elapsed());
        if !event::poll(timeout)? {
            app.on_tick().await;
            last_tick = Instant::now();
            continue;
        }
        let event = event::read()?;
        if let Some(key) = event.as_key_press_event() {
            app.on_key_event(key.code);
        }
        if let Some(mouse) = event.as_mouse_event() {
            app.on_mouse_event(mouse.kind);
        }
        if app.should_quit {
            return Ok(());
        }
    }
}
