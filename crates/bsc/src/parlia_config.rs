use ethereum_types::{Address, H160};

/// Validator contract address on BSC (0x0000...1000).
pub const VALIDATOR_CONTRACT: Address = H160([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10, 0x00,
]);

/// Slash contract address on BSC (0x0000...1001).
pub const SLASH_CONTRACT: Address = H160([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10, 0x01,
]);

/// System reward contract address on BSC (0x0000...1002).
pub const SYSTEM_REWARD_CONTRACT: Address = H160([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10, 0x02,
]);

/// Stake hub contract address on BSC (0x0000...2002).
pub const STAKE_HUB_CONTRACT: Address = H160([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x20, 0x02,
]);

// ── Epoch lengths ────────────────────────────────────────────────────────────

/// Default epoch length: 200 blocks per validator election cycle.
pub const DEFAULT_EPOCH_LENGTH: u64 = 200;
/// Lorentz epoch length: 500 blocks.
pub const LORENTZ_EPOCH_LENGTH: u64 = 500;
/// Maxwell epoch length: 1000 blocks.
pub const MAXWELL_EPOCH_LENGTH: u64 = 1000;

// ── Block intervals (milliseconds) ───────────────────────────────────────────

/// Default block interval: 3 000 ms.
pub const DEFAULT_BLOCK_INTERVAL: u64 = 3_000;
/// Lorentz block interval: 1 500 ms.
pub const LORENTZ_BLOCK_INTERVAL: u64 = 1_500;
/// Maxwell block interval: 750 ms.
pub const MAXWELL_BLOCK_INTERVAL: u64 = 750;
/// Fermi block interval: 450 ms.
pub const FERMI_BLOCK_INTERVAL: u64 = 450;

/// Default turn length: 1 consecutive block per validator per turn.
pub const DEFAULT_TURN_LENGTH: u8 = 1;

/// Finality reward is distributed every 200 blocks.
///
/// Must be smaller than `inMemorySnapshots` in the BSC reference client to avoid
/// excessive re-computation when collecting vote weights over the reward window.
/// Reference: `consensus/parlia/parlia.go` constant `finalityRewardInterval`.
pub const FINALITY_REWARD_INTERVAL: u64 = 200;

/// Breathe block interval in seconds (1 UTC day = 86 400 s).
///
/// A block is a "breathe block" when
/// `parent_timestamp / BREATHE_BLOCK_INTERVAL != block_timestamp / BREATHE_BLOCK_INTERVAL`,
/// i.e. the two timestamps straddle a UTC day boundary.  Breathe blocks trigger
/// `updateValidatorSetV2` to elect the next validator set.
/// Reference: `params/config.go` `BreatheBlockInterval` and
/// `consensus/parlia/feynmanfork.go` `isBreatheBlock`.
pub const BREATHE_BLOCK_INTERVAL: u64 = 86_400;

// ── Fork activation timestamps ────────────────────────────────────────────────
//
// BSC mainnet:
//   Lorentz  1745913600  (2025-04-29)
//   Maxwell  1751270400  (2025-06-30)
//   Fermi    1768406400  (2026-01-14)
//
// Chapel testnet:
//   Lorentz  1744070400  (2025-04-08)
//   Maxwell  1748217600  (2025-05-26)
//   Fermi    1762790400  (2025-11-10)

/// BSC mainnet fork activation timestamps.
#[derive(Debug, Clone)]
pub struct ForkTimestamps {
    pub lorentz: u64,
    pub maxwell: u64,
    pub fermi: u64,
}

impl ForkTimestamps {
    /// BSC mainnet fork activation timestamps.
    pub const MAINNET: ForkTimestamps = ForkTimestamps {
        lorentz: 1_745_913_600,
        maxwell: 1_751_270_400,
        fermi: 1_768_406_400,
    };

    /// Chapel testnet fork activation timestamps.
    pub const CHAPEL: ForkTimestamps = ForkTimestamps {
        lorentz: 1_744_070_400,
        maxwell: 1_748_217_600,
        fermi: 1_762_790_400,
    };
}

/// ParliaConfig holds the consensus parameters for BSC's Parlia engine.
///
/// Fork-dependent parameters (epoch length, block interval) are computed at
/// call time via [`epoch_length`] and [`block_interval`] rather than stored as
/// static fields.
#[derive(Debug, Clone)]
pub struct ParliaConfig {
    /// Chain ID — used to differentiate mainnet/testnet fork timestamps.
    pub chain_id: u64,
    /// Fork activation timestamps for this chain.
    pub forks: ForkTimestamps,
}

impl ParliaConfig {
    /// BSC mainnet configuration (chain ID 56).
    pub fn mainnet() -> Self {
        Self {
            chain_id: 56,
            forks: ForkTimestamps::MAINNET,
        }
    }

    /// Chapel testnet configuration (chain ID 97).
    pub fn chapel() -> Self {
        Self {
            chain_id: 97,
            forks: ForkTimestamps::CHAPEL,
        }
    }

    // ── Fork predicates ───────────────────────────────────────────────────────

    pub fn is_lorentz(&self, timestamp: u64) -> bool {
        timestamp >= self.forks.lorentz
    }

