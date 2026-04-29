use std::{
    fmt::{self},
    path::PathBuf,
};

use ethrex_common::types::{ChainConfig, Genesis, GenesisError};
use ethrex_polygon::genesis::{amoy_genesis, polygon_mainnet_genesis};
use serde::{Deserialize, Serialize};

//TODO: Look for a better place to move these files
const MAINNET_BOOTNODES: &str = include_str!("../../../cmd/ethrex/networks/mainnet/bootnodes.json");
const SEPOLIA_BOOTNODES: &str = include_str!("../../../cmd/ethrex/networks/sepolia/bootnodes.json");
const HOODI_BOOTNODES: &str = include_str!("../../../cmd/ethrex/networks/hoodi/bootnodes.json");

pub const MAINNET_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/mainnet/genesis.json");
pub const SEPOLIA_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/sepolia/genesis.json");
pub const HOODI_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/hoodi/genesis.json");

pub const MAINNET_CHAIN_ID: u64 = 1;
pub const SEPOLIA_CHAIN_ID: u64 = 11155111;
pub const HOODI_CHAIN_ID: u64 = 560048;
pub const POLYGON_MAINNET_CHAIN_ID: u64 = 137;
pub const AMOY_CHAIN_ID: u64 = 80002;

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
    Mainnet,
    Sepolia,
    Hoodi,
    Polygon,
    Amoy,
}

impl From<&str> for Network {
    fn from(value: &str) -> Self {
        match value {
            "mainnet" => Network::PublicNetwork(PublicNetwork::Mainnet),
            "sepolia" => Network::PublicNetwork(PublicNetwork::Sepolia),
            "hoodi" => Network::PublicNetwork(PublicNetwork::Hoodi),
            "polygon" => Network::PublicNetwork(PublicNetwork::Polygon),
            "amoy" => Network::PublicNetwork(PublicNetwork::Amoy),
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
            POLYGON_MAINNET_CHAIN_ID => Ok(Network::PublicNetwork(PublicNetwork::Polygon)),
            AMOY_CHAIN_ID => Ok(Network::PublicNetwork(PublicNetwork::Amoy)),
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
            Network::PublicNetwork(PublicNetwork::Mainnet) => write!(f, "mainnet"),
            Network::PublicNetwork(PublicNetwork::Sepolia) => write!(f, "sepolia"),
            Network::PublicNetwork(PublicNetwork::Hoodi) => write!(f, "hoodi"),
            Network::PublicNetwork(PublicNetwork::Polygon) => write!(f, "polygon"),
            Network::PublicNetwork(PublicNetwork::Amoy) => write!(f, "amoy"),
            Network::LocalDevnet => write!(f, "local-devnet"),
            Network::LocalDevnetL2 => write!(f, "local-devnet-l2"),
            Network::L2Chain(chain_id) => write!(f, "l2-chain-{}", chain_id),
            Network::GenesisPath(path_buf) => write!(f, "{path_buf:?}"),
        }
    }
}

pub const LOCAL_DEVNET_GENESIS_CONTENTS: &str = include_str!("../../../fixtures/genesis/l1.json");
pub const LOCAL_DEVNETL2_GENESIS_CONTENTS: &str = include_str!("../../../fixtures/genesis/l2.json");

pub const LOCAL_DEVNET_PRIVATE_KEYS: &str =
    include_str!("../../../fixtures/keys/private_keys_l1.txt");

impl Network {
    pub fn polygon() -> Self {
        Network::PublicNetwork(PublicNetwork::Polygon)
    }

