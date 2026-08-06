//! EIP-7805 (FOCIL) inclusion-list builder. Reads from the local public
//! mempool and produces a `Vec<Transaction>` (≤ 8 KiB total RLP) that the
//! `engine_getInclusionListV1` handler returns to the consensus layer.
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
    Address, H256, U256,
    constants::EMPTY_KECCAK_HASH,
    types::{MempoolTransaction, Transaction, utxo_vault},
};
use rustc_hash::FxHashMap;

use crate::focil_eligibility::SenderCode;
use crate::mempool::Mempool;

/// Hard byte cap on the total RLP-encoded size of the returned inclusion list.
/// Re-exported so builder callers do not depend on a second definition drifting
/// from the one the decoder enforces.
pub use ethrex_common::types::MAX_BYTES_PER_INCLUSION_LIST;

/// Default per-sender cap for the IL builder. Matches the `--il-per-sender-cap`
/// CLI default.
pub const DEFAULT_PER_SENDER_CAP: usize = 2;

/// Account snapshot used to validate IL candidates against parent-state.
/// `None` from [`IlStateProvider::get_account`] means the account is empty
/// (`nonce = 0`, `balance = 0`, empty code).
#[derive(Clone, Copy, Debug)]
pub struct AccountStateView {
    pub nonce: u64,
    pub balance: U256,
    pub code_hash: H256,
}

impl Default for AccountStateView {
    /// An absent account: nonce 0, balance 0, and — load-bearing — the
    /// *empty-code* hash, not `H256::zero()`. A derived `Default` would
    /// yield `H256::zero()`, which is not `EMPTY_KECCAK_HASH`, so every
    /// absent account would misclassify as code-bearing.
    fn default() -> Self {
        Self {
            nonce: 0,
            balance: U256::zero(),
            code_hash: *EMPTY_KECCAK_HASH,
        }
    }
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

    /// Classify the code at `code_hash` per EIP-3607/EIP-7702. Callers MUST
    /// short-circuit `code_hash == EMPTY_KECCAK_HASH` to `SenderCode::Eoa`
    /// themselves before calling this; an implementation may assume
    /// `code_hash` is always non-empty here. Implementations may consult a
    /// code-length index rather than loading the code body, since only a
    /// body of exactly `EIP7702_DELEGATED_CODE_LEN` bytes can be a
    /// delegation indicator.
    fn classify_code(&self, code_hash: H256) -> Result<SenderCode, IlStateProviderError>;
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

/// Whether a candidate's nonce lives in the sender's linear account-nonce
/// domain. False only for [EIP-8250] keyed frame transactions, whose `nonce_seq`
/// is one sequence per `(sender, nonce_key)` in the NONCE_MANAGER predeploy.
fn is_linear_nonce_domain(tx: &Transaction) -> bool {
    !matches!(tx, Transaction::FrameTransaction(frame_tx) if frame_tx.is_keyed())
}

/// Shape exclusions that apply to every candidate regardless of nonce domain:
/// blob carriers (an [EIP-7805] spec rule) and L2 privileged transactions
/// (FOCIL is L1-only).
fn is_il_eligible_shape(tx: &Transaction) -> bool {
    !matches!(
        tx,
        Transaction::EIP4844Transaction(_) | Transaction::PrivilegedL2Transaction(_)
    )
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
    ///
    /// Candidates are split by nonce domain first. Only the linear domain —
    /// non-frame transactions and key-0 frame transactions, whose `nonce_seq`
    /// *is* the account's linear nonce — is walked against the sender's account
    /// nonce. [EIP-8250] keyed frame transactions carry `nonce_seq` in the
    /// NONCE_MANAGER domain, one sequence per `(sender, nonce_key)`, so the
    /// account nonce is the wrong yardstick and a linear walk drops them all.
    /// This mirrors the mempool's `is_keyed_frame_tx` handling.
    fn filter_and_cap(
        &self,
        txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>>,
        head_state: &dyn IlStateProvider,
    ) -> Vec<MempoolTransaction> {
        let mut survivors: Vec<MempoolTransaction> = Vec::new();

        for (sender, sender_txs) in txs_by_sender {
            // Look up the sender's account once per sender. A read failure
            // is treated as "skip this sender" — being conservative, since
            // we can't validate them.
            let acct = match head_state.get_account(sender) {
                Ok(Some(view)) => view,
                Ok(None) => AccountStateView::default(),
                Err(_) => continue,
            };

            // EIP-8312: every UTXO spend shares the vault sender, so per-sender
            // limits there are network-wide limits. Their conflict domain is the
            // input-index set, not the sender, and the 8 KiB list cap already
            // bounds how many reach the list.
            let is_vault_sender = sender == utxo_vault();

            let (mut linear, non_linear): (Vec<_>, Vec<_>) = sender_txs
                .into_iter()
                .partition(|mtx| is_linear_nonce_domain(mtx.transaction()) && !is_vault_sender);

            // Sort by ascending nonce. The mempool `Ord` ranks by tip, not
            // nonce, so do not rely on it for the per-sender cap.
            linear.sort_by_key(|mtx| mtx.nonce());

            // Track the expected next nonce as we walk; if there's a gap we
            // stop accepting from this sender (the gapped tx can't be
            // included until the prior nonce lands).
            let mut expected_nonce = acct.nonce;
            let mut running_balance = acct.balance;
            let mut taken: usize = 0;

            for mtx in linear {
                if taken >= self.per_sender_cap {
                    break;
                }

                let tx: &Transaction = mtx.transaction();

                if !is_il_eligible_shape(tx) {
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

                // Frame transactions are not balance-gated: the payer is a
                // paymaster resolved during the validation prefix, so the
                // sender's balance says nothing about affordability.
                if !matches!(tx, Transaction::FrameTransaction(_)) {
                    let cost = match tx.cost_without_base_fee() {
                        Some(c) => c,
                        None => continue, // malformed fee data — skip
                    };
                    if cost > running_balance {
                        break; // sender can't afford this tx, can't afford follow-ups either
                    }
                    running_balance = running_balance.saturating_sub(cost);
                }

                // Account moves forward as if this tx were included next.
                expected_nonce = expected_nonce.saturating_add(1);
                taken = taken.saturating_add(1);
                survivors.push(mtx);
            }

            for mtx in non_linear {
                // The per-sender cap still bounds one sender's share of the
                // list, but a vault-sender spend belongs to whoever built it,
                // so it is exempt.
                if !is_vault_sender && taken >= self.per_sender_cap {
                    break;
                }
                if !is_il_eligible_shape(mtx.transaction()) {
                    continue;
                }
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
