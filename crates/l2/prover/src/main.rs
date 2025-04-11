use ethrex_l2::utils::config::{
    prover_client::ProverClientConfig, read_env_file_by_config, ConfigMode,
};
use ethrex_prover_lib::init_client;
use tracing::{self, debug, error, Level};

#[tokio::main]
async fn main() {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        // Hiding debug!() logs.
        .with_max_level(Level::INFO)
        .finish();
    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        error!("Failed setting tracing::subscriber: {e}");
        return;
    }

    if let Err(e) = ProverClientConfig::toml_to_env() {
        warn!("Failed to parse .toml file: {e}");
    }

    if let Err(e) = read_env_file_by_config(ConfigMode::ProverClient) {
        warn!("Failed to read .env file. It is '.env.prover' by default: {e}");
    }

    let Ok(config) = ProverClientConfig::from_env() else {
        error!("Failed to read ProverClientConfig from environment variables. You can set them in a .env or .toml file");
        return;
    };

    debug!("Prover Client has started");
    init_client(config).await;
}
