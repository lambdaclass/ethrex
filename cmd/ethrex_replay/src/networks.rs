use std::{
    fmt::{self},
    path::{Path, PathBuf},
};

use ethrex_common::types::{Genesis, GenesisError};
use lazy_static::lazy_static;

pub const HOLESKY_GENESIS_CONTENTS: &str = include_str!("../../ethrex/networks/holesky/genesis.json");
pub const SEPOLIA_GENESIS_CONTENTS: &str = include_str!("../../ethrex/networks/sepolia/genesis.json");
pub const HOODI_GENESIS_CONTENTS: &str = include_str!("../../ethrex/networks/hoodi/genesis.json");
pub const MAINNET_GENESIS_CONTENTS: &str = include_str!("../../ethrex/networks/mainnet/genesis.json");


#[derive(Debug, Clone)]
pub enum Network {
    PublicNetwork(PublicNetwork),
}

#[derive(Debug, Clone, Copy)]
pub enum PublicNetwork {
    Hoodi,
    Holesky,
    Sepolia,
    Mainnet,
}

impl From<&str> for Network {
    fn from(value: &str) -> Self {
        match value {
            "hoodi" => Network::PublicNetwork(PublicNetwork::Hoodi),
            "holesky" => Network::PublicNetwork(PublicNetwork::Holesky),
            "sepolia" => Network::PublicNetwork(PublicNetwork::Sepolia),
            &_ =>  Network::PublicNetwork(PublicNetwork::Mainnet),
        }
    }
}

impl Default for Network {
    fn default() -> Self {
        Network::PublicNetwork(PublicNetwork::Mainnet)
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Network::PublicNetwork(PublicNetwork::Holesky) => write!(f, "holesky"),
            Network::PublicNetwork(PublicNetwork::Hoodi) => write!(f, "hoodi"),
            Network::PublicNetwork(PublicNetwork::Mainnet) => write!(f, "mainnet"),
            Network::PublicNetwork(PublicNetwork::Sepolia) => write!(f, "sepolia"),
        }
    }
}

impl Network {
    pub fn mainnet() -> Self {
        Network::PublicNetwork(PublicNetwork::Mainnet)
    }

    pub fn get_genesis(&self) -> Result<Genesis, GenesisError> {
        match self {
            Network::PublicNetwork(public_network) => {
                Ok(serde_json::from_str(get_genesis_contents(*public_network))?)
            }
        }
    }
}

fn get_genesis_contents(network: PublicNetwork) -> &'static str {
    match network {
        PublicNetwork::Holesky => HOLESKY_GENESIS_CONTENTS,
        PublicNetwork::Hoodi => HOODI_GENESIS_CONTENTS,
        PublicNetwork::Mainnet => MAINNET_GENESIS_CONTENTS,
        PublicNetwork::Sepolia => SEPOLIA_GENESIS_CONTENTS,
    }
}
