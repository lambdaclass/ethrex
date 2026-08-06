//! BSC hardfork system-contract bytecode upgrades.
//!
//! At certain fork boundaries the Parlia consensus engine patches the bytecode
//! of on-chain system contracts. This is a pure consensus state change (a
//! `SetCode`, not a transaction) applied once, on the first block whose
//! timestamp crosses the fork activation time.
//!
//! Reference: bsc-geth `core/systemcontracts/upgrade.go`
//! (`applySystemContractUpgrade` -> `statedb.SetCode`).
//!
//! Currently only the **Pasteur** upgrade is implemented (the first fork
//! upgrade ethrex encounters on the forward-sync range). Pasteur replaces the
//! StakeHub (`0x…2002`) and Governor (`0x…2004`) bytecode — the code delivery
//! of BEP-695 (staking & governance security hardening). It touches code only:
//! nonce, balance and storage are left unchanged.
//!
//! The caller (the VM's `prepare_block`) applies these idempotently: for every
//! Pasteur-active block it checks each target's on-chain code hash and rewrites
//! it only when it still differs from the fork bytecode, i.e. exactly once, on
//! the transition block.

use std::sync::LazyLock;

use ethrex_common::{Address, H256, types::ChainConfig, utils::keccak};

use crate::parlia_config::{GOVERNOR_CONTRACT, STAKE_HUB_CONTRACT};

// Pasteur system-contract bytecode, taken verbatim from bsc-geth
// `core/systemcontracts/pasteur/{mainnet,chapel}/{StakeHubContract,GovernorContract}`
// (bsc-genesis-contract commit 041881a02475638b19f3d840871b7621cdebd8f8).
const MAINNET_STAKEHUB: &[u8] = include_bytes!("pasteur/mainnet_stakehub.bin");
const MAINNET_GOVERNOR: &[u8] = include_bytes!("pasteur/mainnet_governor.bin");
const CHAPEL_STAKEHUB: &[u8] = include_bytes!("pasteur/chapel_stakehub.bin");
const CHAPEL_GOVERNOR: &[u8] = include_bytes!("pasteur/chapel_governor.bin");

/// A single system-contract code replacement: the target address, the new
/// bytecode, and its precomputed keccak code hash (used for the idempotent
/// "already applied?" check without re-hashing the bytecode every block).
pub struct SystemContractUpgrade {
    pub address: Address,
    pub code: &'static [u8],
    pub code_hash: H256,
}

static MAINNET_PASTEUR: LazyLock<[SystemContractUpgrade; 2]> = LazyLock::new(|| {
    [
        SystemContractUpgrade {
            address: STAKE_HUB_CONTRACT,
            code: MAINNET_STAKEHUB,
            code_hash: keccak(MAINNET_STAKEHUB),
        },
        SystemContractUpgrade {
            address: GOVERNOR_CONTRACT,
            code: MAINNET_GOVERNOR,
            code_hash: keccak(MAINNET_GOVERNOR),
        },
    ]
});

static CHAPEL_PASTEUR: LazyLock<[SystemContractUpgrade; 2]> = LazyLock::new(|| {
    [
        SystemContractUpgrade {
            address: STAKE_HUB_CONTRACT,
            code: CHAPEL_STAKEHUB,
            code_hash: keccak(CHAPEL_STAKEHUB),
        },
        SystemContractUpgrade {
            address: GOVERNOR_CONTRACT,
            code: CHAPEL_GOVERNOR,
            code_hash: keccak(CHAPEL_GOVERNOR),
        },
    ]
});

/// Returns true if `timestamp` is at or after the Pasteur activation time.
///
/// Mirrors bsc-geth `ChainConfig.IsPasteur` (London is always active on BSC by
/// the time Pasteur activates, so only the timestamp gate matters here).
pub fn is_pasteur(chain_config: &ChainConfig, timestamp: u64) -> bool {
    matches!(chain_config.pasteur_time, Some(t) if timestamp >= t)
}

/// The Pasteur system-contract code replacements for the given BSC network, or
/// an empty slice for non-BSC networks (whose bytecode is not embedded here).
pub fn pasteur_upgrades(chain_id: u64) -> &'static [SystemContractUpgrade] {
    match chain_id {
        56 => &*MAINNET_PASTEUR,
        97 => &*CHAPEL_PASTEUR,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_bytecode_sizes_match_canonical() {
        // Sizes verified against BSC Chapel archive at the fork boundary
        // (block 120,346,349): StakeHub 47087 bytes, Governor 28489 bytes.
        assert_eq!(CHAPEL_STAKEHUB.len(), 47087);
        assert_eq!(CHAPEL_GOVERNOR.len(), 28489);
        assert_eq!(MAINNET_STAKEHUB.len(), 47087);
        assert_eq!(MAINNET_GOVERNOR.len(), 28490);
    }

    #[test]
    fn is_pasteur_gates_on_activation_time() {
        let cfg = ChainConfig {
            chain_id: 97,
            pasteur_time: Some(1_784_601_000),
            ..Default::default()
        };
        assert!(!is_pasteur(&cfg, 1_784_600_999));
        assert!(is_pasteur(&cfg, 1_784_601_000));
        assert!(is_pasteur(&cfg, 1_784_601_001));

        let no_fork = ChainConfig {
            chain_id: 97,
            pasteur_time: None,
            ..Default::default()
        };
        assert!(!is_pasteur(&no_fork, 1_784_601_000));
    }

    #[test]
    fn pasteur_upgrades_target_stakehub_and_governor() {
        for chain_id in [56u64, 97] {
            let ups = pasteur_upgrades(chain_id);
            assert_eq!(ups.len(), 2);
            assert_eq!(ups[0].address, STAKE_HUB_CONTRACT);
            assert_eq!(ups[1].address, GOVERNOR_CONTRACT);
            // code_hash must equal keccak(code) so the idempotency check is sound.
            assert_eq!(ups[0].code_hash, keccak(ups[0].code));
            assert_eq!(ups[1].code_hash, keccak(ups[1].code));
        }
        assert!(pasteur_upgrades(1).is_empty());
    }
}
