use std::{
    fmt::{self},
    path::PathBuf,
};

use ethrex_common::types::{Genesis, GenesisError};

pub const HOLESKY_GENESIS_CONTENTS: &str =
    include_str!("../../ethrex/networks/holesky/genesis.json");
pub const SEPOLIA_GENESIS_CONTENTS: &str =
    include_str!("../../ethrex/networks/sepolia/genesis.json");
pub const HOODI_GENESIS_CONTENTS: &str = include_str!("../../ethrex/networks/hoodi/genesis.json");
pub const MAINNET_GENESIS_CONTENTS: &str =
    include_str!("../../ethrex/networks/mainnet/genesis.json");
pub const LOCAL_DEVNET_GENESIS_CONTENTS: &str =
    include_str!("../../../fixtures/genesis/l1-dev.json");
pub const LOCAL_DEVNETL2_GENESIS_CONTENTS: &str = include_str!("../../../fixtures/genesis/l2.json");

#[expect(clippy::enum_variant_names)]
#[derive(Debug, Clone)]
pub enum Network {
    PublicNetwork(PublicNetwork),
    LocalDevnet,
    LocalDevnetL2,
    GenesisPath(PathBuf),
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
            "mainnet" => Network::PublicNetwork(PublicNetwork::Mainnet),
            "sepolia" => Network::PublicNetwork(PublicNetwork::Sepolia),
            "local-devnet" => Network::LocalDevnet,
            "local-devnet-l2" => Network::LocalDevnetL2,
            s => Network::GenesisPath(PathBuf::from(s)),
        }
    }
}

impl From<PathBuf> for Network {
    fn from(value: PathBuf) -> Self {
        Network::GenesisPath(value)
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
            Network::LocalDevnet => write!(f, "local-devnet"),
            Network::LocalDevnetL2 => write!(f, "local-devnet-l2"),
            Network::GenesisPath(path_buf) => write!(f, "{path_buf:?}"),
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
            Network::LocalDevnet => Ok(serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS)?),
            Network::LocalDevnetL2 => Ok(serde_json::from_str(LOCAL_DEVNETL2_GENESIS_CONTENTS)?),
            Network::GenesisPath(s) => Genesis::try_from(s.as_path()),
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
