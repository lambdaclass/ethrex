use std::path::{Path, PathBuf};

use ethrex_p2p::types::Node;
use lazy_static::lazy_static;

pub const HOLESKY_GENESIS_PATH: &str = "cmd/ethrex/networks/holesky/genesis.json";
const HOLESKY_BOOTNODES_PATH: &str = "cmd/ethrex/networks/holesky/bootnodes.json";

pub const SEPOLIA_GENESIS_PATH: &str = "cmd/ethrex/networks/sepolia/genesis.json";
const SEPOLIA_BOOTNODES_PATH: &str = "cmd/ethrex/networks/sepolia/bootnodes.json";

pub const HOODI_GENESIS_PATH: &str = "cmd/ethrex/networks/hoodi/genesis.json";
const HOODI_BOOTNODES_PATH: &str = "cmd/ethrex/networks/hoodi/bootnodes.json";

pub const MAINNET_GENESIS_PATH: &str = "cmd/ethrex/networks/mainnet/genesis.json";
const MAINNET_BOOTNODES_PATH: &str = "cmd/ethrex/networks/mainnet/bootnodes.json";

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
    PublicNetwork(PublicNetworkType),
    GenesisPath(PathBuf),
}
#[derive(Debug, Clone)]
pub enum PublicNetworkType {
    Hoodi,
    Holesky,
    Sepolia,
    Mainnet,
}

impl From<&str> for Network {
    fn from(value: &str) -> Self {
        match value {
            "hoodi" => Network::PublicNetwork(PublicNetworkType::Hoodi),
            "holesky" => Network::PublicNetwork(PublicNetworkType::Holesky),
            "mainnet" => Network::PublicNetwork(PublicNetworkType::Mainnet),
            "sepolia" => Network::PublicNetwork(PublicNetworkType::Sepolia),
            s => Network::GenesisPath(PathBuf::from(s)),
        }
    }
}

impl From<PathBuf> for Network {
    fn from(value: PathBuf) -> Self {
        Network::GenesisPath(value)
    }
}

impl Network {
    pub fn get_path(&self) -> &Path {
        match self {
            Network::PublicNetwork(PublicNetworkType::Holesky) => Path::new(HOLESKY_GENESIS_PATH),
            Network::PublicNetwork(PublicNetworkType::Hoodi) => Path::new(HOODI_GENESIS_PATH),
            Network::PublicNetwork(PublicNetworkType::Mainnet) => Path::new(MAINNET_GENESIS_PATH),
            Network::PublicNetwork(PublicNetworkType::Sepolia) => Path::new(SEPOLIA_GENESIS_PATH),
            Network::GenesisPath(s) => s,
        }
    }
}
