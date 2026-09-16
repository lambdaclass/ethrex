//! MATCHA: mempool account transaction capacity from historical activity.
//!
//! Lets one `sender` hold several pending EIP-8141 frame transactions with independent
//! EIP-8250 nonce keys without the client whitelisting the application, paymaster,
//! verifier or proof system behind them. See `docs/matcha.md` for the design and the
//! places it departs from the proposal.
//!
//! The unit of account is **width**: capacity for pending frame transactions beyond the
//! one EIP-8141 already allows. A sender starts with none, earns it from the gas its own
//! frame transactions used in newly finalized blocks, and spends it whenever the client
//! does admission work on its behalf. Spending is never reversed: the client did the
//! work whether or not the transaction survived, and refunding on invalidation would
//! make mass invalidation free for whoever caused it.
//!
//! This module is pure bookkeeping with no I/O and no locking. The mempool owns an
//! instance and drives it under its own write lock.

use std::time::Duration;

use ethrex_common::Address;
use ethrex_common::types::{
    BlockNumber, FRAME_TX_INTRINSIC_COST, FRAME_TX_PER_FRAME_COST, FrameTransaction,
};
use rustc_hash::FxHashMap;

/// Gas charged per EIP-8250 nonce key for the admission-time sequence check.
///
/// The proposal lists the nonce checks as a term of `admission_gas` without pricing them.
/// Each key is one keyed-slot read at the head, so it is charged as a cold `SLOAD`, and
/// per key rather than per transaction because the work is per key.
pub const NONCE_KEY_CHECK_GAS: u64 = 2_100;

/// Default cap on accumulated width: about one block of gas, so a long-quiet sender
/// cannot bank unbounded admission work and spend it in a burst.
pub const DEFAULT_WIDTH_CAP: u64 = 30_000_000;

/// Default safety factor `3/2`. Above one because the client's real cost exceeds the gas
/// it can attribute: admission also touches the pool's own structures, the reservation
/// maps and the eviction bookkeeping, none of which appears in a gas figure.
pub const DEFAULT_SAFETY_FACTOR_NUM: u64 = 3;
pub const DEFAULT_SAFETY_FACTOR_DEN: u64 = 2;

/// Default maximum time a frame transaction may stay pending. After this it is removed
/// and must be admitted again, spending another charge if it is additional.
pub const DEFAULT_MAX_PENDING_LIFETIME: Duration = Duration::from_secs(3 * 60 * 60);

/// Local policy for the width mechanism. None of it is consensus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchaConfig {
    pub enabled: bool,
    /// Maximum width a sender may accumulate.
    pub width_cap: u64,
    /// `charge = ceil(safety_factor * admission_gas)`, as a rational so the ceiling is exact.
    pub safety_factor_num: u64,
    pub safety_factor_den: u64,
    /// Starting floor for the optional linear fee on additional transactions. Zero
    /// disables it, which is the default: the proposal presents it as optional deterrence
    /// and leaves open whether FOCIL alone suffices.
    pub base_price: u64,
    /// An additional transaction's EIP-8141 expiry and every EIP-8272 recent root must
    /// stay valid for at least this many slots past the next block. Zero disables it.
    pub min_validity_slots: u64,
    /// Every pending frame transaction is removed once it has been pending this long.
    /// Zero disables it.
    pub max_pending_lifetime: Duration,
}

impl Default for MatchaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            width_cap: DEFAULT_WIDTH_CAP,
            safety_factor_num: DEFAULT_SAFETY_FACTOR_NUM,
            safety_factor_den: DEFAULT_SAFETY_FACTOR_DEN,
            base_price: 0,
            min_validity_slots: 0,
            max_pending_lifetime: DEFAULT_MAX_PENDING_LIFETIME,
        }
    }
}

/// Why an additional transaction was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthError {
    Insufficient { have: u64, need: u64 },
    BelowFloor { offered: u64, floor: u64 },
}

