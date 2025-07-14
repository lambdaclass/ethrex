use std::{
    fmt::{self},
    path::{Path, PathBuf},
};

use ethrex_common::types::{Genesis, GenesisError};
use ethrex_p2p::types::Node;
use lazy_static::lazy_static;

pub const HOLESKY_GENESIS_PATH: &str = "cmd/ethrex/networks/holesky/genesis.json";
pub const HOLESKY_GENESIS_CONTENTS: &str = include_str!("networks/holesky/genesis.json");
const HOLESKY_BOOTNODES_PATH: &str = "cmd/ethrex/networks/holesky/bootnodes.json";

pub const SEPOLIA_GENESIS_PATH: &str = "cmd/ethrex/networks/sepolia/genesis.json";
pub const SEPOLIA_GENESIS_CONTENTS: &str = include_str!("networks/sepolia/genesis.json");
const SEPOLIA_BOOTNODES_PATH: &str = "cmd/ethrex/networks/sepolia/bootnodes.json";

pub const HOODI_GENESIS_PATH: &str = "cmd/ethrex/networks/hoodi/genesis.json";
pub const HOODI_GENESIS_CONTENTS: &str = include_str!("networks/hoodi/genesis.json");
const HOODI_BOOTNODES_PATH: &str = "cmd/ethrex/networks/hoodi/bootnodes.json";

pub const MAINNET_GENESIS_PATH: &str = "cmd/ethrex/networks/mainnet/genesis.json";
pub const MAINNET_GENESIS_CONTENTS: &str = include_str!("networks/mainnet/genesis.json");
const MAINNET_BOOTNODES_PATH: &str = "cmd/ethrex/networks/mainnet/bootnodes.json";

pub const LOCAL_DEVNET_GENESIS_PATH: &str = "../../fixtures/genesis/l1-dev.json";
#[cfg(feature = "dev")]
pub const LOCAL_DEVNET_GENESIS_CONTENTS: &str = include_str!("../../fixtures/genesis/l1-dev.json");
#[cfg(not(feature = "dev"))]
pub const LOCAL_DEVNET_GENESIS_CONTENTS: &str = "";

lazy_static! {
    pub static ref HOLESKY_BOOTNODES: Vec<Node> = serde_json::from_reader(
        std::fs::File::open(HOLESKY_BOOTNODES_PATH).expect("Failed to open holesky bootnodes file")
    )
    .expect("Failed to parse holesky bootnodes file");
    pub static ref SEPOLIA_BOOTNODES: Vec<Node> = serde_json::from_reader(
        std::fs::File::open(SEPOLIA_BOOTNODES_PATH).expect("Failed to open sepolia bootnodes file")
    )
    .expect("Failed to parse sepolia bootnodes file");
    pub static ref HOODI_BOOTNODES: Vec<Node> = serde_json::from_reader(
        std::fs::File::open(HOODI_BOOTNODES_PATH).expect("Failed to open hoodi bootnodes file")
    )
    .expect("Failed to parse hoodi bootnodes file");
    pub static ref MAINNET_BOOTNODES: Vec<Node> = serde_json::from_reader(
        std::fs::File::open(MAINNET_BOOTNODES_PATH).expect("Failed to open mainnet bootnodes file")
    )
    .expect("Failed to parse mainnet bootnodes file");
}

#[derive(Debug, Clone)]
pub enum Network {
    PublicNetwork(PublicNetwork),
    LocalDevnet,
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
            Network::GenesisPath(s) => s,
        }
    }

    pub fn get_genesis(&self) -> Result<Genesis, GenesisError> {
        match self {
            Network::PublicNetwork(public_network) => {
                Ok(serde_json::from_str(get_genesis_contents(*public_network))?)
            }
            Network::LocalDevnet => Ok(serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS)?),
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
