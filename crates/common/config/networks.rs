use std::{
    fmt::{self},
    path::PathBuf,
};

use ethrex_common::types::{ChainConfig, Genesis, GenesisError};
use ethrex_polygon::genesis::{amoy_genesis, polygon_mainnet_genesis};
use serde::{Deserialize, Serialize};

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
    Polygon,
    Amoy,
}

impl From<&str> for Network {
    fn from(value: &str) -> Self {
        match value {
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
        Network::PublicNetwork(PublicNetwork::Polygon)
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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

    pub fn get_bootnodes(&self) -> Vec<ethrex_p2p::types::Node> {
        use ethrex_p2p::types::Node;

        // Source: Bor params/bootnodes.go (maticnetwork/bor)
        let enodes: &[&str] = match self {
            Network::PublicNetwork(PublicNetwork::Polygon) => &[
                // BorMainnetBootnodes
                "enode://e4fb013061eba9a2c6fb0a41bbd4149f4808f0fb7e88ec55d7163f19a6f02d64d0ce5ecc81528b769ba552a7068057432d44ab5e9e42842aff5b4709aa2c3f3b@34.89.75.187:30303",
                "enode://a49da6300403cf9b31e30502eb22c142ba4f77c9dda44990bccce9f2121c3152487ee95ee55c6b92d4cdce77845e40f59fd927da70ea91cf935b23e262236d75@34.142.43.249:30303",
                // MainnetBootnodes (also used by Bor)
                "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
                "enode://22a8232c3abc76a16ae9d6c3b164f98775fe226f0917b0ca871128a74a8e9630b458460865bab457221f1d448dd9791d24c4e5d88786180ac185df813a68d4de@3.209.45.79:30303",
            ],
            Network::PublicNetwork(PublicNetwork::Amoy) => &[
                // Amoy static-nodes — from Bor packaging templates & official docs
                // (The bootnodes in bootnodes.go at 34.89.39.114 / 35.197.249.21 are dead)
                "enode://d40ab6b340be9f78179bd1ec7aa4df346d43dc1462d85fb44c5d43f595991d2ec215d7c778a7588906cb4edf175b3df231cecce090986a739678cd3c620bf580@34.89.255.109:30303",
                "enode://13abba15caa024325f2209d3566fa77cd864281dda4f73bca4296277bfd919ac68cef4dbb508028e0310a24f6f9e23c761fa41ac735cdc87efdee76d5ff985a7@34.185.137.160:30303",
                "enode://fc5bd3856a4ce6389eef1d6bc637ce7617e6ba8013f7d722d9878cf13f1c5a5a95a9e26ccb0b38bcc330343941ce117ab50db9f61e72ba450dd528a1184d8e6a@34.89.119.250:30303",
                "enode://945e11d11bdeed301fb23a5c05aae77bfdde39a8f70308131682a5d2fc1f080531314554afc78718a72ae25cc09be7833f760bf8681516b4315ed36217fa8dab@34.89.40.235:30303",
            ],
            _ => &[],
        };

        enodes
            .iter()
            .filter_map(|e| Node::from_enode_url(e).ok())
            .collect()
    }
}
