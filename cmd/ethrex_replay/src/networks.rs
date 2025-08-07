use std::{
    fmt::{self},
    path::{Path, PathBuf},
};

use ethrex_common::types::{Genesis, GenesisError};
use ethrex_p2p::types::Node;

pub const HOLESKY_GENESIS_PATH: &str = "cmd/ethrex/networks/holesky/genesis.json";
pub const HOLESKY_GENESIS_CONTENTS: &str =
    include_str!("../../ethrex/networks/holesky/genesis.json");
const HOLESKY_BOOTNODES: &str = include_str!("../../ethrex/networks/holesky/bootnodes.json");

pub const SEPOLIA_GENESIS_PATH: &str = "cmd/ethrex/networks/sepolia/genesis.json";
pub const SEPOLIA_GENESIS_CONTENTS: &str =
    include_str!("../../ethrex/networks/sepolia/genesis.json");
const SEPOLIA_BOOTNODES: &str = include_str!("../../ethrex/networks/sepolia/bootnodes.json");

pub const HOODI_GENESIS_PATH: &str = "cmd/ethrex/networks/hoodi/genesis.json";
pub const HOODI_GENESIS_CONTENTS: &str = include_str!("../../ethrex/networks/hoodi/genesis.json");
const HOODI_BOOTNODES: &str = include_str!("../../ethrex/networks/hoodi/bootnodes.json");

pub const MAINNET_GENESIS_PATH: &str = "cmd/ethrex/networks/mainnet/genesis.json";
pub const MAINNET_GENESIS_CONTENTS: &str =
    include_str!("../../ethrex/networks/mainnet/genesis.json");
const MAINNET_BOOTNODES: &str = include_str!("../../ethrex/networks/mainnet/bootnodes.json");

pub const LOCAL_DEVNET_GENESIS_PATH: &str = "../../fixtures/genesis/l1-dev.json";
pub const LOCAL_DEVNETL2_GENESIS_PATH: &str = "../../fixtures/genesis/l2.json";
pub const LOCAL_DEVNET_GENESIS_CONTENTS: &str =
    include_str!("../../../fixtures/genesis/l1-dev.json");
pub const LOCAL_DEVNETL2_GENESIS_CONTENTS: &str = include_str!("../../../fixtures/genesis/l2.json");

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
            // Note that we don't allow to manually specify the local devnet genesis
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

    pub fn get_genesis_path(&self) -> &Path {
        match self {
            Network::PublicNetwork(PublicNetwork::Holesky) => Path::new(HOLESKY_GENESIS_PATH),
            Network::PublicNetwork(PublicNetwork::Hoodi) => Path::new(HOODI_GENESIS_PATH),
            Network::PublicNetwork(PublicNetwork::Mainnet) => Path::new(MAINNET_GENESIS_PATH),
            Network::PublicNetwork(PublicNetwork::Sepolia) => Path::new(SEPOLIA_GENESIS_PATH),
            Network::LocalDevnet => Path::new(LOCAL_DEVNET_GENESIS_PATH),
            Network::LocalDevnetL2 => Path::new(LOCAL_DEVNETL2_GENESIS_PATH),
            Network::GenesisPath(s) => s,
        }
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

    pub fn get_bootnodes(&self) -> Vec<Node> {
        let bootnodes = match self {
            Network::PublicNetwork(PublicNetwork::Holesky) => HOLESKY_BOOTNODES,
            Network::PublicNetwork(PublicNetwork::Hoodi) => HOODI_BOOTNODES,
            Network::PublicNetwork(PublicNetwork::Mainnet) => MAINNET_BOOTNODES,
            Network::PublicNetwork(PublicNetwork::Sepolia) => SEPOLIA_BOOTNODES,
            _ => return vec![],
        };
        serde_json::from_str(bootnodes).expect("bootnodes file should be valid JSON")
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
