use ethrex_bsc::genesis::{BSC_CHAPEL_CHAIN_ID, BSC_MAINNET_CHAIN_ID};
use ethrex_p2p::types::Node;
use std::{
    fmt::{self},
    path::PathBuf,
};

use ethrex_common::types::{ChainConfig, Genesis, GenesisError};
use serde::{Deserialize, Serialize};

//TODO: Look for a better place to move these files
const MAINNET_BOOTNODES: &str = include_str!("../../../cmd/ethrex/networks/mainnet/bootnodes.json");
const SEPOLIA_BOOTNODES: &str = include_str!("../../../cmd/ethrex/networks/sepolia/bootnodes.json");
const HOODI_BOOTNODES: &str = include_str!("../../../cmd/ethrex/networks/hoodi/bootnodes.json");

pub const MAINNET_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/mainnet/genesis.json");
pub const HOODI_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/hoodi/genesis.json");
pub const SEPOLIA_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/sepolia/genesis.json");
pub const LOCAL_DEVNET_GENESIS_CONTENTS: &str = include_str!("../../../fixtures/genesis/l1.json");
pub const LOCAL_DEVNETL2_GENESIS_CONTENTS: &str = include_str!("../../../fixtures/genesis/l2.json");

pub const LOCAL_DEVNET_PRIVATE_KEYS: &str =
    include_str!("../../../fixtures/keys/private_keys_l1.txt");

