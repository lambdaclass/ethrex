mod cli;
mod prover_client;

use crate::{cli::ProverCLI, prover_client::start_prover};
use clap::Parser;

#[tokio::main]
async fn main() {
    let options = ProverCLI::parse();

    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(options.prover_client_options.log_level)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set global tracing subscriber");

    start_prover(options.prover_client_options.into()).await;
}
