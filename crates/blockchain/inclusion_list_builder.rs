//! EIP-7805 (FOCIL) inclusion-list builder. Reads from the local public
//! mempool and produces a `Vec<Transaction>` (≤ 8 KiB total RLP) that the
//! `engine_getInclusionListV1` handler returns to the consensus layer.
//!
//! Spec contract:
//! `openspec/changes/eip-7805-focil-execution-layer/specs/inclusion-list-construction/spec.md`.
//!
//! ## State abstraction
//!
//! Builder needs to read `(nonce, balance)` for each candidate sender against
//! the parent block's pre-state. The cleanest synchronous API in ethrex is
//! `Store::get_account_state_by_root`, but binding the builder to `&Store`
//! couples it to the storage crate and forces the builder's tests to spin up
//! a real `Store`. Instead we define a minimal local trait
//! [`IlStateProvider`] that exposes only the read we need. The Phase 4
//! engine handler will provide a one-line `StoreVmDatabase`-backed adapter.
//!
//! Choosing the local trait over `&dyn VmDatabase` (which `StoreVmDatabase`
//! already implements) keeps the error type narrow (no `EvmError`) and
//! avoids pulling code/storage methods we never call.

use std::time::{SystemTime, UNIX_EPOCH};

use ethrex_common::{
    Address, U256,
    types::{MempoolTransaction, Transaction},
};
use rustc_hash::FxHashMap;

use crate::mempool::Mempool;

/// Hard byte cap on the total RLP-encoded size of the returned inclusion list,
/// matching `MAX_BYTES_PER_INCLUSION_LIST` in the execution-apis spec.
pub const MAX_BYTES_PER_INCLUSION_LIST: usize = 8192;

/// Default per-sender cap for the IL builder. Matches the `--il-per-sender-cap`
/// CLI default.
pub const DEFAULT_PER_SENDER_CAP: usize = 2;

/// Account snapshot used to validate IL candidates against parent-state.
/// `None` from [`IlStateProvider::get_account`] means the account is empty
/// (`nonce = 0`, `balance = 0`).
#[derive(Clone, Copy, Debug, Default)]
pub struct AccountStateView {
    pub nonce: u64,
    pub balance: U256,
}

/// Synchronous, account-only state read used by the IL builder against a
/// fixed state root (typically the parent block's pre-state). The trait is
/// purpose-built so the builder can be unit-tested with a small in-memory
/// fake without pulling in `Store` or the full `VmDatabase` surface.
pub trait IlStateProvider {
    fn get_account(
        &self,
        address: Address,
    ) -> Result<Option<AccountStateView>, IlStateProviderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum IlStateProviderError {
    #[error("state read error: {0}")]
    Read(String),
}

/// IL selection policy. The default `Production` policy weighs age and
/// priority fee per Decision 3 of `design.md`. `PriorityFee` and `Random`
/// are operator-selectable debug knobs exposed via `--il-policy`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IlPolicy {
    #[default]
    Production,
    PriorityFee,
    Random,
}

/// Inclusion-list builder. Stateless across calls — instances are cheap to
/// construct per `engine_getInclusionListV1` invocation.
#[derive(Clone, Copy, Debug)]
pub struct InclusionListBuilder {
    pub policy: IlPolicy,
    pub per_sender_cap: usize,
    pub max_bytes: usize,
}

impl Default for InclusionListBuilder {
    fn default() -> Self {
        Self {
            policy: IlPolicy::default(),
            per_sender_cap: DEFAULT_PER_SENDER_CAP,
            max_bytes: MAX_BYTES_PER_INCLUSION_LIST,
        }
    }
}

impl InclusionListBuilder {
    pub fn new(policy: IlPolicy, per_sender_cap: usize, max_bytes: usize) -> Self {
        Self {
            policy,
            per_sender_cap,
            max_bytes,
        }
    }