pub const MAINNET_CHAIN_ID: u64 = 0x1;
pub const HOODI_CHAIN_ID: u64 = 0x88bb0;
pub const SEPOLIA_CHAIN_ID: u64 = 0xAA36A7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Network {
    PublicNetwork(PublicNetwork),
    LocalDevnet,
    LocalDevnetL2,
    L2Chain(u64),
    #[serde(skip)]
    GenesisPath(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicNetwork {
    Hoodi,
    Sepolia,
    Mainnet,
    BscMainnet,
    BscTestnet,
}

impl From<&str> for Network {
    fn from(value: &str) -> Self {
        match value {
            "hoodi" => Network::PublicNetwork(PublicNetwork::Hoodi),
            "mainnet" => Network::PublicNetwork(PublicNetwork::Mainnet),
            "sepolia" => Network::PublicNetwork(PublicNetwork::Sepolia),
            "bsc-mainnet" | "bsc" => Network::PublicNetwork(PublicNetwork::BscMainnet),
            "bsc-testnet" | "chapel" => Network::PublicNetwork(PublicNetwork::BscTestnet),
            // Note that we don't allow to manually specify the local devnet genesis
            s => Network::GenesisPath(PathBuf::from(s)),
        }
    }
}

impl TryFrom<u64> for Network {
    type Error = String;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            MAINNET_CHAIN_ID => Ok(Network::PublicNetwork(PublicNetwork::Mainnet)),
            SEPOLIA_CHAIN_ID => Ok(Network::PublicNetwork(PublicNetwork::Sepolia)),
            HOODI_CHAIN_ID => Ok(Network::PublicNetwork(PublicNetwork::Hoodi)),
            BSC_MAINNET_CHAIN_ID => Ok(Network::PublicNetwork(PublicNetwork::BscMainnet)),
            BSC_CHAPEL_CHAIN_ID => Ok(Network::PublicNetwork(PublicNetwork::BscTestnet)),
            _ => Err(format!("Unknown chain ID: {}", value)),
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
            Network::PublicNetwork(PublicNetwork::Hoodi) => write!(f, "hoodi"),
            Network::PublicNetwork(PublicNetwork::Mainnet) => write!(f, "mainnet"),
            Network::PublicNetwork(PublicNetwork::Sepolia) => write!(f, "sepolia"),
            Network::PublicNetwork(PublicNetwork::BscMainnet) => write!(f, "bsc-mainnet"),
            Network::PublicNetwork(PublicNetwork::BscTestnet) => write!(f, "bsc-testnet"),
            Network::LocalDevnet => write!(f, "local-devnet"),
            Network::LocalDevnetL2 => write!(f, "local-devnet-l2"),
            Network::L2Chain(chain_id) => write!(f, "l2-chain-{}", chain_id),
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
            Network::PublicNetwork(PublicNetwork::BscMainnet) => {
                Ok(ethrex_bsc::genesis::bsc_mainnet_genesis())
            }
            Network::PublicNetwork(PublicNetwork::BscTestnet) => {
                Ok(ethrex_bsc::genesis::bsc_chapel_genesis())
            }
            Network::PublicNetwork(public_network) => {
                Ok(serde_json::from_str(get_genesis_contents(*public_network))?)
            }
            Network::LocalDevnet => Ok(serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS)?),
            Network::LocalDevnetL2 => Ok(serde_json::from_str(LOCAL_DEVNETL2_GENESIS_CONTENTS)?),
            Network::L2Chain(chain_id) => Ok(Genesis {
                config: ChainConfig {
                    chain_id: *chain_id,
                    prague_time: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            }),
            Network::GenesisPath(s) => Genesis::try_from(s.as_path()),
        }
    }

    /// Returns true if this is a BSC network (mainnet or testnet).
    pub fn is_bsc(&self) -> bool {
        matches!(
            self,
            Network::PublicNetwork(PublicNetwork::BscMainnet | PublicNetwork::BscTestnet)
        )
    }

    /// Returns the network-specific subdirectory name for the datadir.
    /// Public networks get a named suffix; custom genesis files and L2 chains
    /// use their chain ID as suffix.
    pub fn datadir_suffix(&self) -> Option<String> {
        match self {
            Network::PublicNetwork(PublicNetwork::Mainnet) => Some("mainnet".to_owned()),
            Network::PublicNetwork(PublicNetwork::Hoodi) => Some("hoodi".to_owned()),
            Network::PublicNetwork(PublicNetwork::Sepolia) => Some("sepolia".to_owned()),
            Network::PublicNetwork(PublicNetwork::BscMainnet) => Some("bsc-mainnet".to_owned()),
            Network::PublicNetwork(PublicNetwork::BscTestnet) => Some("bsc-testnet".to_owned()),
            Network::LocalDevnet => None,
            Network::LocalDevnetL2 => None,
            Network::L2Chain(chain_id) => Some(format!("chain-{chain_id}")),
            Network::GenesisPath(_) => {
                let chain_id = self.get_genesis().ok()?.config.chain_id;
                Some(format!("chain-{chain_id}"))
            }
        }
    }

    /// Returns all possible datadir subdirectory names (public networks + "dev").
    /// Used by migration logic to detect existing network subdirectories.
    pub fn all_datadir_suffixes() -> &'static [&'static str] {
        // Explicit list derived from PublicNetwork variants + dev mode.
        // Update this when adding new PublicNetwork variants.
        &[
            "mainnet",     // PublicNetwork::Mainnet
            "hoodi",       // PublicNetwork::Hoodi
            "sepolia",     // PublicNetwork::Sepolia
            "bsc-mainnet", // PublicNetwork::BscMainnet
            "bsc-testnet", // PublicNetwork::BscTestnet
            "dev",         // dev mode
        ]
    }

    pub fn get_bootnodes(&self) -> Vec<Node> {
        match self {
            Network::PublicNetwork(
                PublicNetwork::Hoodi | PublicNetwork::Mainnet | PublicNetwork::Sepolia,
            ) => {
                let bootnodes = match self {
                    Network::PublicNetwork(PublicNetwork::Hoodi) => HOODI_BOOTNODES,
                    Network::PublicNetwork(PublicNetwork::Mainnet) => MAINNET_BOOTNODES,
                    Network::PublicNetwork(PublicNetwork::Sepolia) => SEPOLIA_BOOTNODES,
                    _ => unreachable!(),
                };
                serde_json::from_str(bootnodes).expect("bootnodes file should be valid JSON")
            }
            Network::PublicNetwork(PublicNetwork::BscTestnet) => {
                // BSC Chapel testnet bootnodes (port 30311)
                let enodes: &[&str] = &[
                    "enode://db1e2c76cbeb72e0b1a6e27bd78a8e594aa498d3a2792e0f1fa5e3748728a2e3d14bf0bdd8be8a3fec3ad0a13a3e3d69a05abec33bfac9dcbc0afaafba32e5301@52.199.214.252:30311",
                    "enode://e5c4320e7c3ad28ac6e61a6126e5e9c9c0bca6d7b36f7ed74f76ad404bbbe09a6e0ad59d7eb9f56d6a33e4e40d3e7fbd19c59c7c2fd44b04c29b2e23c1ba4eae9@52.51.80.128:30311",
                    "enode://7560180973a5fa2e8b76e4d0a94e2c7fe4e7eaceb93dbd2215be2dba5d6c59073c7b78e06d1c6ddb2d0236e67c80d6be14c42fcb2d99af91dd30eb1c5969c07c0@3.209.122.123:30311",
                    "enode://665cf77ca950e41223547a8153b9a33b3acd6e24f91cca37f44fd1e8d14e58599e38434e9a0964e32d4a03109a94a3f56e7bae4c67c7fce32d45e3d791d4d9cd7@52.72.123.113:30311",
                ];
                enodes
                    .iter()
                    .filter_map(|e| Node::from_enode_url(e).ok())
                    .collect()
            }
            _ => vec![],
        }
    }
}

