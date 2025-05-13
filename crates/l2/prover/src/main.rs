pub mod cli;
use crate::cli::ProverClientOptions;
use clap::Parser;
use ethrex_prover_lib::init_client;
use tracing::{self, debug, error};

#[tokio::main]
async fn main() {
    let options = ProverClientOptions::parse();

    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(options.log_level)
        .finish();
    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        error!("Failed setting tracing::subscriber: {e}");
        return;
    }

    debug!("Prover Client has started");
    init_client(options.into()).await;
}