    /// Build a public-mempool-derived inclusion list. Returns an empty `Vec`
    /// if the mempool is empty, the provider errors out, or no candidate
    /// transaction validates against parent state. `base_fee` is the parent
    /// block's `base_fee_per_gas` and is kept for API symmetry with future
    /// extensions; selection policies do not consume it today.
    pub fn build(
        &self,
        mempool: &Mempool,
        _base_fee: u64,
        head_state: &dyn IlStateProvider,
    ) -> Vec<Transaction> {
        // 1. Pull every transaction from the public mempool grouped by sender.
        let txs_by_sender = match mempool.get_all_txs_by_sender() {
            Ok(map) => map,
            Err(_) => return Vec::new(),
        };

        // 2. Per-sender filter + truncation. Apply BEFORE scoring so we don't
        //    waste score-compute on dropped candidates. `get_all_txs_by_sender`
        //    already sorts each sender's vec, but ordering relies on the
        //    `MempoolTransaction: Ord` impl which prioritizes tip — we want
        //    ascending nonce for the cap, so re-sort defensively.
        let candidates = self.filter_and_cap(txs_by_sender, head_state);

        if candidates.is_empty() {
            return Vec::new();
        }

        // 3. Score & order by policy.
        let ordered = self.order_by_policy(candidates);

        // 4. Greedy 8-KiB packer. Skip a candidate that would overflow rather
        //    than truncating; subsequent (smaller) candidates may still fit.
        let mut packed = Vec::with_capacity(ordered.len());
        let mut bytes_used = 0usize;
        for tx in ordered {
            let rlp_len = tx.encode_canonical_to_vec().len();
            if bytes_used.saturating_add(rlp_len) > self.max_bytes {
                continue;
            }
            bytes_used = bytes_used.saturating_add(rlp_len);
            packed.push(tx);
        }
        packed
    }

    /// Apply blob/L2 exclusion, parent-state validity, and per-sender cap.
    /// Returns the surviving candidates as a flat list of
    /// `(MempoolTransaction, Transaction)` pairs (the mempool wrapper is
    /// retained for `time()` access during scoring).
    fn filter_and_cap(
        &self,
        txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>>,
        head_state: &dyn IlStateProvider,
    ) -> Vec<MempoolTransaction> {
        let mut survivors: Vec<MempoolTransaction> = Vec::new();

        for (sender, mut sender_txs) in txs_by_sender {
            // Look up the sender's account once per sender. A read failure
            // is treated as "skip this sender" — being conservative, since
            // we can't validate them.
            let acct = match head_state.get_account(sender) {
                Ok(Some(view)) => view,
                Ok(None) => AccountStateView::default(),
                Err(_) => continue,
            };

            // Sort by ascending nonce. The mempool `Ord` ranks by tip, not
            // nonce, so do not rely on it for the per-sender cap.
            sender_txs.sort_by_key(|mtx| mtx.nonce());

            // Track the expected next nonce as we walk; if there's a gap we
            // stop accepting from this sender (the gapped tx can't be
            // included until the prior nonce lands).
            let mut expected_nonce = acct.nonce;
            let mut running_balance = acct.balance;
            let mut taken: usize = 0;

            for mtx in sender_txs {
                if taken >= self.per_sender_cap {
                    break;
                }

                let tx: &Transaction = mtx.transaction();

                // Blob exclusion (spec rule, non-negotiable).
                if matches!(tx, Transaction::EIP4844Transaction(_)) {
                    continue;
                }
                // L2 privileged exclusion (FOCIL is L1-only).
                if matches!(tx, Transaction::PrivilegedL2Transaction(_)) {
                    continue;
                }

                // Validity against parent state.
                if tx.nonce() != expected_nonce {
                    // Either already-included (nonce too low) or future
                    // (nonce too high). Either way, not includable now.
                    if tx.nonce() < expected_nonce {
                        continue; // skip stale, keep looking
                    }
                    break; // gap: stop walking this sender
                }
                let cost = match tx.cost_without_base_fee() {
                    Some(c) => c,
                    None => continue, // malformed fee data — skip
                };
                if cost > running_balance {
                    break; // sender can't afford this tx, can't afford follow-ups either
                }

                // Account moves forward as if this tx were included next.
                expected_nonce = expected_nonce.saturating_add(1);
                running_balance = running_balance.saturating_sub(cost);
                taken = taken.saturating_add(1);
                survivors.push(mtx);
            }
        }

        survivors
    }

    fn order_by_policy(&self, candidates: Vec<MempoolTransaction>) -> Vec<Transaction> {
        match self.policy {
            IlPolicy::Production => order_by_production_score(candidates),
            IlPolicy::PriorityFee => order_by_priority_fee(candidates),
            IlPolicy::Random => order_by_random(candidates),
        }
    }
}

