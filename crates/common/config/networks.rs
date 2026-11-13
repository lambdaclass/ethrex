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
const PLATABERGET_BOOTNODES: &str =
    include_str!("../../../cmd/ethrex/networks/plataberget/bootnodes.json");

pub const MAINNET_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/mainnet/genesis.json");
pub const HOODI_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/hoodi/genesis.json");
pub const SEPOLIA_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/sepolia/genesis.json");
pub const PLATABERGET_GENESIS_CONTENTS: &str =
    include_str!("../../../cmd/ethrex/networks/plataberget/genesis.json");
pub const LOCAL_DEVNET_GENESIS_CONTENTS: &str = include_str!("../../../fixtures/genesis/l1.json");
pub const LOCAL_DEVNETL2_GENESIS_CONTENTS: &str = include_str!("../../../fixtures/genesis/l2.json");

pub const LOCAL_DEVNET_PRIVATE_KEYS: &str =
    include_str!("../../../fixtures/keys/private_keys_l1.txt");

pub const MAINNET_CHAIN_ID: u64 = 0x1;
pub const HOODI_CHAIN_ID: u64 = 0x88bb0;
pub const SEPOLIA_CHAIN_ID: u64 = 0xAA36A7;
pub const PLATABERGET_CHAIN_ID: u64 = 7091047534;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Network {
    PublicNetwork(PublicNetwork),
    LocalDevnet,
    LocalDevnetL2,
    L2Chain(u64),
    #[serde(skip)]
    GenesisPath(PathBuf),
    /// A named network defined outside this crate (see [`NetworkPreset`]).
    #[serde(skip)]
    Custom(NetworkPreset),
}

/// A named network supplied by a chain integration: what `--network <name>`
/// loads for a network this crate does not know about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkPreset {
    /// Name accepted by `--network`. Also the datadir subdirectory.
    pub name: &'static str,
    /// Contents of the genesis file, in JSON.
    pub genesis: &'static str,
    /// JSON array of enode URLs, in the format of this crate's `bootnodes.json`
    /// files. Use `"[]"` for a network without bootnodes.
    pub bootnodes: &'static str,
}

