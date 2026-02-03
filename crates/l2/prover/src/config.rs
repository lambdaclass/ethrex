use serde::Deserialize;
use url::Url;

use crate::backend::BackendType;

#[derive(Deserialize, Debug)]
pub struct ProverConfig {
    pub backend: BackendType,
    pub proof_coordinators: Vec<Url>,
    pub proving_time_ms: u64,
    #[cfg(all(feature = "sp1", feature = "gpu"))]
    pub sp1_server: Option<Url>,
}
