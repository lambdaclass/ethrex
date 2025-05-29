use std::path::Path;

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
pub enum Networks {
    PublicNetwork(PublicNetworkType),
    GenesisPath(String),
}

pub enum PublicNetworkType {
    Hoodi,
    Holesky,
    Sepolia,
    Mainnet,
}

impl From<&str> for Networks {
    fn from(value: &str) -> Self {
        match value {
            "hoodi" => Networks::PublicNetwork(PublicNetworkType::Hoodi),
            "holesky" => Networks::PublicNetwork(PublicNetworkType::Holesky),
            "mainnet" => Networks::PublicNetwork(PublicNetworkType::Mainnet),
            "sepolia" => Networks::PublicNetwork(PublicNetworkType::Sepolia),
            s => Networks::GenesisPath(String::from(s)),
        }
    }
}

impl Networks {
    pub fn get_path(&self) -> &Path {
        match self {
            Networks::PublicNetwork(PublicNetworkType::Holesky) => Path::new(HOLESKY_GENESIS_PATH),
            Networks::PublicNetwork(PublicNetworkType::Hoodi) => Path::new(HOODI_GENESIS_PATH),
            Networks::PublicNetwork(PublicNetworkType::Mainnet) => Path::new(MAINNET_GENESIS_PATH),
            Networks::PublicNetwork(PublicNetworkType::Sepolia) => Path::new(SEPOLIA_GENESIS_PATH),
            Networks::GenesisPath(s) => Path::new(s),
        }
    }

    pub fn get_bootnodes(&self) -> &Path {
        match self {
            Networks::PublicNetwork(PublicNetworkType::Holesky) => Path::new(HOLESKY_GENESIS_PATH),
            Networks::PublicNetwork(PublicNetworkType::Hoodi) => Path::new(HOODI_GENESIS_PATH),
            Networks::PublicNetwork(PublicNetworkType::Mainnet) => Path::new(MAINNET_GENESIS_PATH),
            Networks::PublicNetwork(PublicNetworkType::Sepolia) => Path::new(SEPOLIA_GENESIS_PATH),
            Networks::GenesisPath(s) => Path::new(s),
        }
    }
}