/// What an additional frame transaction must pay, computed before the mempool lock.
///
/// Carried as one value so the admission path cannot pass a charge without the two
/// facts it is judged against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchaCharge {
    /// `ceil(safety_factor * admission_gas)`.
    pub charge: u64,
    /// The priority fee the sender would actually pay in the next block, which the
    /// optional linear floor is compared against.
    pub effective_priority_fee: u64,
    /// Whether the transaction's expiry and recent roots outlast `min_validity_slots`.
    /// Only enforced for additional transactions.
    pub meets_validity_floor: bool,
}

/// The gas a frame transaction costs the client to admit.
///
/// Per the proposal: transaction data, signatures, the EIP-8250 nonce checks, the
/// EIP-8272 recent-root checks, and EIP-8141 execution through payment approval. It
/// excludes application execution after the payer is established and the state
/// dimension entirely, because neither is work the client does at admission.
///
/// `prefix_execution_gas` is the sum of `limits.execution` over the validation prefix
/// frames and the recent-root verifier frame, the same figure `MAX_VERIFY_GAS` is
/// measured against. It is passed in rather than recomputed so the charge and the
/// budget cannot disagree about which frames count as validation work.
pub fn admission_gas(tx: &FrameTransaction, prefix_execution_gas: u64) -> u64 {
    let frames = tx.frames.len() as u64;
    let keys = tx.nonce_keys.len() as u64;
    FRAME_TX_INTRINSIC_COST
        .saturating_add(frames.saturating_mul(FRAME_TX_PER_FRAME_COST))
        .saturating_add(tx.signature_verification_cost())
        .saturating_add(tx.data_cost())
        .saturating_add(prefix_execution_gas)
        .saturating_add(keys.saturating_mul(NONCE_KEY_CHECK_GAS))
}

/// `ceil(safety_factor * admission_gas)` in integers. A zero denominator is treated as a
/// factor of one rather than a panic, since the config is operator-supplied.
pub fn charge_for(config: &MatchaConfig, admission_gas: u64) -> u64 {
    if config.safety_factor_den == 0 {
        return admission_gas;
    }
    let scaled = (admission_gas as u128).saturating_mul(config.safety_factor_num as u128);
    u64::try_from(scaled.div_ceil(config.safety_factor_den as u128)).unwrap_or(u64::MAX)
}

/// Per-sender width, and the load the optional linear fee reads.
#[derive(Debug, Default)]
pub struct WidthLedger {
    config: MatchaConfig,
    /// Earned, unspent width. Absent means zero, and a sender is dropped from the map
    /// the moment it reaches zero, so the map is bounded by senders with a live balance.
    width: FxHashMap<Address, u64>,
    /// Highest finalized block credited so far. Keyed on number, not hash: after a reorg
    /// two blocks at one height must not both mint width.
    last_credited: Option<BlockNumber>,
    /// Sum of `charge` over pending additional transactions. Unlike spent width this falls
    /// when a transaction leaves, because it measures present pressure, not past work.
    load: u64,
}

