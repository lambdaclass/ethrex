use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct ProverConfig {
    pub http_addr: String,
    pub http_port: u16,
    pub proving_time_ms: u64,
}
