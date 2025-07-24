use serde::Deserialize;

use crate::backends::Backend;

#[derive(Deserialize, Debug)]
pub struct ProverConfig {
    pub backend: Backend,
    pub http_addr: String,
    pub http_port: u16,
    pub proving_time_ms: u64,
    pub aligned_mode: bool,
}
