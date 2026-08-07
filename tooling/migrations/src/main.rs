mod cli;
mod utils;

use crate::cli::CLI;
use clap::Parser;

#[tokio::main]
async fn main() {
    // Without a subscriber, tracing logs from the migration, store, and
    // blockchain crates are silently dropped.
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let CLI { command } = CLI::parse();

    command.run().await;
}
