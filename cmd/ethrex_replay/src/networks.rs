use ethrex_common::types::{Genesis, GenesisError};

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

impl Network {
    pub fn from_network_name(value: &str) -> eyre::Result<Self> { // TODO: consider removing the () for a proper error
        match value {
            "hoodi" => Ok(Network::PublicNetwork(PublicNetwork::Hoodi)),
            "holesky" => Ok(Network::PublicNetwork(PublicNetwork::Holesky)),
            "mainnet" => Ok(Network::PublicNetwork(PublicNetwork::Mainnet)),
            "sepolia" => Ok(Network::PublicNetwork(PublicNetwork::Sepolia)),
            &_ => Err(eyre::Error::msg("Network not known"))
        }
    }
}

impl Network {
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
