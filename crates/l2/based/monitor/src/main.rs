use std::error::Error;
use std::time::Duration;

use clap::Parser;

mod monitor;
mod runner;

mod ui;

#[derive(Debug, Parser)]
struct MonitorCLI {
    /// time in ms between two ticks.
    #[arg(short, long, default_value_t = 250)]
    tick_rate: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = MonitorCLI::parse();
    let tick_rate = Duration::from_millis(cli.tick_rate);
    crate::runner::run(tick_rate).await?;
    Ok(())
}
