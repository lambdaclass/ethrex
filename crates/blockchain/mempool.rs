use std::{
    cmp::Reverse,
    collections::{BTreeMap, VecDeque, hash_map::Entry},
    sync::RwLock,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::MempoolError;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        BlobTuple, BlobsBundle, BlockHeader, ChainConfig,
        FRAME_TX_MAX_PENDING_NONCANONICAL_PAYMASTER, Fork, MempoolTransaction, Transaction, TxType,
        kzg_commitment_to_versioned_hash,
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::error::StoreError;
use ethrex_vm::{intrinsic_gas_dimensions, intrinsic_gas_floor};
use tracing::warn;

/// Maximum number of alternate announcers tracked per hash. Bounds the memory
/// used by the alternates map and prevents pathological peers from filling it.
///
/// TODO(#6849): expose this through `BlockchainOptions` / CLI like the
/// other mempool ceilings (`max_mempool_size`, RBF price-bumps). 8 is
/// conservative; high-fan-in benchmarks and Hive adversarial-mempool scenarios
/// might want to raise it. FIFO eviction keeps the cap safe regardless.
pub const MAX_ALTERNATES_PER_HASH: usize = 8;

/// Maximum number of blob (EIP-4844) transactions retained in the mempool,
/// independent of `max_mempool_size`. Blob txs live in a dedicated sub-pool so a
/// flood of regular transactions cannot evict them, and the sub-pool itself is
/// evicted by value/nonce (see `remove_worst_blob_transaction`), never FIFO, so
/// the node keeps the (scarce, high-value, includable) blob txs it needs to build
/// full blocks.
///
/// Sized to comfortably hold several blocks' worth of includable blobs (Amsterdam
/// allows up to 21 blobs/block) while bounding worst-case memory: blobs are held
/// in RAM, so the bound is this count times the per-tx limit (`MAX_BLOB_TX_SIZE`,
/// ~1 MiB) ⇒ ~0.5 GiB worst case.
///
/// TODO(#6849): expose through CLI and prefer a byte-based cap (like geth's
/// blobpool `datacap`) so memory is bounded regardless of blobs-per-tx.
pub const MAX_BLOB_MEMPOOL_SIZE: usize = 512;

/// Keccak-256 hash of the canonical paymaster bytecode (EIP-8141).
///
/// OQ1 (canonical paymaster bytecode): UNRESOLVED. The draft EIP does not pin
/// the canonical paymaster's bytecode, and no reference implementation
/// (execution-specs, execution-spec-tests, geth, goevmlab, hive) ships one.
/// Until it is pinned, this is a sentinel (`H256::zero()`) that no real account
/// code can hash to, so [`is_canonical_paymaster`] returns `false` for every
/// paymaster. That is the conservative interim: ALL paymasters are treated as
/// non-canonical, which only ever over-rejects (de-facto limit of one pending
/// sponsored frame tx per paymaster), never under-rejects. When the canonical
/// hash is pinned, replace this sentinel and the exact-match body below flips on
/// with no other change.
pub const FRAME_CANONICAL_PAYMASTER_CODE_HASH: H256 = H256::zero();

/// Whether `code` is the canonical EIP-8141 paymaster bytecode.
///
/// OQ1 interim: returns `false` for all paymasters because
/// [`FRAME_CANONICAL_PAYMASTER_CODE_HASH`] is an unresolved sentinel that no
/// real bytecode hashes to. The exact-keccak-match body is kept so this flips on
/// for free once the canonical hash is pinned upstream.
pub fn is_canonical_paymaster(code: &[u8]) -> bool {
    keccak(code) == FRAME_CANONICAL_PAYMASTER_CODE_HASH
}

/// A paymaster reservation recorded for a pending frame transaction (EIP-8141).
///
/// Computed during admission (`Blockchain::validate_transaction`) and threaded
/// into the locked insert in [`Mempool::add_transaction`], so a frame tx that
/// fails a later admission check never leaks a reservation. Decremented from all
/// reservation maps in the single removal path
/// ([`MempoolInner::remove_transaction_with_lock`]).
#[derive(Debug, Clone)]
pub struct FramePaymasterReservation {
    /// The paymaster (payer) that covers this transaction's max cost. For a
    /// self-funded sender this is the sender itself (OQ2).
    pub paymaster: Address,
    /// The max cost (TXPARAM 0x06) reserved against the paymaster's balance.
    pub reserved_cost: U256,
    /// Whether the paymaster's code matched the canonical paymaster hash. Always
    /// `false` today (OQ1); non-canonical paymasters are subject to the
    /// one-pending-tx limit.
    pub is_canonical: bool,
    /// The paymaster's head balance captured at admission time, so the locked
    /// re-check in [`Mempool::add_transaction`] can re-validate availability
    /// against the live reservation map without an async storage read while
    /// holding the write lock.
    pub paymaster_balance: U256,
}

/// A pending frame transaction's revalidation descriptor: `(hash, sender,
/// paymaster)`. Returned by [`Mempool::pending_frame_txs`] for the post-block
/// revalidation pass.
pub type PendingFrameTx = (H256, Address, Address);

/// An alternate announcer for a known-in-flight transaction hash. Carries the
/// announcer's own announced type and size so the eventual retry can validate
/// the response against the alternate's metadata (which may differ from the
/// primary announcer's, e.g. when one peer advertises a bare blob tx while
/// another advertises the full sidecar).
#[derive(Debug, Clone, Copy)]
pub struct Alternate {
    pub peer_id: H256,
    pub tx_type: u8,
    pub tx_size: usize,
}

#[derive(Debug, Default)]
struct MempoolInner {
    broadcast_pool: FxHashSet<H256>,
    transaction_pool: FxHashMap<H256, MempoolTransaction>,
    blobs_bundle_pool: FxHashMap<H256, BlobsBundle>,
    /// Transaction hashes that have been requested via GetPooledTransactions
    /// but whose responses haven't arrived yet. Used to avoid sending duplicate
    /// requests when multiple peers announce the same transaction.
    in_flight_txs: FxHashSet<H256>,
    /// For each announced hash, the queue of *alternate* announcers that also
    /// advertised it while the hash was already in-flight from someone else.
    /// Each entry carries the announcer's own announced type and size so the
    /// retry can validate the response against the alternate's metadata (which
    /// may differ from the primary's). Used as a fallback list when an in-flight
    /// request fails or the responding peer disconnects. The `Instant` records
    /// the last time the entry was touched so a periodic pruner can drop stale
    /// entries.
    alternates: FxHashMap<H256, (VecDeque<Alternate>, Instant)>,
    /// Maps blob versioned hashes to transaction hashes that include them and a position inside
    /// blob bundle where blob and its adjacent data is available.
    blobs_bundle_by_versioned_hash: FxHashMap<H256, FxHashMap<H256, usize>>,
    txs_by_sender_nonce: BTreeMap<(H160, u64), H256>,
    txs_order: VecDeque<H256>,
    max_mempool_size: usize,
    max_blob_mempool_size: usize,
    // Max number of transactions to let the mempool order queue grow before pruning it
    mempool_prune_threshold: usize,
    /// Tracks the currently pending frame transaction hash per sender (EIP-8141).
    /// At most one pending frame tx per sender is allowed to avoid ordering
    /// ambiguity and DoS. Populated on insert; cleared on removal.
    /// Must be kept consistent with `remove_transaction_with_lock`.
    pending_frame_tx_by_sender: FxHashMap<Address, (H256, u64)>,
    /// Sum of reserved max-cost (TXPARAM 0x06) per paymaster across all pending
    /// frame txs that paymaster sponsors (EIP-8141). Admission checks a
    /// paymaster's balance against this running total so concurrently-pending
    /// sponsored txs cannot collectively overdraw it. Incremented in the locked
    /// section of `add_transaction`; decremented in `remove_transaction_with_lock`.
    reserved_pending_cost: FxHashMap<Address, U256>,
    /// Count of pending frame txs sponsored by each NON-canonical paymaster
    /// (EIP-8141). Bounded by `FRAME_TX_MAX_PENDING_NONCANONICAL_PAYMASTER`.
    /// Incremented in the locked section of `add_transaction`; decremented in
    /// `remove_transaction_with_lock`.
    noncanonical_paymaster_pending: FxHashMap<Address, u8>,
    /// Per-frame-tx reservation record, keyed by tx hash. Carries the paymaster,
    /// reserved cost, canonical flag, and touched sender slots so the single
    /// removal path can decrement the other maps and the post-block revalidation
    /// can bound its affected set. Populated on insert; removed on removal.
    frame_tx_paymaster: FxHashMap<H256, FramePaymasterReservation>,
}

impl MempoolInner {
    fn new(max_mempool_size: usize) -> Self {
        MempoolInner {
            txs_order: VecDeque::with_capacity(max_mempool_size * 2),
            transaction_pool: FxHashMap::with_capacity_and_hasher(
                max_mempool_size,
                Default::default(),
            ),
            max_mempool_size,
            max_blob_mempool_size: MAX_BLOB_MEMPOOL_SIZE,
            mempool_prune_threshold: max_mempool_size + max_mempool_size / 2,
            ..Default::default()
        }
    }

    /// Remove a transaction from the pool with the transaction pool lock already taken
    fn remove_transaction_with_lock(&mut self, hash: &H256) -> Result<(), StoreError> {
        let Some(tx) = self.transaction_pool.remove(hash) else {
            return Ok(());
        };
        if matches!(tx.tx_type(), TxType::EIP4844) {
            self.remove_blob_bundle(hash);
        }

        self.txs_by_sender_nonce.remove(&(tx.sender(), tx.nonce()));
        self.broadcast_pool.remove(hash);

        // Clear ALL frame-tx reservation state in this single removal path
        // (eviction / inclusion / reorg all funnel through here), so no outer
        // call site must decrement anything (which would risk double-decrement).
        if matches!(tx.tx_type(), TxType::Frame) {
            let sender = tx.sender();
            if self
                .pending_frame_tx_by_sender
                .get(&sender)
                .is_some_and(|(h, _)| h == hash)
            {
                self.pending_frame_tx_by_sender.remove(&sender);
            }

            // Decrement the paymaster reservation maps using the recorded
            // reservation for this tx (if any).
            if let Some(reservation) = self.frame_tx_paymaster.remove(hash) {
                let paymaster = reservation.paymaster;
                if let Entry::Occupied(mut entry) = self.reserved_pending_cost.entry(paymaster) {
                    let remaining = entry.get().saturating_sub(reservation.reserved_cost);
                    if remaining.is_zero() {
                        entry.remove();
                    } else {
                        *entry.get_mut() = remaining;
                    }
                }
                if !reservation.is_canonical
                    && let Entry::Occupied(mut entry) =
                        self.noncanonical_paymaster_pending.entry(paymaster)
                {
                    let remaining = entry.get().saturating_sub(1);
                    if remaining == 0 {
                        entry.remove();
                    } else {
                        *entry.get_mut() = remaining;
                    }
                }
            }
        }

        Ok(())
    }

    /// Remove a blobs bundle from the pool
    pub fn remove_blob_bundle(&mut self, hash: &H256) {
        let Some(h) = self.blobs_bundle_pool.remove(hash) else {
            return;
        };

        for commitment in &h.commitments {
            let versioned_hash = kzg_commitment_to_versioned_hash(commitment);
            if let Entry::Occupied(mut entry) =
                self.blobs_bundle_by_versioned_hash.entry(versioned_hash)
            {
                let txn_to_bundle = entry.get_mut();
                txn_to_bundle.remove(hash);
                if txn_to_bundle.is_empty() {
                    entry.remove();
                }
            }
        }
    }

    /// Number of blob (EIP-4844) txs currently in the pool. Each blob tx has
    /// exactly one bundle entry, so the bundle pool size is the blob tx count.
    fn blob_tx_count(&self) -> usize {
        self.blobs_bundle_pool.len()
    }

    /// Number of non-blob txs currently in the pool.
    fn regular_tx_count(&self) -> usize {
        // `saturating_sub`: a blob bundle is inserted before its tx (see
        // `add_blob_transaction_to_pool`), so in that window the bundle count can
        // briefly exceed the tx entries. Treat the undercount as 0 regular txs
        // rather than underflowing (which would wrongly trigger eviction).
        self.transaction_pool
            .len()
            .saturating_sub(self.blob_tx_count())
    }

    /// Evict the oldest regular (non-blob) transactions until the regular pool is
    /// back under its cap. Only drains `txs_order`, so blob txs are never evicted
    /// by regular-tx pressure.
    fn remove_oldest_regular_transaction(&mut self) -> Result<(), StoreError> {
        while self.regular_tx_count() >= self.max_mempool_size {
            if let Some(oldest_hash) = self.txs_order.pop_front() {
                self.remove_transaction_with_lock(&oldest_hash)?;
            } else {
                warn!(
                    "Regular mempool is full but there are no transactions to remove, this should not happen and will make the mempool grow indefinitely"
                );
                break;
            }
        }

        Ok(())
    }

    /// Evict blob transactions until the blob sub-pool is back under its cap.
    ///
    /// Unlike a FIFO, this drops the *least includable* blob tx first. "Least
    /// includable" is approximated by how deep a tx sits in its own sender's
    /// queue: the nonce offset from that sender's lowest pooled blob nonce. A
    /// large offset means the tx sits behind earlier same-sender blobs and
    /// can't be included until those clear, so it is the safest to drop. Ties
    /// are broken by lowest blob fee.
    ///
    /// The offset is measured per-sender on purpose. A raw cross-sender nonce
    /// comparison would penalize long-lived high-throughput senders (e.g. a
    /// rollup sequencer) whose on-wire nonces are large but whose txs are
    /// perfectly includable. Measuring within a sender preserves the
    /// low-offset, ready-to-include blobs the block builder actually needs
    /// instead of FIFO-evicting them just because they arrived early.
    fn remove_worst_blob_transaction(&mut self) -> Result<(), StoreError> {
        while self.blob_tx_count() > self.max_blob_mempool_size {
            // `blobs_bundle_pool` is keyed by blob-tx hash, so its keys are
            // exactly the blob txs currently held. First pass: lowest pooled
            // blob nonce per sender, the per-sender baseline for the offset.
            let mut min_nonce_by_sender: FxHashMap<Address, u64> = FxHashMap::default();
            for tx in self
                .blobs_bundle_pool
                .keys()
                .filter_map(|hash| self.transaction_pool.get(hash))
            {
                min_nonce_by_sender
                    .entry(tx.sender())
                    .and_modify(|n| *n = (*n).min(tx.nonce()))
                    .or_insert(tx.nonce());
            }
            // O(N) scan over the blob sub-pool (N <= max_blob_mempool_size, 512
            // today). Fine at this cap; revisit (e.g. a priority index) before
            // exposing a much larger cap via CLI.
            let worst = self
                .blobs_bundle_pool
                .keys()
                .filter_map(|hash| self.transaction_pool.get(hash).map(|tx| (*hash, tx)))
                .max_by_key(|(_, tx)| {
                    let baseline = min_nonce_by_sender.get(&tx.sender()).copied().unwrap_or(0);
                    let offset = tx.nonce().saturating_sub(baseline);
                    (offset, Reverse(tx.max_fee_per_blob_gas()))
                })
                .map(|(hash, _)| hash);
            match worst {
                Some(hash) => self.remove_transaction_with_lock(&hash)?,
                None => {
                    warn!(
                        "Blob mempool is over cap but no evictable blob transaction is present, this should not happen"
                    );
                    break;
                }
            }
        }

        Ok(())
    }

    /// Check whether a new frame transaction from `sender` at `nonce` can be
    /// admitted under the one-pending-frame-tx-per-sender policy.
    ///
    /// - If no frame tx from `sender` is pending: permit (return `Ok(None)`).
    /// - If a frame tx with the **same nonce** is pending: defer to fee-bump
    ///   replacement (`find_tx_to_replace` handles price checks); return the
    ///   existing hash so the caller can remove it first.
    /// - If a frame tx with a **different nonce** is pending: reject with
    ///   `FrameTxSenderAlreadyPending`.
    ///
    /// Must be called under the mempool write lock so the check and the
    /// subsequent insert are atomic (no TOCTOU race).
    fn check_frame_tx_sender_pending(
        &self,
        sender: Address,
        nonce: u64,
        incoming_hash: H256,
    ) -> Result<Option<H256>, MempoolError> {
        let Some(&(existing_hash, existing_nonce)) = self.pending_frame_tx_by_sender.get(&sender)
        else {
            return Ok(None);
        };
        if existing_hash == incoming_hash {
            // Same tx already in pool (re-announced); not a conflict.
            return Ok(None);
        }
        if existing_nonce == nonce {
            // Same nonce: the incoming tx is a fee-bump replacement; let
            // `find_tx_to_replace` validate the price bump.
            Ok(Some(existing_hash))
        } else {
            // Different nonce: a live frame tx from this sender is already
            // pending at a different nonce, reject.
            Err(MempoolError::FrameTxSenderAlreadyPending)
        }
    }
}

#[derive(Debug, Default)]
pub struct Mempool {
    inner: RwLock<MempoolInner>,
    /// Signaled on transaction and blobs bundle insertions so payload
    /// builders can await new work instead of busy-looping.
    tx_added: tokio::sync::Notify,
    /// Monotonic counter incremented on every transaction insertion. Used by
    /// the payload builder to detect whether new txs landed since it last
    /// snapshotted the mempool, so it can decide whether a stale build is safe
    /// to return.
    tx_seq: AtomicU64,
}

impl Mempool {
    pub fn new(max_mempool_size: usize) -> Self {
        Mempool {
            inner: RwLock::new(MempoolInner::new(max_mempool_size)),
            tx_added: tokio::sync::Notify::new(),
            tx_seq: AtomicU64::new(0),
        }
    }

    /// Override the blob sub-pool capacity (defaults to [`MAX_BLOB_MEMPOOL_SIZE`]).
    /// Builder-style; intended for configuration and tests.
    pub fn with_max_blob_mempool_size(self, max_blob_mempool_size: usize) -> Self {
        if let Ok(mut inner) = self.inner.write() {
            inner.max_blob_mempool_size = max_blob_mempool_size;
        }
        self
    }

    pub(crate) fn tx_added(&self) -> &tokio::sync::Notify {
        &self.tx_added
    }

    pub(crate) fn tx_seq(&self) -> u64 {
        self.tx_seq.load(Ordering::Acquire)
    }

    fn write(&self) -> Result<std::sync::RwLockWriteGuard<'_, MempoolInner>, StoreError> {
        self.inner
            .write()
            .map_err(|error| StoreError::MempoolWriteLock(error.to_string()))
    }

    fn read(&self) -> Result<std::sync::RwLockReadGuard<'_, MempoolInner>, StoreError> {
        self.inner
            .read()
            .map_err(|error| StoreError::MempoolReadLock(error.to_string()))
    }

    /// Add transaction to the pool without doing validity checks, except for the
    /// one-pending-frame-tx-per-sender policy which must run under this lock to
    /// avoid a TOCTOU race (EIP-8141, review fix 1.6).
    pub fn add_transaction(
        &self,
        hash: H256,
        sender: Address,
        transaction: MempoolTransaction,
        frame_reservation: Option<FramePaymasterReservation>,
    ) -> Result<(), MempoolError> {
        let mut inner = self.write()?;
        let is_frame = matches!(transaction.tx_type(), TxType::Frame);
        let is_blob = matches!(transaction.tx_type(), TxType::EIP4844);

        // One-pending-frame-tx-per-sender gate (EIP-8141 §Mempool, review fix 1.6).
        // Must run under the write lock so the check and insert are atomic.
        if is_frame {
            let nonce = transaction.nonce();
            // Same-nonce replacement: capture the old tx's hash WITHOUT removing
            // it yet. Removal must be atomic with the re-check below so a
            // rejected fee-bump never leaves the sender with neither the old nor
            // the new tx. Price validation already ran in validate_transaction.
            let existing_frame_hash = inner.check_frame_tx_sender_pending(sender, nonce, hash)?;

            // Paymaster availability + non-canonical-limit re-check under the
            // write lock. The check in `validate_transaction` is an unlocked
            // pre-filter; this locked re-check against the live reservation maps
            // is what holds the limit and the availability invariant under
            // concurrent admissions for different senders sharing one paymaster
            // (the same TOCTOU class review fix 1.6 closed for the per-sender
            // gate). Runs before any insertion or removal so a rejection has no
            // side effect.
            if let Some(reservation) = &frame_reservation {
                // Account for the impending removal of the old same-nonce tx: if
                // it shares the new tx's paymaster, its reservation will be
                // released the moment we remove it, so it must not block a
                // same-paymaster fee-bump. Subtract the old tx's reserved cost
                // (availability) and one pending slot (non-canonical limit).
                let old_reservation = existing_frame_hash
                    .as_ref()
                    .and_then(|old_hash| inner.frame_tx_paymaster.get(old_hash))
                    .filter(|old| old.paymaster == reservation.paymaster);

                if !reservation.is_canonical {
                    let mut pending = inner
                        .noncanonical_paymaster_pending
                        .get(&reservation.paymaster)
                        .copied()
                        .unwrap_or(0);
                    if old_reservation.is_some_and(|old| !old.is_canonical) {
                        pending = pending.saturating_sub(1);
                    }
                    if pending >= FRAME_TX_MAX_PENDING_NONCANONICAL_PAYMASTER {
                        return Err(MempoolError::FrameTxNonCanonicalPaymasterLimit);
                    }
                }
                let mut reserved = inner
                    .reserved_pending_cost
                    .get(&reservation.paymaster)
                    .copied()
                    .unwrap_or_default();
                if let Some(old) = old_reservation {
                    reserved = reserved.saturating_sub(old.reserved_cost);
                }
                if reservation.paymaster_balance.saturating_sub(reserved)
                    < reservation.reserved_cost
                {
                    return Err(MempoolError::FrameTxPaymasterUnderfunded);
                }
            }

            // Re-check passed: now remove whatever tx currently occupies this
            // sender's nonce slot (releasing any reservation) so the new frame tx
            // can take it. Done only after the re-check so a rejection leaves the
            // original pending tx intact. The predecessor may be a NON-frame tx:
            // `find_tx_to_replace` (which validated the fee bump in
            // `validate_transaction`) matches any tx type, but
            // `check_frame_tx_sender_pending`/`existing_frame_hash` only sees frame
            // predecessors. Removing by the (sender, nonce) slot — instead of just
            // `existing_frame_hash` — covers both, so a same-nonce legacy/EIP-1559
            // tx is properly replaced rather than orphaned in the pool (its index
            // entry is overwritten below while the tx itself leaks). When the
            // predecessor is the same-nonce frame tx, the slot already points to it,
            // so this is equivalent to the previous `existing_frame_hash` removal.
            if let Some(&old_hash) = inner.txs_by_sender_nonce.get(&(sender, nonce)) {
                inner.remove_transaction_with_lock(&old_hash)?;
            }
        }

        // Prune the regular order queue if it has grown too much
        if inner.txs_order.len() > inner.mempool_prune_threshold {
            // NOTE: we do this to avoid borrow checker errors
            let txpool = core::mem::take(&mut inner.transaction_pool);
            inner.txs_order.retain(|tx| txpool.contains_key(tx));
            inner.transaction_pool = txpool;
        }
        // Blob txs are evicted against their own cap so a flood of regular txs
        // can't push them out (and vice versa). Blob eviction is value/nonce
        // ordered (see `remove_worst_blob_transaction`), not FIFO, so it never
        // drops the next-includable blob tx; regular txs stay FIFO.
        if is_blob {
            // The bundle is inserted before the tx (see add_blob_transaction_to_pool),
            // so the incoming blob is already counted by `blob_tx_count`.
            if inner.blob_tx_count() > inner.max_blob_mempool_size {
                inner.remove_worst_blob_transaction()?;
            }
        } else {
            // The regular tx isn't in the pool yet (inserted below), so
            // `regular_tx_count()` is the count *before* this tx: `>= max` means
            // we're already at cap and must evict to make room. (Mirror of the
            // blob branch, which uses `>` because the bundle is inserted first
            // and is therefore already counted by `blob_tx_count`.)
            if inner.regular_tx_count() >= inner.max_mempool_size {
                inner.remove_oldest_regular_transaction()?;
            }
            inner.txs_order.push_back(hash);
        }
        let tx_nonce = transaction.nonce();
        inner.txs_by_sender_nonce.insert((sender, tx_nonce), hash);
        inner.transaction_pool.insert(hash, transaction);
        inner.broadcast_pool.insert(hash);
        inner.alternates.remove(&hash);

        // Track per-sender pending frame tx for EIP-8141 admission gating.
        // Storing the nonce alongside the hash keeps the conflict check O(1).
        if is_frame {
            inner
                .pending_frame_tx_by_sender
                .insert(sender, (hash, tx_nonce));

            // Increment the paymaster reservation maps for this frame tx. The
            // reservation was computed during admission (validate_transaction)
            // and is applied here, under the write lock, only once the tx has
            // cleared every admission check and is actually being inserted (so a
            // tx rejected after the availability check never leaks a
            // reservation). Decremented atomically in the single removal path.
            if let Some(reservation) = frame_reservation {
                let paymaster = reservation.paymaster;
                *inner
                    .reserved_pending_cost
                    .entry(paymaster)
                    .or_insert(U256::zero()) += reservation.reserved_cost;
                if !reservation.is_canonical {
                    let count = inner
                        .noncanonical_paymaster_pending
                        .entry(paymaster)
                        .or_insert(0);
                    *count = count.saturating_add(1);
                }
                inner.frame_tx_paymaster.insert(hash, reservation);
            }
        }

        // Drop the write lock before notifying to avoid holding it while waking waiters
        drop(inner);
        // Bump `tx_seq` *after* releasing the write lock. The payload builder
        // snapshots `tx_seq` before reading the mempool; with this ordering,
        // any reader that observes the new tx is guaranteed to also observe a
        // bumped seq on its next load, so the builder never misses a tx it
        // already incorporated as "new since last build".
        self.tx_seq.fetch_add(1, Ordering::Release);
        self.tx_added.notify_waiters();

        Ok(())
    }

    pub fn get_txs_for_broadcast(&self) -> Result<Vec<MempoolTransaction>, StoreError> {
        let inner = self.read()?;
        let txs = inner
            .transaction_pool
            .iter()
            .filter_map(|(hash, tx)| {
                if !inner.broadcast_pool.contains(hash) {
                    None
                } else {
                    Some(tx.clone())
                }
            })
            .collect::<Vec<_>>();
        Ok(txs)
    }

    pub fn remove_broadcasted_txs(&self, hashes: &[H256]) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        for hash in hashes {
            inner.broadcast_pool.remove(hash);
        }
        Ok(())
    }

    /// `(hash, sender, nonce)` for every blob tx in the pool. `blobs_bundle_pool`
    /// is keyed by blob-tx hash, so its keys are exactly the held blob txs.
    pub fn blob_txs(&self) -> Result<Vec<(H256, Address, u64)>, StoreError> {
        let inner = self.read()?;
        Ok(inner
            .blobs_bundle_pool
            .keys()
            .filter_map(|hash| {
                inner
                    .transaction_pool
                    .get(hash)
                    .map(|tx| (*hash, tx.sender(), tx.nonce()))
            })
            .collect())
    }

    /// Add a blobs bundle to the pool by its blob transaction hash
    pub fn add_blobs_bundle(
        &self,
        tx_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<(), StoreError> {
        let mut mempool = self.write()?;
        for (i, c) in blobs_bundle.commitments.iter().enumerate() {
            let versioned_hash = kzg_commitment_to_versioned_hash(c);
            mempool
                .blobs_bundle_by_versioned_hash
                .entry(versioned_hash)
                .or_default()
                .insert(tx_hash, i);
        }
        mempool.blobs_bundle_pool.insert(tx_hash, blobs_bundle);
        Ok(())
    }

    /// Get a blobs bundle to the pool given its blob transaction hash
    pub fn get_blobs_bundle(&self, tx_hash: H256) -> Result<Option<BlobsBundle>, StoreError> {
        Ok(self.read()?.blobs_bundle_pool.get(&tx_hash).cloned())
    }

    /// Remove a transaction from the pool
    pub fn remove_transaction(&self, hash: &H256) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        inner.remove_transaction_with_lock(hash)?;
        Ok(())
    }

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    pub fn filter_transactions(
        &self,
        filter: &PendingTxFilter,
    ) -> Result<FxHashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let filter_tx = |tx: &Transaction| -> bool {
            // Filter by tx type
            let is_blob_tx = matches!(tx, Transaction::EIP4844Transaction(_));
            if filter.only_plain_txs && is_blob_tx || filter.only_blob_txs && !is_blob_tx {
                return false;
            }

            // Filter by tip & base_fee
            if let Some(min_tip) = filter.min_tip.map(U256::from) {
                if tx
                    .effective_gas_tip(filter.base_fee)
                    .is_none_or(|tip| tip < min_tip)
                {
                    return false;
                }
            // This is a temporary fix to avoid invalid transactions to be included.
            // This should be removed once https://github.com/lambdaclass/ethrex/issues/680
            // is addressed.
            } else if tx.effective_gas_tip(filter.base_fee).is_none() {
                return false;
            }

            // Filter by blob gas fee
            if is_blob_tx
                && let Some(blob_fee) = filter.blob_fee
                && tx
                    .max_fee_per_blob_gas()
                    .is_none_or(|fee| fee < blob_fee.into())
            {
                return false;
            }
            true
        };
        self.filter_transactions_with_filter_fn(&filter_tx)
    }

    /// Gets all the transactions in the mempool
    pub fn get_all_txs_by_sender(
        &self,
    ) -> Result<FxHashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let mut txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>> =
            FxHashMap::with_capacity_and_hasher(128, Default::default());
        let tx_pool = &self.read()?.transaction_pool;

        for (_, tx) in tx_pool.iter() {
            txs_by_sender
                .entry(tx.sender())
                .or_insert_with(|| Vec::with_capacity(128))
                .push(tx.clone())
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort());
        Ok(txs_by_sender)
    }

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    pub fn filter_transactions_with_filter_fn(
        &self,
        filter: &dyn Fn(&Transaction) -> bool,
    ) -> Result<FxHashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let mut txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>> =
            FxHashMap::with_capacity_and_hasher(128, Default::default());
        let tx_pool = &self.read()?.transaction_pool;

        for (_, tx) in tx_pool.iter() {
            if filter(tx) {
                txs_by_sender
                    .entry(tx.sender())
                    .or_insert_with(|| Vec::with_capacity(128))
                    .push(tx.clone())
            }
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort());
        Ok(txs_by_sender)
    }

    /// Filters hashes to those not already in the mempool or in-flight, and
    /// atomically marks the returned hashes as in-flight under a single write
    /// lock so that concurrent peer handlers cannot request the same hashes.
    ///
    /// For hashes that get filtered out *because they're already in-flight
    /// from another peer*, records `announcer` as a fallback so the request
    /// can be retried against this peer if the original responder fails. New
    /// hashes that the caller is about to request do not need an alternates
    /// entry yet: the caller is the primary, and one will be created only if
    /// some other peer later announces the same hash while it's in-flight.
    /// Reserve hashes the caller wants to request, returning only those that are
    /// neither already in-flight nor already in the pool. Any hash filtered out
    /// because it's in-flight from another peer is registered with the caller's
    /// own (type, size) metadata as an alternate, so a later retry can validate
    /// the response against this announcer's announcement.
    ///
    /// `hashes`, `types`, and `sizes` must be the same length (one entry per
    /// announced hash).
    pub fn reserve_unknown_hashes(
        &self,
        hashes: &[H256],
        types: &[u8],
        sizes: &[usize],
        announcer: H256,
    ) -> Result<Vec<H256>, StoreError> {
        debug_assert_eq!(hashes.len(), types.len());
        debug_assert_eq!(hashes.len(), sizes.len());

        let mut inner = self.write()?;

        let unknown: Vec<H256> = hashes
            .iter()
            .filter(|hash| {
                !inner.in_flight_txs.contains(hash) && !inner.transaction_pool.contains_key(hash)
            })
            .copied()
            .collect();

        inner.in_flight_txs.extend(unknown.iter().copied());

        // Register alternates only for hashes the caller will *not* request
        // (i.e. those already in-flight from someone else). Skip pool hits
        // and skip hashes we just reserved for this peer.
        if hashes.len() > unknown.len() {
            let unknown_set: FxHashSet<H256> = unknown.iter().copied().collect();
            let now = Instant::now();
            for (i, hash) in hashes.iter().enumerate() {
                if unknown_set.contains(hash) || inner.transaction_pool.contains_key(hash) {
                    continue;
                }
                let alt = Alternate {
                    peer_id: announcer,
                    tx_type: types[i],
                    tx_size: sizes[i],
                };
                let entry = inner
                    .alternates
                    .entry(*hash)
                    .or_insert_with(|| (VecDeque::new(), now));
                entry.1 = now;
                if !entry.0.iter().any(|a| a.peer_id == announcer) {
                    if entry.0.len() >= MAX_ALTERNATES_PER_HASH {
                        entry.0.pop_front();
                    }
                    entry.0.push_back(alt);
                }
            }
        }

        Ok(unknown)
    }

    /// Removes transaction hashes from the in-flight set, typically called
    /// when the GetPooledTransactions response arrives (or the connection drops).
    pub fn clear_in_flight_txs(&self, hashes: &[H256]) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        for hash in hashes {
            inner.in_flight_txs.remove(hash);
        }
        Ok(())
    }

    /// Pops the next alternate announcer for the given hash, if any. Returns
    /// `Ok(None)` when no alternates remain. The caller uses the popped
    /// `Alternate` to look up the peer connection and build a retry request
    /// against that peer's own announcement metadata.
    pub fn pop_alternate(&self, hash: H256) -> Result<Option<Alternate>, StoreError> {
        let mut inner = self.write()?;
        let Some(entry) = inner.alternates.get_mut(&hash) else {
            return Ok(None);
        };
        let popped = entry.0.pop_front();
        if entry.0.is_empty() {
            inner.alternates.remove(&hash);
        }
        Ok(popped)
    }

    /// Drop alternates entries that haven't been touched in the last `ttl`.
    /// Called periodically to bound the size of the alternates map when
    /// announced txs never make it into the pool.
    pub fn prune_alternates(&self, ttl: Duration) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        let now = Instant::now();
        inner
            .alternates
            .retain(|_, (_, last_seen)| now.saturating_duration_since(*last_seen) < ttl);
        Ok(())
    }

    pub fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let tx = self
            .read()?
            .transaction_pool
            .get(&transaction_hash)
            .map(|e| e.transaction().clone());

        Ok(tx)
    }

    pub fn get_nonce(&self, address: &Address) -> Result<Option<u64>, MempoolError> {
        Ok(self
            .read()?
            .txs_by_sender_nonce
            .range((*address, 0)..=(*address, u64::MAX))
            .last()
            .map(|((_address, nonce), _hash)| nonce + 1))
    }

    pub fn get_mempool_size(&self) -> Result<(u64, u64), MempoolError> {
        let txs_size = {
            let pool_lock = &self.read()?.transaction_pool;
            pool_lock.len()
        };
        let blobs_size = {
            let pool_lock = &self.read()?.blobs_bundle_pool;
            pool_lock.len()
        };

        Ok((txs_size as u64, blobs_size as u64))
    }

    /// Returns all transactions currently in the pool
    pub fn content(&self) -> Result<Vec<Transaction>, MempoolError> {
        let pooled_transactions = &self.read()?.transaction_pool;
        Ok(pooled_transactions
            .values()
            .map(MempoolTransaction::transaction)
            .cloned()
            .collect())
    }

    /// Returns all blobs bundles currently in the pool
    pub fn get_blobs_bundle_pool(&self) -> Result<Vec<BlobsBundle>, MempoolError> {
        let blobs_bundle_pool = &self.read()?.blobs_bundle_pool;
        Ok(blobs_bundle_pool.values().cloned().collect())
    }

    /// Returns blobs data (blob, commitment, proof) associated with the versioned hashes
    pub fn get_blobs_data_by_versioned_hashes(
        &self,
        versioned_hashes: &[H256],
    ) -> Result<Vec<Option<BlobTuple>>, MempoolError> {
        let mempool = self.read()?;
        let blobs_bundle_pool = &mempool.blobs_bundle_pool;
        let blobs_bundle_by_versioned_hash = &mempool.blobs_bundle_by_versioned_hash;
        let mut res = vec![None; versioned_hashes.len()];
        for (idx, vh) in versioned_hashes.iter().enumerate() {
            if let Some((found_hash, inner_pos)) = blobs_bundle_by_versioned_hash
                .get(vh)
                .and_then(|h| h.iter().next())
            {
                res[idx] = blobs_bundle_pool
                    .get(found_hash)
                    .and_then(|b| b.get_blob_tuple_by_index(*inner_pos))
            }
        }
        Ok(res)
    }

    /// Returns the status of the mempool, which is the number of transactions currently in
    /// the pool. Until we add "queue" transactions.
    pub fn status(&self) -> Result<u64, MempoolError> {
        let pool_lock = &self.read()?.transaction_pool;

        Ok(pool_lock.len() as u64)
    }

    pub fn contains_sender_nonce(
        &self,
        sender: Address,
        nonce: u64,
        received_hash: H256,
    ) -> Result<Option<MempoolTransaction>, MempoolError> {
        let Some(hash) = self
            .read()?
            .txs_by_sender_nonce
            .get(&(sender, nonce))
            .cloned()
        else {
            return Ok(None);
        };
        if hash == received_hash {
            return Ok(None);
        }

        let transaction_pool = &self.read()?.transaction_pool;
        let tx = transaction_pool.get(&hash).cloned();
        Ok(tx)
    }

    pub fn contains_tx(&self, tx_hash: H256) -> Result<bool, MempoolError> {
        let contains = self.read()?.transaction_pool.contains_key(&tx_hash);
        Ok(contains)
    }

    pub fn find_tx_to_replace(
        &self,
        sender: Address,
        nonce: u64,
        tx: &Transaction,
    ) -> Result<Option<H256>, MempoolError> {
        let Some(tx_in_pool) = self.contains_sender_nonce(sender, nonce, tx.hash(&NativeCrypto))?
        else {
            return Ok(None);
        };
        let is_a_replacement_tx = {
            // EIP-1559 values
            let old_tx_max_fee_per_gas = tx_in_pool.max_fee_per_gas().unwrap_or_default();
            let old_tx_max_priority_fee_per_gas = tx_in_pool.max_priority_fee().unwrap_or_default();
            let new_tx_max_fee_per_gas = tx.max_fee_per_gas().unwrap_or_default();
            let new_tx_max_priority_fee_per_gas = tx.max_priority_fee().unwrap_or_default();

            // Legacy tx values
            let old_tx_gas_price = tx_in_pool.gas_price();
            let new_tx_gas_price = tx.gas_price();

            // EIP-4844 values
            let old_tx_max_fee_per_blob = tx_in_pool.max_fee_per_blob_gas();
            let new_tx_max_fee_per_blob = tx.max_fee_per_blob_gas();

            let eip4844_higher_fees = if let (Some(old_blob_fee), Some(new_blob_fee)) =
                (old_tx_max_fee_per_blob, new_tx_max_fee_per_blob)
            {
                new_blob_fee > old_blob_fee
            } else {
                true // We are marking it as always true if the tx is not eip-4844
            };

            let eip1559_higher_fees = new_tx_max_fee_per_gas > old_tx_max_fee_per_gas
                && new_tx_max_priority_fee_per_gas > old_tx_max_priority_fee_per_gas;
            let legacy_higher_fees = new_tx_gas_price > old_tx_gas_price;

            eip4844_higher_fees && (eip1559_higher_fees || legacy_higher_fees)
        };

        if !is_a_replacement_tx {
            return Err(MempoolError::UnderpricedReplacement);
        }

        Ok(Some(tx_in_pool.hash(&NativeCrypto)))
    }

    /// Current reserved max-cost total for `paymaster` across pending frame txs
    /// (EIP-8141). Returns zero when the paymaster sponsors no pending frame tx.
    pub fn reserved_pending_cost(&self, paymaster: Address) -> Result<U256, StoreError> {
        Ok(self
            .read()?
            .reserved_pending_cost
            .get(&paymaster)
            .copied()
            .unwrap_or_else(U256::zero))
    }

    /// Number of pending frame txs sponsored by `paymaster` as a NON-canonical
    /// paymaster (EIP-8141). Returns zero when none are pending.
    pub fn noncanonical_paymaster_pending(&self, paymaster: Address) -> Result<u8, StoreError> {
        Ok(self
            .read()?
            .noncanonical_paymaster_pending
            .get(&paymaster)
            .copied()
            .unwrap_or(0))
    }

    /// Snapshot of every pending frame transaction's `(hash, sender, paymaster)`
    /// for the post-block revalidation pass (EIP-8141, task 3.5). Cloned under
    /// the read lock so revalidation can re-simulate without holding it.
    pub fn pending_frame_txs(&self) -> Result<Vec<PendingFrameTx>, StoreError> {
        let inner = self.read()?;
        Ok(inner
            .frame_tx_paymaster
            .iter()
            .filter_map(|(hash, reservation)| {
                inner
                    .transaction_pool
                    .get(hash)
                    .map(|tx| (*hash, tx.sender(), reservation.paymaster))
            })
            .collect())
    }

    /// The transaction stored under `hash`, if any. Used by revalidation to
    /// re-simulate a pending frame tx.
    pub fn get_mempool_transaction_by_hash(
        &self,
        hash: H256,
    ) -> Result<Option<MempoolTransaction>, StoreError> {
        Ok(self.read()?.transaction_pool.get(&hash).cloned())
    }

    /// Sizes of the four frame-tx tracking maps:
    /// `(pending_frame_tx_by_sender, reserved_pending_cost,
    /// noncanonical_paymaster_pending, frame_tx_paymaster)`. Exposed for tests
    /// that assert the maps return to empty after add + remove (EIP-8141).
    pub fn frame_tracking_map_sizes(&self) -> Result<(usize, usize, usize, usize), StoreError> {
        let inner = self.read()?;
        Ok((
            inner.pending_frame_tx_by_sender.len(),
            inner.reserved_pending_cost.len(),
            inner.noncanonical_paymaster_pending.len(),
            inner.frame_tx_paymaster.len(),
        ))
    }
}

