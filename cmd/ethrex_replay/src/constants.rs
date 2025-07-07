// Blockchain related constants

use std::str::FromStr;

use ethrex_common::{
    H160,
    types::{BlobSchedule, ChainConfig},
};
use crate::networks::Network;

pub fn get_chain_config(name: &str) -> eyre::Result<ChainConfig> {
    Ok(Network::from(name).get_genesis().map_err(|_| eyre::Error::msg("network not found"))?.config)
}
