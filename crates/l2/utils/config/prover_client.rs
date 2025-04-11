use crate::utils::parse::url_deserializer;
use reqwest::Url;
use serde::Deserialize;

use super::L2Config;

#[derive(Deserialize, Debug)]
pub struct ProverClientConfig {
    #[serde(deserialize_with = "url_deserializer")]
    pub prover_server_endpoint: Url,
    pub proving_time_ms: u64,
}

impl L2Config for ProverClientConfig {
    const PREFIX: &str = "PROVER_CLIENT_";

    fn to_env(&self) -> String {
        format!(
            "
{prefix}_PROVER_SERVER_ENDPOINT={}
{prefix}_PROVING_TIME_MS={}
",
            self.prover_server_endpoint,
            self.proving_time_ms,
            prefix = Self::PREFIX
        )
    }
}