impl NetworkPreset {
    /// Checks that the genesis and the bootnodes parse, so a malformed preset is
    /// reported once at startup instead of wherever it is first used.
    pub fn validate(&self) -> Result<(), String> {
        serde_json::from_str::<Genesis>(self.genesis)
            .map_err(|err| format!("network preset {:?}: invalid genesis: {err}", self.name))?;
        serde_json::from_str::<Vec<Node>>(self.bootnodes)
            .map_err(|err| format!("network preset {:?}: invalid bootnodes: {err}", self.name))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicNetwork {
    Hoodi,
    Sepolia,
    Mainnet,
    /// Platåberget: the first public Glamsterdam testnet, run on the
    /// glamsterdam-devnet-8 spec (genesis 2026-08-13, Amsterdam at
    /// timestamp 1787212224). Configs:
    /// https://github.com/ethpandaops/glamsterdam-devnets/tree/master/network-configs/devnet-8
    Plataberget,
}

impl From<&str> for Network {
    fn from(value: &str) -> Self {
        match value {
            "hoodi" => Network::PublicNetwork(PublicNetwork::Hoodi),
            "mainnet" => Network::PublicNetwork(PublicNetwork::Mainnet),
            "sepolia" => Network::PublicNetwork(PublicNetwork::Sepolia),
            "plataberget" => Network::PublicNetwork(PublicNetwork::Plataberget),
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
            PLATABERGET_CHAIN_ID => Ok(Network::PublicNetwork(PublicNetwork::Plataberget)),
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
            Network::PublicNetwork(PublicNetwork::Plataberget) => write!(f, "plataberget"),
            Network::LocalDevnet => write!(f, "local-devnet"),
            Network::LocalDevnetL2 => write!(f, "local-devnet-l2"),
            Network::L2Chain(chain_id) => write!(f, "l2-chain-{}", chain_id),
            Network::GenesisPath(path_buf) => write!(f, "{path_buf:?}"),
            Network::Custom(preset) => write!(f, "{}", preset.name),
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
            Network::L2Chain(chain_id) => Ok(Genesis {
                config: ChainConfig {
                    chain_id: *chain_id,
                    prague_time: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            }),
            Network::GenesisPath(s) => Genesis::try_from(s.as_path()),
            Network::Custom(preset) => Ok(serde_json::from_str(preset.genesis)?),
        }
    }

    /// Returns the network-specific subdirectory name for the datadir.
    /// Public networks get a named suffix; custom genesis files and L2 chains
    /// use their chain ID as suffix.
    pub fn datadir_suffix(&self) -> Option<String> {
        match self {
            Network::PublicNetwork(PublicNetwork::Mainnet) => Some("mainnet".to_owned()),
            Network::PublicNetwork(PublicNetwork::Hoodi) => Some("hoodi".to_owned()),
            Network::PublicNetwork(PublicNetwork::Sepolia) => Some("sepolia".to_owned()),
            Network::PublicNetwork(PublicNetwork::Plataberget) => Some("plataberget".to_owned()),
            Network::LocalDevnet => None,
            Network::LocalDevnetL2 => None,
            Network::L2Chain(chain_id) => Some(format!("chain-{chain_id}")),
            Network::GenesisPath(_) => {
                let chain_id = self.get_genesis().ok()?.config.chain_id;
                Some(format!("chain-{chain_id}"))
            }
            Network::Custom(preset) => Some(preset.name.to_owned()),
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
            "plataberget", // PublicNetwork::Plataberget
            "dev",         // dev mode
        ]
    }

    pub fn get_bootnodes(&self) -> Vec<Node> {
        let bootnodes = match self {
            Network::PublicNetwork(PublicNetwork::Hoodi) => HOODI_BOOTNODES,
            Network::PublicNetwork(PublicNetwork::Mainnet) => MAINNET_BOOTNODES,
            Network::PublicNetwork(PublicNetwork::Sepolia) => SEPOLIA_BOOTNODES,
            Network::PublicNetwork(PublicNetwork::Plataberget) => PLATABERGET_BOOTNODES,
            Network::Custom(preset) => preset.bootnodes,
            _ => return vec![],
        };
        serde_json::from_str(bootnodes).expect("bootnodes file should be valid JSON")
    }
}

fn get_genesis_contents(network: PublicNetwork) -> &'static str {
    match network {
        PublicNetwork::Hoodi => HOODI_GENESIS_CONTENTS,
        PublicNetwork::Mainnet => MAINNET_GENESIS_CONTENTS,
        PublicNetwork::Sepolia => SEPOLIA_GENESIS_CONTENTS,
        PublicNetwork::Plataberget => PLATABERGET_GENESIS_CONTENTS,
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
    fn test_sepolia_amsterdam_activation() {
        // Epoch 353024, 2026-10-06 13:53:36 UTC. Decided in ethereum/pm#2205 and
        // recorded in EIP-7773 by ethereum/EIPs#12355; the client configuration is
        // eth-clients/sepolia#126 (`metadata/genesis.json`, `amsterdamTime`).
        // 1655733600 + 353024 * 32 * 12 = 1791294816.
        let genesis = Network::PublicNetwork(PublicNetwork::Sepolia)
            .get_genesis()
            .unwrap();
        assert_eq!(genesis.config.amsterdam_time, Some(1_791_294_816));
    }

    #[test]
    fn test_plataberget_genesis_block_hash() {
        // Value published on the network's config page (https://plataberget.dev/,
        // "Genesis Hash (EL)"), served by
        // https://github.com/ethpandaops/glamsterdam-devnets/tree/master/network-configs/devnet-8
        assert_genesis_hash(
            PublicNetwork::Plataberget,
            "ee33ef92bbabcf07bcf44fea1d18a7925c5f7f9da8f81334ea19b0f3cb892b31",
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

    const EXAMPLE_PRESET: NetworkPreset = NetworkPreset {
        name: "example",
        genesis: LOCAL_DEVNET_GENESIS_CONTENTS,
        bootnodes: r#"["enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303"]"#,
    };

    #[test]
    fn custom_preset_supplies_genesis_bootnodes_and_datadir() {
        EXAMPLE_PRESET.validate().unwrap();
        let network = Network::Custom(EXAMPLE_PRESET);
        assert_eq!(network.to_string(), "example");
        assert_eq!(network.datadir_suffix(), Some("example".into()));
        assert_eq!(
            network.get_genesis().unwrap().config.chain_id,
            Network::LocalDevnet.get_genesis().unwrap().config.chain_id
        );
        assert_eq!(network.get_bootnodes().len(), 1);
    }

    #[test]
    fn malformed_custom_preset_is_reported_by_validate() {
        let bad_genesis = NetworkPreset {
            genesis: "{",
            ..EXAMPLE_PRESET
        };
        assert!(
            bad_genesis
                .validate()
                .unwrap_err()
                .contains("invalid genesis")
        );
        let bad_bootnodes = NetworkPreset {
            bootnodes: r#"["not-an-enode"]"#,
            ..EXAMPLE_PRESET
        };
        assert!(
            bad_bootnodes
                .validate()
                .unwrap_err()
                .contains("invalid bootnodes")
        );
    }

    #[test]
    fn test_get_bootnodes_works_for_public_networks() {
        Network::PublicNetwork(PublicNetwork::Hoodi).get_bootnodes();
        Network::PublicNetwork(PublicNetwork::Mainnet).get_bootnodes();
        Network::PublicNetwork(PublicNetwork::Sepolia).get_bootnodes();
    }
}
