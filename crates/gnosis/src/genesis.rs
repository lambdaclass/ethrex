use ethereum_types::{Address, H256};
use ethrex_common::types::Genesis;
use std::str::FromStr;

/// Gnosis Chain mainnet chain ID.
pub const GNOSIS_MAINNET_CHAIN_ID: u64 = 100;
/// Chiado testnet chain ID.
pub const CHIADO_CHAIN_ID: u64 = 10200;

/// Canonical Chiado genesis block hash.
///
/// Chiado's genesis block was produced by an AuRa client (OpenEthereum /
/// Nethermind), whose header RLP differs from the geth-style format used by
/// ethrex: positions 14 and 15 hold `seal_step` (variable-length uint) and
/// `seal_signature` (65-byte AuRa signature), respectively, instead of
/// `mix_hash` (32 bytes) + `nonce` (8 bytes). Because of this, recomputing
/// the hash from the canonical header fields in geth format produces a
/// DIFFERENT value than the on-chain genesis hash.
///
/// We pin the hash here so the EIP-2124 fork ID and the parent-hash chain
/// at block 1 match the live network.
pub const CHIADO_GENESIS_HASH: &str =
    "0xada44fd8d2ecab8b08f256af07ad3e777f17fb434f8f8e678b312f576212ba9a";

/// Canonical Gnosis Chain mainnet genesis block hash. Same rationale as
/// Chiado — the mainnet genesis was also AuRa-produced. (Verified against
/// `eth_getBlockByNumber 0` on a public Gnosis mainnet RPC.)
pub const GNOSIS_MAINNET_GENESIS_HASH: &str =
    "0x4f1dd23188aab3a76b463e4af801b52b1248ef073c648cbdc4c9333d3da79756";

/// Embedded Chiado testnet genesis JSON (canonical alloc + post-merge config).
const CHIADO_GENESIS_CONTENTS: &str = include_str!("../allocs/chiado_genesis.json");

/// Embedded Chiado bootnodes as JSON array of `enr:` URLs.
pub const CHIADO_BOOTNODES: &str = include_str!("../allocs/chiado_bootnodes.json");

/// Embedded Gnosis Chain mainnet genesis JSON (canonical 4-entry precompile
/// alloc + post-merge config; AuRa fields stripped — see CHIADO_GENESIS_HASH
/// rationale).
const GNOSIS_MAINNET_GENESIS_CONTENTS: &str = include_str!("../allocs/gnosis_genesis.json");

/// Embedded Gnosis Chain mainnet EL bootnodes (`enode://` URLs).
pub const GNOSIS_MAINNET_BOOTNODES: &str = include_str!("../allocs/gnosis_bootnodes.json");

/// Returns the Genesis for Chiado testnet (chain ID 10200), with the
/// canonical genesis hash override applied so it matches the live network's
/// fork ID.
pub fn chiado_genesis() -> Genesis {
    let mut genesis: Genesis =
        serde_json::from_str(CHIADO_GENESIS_CONTENTS).expect("chiado genesis should be valid JSON");
    genesis.genesis_hash_override = Some(
        H256::from_str(CHIADO_GENESIS_HASH)
            .expect("CHIADO_GENESIS_HASH is a valid hex H256 constant"),
    );
    genesis
}

/// Returns the Genesis for Gnosis Chain mainnet (chain ID 100), with the
/// canonical genesis hash override applied so it matches the live network's
/// fork ID.
pub fn gnosis_mainnet_genesis() -> Genesis {
    let mut genesis: Genesis = serde_json::from_str(GNOSIS_MAINNET_GENESIS_CONTENTS)
        .expect("gnosis mainnet genesis should be valid JSON");
    genesis.genesis_hash_override = Some(
        H256::from_str(GNOSIS_MAINNET_GENESIS_HASH)
            .expect("GNOSIS_MAINNET_GENESIS_HASH is a valid hex H256 constant"),
    );
    genesis
}

/// Returns true if the chain ID is a known Gnosis network (mainnet or Chiado).
pub fn is_gnosis_chain(chain_id: u64) -> bool {
    chain_id == GNOSIS_MAINNET_CHAIN_ID || chain_id == CHIADO_CHAIN_ID
}

/// Gnosis system sender used for post-block system contract calls.
/// Address `0xfffffffffffffffffffffffffffffffffffffffe` (19×0xff + 0xfe).
pub fn system_sender() -> Address {
    let mut a = [0xffu8; 20];
    a[19] = 0xfe;
    Address::from(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chiado_genesis_parses() {
        let genesis = chiado_genesis();
        assert_eq!(genesis.config.chain_id, CHIADO_CHAIN_ID);
        assert!(
            genesis.config.terminal_total_difficulty_passed,
            "Chiado is post-merge"
        );
        assert!(
            genesis.alloc.len() >= 20,
            "Chiado has many system contracts"
        );
        // The override must be set so the resulting block hash matches the
        // canonical Chiado genesis (otherwise P2P fork ID won't match peers).
        assert_eq!(
            genesis.genesis_hash_override.unwrap(),
            H256::from_str(CHIADO_GENESIS_HASH).unwrap()
        );
    }

    #[test]
    fn test_chiado_genesis_block_hash_matches_canonical() {
        let genesis = chiado_genesis();
        let block = genesis.get_block();
        assert_eq!(
            format!("0x{:x}", block.hash()),
            CHIADO_GENESIS_HASH,
            "ethrex genesis block hash must match canonical Chiado genesis hash"
        );
    }

    #[test]
    fn test_chiado_bootnodes_parse() {
        let v: Vec<String> =
            serde_json::from_str(CHIADO_BOOTNODES).expect("bootnodes JSON must parse");
        assert!(v.len() >= 5);
        for entry in &v {
            assert!(
                entry.starts_with("enr:") || entry.starts_with("enode://"),
                "expected enr: or enode:// URL: {entry}"
            );
        }
    }

    #[test]
    fn test_gnosis_mainnet_genesis_parses() {
        let genesis = gnosis_mainnet_genesis();
        assert_eq!(genesis.config.chain_id, GNOSIS_MAINNET_CHAIN_ID);
        assert!(
            genesis.config.terminal_total_difficulty_passed,
            "Gnosis mainnet is post-merge"
        );
        // Genesis alloc on Gnosis mainnet is just the 4 precompile placeholders.
        assert_eq!(genesis.alloc.len(), 4);
        assert_eq!(
            genesis.genesis_hash_override.unwrap(),
            H256::from_str(GNOSIS_MAINNET_GENESIS_HASH).unwrap()
        );
    }

    #[test]
    fn test_gnosis_mainnet_genesis_block_hash_matches_canonical() {
        let genesis = gnosis_mainnet_genesis();
        let block = genesis.get_block();
        assert_eq!(
            format!("0x{:x}", block.hash()),
            GNOSIS_MAINNET_GENESIS_HASH,
            "ethrex genesis block hash must match canonical Gnosis mainnet genesis hash"
        );
    }

    #[test]
    fn test_gnosis_mainnet_bootnodes_parse() {
        let v: Vec<String> =
            serde_json::from_str(GNOSIS_MAINNET_BOOTNODES).expect("bootnodes JSON must parse");
        assert!(v.len() >= 10);
        for entry in &v {
            assert!(
                entry.starts_with("enode://"),
                "mainnet EL bootnodes must be enode:// URLs: {entry}"
            );
        }
    }
}