/// `score = age_seconds * (1.0 + ln(max_priority_fee_per_gas + 1.0))`.
/// Legacy and EIP-2930 transactions return `None` from `max_priority_fee()`;
/// for those we treat the tip as zero (the log term collapses to `ln(1) = 0`,
/// so `score = age_seconds`).
fn order_by_production_score(candidates: Vec<MempoolTransaction>) -> Vec<Transaction> {
    let now_micros = now_unix_micros();
    let mut scored: Vec<(f64, Transaction)> = candidates
        .into_iter()
        .map(|mtx| {
            let age_micros = now_micros.saturating_sub(mtx.time());
            // Cast through `u128 -> f64`. ~2^53 micros ≈ 285 years; safe.
            #[allow(clippy::cast_precision_loss)]
            let age_seconds = (age_micros as f64) / 1_000_000.0;
            let tip = mtx.transaction().max_priority_fee().unwrap_or(0);
            #[allow(clippy::cast_precision_loss)]
            let tip_term = (tip as f64 + 1.0).ln();
            let score = age_seconds * (1.0 + tip_term);
            (score, mtx.transaction().clone())
        })
        .collect();
    // Descending by score. `f64` doesn't implement `Ord`; total-order via
    // `partial_cmp`, fall back to `Equal` on NaN (which shouldn't occur here
    // since age >= 0 and `ln(x>=1) >= 0`, so `score >= 0` always).
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(_, tx)| tx).collect()
}

/// Highest priority fee first. Legacy/EIP-2930 txs treated as tip 0.
fn order_by_priority_fee(candidates: Vec<MempoolTransaction>) -> Vec<Transaction> {
    let mut txs: Vec<Transaction> = candidates
        .into_iter()
        .map(|mtx| mtx.transaction().clone())
        .collect();
    txs.sort_by(|a, b| {
        let a_tip = a.max_priority_fee().unwrap_or(0);
        let b_tip = b.max_priority_fee().unwrap_or(0);
        b_tip.cmp(&a_tip)
    });
    txs
}

/// Deterministic shuffle keyed on the system clock so we don't need a `rand`
/// dependency. The Random policy is a debug knob — its only contract is
/// "terminates and respects the byte budget"; it is not required to be
/// cryptographically random.
fn order_by_random(candidates: Vec<MempoolTransaction>) -> Vec<Transaction> {
    let mut txs: Vec<Transaction> = candidates
        .into_iter()
        .map(|mtx| mtx.transaction().clone())
        .collect();
    if txs.len() < 2 {
        return txs;
    }
    let seed = now_unix_micros() as u64;
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    for i in (1..txs.len()).rev() {
        // SplitMix64-flavored step for the swap index.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let j = (z as usize) % (i + 1);
        txs.swap(i, j);
    }
    txs
}

