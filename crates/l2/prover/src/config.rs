use serde::Deserialize;
use url::Url;

use crate::backends::Backend;

#[derive(Deserialize, Debug)]
pub struct ProverConfig {
    pub backend: Backend,
    pub proof_coordinator: Url,
    pub proving_time_ms: u64,
    pub aligned_mode: bool,
}
