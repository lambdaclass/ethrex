use ethereum_rust_l2::utils::config::{prover_client::ProverClientConfig, read_env_file};
use ethereum_rust_prover_lib::init_client;

use tracing::{self, debug, warn, Level};

#[tokio::main]
async fn main() {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    if let Err(e) = read_env_file() {
        warn!("Failed to read .env file: {e}");
    }

    let config = ProverClientConfig::from_env().unwrap();
    debug!("Prover Client has started");
    init_client(config).await;
}
