use ethrex_l2::utils::{
    config::{prover_client::ProverClientConfig, read_env_file},
    prover::proving_systems::ProverType,
};
use ethrex_prover_lib::init_client;
use std::env;
use tracing::{self, debug, error, warn, Level};

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

    if let Err(e) = read_env_file() {
        warn!("Failed to read .env file: {e}");
    }

    let Ok(config) = ProverClientConfig::from_env() else {
        error!("Failed to read ProverClientConfig from .env file");
        return;
    };

    let args: Vec<String> = env::args().collect();

    let prover_type = match args.len() {
        2 => {
            let prover_type_str = args.get(1).map_or("none", |v| v);
            match prover_type_str {
                "sp1" => ProverType::SP1,
                "risc0" => ProverType::RISC0,
                _ => {
                    error!("Wrong argument, try with 'risc0' or 'sp1'.");
                    return;
                }
            }
        }
        _ => {
            error!("Try passing 'risc0' or 'sp1' as argument.");
            return;
        }
    };

    debug!("Prover Client has started");
    init_client(config, prover_type).await;
}
