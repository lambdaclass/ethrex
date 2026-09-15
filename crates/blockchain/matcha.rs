//! MATCHA: mempool account transaction capacity from historical activity.
//!
//! Implements the mechanism described in *Mempool Account Transaction Capacity from
//! Historical Activity*, which lets one `sender` hold several pending EIP-8141 frame
//! transactions with independent EIP-8250 nonce keys without the client whitelisting the
//! application, paymaster, verifier or proof system behind them.
//!
//! The unit of account is **width**: the capacity a sender has for pending frame
//! transactions beyond the one EIP-8141 already allows it. A sender starts with none,
//! earns it from the gas its own transactions actually used in newly finalized blocks,
//! and spends it whenever the client does admission work on its behalf.
//!
//! Three properties make this a DoS defence rather than a fee:
//!
//! - **Earned, not granted.** Width comes only from finalized gas, so a sender with no
//!   history has exactly the baseline capacity, and buying capacity means first paying
//!   for blockspace that was actually included.
//! - **Spent irreversibly.** Width is not returned when a transaction is included,
//!   invalidated, replaced or evicted. The client did the work either way, and refunding
//!   on invalidation would make mass invalidation free for the attacker who caused it.
//! - **Charged for work, not for bytes.** The charge tracks admission gas, which is what
//!   the client spends deciding whether to keep the transaction, including every later
//!   revalidation.
//!
//! This module is pure bookkeeping. It performs no I/O and holds no locks; the mempool
//! owns an instance and calls it under its own write lock.
//!
//! Divergences from the post, and the reasoning behind them, are recorded in
//! `docs/matcha.md`.

use ethrex_common::Address;
use ethrex_common::types::{BlockNumber, FrameTransaction};
use rustc_hash::FxHashMap;

/// Gas charged per EIP-8250 nonce key for the admission-time sequence check.
///
/// The post names "EIP-8250 nonce checks" as a term of `admission_gas` without pricing
/// them. Each key costs the client one keyed-slot read at the current head, so this is a
/// cold `SLOAD` under EIP-2929. Pricing it per key rather than per transaction is what
/// makes a sixteen-key transaction cost more to admit than a one-key transaction, which
/// is the honest shape: the work is per key.
pub const NONCE_KEY_CHECK_GAS: u64 = 2_100;

/// Default cap on accumulated width, as a multiple of nothing in particular: it is local
/// policy, and the post says so. Chosen as roughly one block of gas, so a sender that has
/// been quiet for a long time cannot bank unbounded admission work and spend it in a
/// burst. Lowering it makes the mechanism stricter; raising it lets a heavy application
/// absorb bigger spikes.
pub const DEFAULT_WIDTH_CAP: u64 = 30_000_000;

/// Default safety factor numerator and denominator: `charge = ceil(3/2 * admission_gas)`.
///
/// Above one because the client's real cost exceeds the gas it can attribute. Admission
/// also touches the pool's own structures, the reservation maps and the eviction
/// bookkeeping, and none of that appears in a gas figure.
pub const DEFAULT_SAFETY_FACTOR_NUM: u64 = 3;
pub const DEFAULT_SAFETY_FACTOR_DEN: u64 = 2;

/// Local policy for the width mechanism.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchaConfig {
    /// Maximum width a sender may accumulate.
    pub width_cap: u64,
    /// `charge = ceil(safety_factor * admission_gas)`, as a rational so the ceiling is
    /// exact rather than a float rounding.
    pub safety_factor_num: u64,
    pub safety_factor_den: u64,
    /// Starting floor for the optional linear fee. Zero disables the policy, which is the
    /// default: the post presents it as optional deterrence, and FOCIL may make it
    /// unnecessary.
    pub base_price: u64,
    /// When false, width is not consulted and the client keeps whatever structural rule
    /// it had. Provided so the mechanism can be switched off in one place.
    pub enabled: bool,
}

impl Default for MatchaConfig {
    fn default() -> Self {
        Self {
            width_cap: DEFAULT_WIDTH_CAP,
            safety_factor_num: DEFAULT_SAFETY_FACTOR_NUM,
            safety_factor_den: DEFAULT_SAFETY_FACTOR_DEN,
            base_price: 0,
            enabled: true,
        }
    }
}

