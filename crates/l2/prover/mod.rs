use tracing::info;

pub mod prover;
pub mod prover_client;

pub async fn start_prover() {
    let prover_client = tokio::spawn(prover_client::start_prover_client());

    tokio::try_join!(prover_client).unwrap();
    info!("Prover finished!");
}
