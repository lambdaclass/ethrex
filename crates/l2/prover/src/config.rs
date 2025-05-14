use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct ProverConfig {
    pub prover_server_endpoint: String,
    pub proving_time_ms: u64,
}
