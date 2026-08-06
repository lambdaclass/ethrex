//! BSC hardfork system-contract bytecode upgrades.
//!
//! At certain fork boundaries the Parlia consensus engine patches the bytecode
//! of on-chain system contracts. This is a pure consensus state change (a
//! `SetCode`, not a transaction) applied once, on the first block whose
//! timestamp crosses the fork activation time.
//!
//! Reference: bsc-geth `core/systemcontracts/upgrade.go`
//! (`applySystemContractUpgrade` -> `statedb.SetCode`), triggered from
//! `consensus/parlia/parlia.go` `Finalize` **after** all block transactions
//! run and **before** the state root is computed.
//!
//! Currently only the **Pasteur** upgrade is implemented (the first fork
//! upgrade ethrex encounters on the forward-sync range). Pasteur replaces the
//! StakeHub (`0x…2002`) and Governor (`0x…2004`) bytecode — the code delivery
//! of BEP-695 (staking & governance security hardening). It touches code only:
//! nonce, balance and storage are left unchanged.

use ethrex_common::{Address, types::ChainConfig};

use crate::parlia_config::{GOVERNOR_CONTRACT, STAKE_HUB_CONTRACT};

// Pasteur system-contract bytecode, taken verbatim from bsc-geth
// `core/systemcontracts/pasteur/{mainnet,chapel}/{StakeHubContract,GovernorContract}`
// (bsc-genesis-contract commit 041881a02475638b19f3d840871b7621cdebd8f8).
const MAINNET_STAKEHUB: &[u8] = include_bytes!("pasteur/mainnet_stakehub.bin");
const MAINNET_GOVERNOR: &[u8] = include_bytes!("pasteur/mainnet_governor.bin");
const CHAPEL_STAKEHUB: &[u8] = include_bytes!("pasteur/chapel_stakehub.bin");
const CHAPEL_GOVERNOR: &[u8] = include_bytes!("pasteur/chapel_governor.bin");

/// Returns true if `timestamp` is at or after the Pasteur activation time.
///
/// Mirrors bsc-geth `ChainConfig.IsPasteur` (London is always active on BSC by
/// the time Pasteur activates, so only the timestamp gate matters here).
pub fn is_pasteur(chain_config: &ChainConfig, timestamp: u64) -> bool {
    matches!(chain_config.pasteur_time, Some(t) if timestamp >= t)
}

/// Returns true only for the single Pasteur transition block: the parent is
/// pre-Pasteur and this block is Pasteur.
///
/// Mirrors bsc-geth `ChainConfig.IsOnPasteur(num, lastBlockTime, blockTime)`.
pub fn is_on_pasteur(
    chain_config: &ChainConfig,
    parent_timestamp: u64,
    block_timestamp: u64,
) -> bool {
    !is_pasteur(chain_config, parent_timestamp) && is_pasteur(chain_config, block_timestamp)
}

/// System-contract bytecode replacements to apply as a `SetCode` on this block,
/// or an empty slice if no fork upgrade activates here.
///
/// The caller must apply each `(address, code)` to the post-transaction state,
/// before computing the block's state root (matching bsc-geth's `Finalize`
/// ordering). Only code is replaced — nonce, balance and storage are untouched.
pub fn system_contract_code_upgrades(
    chain_config: &ChainConfig,
    parent_timestamp: u64,
    block_timestamp: u64,
) -> Vec<(Address, &'static [u8])> {
    let mut upgrades = Vec::new();

    if is_on_pasteur(chain_config, parent_timestamp, block_timestamp) {
        let (stakehub, governor) = match chain_config.chain_id {
            56 => (MAINNET_STAKEHUB, MAINNET_GOVERNOR),
            97 => (CHAPEL_STAKEHUB, CHAPEL_GOVERNOR),
            // Pasteur bytecode is network-specific; unknown BSC networks get no
            // upgrade rather than the wrong bytecode.
            _ => return upgrades,
        };
        upgrades.push((STAKE_HUB_CONTRACT, stakehub));
        upgrades.push((GOVERNOR_CONTRACT, governor));
    }

    upgrades
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapel_cfg() -> ChainConfig {
        ChainConfig {
            chain_id: 97,
            pasteur_time: Some(1_784_601_000),
            ..Default::default()
        }
    }

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
    fn upgrade_fires_only_on_transition_block() {
        let cfg = chapel_cfg();
        // parent pre-fork (…999), block at activation (…000) -> transition.
        let ups = system_contract_code_upgrades(&cfg, 1_784_600_999, 1_784_601_000);
        assert_eq!(ups.len(), 2);
        assert_eq!(ups[0].0, STAKE_HUB_CONTRACT);
        assert_eq!(ups[1].0, GOVERNOR_CONTRACT);
        assert_eq!(ups[0].1.len(), 47087);
    }

    #[test]
    fn no_upgrade_before_or_after_transition() {
        let cfg = chapel_cfg();
        // Both pre-fork.
        assert!(system_contract_code_upgrades(&cfg, 1_784_600_998, 1_784_600_999).is_empty());
        // Both post-fork (parent already Pasteur).
        assert!(system_contract_code_upgrades(&cfg, 1_784_601_000, 1_784_601_001).is_empty());
    }

    #[test]
    fn no_upgrade_without_pasteur_time() {
        let cfg = ChainConfig {
            chain_id: 97,
            pasteur_time: None,
            ..Default::default()
        };
        assert!(system_contract_code_upgrades(&cfg, 1_784_600_999, 1_784_601_000).is_empty());
    }
}