    pub fn get_genesis(&self) -> Result<Genesis, GenesisError> {
        match self {
            Network::PublicNetwork(PublicNetwork::Mainnet) => {
                Ok(serde_json::from_str(MAINNET_GENESIS_CONTENTS)?)
            }
            Network::PublicNetwork(PublicNetwork::Sepolia) => {
                Ok(serde_json::from_str(SEPOLIA_GENESIS_CONTENTS)?)
            }
            Network::PublicNetwork(PublicNetwork::Hoodi) => {
                Ok(serde_json::from_str(HOODI_GENESIS_CONTENTS)?)
            }
            Network::PublicNetwork(PublicNetwork::Polygon) => Ok(polygon_mainnet_genesis()),
            Network::PublicNetwork(PublicNetwork::Amoy) => Ok(amoy_genesis()),
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

    /// Returns true if this is a Polygon PoS network (mainnet or testnet).
    pub fn is_polygon(&self) -> bool {
        matches!(
            self,
            Network::PublicNetwork(PublicNetwork::Polygon | PublicNetwork::Amoy)
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
            Network::PublicNetwork(PublicNetwork::Polygon) => Some("polygon".to_owned()),
            Network::PublicNetwork(PublicNetwork::Amoy) => Some("amoy".to_owned()),
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
            "mainnet", // PublicNetwork::Mainnet
            "hoodi",   // PublicNetwork::Hoodi
            "sepolia", // PublicNetwork::Sepolia
            "polygon", // PublicNetwork::Polygon
            "amoy",    // PublicNetwork::Amoy
            "dev",     // dev mode
        ]
    }

    pub fn get_bootnodes(&self) -> Vec<ethrex_p2p::types::Node> {
        use ethrex_p2p::types::Node;

        match self {
            Network::PublicNetwork(PublicNetwork::Hoodi) => {
                serde_json::from_str(HOODI_BOOTNODES).expect("bootnodes file should be valid JSON")
            }
            Network::PublicNetwork(PublicNetwork::Mainnet) => {
                serde_json::from_str(MAINNET_BOOTNODES)
                    .expect("bootnodes file should be valid JSON")
            }
            Network::PublicNetwork(PublicNetwork::Sepolia) => {
                serde_json::from_str(SEPOLIA_BOOTNODES)
                    .expect("bootnodes file should be valid JSON")
            }
            Network::PublicNetwork(PublicNetwork::Polygon) => {
                // Source: Bor params/bootnodes.go (maticnetwork/bor)
                let enodes: &[&str] = &[
                    "enode://e4fb013061eba9a2c6fb0a41bbd4149f4808f0fb7e88ec55d7163f19a6f02d64d0ce5ecc81528b769ba552a7068057432d44ab5e9e42842aff5b4709aa2c3f3b@34.89.75.187:30303",
                    "enode://a49da6300403cf9b31e30502eb22c142ba4f77c9dda44990bccce9f2121c3152487ee95ee55c6b92d4cdce77845e40f59fd927da70ea91cf935b23e262236d75@34.142.43.249:30303",
                    "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
                    "enode://22a8232c3abc76a16ae9d6c3b164f98775fe226f0917b0ca871128a74a8e9630b458460865bab457221f1d448dd9791d24c4e5d88786180ac185df813a68d4de@3.209.45.79:30303",
                ];
                enodes
                    .iter()
                    .filter_map(|e| Node::from_enode_url(e).ok())
                    .collect()
            }
            Network::PublicNetwork(PublicNetwork::Amoy) => {
                let enodes: &[&str] = &[
                    "enode://d40ab6b340be9f78179bd1ec7aa4df346d43dc1462d85fb44c5d43f595991d2ec215d7c778a7588906cb4edf175b3df231cecce090986a739678cd3c620bf580@34.89.255.109:30303",
                    "enode://13abba15caa024325f2209d3566fa77cd864281dda4f73bca4296277bfd919ac68cef4dbb508028e0310a24f6f9e23c761fa41ac735cdc87efdee76d5ff985a7@34.185.137.160:30303",
                    "enode://fc5bd3856a4ce6389eef1d6bc637ce7617e6ba8013f7d722d9878cf13f1c5a5a95a9e26ccb0b38bcc330343941ce117ab50db9f61e72ba450dd528a1184d8e6a@34.89.119.250:30303",
                    "enode://945e11d11bdeed301fb23a5c05aae77bfdde39a8f70308131682a5d2fc1f080531314554afc78718a72ae25cc09be7833f760bf8681516b4315ed36217fa8dab@34.89.40.235:30303",
                    "enode://48e6326841ce106f6b4e229a1be7e98a1d12be57e328b08cb461f6744ae4e78f5ec2340996ce9b40928a1a90137aadea13e25ca34774b52a3600d13a52c5c7bb@34.185.209.56:30303",
                    "enode://8ab6905fe76aa9001adb77135250e918db888cac216870c0e95cf26650d83d31d8c2c93d54c3333e0a2196517c41651d174b743ec3e11f44e595f62b77fec7ba@34.185.162.14:30303",
                    "enode://02e0b33cf60fb1f88f853c7c04830156151f4acd1c36173cd3fe1f375801fb4f5be5b3a89c98527915d37ed217752933c3faf4c820df740c9dd681294caebcf6@34.179.171.228:30303",
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
        PublicNetwork::Mainnet => MAINNET_GENESIS_CONTENTS,
        PublicNetwork::Sepolia => SEPOLIA_GENESIS_CONTENTS,
        PublicNetwork::Hoodi => HOODI_GENESIS_CONTENTS,
        // Polygon genesis is built programmatically, not from a JSON file
        PublicNetwork::Polygon | PublicNetwork::Amoy => "",
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
        assert_genesis_hash(
            PublicNetwork::Sepolia,
            "25a5cc106eea7138acab33231d7160d69cb777ee0c2c553fcddf5138993e6dd9",
        );
    }

    #[test]
    fn test_hoodi_genesis_block_hash() {
        assert_genesis_hash(
            PublicNetwork::Hoodi,
            "bbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b",
        );
    }

    #[test]
    fn test_mainnet_genesis_block_hash() {
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
        assert_eq!(
            Network::GenesisPath(PathBuf::from("/tmp/nonexistent.json")).datadir_suffix(),
            None
        );
    }

    #[test]
    fn test_all_datadir_suffixes_covers_all_public_networks() {
        let all = Network::all_datadir_suffixes();
        let networks = [
            Network::PublicNetwork(PublicNetwork::Mainnet),
            Network::PublicNetwork(PublicNetwork::Hoodi),
            Network::PublicNetwork(PublicNetwork::Sepolia),
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