/// Why an additional transaction was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthError {
    /// The sender has not earned enough width for this transaction's charge.
    Insufficient { have: u64, need: u64 },
    /// The effective priority fee is below the load-dependent floor.
    BelowFloor { offered: u64, floor: u64 },
}

/// The admission gas a frame transaction costs the client to consider.
///
/// Per the post: transaction data, signatures, the EIP-8250 nonce checks, the EIP-8272
/// recent-root checks, and EIP-8141 execution through payment approval. It deliberately
/// **excludes** application execution after the payer is established, and excludes the
/// state dimension entirely, because neither is work the client does at admission.
///
/// `prefix_execution_gas` is the caller's sum of `limits.execution` over the validation
/// prefix frames plus the EIP-8272 recent-root verifier frame, which is the same figure
/// `MAX_VERIFY_GAS` is measured against. It is passed in rather than recomputed so this
/// function cannot drift from the budget the admission path already enforces.
pub fn admission_gas(tx: &FrameTransaction, prefix_execution_gas: u64) -> u64 {
    // `mandatory_gas` already carries signature verification, the intrinsic cost, the
    // per-frame cost and the value-transfer cost.
    tx.mandatory_gas()
        .saturating_add(tx.data_cost())
        .saturating_add(prefix_execution_gas)
        .saturating_add((tx.nonce_keys.len() as u64).saturating_mul(NONCE_KEY_CHECK_GAS))
}

/// `charge = ceil(safety_factor * admission_gas)`, computed in integers.
pub fn charge_for(config: &MatchaConfig, admission_gas: u64) -> u64 {
    if config.safety_factor_den == 0 {
        return admission_gas;
    }
    let scaled = (admission_gas as u128).saturating_mul(config.safety_factor_num as u128);
    let den = config.safety_factor_den as u128;
    let charged = scaled.div_ceil(den);
    u64::try_from(charged).unwrap_or(u64::MAX)
}

/// Per-sender width, and the load the optional linear fee reads.
#[derive(Debug, Default)]
pub struct WidthLedger {
    config: MatchaConfig,
    /// Earned, unspent width per sender. A sender absent from the map holds zero, which
    /// is the correct starting state: width is earned, never granted.
    width: FxHashMap<Address, u64>,
    /// Highest finalized block already credited. The post requires each finalized block
    /// to be credited once; tracking the high-water mark makes a repeated or out-of-order
    /// notification a no-op rather than a double credit.
    last_credited: Option<BlockNumber>,
    /// Sum of `charge` over pending additional transactions. Unlike spent width this
    /// falls when a transaction leaves the pool, because it measures present pressure
    /// rather than past work.
    load: u64,
}

impl WidthLedger {
    pub fn new(config: MatchaConfig) -> Self {
        Self {
            config,
            width: FxHashMap::default(),
            last_credited: None,
            load: 0,
        }
    }

    pub fn config(&self) -> &MatchaConfig {
        &self.config
    }

    pub fn width_of(&self, sender: Address) -> u64 {
        self.width.get(&sender).copied().unwrap_or(0)
    }

    pub fn load(&self) -> u64 {
        self.load
    }

    /// The highest finalized block already credited.
    pub fn last_credited(&self) -> Option<BlockNumber> {
        self.last_credited
    }