    pub fn is_maxwell(&self, timestamp: u64) -> bool {
        timestamp >= self.forks.maxwell
    }

    pub fn is_fermi(&self, timestamp: u64) -> bool {
        timestamp >= self.forks.fermi
    }

    // ── Parameter accessors ───────────────────────────────────────────────────

    /// Returns the epoch length (blocks per validator cycle) applicable at the
    /// given `block_number` and `timestamp`.
    ///
    /// The epoch length transitions are gated on both the fork timestamp *and*
    /// the block number being aligned to the new epoch length.  However, for
    /// simple config queries (e.g. "how long is an epoch at this timestamp?")
    /// we return the canonical value for the fork that is active.
    ///
    /// Reference: BSC `consensus/parlia/parlia.go` constants.
    pub fn epoch_length(&self, _block_number: u64, timestamp: u64) -> u64 {
        if self.is_maxwell(timestamp) {
            MAXWELL_EPOCH_LENGTH
        } else if self.is_lorentz(timestamp) {
            LORENTZ_EPOCH_LENGTH
        } else {
            DEFAULT_EPOCH_LENGTH
        }
    }

    /// Returns the target block interval in **milliseconds** for the given
    /// timestamp.
    ///
    /// Reference: BSC `consensus/parlia/parlia.go` constants (lines 61–64).
    pub fn block_interval(&self, timestamp: u64) -> u64 {
        if self.is_fermi(timestamp) {
            FERMI_BLOCK_INTERVAL
        } else if self.is_maxwell(timestamp) {
            MAXWELL_BLOCK_INTERVAL
        } else if self.is_lorentz(timestamp) {
            LORENTZ_BLOCK_INTERVAL
        } else {
            DEFAULT_BLOCK_INTERVAL
        }
    }

    /// Returns `true` if `block_number` is the start of a new epoch at the
    /// given timestamp.
    ///
    /// Block 0 (genesis) is explicitly excluded — `0 % N == 0` for any N, but
    /// the genesis block is not a validator-election epoch boundary in BSC.
    pub fn is_epoch_block(&self, block_number: u64, timestamp: u64) -> bool {
        if block_number == 0 {
            return false;
        }
        let epoch = self.epoch_length(block_number, timestamp);
        if epoch == 0 {
            return false;
        }
        block_number.is_multiple_of(epoch)
    }

    /// Backward-compatible alias for callers that already existed in this file.
    pub fn is_epoch_start(&self, block_number: u64, timestamp: u64) -> bool {
        self.is_epoch_block(block_number, timestamp)
    }
}

impl Default for ParliaConfig {
    fn default() -> Self {
        Self::mainnet()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_block_interval_progression() {
        let cfg = ParliaConfig::mainnet();
        // Before Lorentz
        assert_eq!(cfg.block_interval(0), DEFAULT_BLOCK_INTERVAL);
        assert_eq!(
            cfg.block_interval(ForkTimestamps::MAINNET.lorentz - 1),
            DEFAULT_BLOCK_INTERVAL
        );
        // Lorentz
        assert_eq!(
            cfg.block_interval(ForkTimestamps::MAINNET.lorentz),
            LORENTZ_BLOCK_INTERVAL
        );
        // Maxwell
        assert_eq!(
            cfg.block_interval(ForkTimestamps::MAINNET.maxwell),
            MAXWELL_BLOCK_INTERVAL
        );
        // Fermi
        assert_eq!(
            cfg.block_interval(ForkTimestamps::MAINNET.fermi),
            FERMI_BLOCK_INTERVAL
        );
    }

    #[test]
    fn mainnet_epoch_length_progression() {
        let cfg = ParliaConfig::mainnet();
        assert_eq!(cfg.epoch_length(0, 0), DEFAULT_EPOCH_LENGTH);
        assert_eq!(
            cfg.epoch_length(0, ForkTimestamps::MAINNET.lorentz),
            LORENTZ_EPOCH_LENGTH
        );
        assert_eq!(
            cfg.epoch_length(0, ForkTimestamps::MAINNET.maxwell),
            MAXWELL_EPOCH_LENGTH
        );
    }

    #[test]
    fn chapel_block_interval_progression() {
        let cfg = ParliaConfig::chapel();
        assert_eq!(
            cfg.block_interval(ForkTimestamps::CHAPEL.lorentz - 1),
            DEFAULT_BLOCK_INTERVAL
        );
        assert_eq!(
            cfg.block_interval(ForkTimestamps::CHAPEL.lorentz),
            LORENTZ_BLOCK_INTERVAL
        );
        assert_eq!(
            cfg.block_interval(ForkTimestamps::CHAPEL.maxwell),
            MAXWELL_BLOCK_INTERVAL
        );
        assert_eq!(
            cfg.block_interval(ForkTimestamps::CHAPEL.fermi),
            FERMI_BLOCK_INTERVAL
        );
    }

    #[test]
    fn is_epoch_block_default() {
        let cfg = ParliaConfig::mainnet();
        // Genesis block (0) is explicitly not an epoch block.
        assert!(!cfg.is_epoch_block(0, 0));
        assert!(cfg.is_epoch_block(200, 0));
        assert!(!cfg.is_epoch_block(201, 0));
    }
}