/// Filter applied by the payload builder when querying pending transactions
/// from the pool. NOT a mempool admission gate — all fields here are
/// query-time filters used to pick block-includable transactions. Admission
/// rules are enforced in `Blockchain::validate_transaction`.
#[derive(Debug, Default)]
pub struct PendingTxFilter {
    /// Minimum effective priority fee for a transaction to be surfaced to
    /// the payload builder. This is a block-building filter, not an
    /// admission check — see `crates/common/types/constants.rs::MIN_GAS_TIP`.
    pub min_tip: Option<u64>,
    pub base_fee: Option<u64>,
    pub blob_fee: Option<u64>,
    pub only_plain_txs: bool,
    pub only_blob_txs: bool,
}

pub fn transaction_intrinsic_gas(
    tx: &Transaction,
    sender: Address,
    header: &BlockHeader,
    config: &ChainConfig,
) -> Result<u64, MempoolError> {
    // EIP-8141 frame txs: gas_limit() IS the computed total_gas_limit(), which
    // already includes the frame-tx intrinsic overhead. The fork-general
    // formula below misprices them (their data() is empty and the base differs),
    // so report exactly the non-frame-gas overhead the VM charges as intrinsic.
    if let Transaction::FrameTransaction(frame_tx) = tx {
        let frame_gas: u64 = frame_tx.frames.iter().map(|f| f.gas_limit).sum();
        return Ok(frame_tx.total_gas_limit().saturating_sub(frame_gas));
    }

    // Mempool admission must charge the same intrinsic gas LEVM enforces at
    // execution, or we admit txs the VM later rejects (pool pollution, wasted
    // payload-builder cycles). Reuse the VM's two helpers directly rather than
    // re-deriving the cost here:
    //   - `intrinsic_gas_dimensions` → (regular, state) including the EIP-7702
    //     per-authorization-tuple cost, EIP-7981 access-list data bytes, and
    //     the Amsterdam EIP-2780/8037/8038 weighted state gas (CREATE base +
    //     per-new-account state bytes), which is why it needs `sender`;
    //   - `intrinsic_gas_floor` → the EIP-7623/7976 calldata floor.
    // The VM requires `gas_limit >= max(intrinsic_regular + intrinsic_state,
    // floor)` (two separate checks in `validate_gas_allowance` +
    // `validate_min_gas_limit`); mirror that max here. This is fork-general,
    // so it covers Prague (auth-list cost + calldata floor) as well as
    // Amsterdam, and keeps mempool admission in lockstep with the VM.
    let fork = config.fork(header.timestamp);
    let (regular, state) = intrinsic_gas_dimensions(tx, sender, fork, header.gas_limit)
        .map_err(|e| MempoolError::IntrinsicGasError(e.to_string()))?;
    let intrinsic = regular
        .checked_add(state)
        .ok_or(MempoolError::TxGasOverflowError)?;
    // The EIP-7623 calldata floor only exists from Prague onward; the VM gates
    // it the same way (`fork >= Fork::Prague` in the default hook). Applying it
    // pre-Prague would spuriously raise the admission threshold.
    let calldata_floor = if fork >= Fork::Prague {
        intrinsic_gas_floor(tx, fork).map_err(|e| MempoolError::IntrinsicGasError(e.to_string()))?
    } else {
        0
    };
    Ok(intrinsic.max(calldata_floor))
}
