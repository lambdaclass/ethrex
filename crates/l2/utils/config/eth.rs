use crate::utils::parse::url_deserializer;
use reqwest::Url;
use serde::Deserialize;

use super::L2Config;

#[derive(Deserialize, Debug)]
pub struct EthConfig {
    #[serde(deserialize_with = "url_deserializer")]
    pub rpc_url: Url,
}

impl L2Config for EthConfig {
    const PREFIX: &str = "ETH_";

    fn to_env(&self) -> String {
        format!("{prefix}RPC_URL={}", self.rpc_url, prefix = Self::PREFIX)
    }
}