fn now_unix_micros() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::{
        H256, U256,
        types::{
            EIP1559Transaction, EIP4844Transaction, LegacyTransaction, MempoolTransaction,
            PrivilegedL2Transaction, Transaction, TxKind,
        },
    };
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// In-memory state provider for unit tests.
    #[derive(Default)]
    struct FakeState {
        accounts: RefCell<HashMap<Address, AccountStateView>>,
    }

    impl FakeState {
        fn set(&self, addr: Address, nonce: u64, balance: U256) {
            self.accounts
                .borrow_mut()
                .insert(addr, AccountStateView { nonce, balance });
        }
    }

    impl IlStateProvider for FakeState {
        fn get_account(
            &self,
            address: Address,
        ) -> Result<Option<AccountStateView>, IlStateProviderError> {
            Ok(self.accounts.borrow().get(&address).copied())
        }
    }

    fn addr(byte: u8) -> Address {
        let mut a = [0u8; 20];
        a[19] = byte;
        Address::from(a)
    }

    fn legacy_tx(nonce: u64, gas_price: u64, gas_limit: u64, value: u64) -> Transaction {
        Transaction::LegacyTransaction(LegacyTransaction {
            nonce,
            gas_price: U256::from(gas_price),
            gas: gas_limit,
            to: TxKind::Call(addr(0xff)),
            value: U256::from(value),
            v: U256::from(27),
            r: U256::from(1),
            s: U256::from(1),
            ..Default::default()
        })
    }

    fn eip1559_tx(
        nonce: u64,
        max_fee: u64,
        max_priority: u64,
        gas_limit: u64,
        value: u64,
    ) -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id: 1,
            nonce,
            max_priority_fee_per_gas: max_priority,
            max_fee_per_gas: max_fee,
            gas_limit,
            to: TxKind::Call(addr(0xff)),
            value: U256::from(value),
            signature_r: U256::from(1),
            signature_s: U256::from(1),
            ..Default::default()
        })
    }

    fn blob_tx(nonce: u64, max_fee: u64, gas_limit: u64) -> Transaction {
        Transaction::EIP4844Transaction(EIP4844Transaction {
            chain_id: 1,
            nonce,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: max_fee,
            gas: gas_limit,
            to: addr(0xff),
            max_fee_per_blob_gas: U256::from(1),
            signature_r: U256::from(1),
            signature_s: U256::from(1),
            ..Default::default()
        })
    }

    fn privileged_tx(nonce: u64) -> Transaction {
        Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction {
            chain_id: 1,
            nonce,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            gas_limit: 21_000,
            to: TxKind::Call(addr(0xff)),
            from: addr(0x01),
            ..Default::default()
        })
    }

    fn insert_tx(mempool: &Mempool, sender: Address, tx: Transaction) -> H256 {
        let mtx = MempoolTransaction::new(tx, sender);
        let hash = mtx.transaction().hash();
        mempool
            .add_transaction(hash, sender, mtx)
            .expect("add_transaction");
        hash
    }

    /// Most callers want a wallet-balanced sender that can pay a few txs.
    fn fund(state: &FakeState, sender: Address, nonce: u64) {
        state.set(sender, nonce, U256::from(u128::MAX));
    }

    #[test]
    fn empty_mempool_returns_empty() {
        let mempool = Mempool::new(64);
        let state = FakeState::default();
        let builder = InclusionListBuilder::default();
        let il = builder.build(&mempool, 0, &state);
        assert!(il.is_empty());
    }

    #[test]
    fn production_policy_excludes_blob_txs() {
        let mempool = Mempool::new(64);
        let state = FakeState::default();
        let sender = addr(0x01);
        fund(&state, sender, 0);

        let blob_hash = insert_tx(&mempool, sender, blob_tx(0, 1_000, 21_000));
        let plain_hash = insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, 0));

        let builder = InclusionListBuilder::default();
        let il = builder.build(&mempool, 0, &state);

        let hashes: Vec<H256> = il.iter().map(|tx| tx.hash()).collect();
        assert!(
            !hashes.contains(&blob_hash),
            "blob tx must not appear in IL"
        );
        assert!(hashes.contains(&plain_hash), "plain tx should appear");
    }

    #[test]
    fn privileged_l2_tx_excluded() {
        let mempool = Mempool::new(64);
        let state = FakeState::default();
        let sender = addr(0x01);
        fund(&state, sender, 0);

        let priv_hash = insert_tx(&mempool, sender, privileged_tx(0));
        let plain_hash = insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, 0));

        let builder = InclusionListBuilder::default();
        let il = builder.build(&mempool, 0, &state);

        let hashes: Vec<H256> = il.iter().map(|tx| tx.hash()).collect();
        assert!(!hashes.contains(&priv_hash));
        assert!(hashes.contains(&plain_hash));
    }

    #[test]
    fn per_sender_cap_respected() {
        let mempool = Mempool::new(64);
        let state = FakeState::default();
        let sender = addr(0x01);
        fund(&state, sender, 0);

        // 5 consecutive nonce txs, all valid.
        for nonce in 0..5u64 {
            insert_tx(&mempool, sender, legacy_tx(nonce, 1, 21_000, 0));
        }

        let builder =
            InclusionListBuilder::new(IlPolicy::Production, 2, MAX_BYTES_PER_INCLUSION_LIST);
        let il = builder.build(&mempool, 0, &state);

        assert_eq!(
            il.len(),
            2,
            "per-sender cap of 2 must produce exactly 2 txs from one sender"
        );
        let mut nonces: Vec<u64> = il.iter().map(|tx| tx.nonce()).collect();
        nonces.sort();
        assert_eq!(nonces, vec![0, 1], "cap must take ascending nonces");
    }

    #[test]
    fn total_rlp_under_8192_bytes() {
        let mempool = Mempool::new(2048);
        let state = FakeState::default();

        // Many distinct senders, each contributing one legacy tx with a
        // unique `value` so hashes differ and the mempool actually stores
        // all of them. 200 ~110-byte txs is comfortably past the 8 KiB cap,
        // so the packer must clip the output.
        for i in 0..200u16 {
            // Use a distinct address per sender (16-bit space).
            let mut bytes = [0u8; 20];
            bytes[18] = (i >> 8) as u8;
            bytes[19] = (i & 0xff) as u8;
            // Skip the zero-address.
            if bytes == [0u8; 20] {
                continue;
            }
            let sender = Address::from(bytes);
            fund(&state, sender, 0);
            insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, u64::from(i) + 1));
        }

        let builder = InclusionListBuilder::default();
        let il = builder.build(&mempool, 0, &state);

        let total_bytes: usize = il.iter().map(|tx| tx.encode_canonical_to_vec().len()).sum();
        assert!(
            total_bytes <= MAX_BYTES_PER_INCLUSION_LIST,
            "total RLP {} exceeded {}",
            total_bytes,
            MAX_BYTES_PER_INCLUSION_LIST
        );
        // Sanity: at ~110 bytes per tx, 8 KiB / 110 ≈ 74 txs fit. The
        // builder should have packed many txs, not just a handful.
        assert!(
            il.len() >= 50,
            "expected packer to take many txs near the byte limit, got {}",
            il.len()
        );
    }

    #[test]
    fn invalid_nonce_excluded() {
        let mempool = Mempool::new(64);
        let state = FakeState::default();
        let sender = addr(0x01);
        // Account is at nonce 5 in parent state but mempool tx claims nonce 0.
        state.set(sender, 5, U256::from(u128::MAX));

        let stale_hash = insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, 0));

        let builder = InclusionListBuilder::default();
        let il = builder.build(&mempool, 0, &state);

        let hashes: Vec<H256> = il.iter().map(|tx| tx.hash()).collect();
        assert!(
            !hashes.contains(&stale_hash),
            "tx with stale nonce must be excluded"
        );
    }

    #[test]
    fn insufficient_balance_excluded() {
        let mempool = Mempool::new(64);
        let state = FakeState::default();
        let sender = addr(0x01);
        // Sender has 0 balance — tx with non-zero gas cost can't pay.
        state.set(sender, 0, U256::zero());

        let broke_hash = insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, 0));

        let builder = InclusionListBuilder::default();
        let il = builder.build(&mempool, 0, &state);

        let hashes: Vec<H256> = il.iter().map(|tx| tx.hash()).collect();
        assert!(!hashes.contains(&broke_hash));
    }

    #[test]
    fn priority_fee_policy_orders_by_fee() {
        let mempool = Mempool::new(64);
        let state = FakeState::default();
        let sender_a = addr(0x01);
        let sender_b = addr(0x02);
        fund(&state, sender_a, 0);
        fund(&state, sender_b, 0);

        // sender_a: low tip; sender_b: high tip. With the priority-fee
        // policy, sender_b's tx should appear first.
        let low = insert_tx(&mempool, sender_a, eip1559_tx(0, 100, 1, 21_000, 0));
        let high = insert_tx(&mempool, sender_b, eip1559_tx(0, 100, 50, 21_000, 0));

        let builder = InclusionListBuilder::new(
            IlPolicy::PriorityFee,
            DEFAULT_PER_SENDER_CAP,
            MAX_BYTES_PER_INCLUSION_LIST,
        );
        let il = builder.build(&mempool, 0, &state);

        assert_eq!(il.len(), 2);
        assert_eq!(il[0].hash(), high, "highest tip first");
        assert_eq!(il[1].hash(), low);
    }

    #[test]
    fn random_policy_terminates() {
        let mempool = Mempool::new(64);
        let state = FakeState::default();
        // Vary tx `value` per sender so each tx hashes differently and the
        // mempool stores distinct entries; otherwise hash-collision would
        // collapse all inserts onto one slot.
        for i in 0..10u8 {
            let sender = addr(i.saturating_add(1));
            fund(&state, sender, 0);
            insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, u64::from(i + 1)));
        }

        let builder = InclusionListBuilder::new(
            IlPolicy::Random,
            DEFAULT_PER_SENDER_CAP,
            MAX_BYTES_PER_INCLUSION_LIST,
        );
        let il = builder.build(&mempool, 0, &state);

        assert_eq!(
            il.len(),
            10,
            "random policy must include all eligible txs that fit"
        );
        let total_bytes: usize = il.iter().map(|tx| tx.encode_canonical_to_vec().len()).sum();
        assert!(total_bytes <= MAX_BYTES_PER_INCLUSION_LIST);
    }
}