impl WidthLedger {
    pub fn new(config: MatchaConfig) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    pub fn config(&self) -> &MatchaConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: MatchaConfig) {
        self.config = config;
    }

    pub fn width_of(&self, sender: Address) -> u64 {
        self.width.get(&sender).copied().unwrap_or(0)
    }

    pub fn load(&self) -> u64 {
        self.load
    }

    pub fn last_credited(&self) -> Option<BlockNumber> {
        self.last_credited
    }

    /// Credit one newly finalized block's per-sender gas, capped.
    ///
    /// Returns false for a block at or below the high-water mark, so a repeated or
    /// out-of-order notification is a no-op rather than a second credit.
    pub fn credit_finalized_block(
        &mut self,
        number: BlockNumber,
        gas_by_sender: &FxHashMap<Address, u64>,
    ) -> bool {
        if self.last_credited.is_some_and(|last| number <= last) {
            return false;
        }
        self.last_credited = Some(number);
        for (sender, gas) in gas_by_sender {
            if *gas == 0 {
                continue;
            }
            let entry = self.width.entry(*sender).or_insert(0);
            *entry = entry.saturating_add(*gas).min(self.config.width_cap);
        }
        true
    }

    /// The minimum effective priority fee an additional transaction must offer: zero when
    /// the policy is off, otherwise `base_price` times one more than the number of
    /// charges of this size already pending.
    pub fn fee_floor(&self, charge: u64) -> u64 {
        if self.config.base_price == 0 || charge == 0 {
            return 0;
        }
        let steps = self.load / charge;
        self.config
            .base_price
            .saturating_mul(steps.saturating_add(1))
    }

    /// Spend `charge` to admit an additional transaction, after checking the fee floor.
    pub fn spend(
        &mut self,
        sender: Address,
        charge: u64,
        effective_priority_fee: u64,
    ) -> Result<(), WidthError> {
        let floor = self.fee_floor(charge);
        if effective_priority_fee < floor {
            return Err(WidthError::BelowFloor {
                offered: effective_priority_fee,
                floor,
            });
        }
        self.debit(sender, charge)?;
        self.load = self.load.saturating_add(charge);
        Ok(())
    }

    /// Spend `charge` from the payer of an additional sponsored transaction. The fee floor
    /// is a property of the transaction and was judged once already, so this only debits
    /// and records the load.
    pub fn spend_as_payer(&mut self, payer: Address, charge: u64) -> Result<(), WidthError> {
        self.debit(payer, charge)?;
        self.load = self.load.saturating_add(charge);
        Ok(())
    }

    /// Spend a pending transaction's stored charge again before revalidating it. A sender
    /// that cannot pay has the transaction dropped instead of revalidated for free.
    pub fn spend_for_revalidation(
        &mut self,
        sender: Address,
        charge: u64,
    ) -> Result<(), WidthError> {
        self.debit(sender, charge)
    }

    /// Release a departing transaction's charge from `load`. Its width stays spent.
    pub fn release_load(&mut self, charge: u64) {
        self.load = self.load.saturating_sub(charge);
    }

    fn debit(&mut self, sender: Address, charge: u64) -> Result<(), WidthError> {
        let have = self.width_of(sender);
        if have < charge {
            return Err(WidthError::Insufficient { have, need: charge });
        }
        let left = have - charge;
        if left == 0 {
            self.width.remove(&sender);
        } else {
            self.width.insert(sender, left);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(byte: u8) -> Address {
        Address::from_low_u64_be(byte as u64)
    }

    fn ledger(cap: u64) -> WidthLedger {
        WidthLedger::new(MatchaConfig {
            width_cap: cap,
            ..Default::default()
        })
    }

    fn credit(l: &mut WidthLedger, number: BlockNumber, sender: Address, gas: u64) -> bool {
        let mut by_sender = FxHashMap::default();
        by_sender.insert(sender, gas);
        l.credit_finalized_block(number, &by_sender)
    }

    #[test]
    fn a_new_sender_has_no_width() {
        let mut l = ledger(1_000_000);
        assert_eq!(l.width_of(addr(1)), 0);
        assert_eq!(
            l.spend(addr(1), 1, 0),
            Err(WidthError::Insufficient { have: 0, need: 1 })
        );
    }

    #[test]
    fn width_accrues_from_finalized_gas_and_stops_at_the_cap() {
        let mut l = ledger(100);
        assert!(credit(&mut l, 1, addr(1), 60));
        assert_eq!(l.width_of(addr(1)), 60);
        assert!(credit(&mut l, 2, addr(1), 60));
        assert_eq!(l.width_of(addr(1)), 100);
    }

    #[test]
    fn a_finalized_block_is_credited_once() {
        let mut l = ledger(1_000_000);
        assert!(credit(&mut l, 7, addr(1), 50));
        assert!(!credit(&mut l, 7, addr(1), 50), "same block again");
        assert!(!credit(&mut l, 6, addr(1), 50), "an older block");
        assert_eq!(l.width_of(addr(1)), 50);
    }

    #[test]
    fn spent_width_is_never_returned() {
        let mut l = ledger(1_000_000);
        credit(&mut l, 1, addr(1), 100);
        l.spend(addr(1), 40, 0).unwrap();
        assert_eq!(l.width_of(addr(1)), 60);
        l.release_load(40);
        assert_eq!(l.width_of(addr(1)), 60, "removal releases load, not width");
        assert_eq!(l.load(), 0);
    }

    #[test]
    fn revalidation_spends_the_stored_charge_and_can_exhaust_a_sender() {
        let mut l = ledger(1_000_000);
        credit(&mut l, 1, addr(1), 100);
        l.spend(addr(1), 30, 0).unwrap();
        l.spend_for_revalidation(addr(1), 30).unwrap();
        l.spend_for_revalidation(addr(1), 30).unwrap();
        assert_eq!(l.width_of(addr(1)), 10);
        assert_eq!(
            l.spend_for_revalidation(addr(1), 30),
            Err(WidthError::Insufficient { have: 10, need: 30 })
        );
    }

    #[test]
    fn a_sender_spent_to_zero_is_forgotten() {
        let mut l = ledger(1_000_000);
        credit(&mut l, 1, addr(1), 30);
        l.spend(addr(1), 30, 0).unwrap();
        assert_eq!(l.width_of(addr(1)), 0);
        assert!(!l.width.contains_key(&addr(1)));
    }

    #[test]
    fn the_charge_is_the_ceiling_of_the_safety_factor() {
        let c = MatchaConfig::default();
        assert_eq!(charge_for(&c, 100), 150);
        assert_eq!(charge_for(&c, 101), 152, "151.5 rounds up");
        assert_eq!(charge_for(&c, 0), 0);
    }

    #[test]
    fn the_linear_floor_rises_one_step_per_pending_charge() {
        let mut l = WidthLedger::new(MatchaConfig {
            width_cap: 1_000_000,
            base_price: 7,
            ..Default::default()
        });
        credit(&mut l, 1, addr(1), 1_000);
        assert_eq!(l.fee_floor(10), 7);
        l.spend(addr(1), 10, 7).unwrap();
        assert_eq!(l.fee_floor(10), 14);
        l.spend(addr(1), 10, 14).unwrap();
        assert_eq!(l.fee_floor(10), 21);
        assert_eq!(
            l.spend(addr(1), 10, 20),
            Err(WidthError::BelowFloor {
                offered: 20,
                floor: 21
            })
        );
    }

    #[test]
    fn load_falls_on_removal_while_width_does_not() {
        let mut l = WidthLedger::new(MatchaConfig {
            width_cap: 1_000_000,
            base_price: 5,
            ..Default::default()
        });
        credit(&mut l, 1, addr(1), 1_000);
        l.spend(addr(1), 10, 5).unwrap();
        l.spend(addr(1), 10, 10).unwrap();
        assert_eq!(l.fee_floor(10), 15);
        l.release_load(10);
        assert_eq!(l.fee_floor(10), 10);
        assert_eq!(l.width_of(addr(1)), 980);
    }

    #[test]
    fn width_does_not_transfer_between_senders() {
        let mut l = ledger(1_000_000);
        credit(&mut l, 1, addr(1), 100);
        assert_eq!(l.width_of(addr(2)), 0);
        assert!(l.spend(addr(2), 1, 0).is_err());
    }

    #[test]
    fn the_fee_floor_is_optional_but_width_is_not() {
        let mut l = ledger(1_000_000);
        assert_eq!(l.fee_floor(10), 0);
        assert!(l.spend(addr(1), 1, 0).is_err());
    }
}