    /// Credit one newly finalized block's per-sender gas, capped.
    ///
    /// Returns false when the block was already credited, so the caller can treat a
    /// duplicate forkchoice notification as a no-op. A block number at or below the
    /// high-water mark is refused: re-crediting after a reorg would mint width for gas
    /// that is no longer finalized.
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
            let entry = self.width.entry(*sender).or_insert(0);
            *entry = entry.saturating_add(*gas).min(self.config.width_cap);
        }
        true
    }

    /// The minimum effective priority fee an additional transaction must offer.
    ///
    /// Zero when the policy is disabled. Otherwise the floor rises linearly with how many
    /// charges of this size already sit in the pool, which reproduces the post's stated
    /// behaviour when every charge is equal: `base_price` for the first additional
    /// transaction, twice that for the next, and so on.
    pub fn fee_floor(&self, charge: u64) -> u64 {
        if self.config.base_price == 0 || charge == 0 {
            return 0;
        }
        let steps = self.load / charge;
        self.config
            .base_price
            .saturating_mul(steps.saturating_add(1))
    }

    /// Spend `charge` for an additional transaction, after checking the fee floor.
    ///
    /// Spending is irreversible by design, so this is the only place width decreases and
    /// there is deliberately no counterpart that returns it.
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
        let have = self.width_of(sender);
        if have < charge {
            return Err(WidthError::Insufficient { have, need: charge });
        }
        self.width.insert(sender, have - charge);
        self.load = self.load.saturating_add(charge);
        Ok(())
    }

    /// Spend a stored charge again before revalidating a pending transaction.
    ///
    /// The post requires rerunning a validation prefix to spend the transaction's stored
    /// charge before the work begins. A sender that cannot pay has its transaction
    /// dropped rather than revalidated for free, which is what stops mass invalidation
    /// from being cheap for the party who caused it.
    pub fn spend_for_revalidation(
        &mut self,
        sender: Address,
        charge: u64,
    ) -> Result<(), WidthError> {
        let have = self.width_of(sender);
        if have < charge {
            return Err(WidthError::Insufficient { have, need: charge });
        }
        self.width.insert(sender, have - charge);
        Ok(())
    }

    /// Release a departing transaction's charge from `load`.
    ///
    /// Only `load` moves. The spent width stays spent: the client already did the work,
    /// and returning it on removal would let an attacker cycle transactions through the
    /// pool at no cost.
    pub fn release_load(&mut self, charge: u64) {
        self.load = self.load.saturating_sub(charge);
    }

    /// Forget a sender with no width and no pending work, to bound the map.
    pub fn forget_if_empty(&mut self, sender: Address) {
        if self.width.get(&sender).is_some_and(|w| *w == 0) {
            self.width.remove(&sender);
        }
    }
}

