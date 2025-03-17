use ethrex_p2p::types::Node;
use lazy_static::lazy_static;

pub const HOLESKY_GENESIS_PATH: &str = "cmd/ethrex/networks/holesky/genesis.json";
pub const SEPOLIA_GENESIS_PATH: &str = "cmd/ethrex/networks/sepolia/genesis.json";
pub const EPHEMERY_GENESIS_PATH: &str = "cmd/ethrex/networks/ephemery/genesis.json";

pub const HOLESKY_BOOTNODES_PATH: &str = "cmd/ethrex/networks/holesky/bootnodes.json";
pub const SEPOLIA_BOOTNODES_PATH: &str = "cmd/ethrex/networks/sepolia/bootnodes.json";
pub const EPHEMERY_BOOTNODES_PATH: &str = "cmd/ethrex/networks/ephemery/bootnodes.json";

lazy_static! {
    pub static ref HOLESKY_BOOTNODES: Vec<Node> = serde_json::from_reader(
        std::fs::File::open(HOLESKY_BOOTNODES_PATH).expect("Failed to open holesky bootnodes file")
    )
    .expect("Failed to parse holesky bootnodes file");
    pub static ref SEPOLIA_BOOTNODES: Vec<Node> = serde_json::from_reader(
        std::fs::File::open(SEPOLIA_BOOTNODES_PATH).expect("Failed to open sepolia bootnodes file")
    )
    .expect("Failed to parse sepolia bootnodes file");
    pub static ref EPHEMERY_BOOTNODES: Vec<Node> = serde_json::from_reader(
        std::fs::File::open(EPHEMERY_BOOTNODES_PATH)
            .expect("Failed to open ephemery bootnodes file")
    )
    .expect("Failed to parse ephemery bootnodes file");
}
