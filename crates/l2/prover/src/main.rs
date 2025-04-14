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

    let config = match ProverClientConfig::load() {
        Ok(config) => config,
        Err(err) => {
            error!("Failed to load config. {err}");
            return;
        }
    };

    debug!("Prover Client has started");
    init_client(config).await;
}