/// What an additional frame transaction must pay, computed before the mempool lock.
///
/// Carried as a unit so the admission path cannot pass a charge without the fee that the
/// load-dependent floor is judged against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchaCharge {
    /// `ceil(safety_factor * admission_gas)`.
    pub charge: u64,
    /// The transaction's effective priority fee at the current head, which the optional
    /// linear-fee floor is compared against.
    pub effective_priority_fee: u64,
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

    /// Width is earned, never granted: a sender the client has never seen has none, so
    /// its very first additional transaction is refused however cheap it is.
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
        let mut gas = FxHashMap::default();
        gas.insert(addr(1), 60);
        assert!(l.credit_finalized_block(1, &gas));
        assert_eq!(l.width_of(addr(1)), 60);
        assert!(l.credit_finalized_block(2, &gas));
        assert_eq!(
            l.width_of(addr(1)),
            100,
            "the cap binds, 120 would exceed it"
        );
    }

    /// The post requires each finalized block to be credited once. Re-notifying the same
    /// block, or an older one after a reorg, must not mint width a second time.
    #[test]
    fn a_finalized_block_is_credited_once() {
        let mut l = ledger(1_000_000);
        let mut gas = FxHashMap::default();
        gas.insert(addr(1), 50);
        assert!(l.credit_finalized_block(7, &gas));
        assert_eq!(l.width_of(addr(1)), 50);
        assert!(!l.credit_finalized_block(7, &gas), "same block again");
        assert!(!l.credit_finalized_block(6, &gas), "an older block");
        assert_eq!(l.width_of(addr(1)), 50);
    }

    /// Spending is irreversible. There is no API that returns width, and this test exists
    /// to make adding one a deliberate act rather than an accident.
    #[test]
    fn spent_width_is_never_returned() {
        let mut l = ledger(1_000_000);
        let mut gas = FxHashMap::default();
        gas.insert(addr(1), 100);
        l.credit_finalized_block(1, &gas);
        assert!(l.spend(addr(1), 40, 0).is_ok());
        assert_eq!(l.width_of(addr(1)), 60);
        // A transaction leaving the pool releases load only.
        l.release_load(40);
        assert_eq!(l.width_of(addr(1)), 60, "removal must not refund width");
        assert_eq!(l.load(), 0);
    }

    /// Revalidation is charged before the work, so a sender that caused mass
    /// invalidation pays for the rerun it forced and runs out if it keeps forcing them.
    #[test]
    fn revalidation_spends_the_stored_charge_and_can_exhaust_a_sender() {
        let mut l = ledger(1_000_000);
        let mut gas = FxHashMap::default();
        gas.insert(addr(1), 100);
        l.credit_finalized_block(1, &gas);
        assert!(l.spend(addr(1), 30, 0).is_ok());
        assert!(l.spend_for_revalidation(addr(1), 30).is_ok());
        assert!(l.spend_for_revalidation(addr(1), 30).is_ok());
        assert_eq!(l.width_of(addr(1)), 10);
        assert_eq!(
            l.spend_for_revalidation(addr(1), 30),
            Err(WidthError::Insufficient { have: 10, need: 30 }),
            "the fourth rerun is refused; the sender pays for the churn it causes"
        );
    }

    #[test]
    fn the_charge_is_the_ceiling_of_the_safety_factor() {
        let c = MatchaConfig::default();
        assert_eq!(c.safety_factor_num, 3);
        assert_eq!(c.safety_factor_den, 2);
        assert_eq!(charge_for(&c, 100), 150);
        assert_eq!(charge_for(&c, 101), 152, "151.5 rounds up, never down");
        assert_eq!(charge_for(&c, 0), 0);
    }

    /// The post's stated behaviour: with equal charges the floor is `base_price`, then
    /// twice it, then three times.
    #[test]
    fn the_linear_floor_rises_one_step_per_pending_charge() {
        let mut l = WidthLedger::new(MatchaConfig {
            width_cap: 1_000_000,
            base_price: 7,
            ..Default::default()
        });
        let mut gas = FxHashMap::default();
        gas.insert(addr(1), 1_000);
        l.credit_finalized_block(1, &gas);

        assert_eq!(l.fee_floor(10), 7, "first additional transaction");
        assert!(l.spend(addr(1), 10, 7).is_ok());
        assert_eq!(l.fee_floor(10), 14, "second");
        assert!(l.spend(addr(1), 10, 14).is_ok());
        assert_eq!(l.fee_floor(10), 21, "third");

        assert_eq!(
            l.spend(addr(1), 10, 20),
            Err(WidthError::BelowFloor {
                offered: 20,
                floor: 21
            })
        );
    }

    /// A departing transaction lowers the floor for the next one, even though its width
    /// stays spent. Load measures present pressure; width measures past work.
    #[test]
    fn load_falls_on_removal_while_width_does_not() {
        let mut l = WidthLedger::new(MatchaConfig {
            width_cap: 1_000_000,
            base_price: 5,
            ..Default::default()
        });
        let mut gas = FxHashMap::default();
        gas.insert(addr(1), 1_000);
        l.credit_finalized_block(1, &gas);

        l.spend(addr(1), 10, 5).unwrap();
        l.spend(addr(1), 10, 10).unwrap();
        assert_eq!(l.fee_floor(10), 15);
        assert_eq!(l.width_of(addr(1)), 980);

        l.release_load(10);
        assert_eq!(l.fee_floor(10), 10, "one fewer pending charge");
        assert_eq!(l.width_of(addr(1)), 980, "width is still spent");
    }

    /// One sender's history cannot fund another's admission.
    #[test]
    fn width_does_not_transfer_between_senders() {
        let mut l = ledger(1_000_000);
        let mut gas = FxHashMap::default();
        gas.insert(addr(1), 100);
        l.credit_finalized_block(1, &gas);
        assert_eq!(l.width_of(addr(2)), 0);
        assert!(l.spend(addr(2), 1, 0).is_err());
    }

    /// Disabling the fee policy must not disable the width requirement: the two are
    /// independent, and the post presents only the fee as optional.
    #[test]
    fn the_fee_floor_is_optional_but_width_is_not() {
        let mut l = ledger(1_000_000);
        assert_eq!(l.config().base_price, 0);
        assert_eq!(l.fee_floor(10), 0);
        assert!(
            l.spend(addr(1), 1, 0).is_err(),
            "no fee floor still means no free width"
        );
    }
}