fn get_genesis_contents(network: PublicNetwork) -> &'static str {
    match network {
        PublicNetwork::Hoodi => HOODI_GENESIS_CONTENTS,
        PublicNetwork::Mainnet => MAINNET_GENESIS_CONTENTS,
        PublicNetwork::Sepolia => SEPOLIA_GENESIS_CONTENTS,
        // BSC genesis is built programmatically, not from a JSON file
        PublicNetwork::BscMainnet | PublicNetwork::BscTestnet => "",
    }
}

#[cfg(test)]
mod tests {
    use ethrex_common::H256;

    use super::*;

    fn assert_genesis_hash(network: PublicNetwork, expected_hash: &str) {
        let genesis = Network::PublicNetwork(network).get_genesis().unwrap();
        let genesis_hash = genesis.get_block().hash();
        let expected_hash = hex::decode(expected_hash).unwrap();
        assert_eq!(genesis_hash, H256::from_slice(&expected_hash));
    }

    #[test]
    fn test_sepolia_genesis_block_hash() {
        // Values taken from the geth codebase:
        // https://github.com/ethereum/go-ethereum/blob/a327ffe9b35289719ac3c484b7332584985b598a/params/config.go#L30-L35
        assert_genesis_hash(
            PublicNetwork::Sepolia,
            "25a5cc106eea7138acab33231d7160d69cb777ee0c2c553fcddf5138993e6dd9",
        );
    }

    #[test]
    fn test_hoodi_genesis_block_hash() {
        // Values taken from the geth codebase:
        // https://github.com/ethereum/go-ethereum/blob/a327ffe9b35289719ac3c484b7332584985b598a/params/config.go#L30-L35
        assert_genesis_hash(
            PublicNetwork::Hoodi,
            "bbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b",
        );
    }

    #[test]
    fn test_mainnet_genesis_block_hash() {
        // Values taken from the geth codebase:
        // https://github.com/ethereum/go-ethereum/blob/a327ffe9b35289719ac3c484b7332584985b598a/params/config.go#L30-L35
        assert_genesis_hash(
            PublicNetwork::Mainnet,
            "d4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3",
        );
    }

    #[test]
    fn test_datadir_suffix_public_networks() {
        assert_eq!(Network::mainnet().datadir_suffix(), Some("mainnet".into()));
        assert_eq!(
            Network::PublicNetwork(PublicNetwork::Hoodi).datadir_suffix(),
            Some("hoodi".into())
        );
        assert_eq!(
            Network::PublicNetwork(PublicNetwork::Sepolia).datadir_suffix(),
            Some("sepolia".into())
        );
    }

    #[test]
    fn test_datadir_suffix_non_public_networks() {
        assert_eq!(Network::LocalDevnet.datadir_suffix(), None);
        assert_eq!(Network::LocalDevnetL2.datadir_suffix(), None);
        assert_eq!(
            Network::L2Chain(42).datadir_suffix(),
            Some("chain-42".into())
        );
        // Invalid genesis path returns None (can't parse chain ID).
        assert_eq!(
            Network::GenesisPath(PathBuf::from("/tmp/nonexistent.json")).datadir_suffix(),
            None
        );
    }

    #[test]
    fn test_all_datadir_suffixes_covers_all_public_networks() {
        let all = Network::all_datadir_suffixes();
        // Every public network suffix must appear in all_datadir_suffixes.
        let networks = [
            Network::PublicNetwork(PublicNetwork::Mainnet),
            Network::PublicNetwork(PublicNetwork::Hoodi),
            Network::PublicNetwork(PublicNetwork::Sepolia),
            Network::PublicNetwork(PublicNetwork::BscMainnet),
            Network::PublicNetwork(PublicNetwork::BscTestnet),
        ];
        for net in &networks {
            let suffix = net
                .datadir_suffix()
                .expect("public networks must have a suffix");
            assert!(
                all.contains(&suffix.as_str()),
                "all_datadir_suffixes() missing suffix {suffix:?} for {net:?}"
            );
        }
        // "dev" must also be present.
        assert!(
            all.contains(&"dev"),
            "all_datadir_suffixes() missing \"dev\""
        );
    }

    #[test]
    fn test_get_bootnodes_works_for_public_networks() {
        Network::PublicNetwork(PublicNetwork::Hoodi).get_bootnodes();
        Network::PublicNetwork(PublicNetwork::Mainnet).get_bootnodes();
        Network::PublicNetwork(PublicNetwork::Sepolia).get_bootnodes();
    }
}
