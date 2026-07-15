# Mempool

This document describes how ethrex stores, validates, and propagates pending transactions, as implemented on `main`.

> **Keep this current with the code.** A PR that changes mempool *behavior* should update the relevant rustdoc next to the code, and — if it changes the *shape* documented here (data structures, admission pipeline, propagation flows) — the matching section of this page, in the same PR. Exact check ordering, error variants, and constants are authoritatively defined in the code; this page describes the durable structure.

## Overview

The mempool is the single in-memory pool of pending transactions held by the node. It is the entry point for every transaction the node learns about — local RPC submissions and gossip from peers — and the source the payload builder pulls from when constructing a block.

```
                   ┌─────────────────────────┐         ┌─────────────────────────┐
                   │   JSON-RPC ingress      │         │   P2P ingress           │
                   │  eth_sendRawTransaction │         │  Transactions /         │
                   │                         │         │  PooledTransactions     │
                   └────────────┬────────────┘         └────────────┬────────────┘
                                │                                    │
                                │  add_transaction_to_pool /         │  add_transaction_to_pool /
                                │  add_blob_transaction_to_pool      │  add_blob_transaction_to_pool
                                ▼                                    ▼
                   ┌──────────────────────────────────────────────────────────────┐
                   │           Blockchain::validate_transaction                    │
                   │   (size, init code, gas caps, nonce, EIP-3607, balance,      │
                   │    cumulative balance, RBF, chain id, gap-admission, …)       │
                   └──────────────────────────────┬───────────────────────────────┘
                                                  │
                                                  ▼
                   ┌──────────────────────────────────────────────────────────────┐
                   │                          Mempool                              │
                   │  transaction_pool · blobs_bundle_pool · broadcast_pool · …    │
                   └─────┬──────────────────────────┬──────────────────────────┬──┘
                         │                          │                          │
                         │ broadcast_pool           │ transaction_pool         │ transaction_pool
                         ▼                          ▼                          ▼
                   ┌───────────────┐         ┌───────────────┐         ┌───────────────┐
                   │ tx_broadcaster│         │ Payload       │         │ Fork-choice   │
                   │ (P2P gossip)  │         │ Builder       │         │ update        │
                   └───────────────┘         └───────────────┘         │ (removes txs  │
                                                                       │  included in  │
                                                                       │  new head)    │
                                                                       └───────────────┘
```

The mempool participates in four flows:

- **RPC ingress** — `eth_sendRawTransaction` calls `Blockchain::add_transaction_to_pool` / `add_blob_transaction_to_pool`.
- **P2P ingress** — incoming `Transactions` / `PooledTransactions` messages call the same `add_transaction_to_pool` / `add_blob_transaction_to_pool` entry points.
- **Block-building source** — the payload builder calls `Mempool::filter_transactions` to pull pending transactions grouped by sender and sorted by nonce.
- **Block-removal source** — when fork-choice promotes a new canonical head, `Blockchain::remove_block_transactions_from_pool` evicts every included transaction.

## Data Structures

The pool is a single `Mempool` value (`crates/blockchain/mempool.rs`) holding one `RwLock<MempoolInner>`, a `tokio::sync::Notify` (`tx_added`) used to wake payload builders on insertion, and a monotonic `AtomicU64` (`tx_seq`) bumped on every insertion for staleness detection.

### `MempoolInner`

The inner state, mutated only under the write lock:

| Field | Type | Purpose |
|-------|------|---------|
| `transaction_pool` | `FxHashMap<H256, MempoolTransaction>` | The authoritative set of pooled transactions, keyed by hash. |
| `broadcast_pool` | `FxHashSet<H256>` | Hashes still pending the next P2P broadcast tick. Drained by `tx_broadcaster`. |
| `blobs_bundle_pool` | `FxHashMap<H256, BlobsBundle>` | EIP-4844 sidecars, keyed by the owning transaction hash. Its key set is the blob txs currently held. |
| `blobs_bundle_by_versioned_hash` | `FxHashMap<H256, FxHashMap<H256, usize>>` | Reverse index: `versioned_hash → { tx_hash → position_in_bundle }`. A blob can be referenced by more than one transaction, so each referencing tx has its own inner entry. |
| `in_flight_txs` | `FxHashSet<H256>` | Hashes for which a `GetPooledTransactions` request was sent but no response has arrived. Prevents duplicate requests across peers. |
| `alternates` | `FxHashMap<H256, (VecDeque<Alternate>, Instant)>` | Per-hash queue of alternate announcers (fallback peers to retry against) plus a last-touched instant. |
| `txs_by_sender_nonce` | `BTreeMap<(H160, u64), H256>` | Sorted `(sender, nonce)` index for nonce lookups, replacement, and the per-sender queued cap. |
| `txs_order` | `VecDeque<H256>` | FIFO insertion order of regular (non-blob) txs. Drives regular eviction. |
| `max_mempool_size` | `usize` | Cap on the regular (non-blob) pool. Set from `--mempool.maxsize`. |
| `max_blob_mempool_size` | `usize` | Cap on the blob sub-pool. |
| `mempool_prune_threshold` | `usize` | Length at which `txs_order` is compacted to drop tombstones (`max + max/2`). |
| `pending_frame_tx_by_sender` | `FxHashMap<Address, (H256, u64)>` | EIP-8141: at most one pending frame tx per sender — `(hash, nonce)`. |
| `reserved_pending_cost` | `FxHashMap<Address, U256>` | EIP-8141: per-paymaster sum of reserved max-cost across pending frame txs. |
| `noncanonical_paymaster_pending` | `FxHashMap<Address, u8>` | EIP-8141: count of pending frame txs per non-canonical paymaster. |
| `frame_tx_paymaster` | `FxHashMap<H256, FramePaymasterReservation>` | EIP-8141: per-frame-tx paymaster reservation record. |

### `MempoolTransaction`

Defined in `crates/common/types/transaction.rs` (in its `mempool` module), this wraps a `Transaction` (`inner: Arc<Transaction>`) together with the recovered `sender: Address` and a `timestamp: u128` (microseconds since epoch, set on entry). The pool stores only `MempoolTransaction`s — never raw transactions — so the sender is recovered once at admission and never re-derived on query.

## Admission Validation

Every transaction entering the pool — through RPC or P2P — passes through `Blockchain::validate_transaction` (`crates/blockchain/blockchain.rs`). It returns `Ok((tx_to_replace, frame_reservation, sender_account_nonce))` — `tx_to_replace` is `Some(hash)` when the transaction replaces an existing one at the same `(sender, nonce)` — or `Err(MempoolError::*)` on rejection. Checks run in this order:

```
┌──────────────────────────────────────────────────────────────────────────┐
│                  Blockchain::validate_transaction                         │
├──────────────────────────────────────────────────────────────────────────┤
│  1. L2-only tx-type rejection (L1 node)              → L2OnlyTransaction… │
│  2. PrivilegedL2 short-circuit                       → Ok (bypasses pool) │
│  3. Frame-tx (EIP-8141) static gates                 → FrameTx…           │
│  4. Per-tx wire-size cap (non-blob)                  → TxSizeExceeded     │
│  5. Init code size cap (Shanghai+, Amsterdam adj.)   → TxMaxInitCodeSize  │
│  6. Post-Osaka gas-limit cap (EIP-7825)              → TxMaxGasLimitExc…  │
│  7. tx.gas_limit ≤ header.gas_limit                  → TxGasLimitExceeded │
│  8. max_priority_fee ≤ max_fee_per_gas               → TxTipAboveFeeCap   │
│  9. EIP-7702 pre-Prague / empty auth list            → Eip7702TxPreFork / │
│                                                        EmptyAuthorization │
│ 10. Intrinsic gas ≤ tx.gas_limit                     → TxIntrinsicGas…    │
│ 11. max_fee_per_blob_gas ≥ MIN_BASE_FEE_PER_BLOB_GAS → TxBlobBaseFeeTooLow│
│ 12. Nonce ≥ account.nonce  &&  nonce ≠ u64::MAX      → NonceTooLow        │
│ 13. EIP-3607 contract-sender check (7702 exc.)       → SenderIsContract   │
│ 14. Single-tx cost ≤ account.balance                 → NotEnoughBalance   │
│ 15. RBF: find_tx_to_replace                          → UnderpricedRepl…   │
│ 16. Cumulative pending cost ≤ balance                → InsufficientCumul… │
│ 17. tx.chain_id matches config.chain_id (if set)     → InvalidChainId     │
│ 18. Gapped-nonce rejection under pool pressure       → GapAdmissionDenied │
│ 19. Frame-tx validation-prefix sim + paymaster       → FrameTx…           │
│ (per-sender queued cap is enforced inside add_transaction, post-validate) │
└──────────────────────────────────────────────────────────────────────────┘
```

Notes on individual checks:

**L2-only rejection.** On an L1 node, L2-only transaction types (`FeeToken` `0x7d`, `PrivilegedL2` `0x7e`) are rejected with `L2OnlyTransactionType` — they are valid only on L2 and unknown to other L1 clients.

**Privileged short-circuit.** `Transaction::PrivilegedL2Transaction` returns early before pool admission; these are produced by the L2 sequencer.

**Frame transactions (EIP-8141).** Frame txs go through an extended set of gates — fork activation (`FrameTxPreFork`), expiry (`FrameTxExpired`), static structural validity (`InvalidFrameTransaction`), a no-blobs rule (`FrameTxBlobsUnsupported`), a signature-verification gas budget (`FrameTxVerifyGasExceeded` / `FrameTxVerifyGasBudgetExceeded`), signature authenticity and low-`s` malleability (`InvalidFrameSignature` / `FrameTxMalleableSignature`), and validation-prefix shape (`FrameTxUnrecognizedPrefix` / `FrameTxInvalidPrefixStructure`). The validation-prefix simulation and paymaster accounting run last (see [Frame transactions](#frame-transactions-eip-8141)).

**Per-tx wire-size cap.** Non-blob transactions are bounded by `MAX_TX_SIZE = 128 KiB` against `Transaction::encode_canonical_len()`. Blob transactions are bounded by `MAX_BLOB_TX_SIZE` in `add_blob_transaction_to_pool` against the wire wrapper, since ethrex stores the core transaction and the sidecar in separate structs.

**Init code size.** Active from Shanghai. Limit is `MAX_INITCODE_SIZE = 48 KiB` (`2 × MAX_CODE_SIZE`); from Amsterdam onward (EIP-7954) it becomes `AMSTERDAM_MAX_INITCODE_SIZE = 64 KiB`.

**Nonce lookup.** Reads `account.nonce` from storage at the latest block. Rejects with `NonceTooLow` when `nonce < account.nonce` or `nonce == u64::MAX`.

**EIP-3607.** Senders with non-empty `code_hash` are rejected with `SenderIsContract`, except when their code is a valid EIP-7702 delegation designation: exactly `EIP7702_DELEGATED_CODE_LEN = 23` bytes — the 3-byte `0xef0100` prefix followed by the 20-byte delegate address. A length-based fast path consults code-metadata length first and only fetches bytecode when the length matches the delegation shape. Skipped for frame txs.

**Single-tx balance.** `tx.cost_without_base_fee()` — `gas_limit × max_fee_per_gas + value`, plus `blob_gas × max_fee_per_blob_gas` for blob transactions — must not exceed the sender's balance. Senders absent from state are rejected. Skipped for frame txs (payer unknown until execution).

**Cumulative-balance check.** Beyond the single-tx balance check, the sum of the sender's already-pooled transaction costs (via `Mempool::sum_cost_for_sender`, excluding the tx being replaced and any obsoleted below-nonce entries) plus the new tx's cost must not exceed the balance; otherwise `InsufficientCumulativeBalance { required, available }`. This prevents a sender from parking many individually-fundable but collectively-unfundable transactions. Skipped for frame txs.

**Chain id.** Only checked when the transaction declares one; mismatch with `ChainConfig::chain_id` rejects.

**Gapped-nonce rejection under pressure.** When the pool is heavily occupied, a new *non-replacement* transaction whose nonce is not the sender's on-chain nonce is rejected with `GapAdmissionDeniedUnderPressure { occupancy_pct, nonce_gap }`. The gate fires only when `occupancy_pct ≥ gap_admit_occupancy_threshold` (default 90; a threshold of 100 disables it), so nonce-gap parking spam is bounded without affecting normal load.

**Per-sender queued cap.** Enforced inside `add_transaction` (see [Insertion path](#insertion-path)), not here, so it re-checks the live pool atomically with the insert.

## Replacement by Fee (RBF)

`Mempool::find_tx_to_replace` decides whether a new transaction at an existing `(sender, nonce)` is accepted as a replacement. A replacement is accepted only when it strictly increases every applicable fee field versus the in-pool transaction:

- Legacy: `gas_price` strictly greater.
- EIP-1559 / EIP-2930 / EIP-7702 / fee-token: both `max_fee_per_gas` and `max_priority_fee_per_gas` strictly greater.
- EIP-4844 blob: additionally `max_fee_per_blob_gas` strictly greater.

If the bump condition isn't met, the replacement is rejected with `UnderpricedReplacement`.

## Per-sender Queued Cap

A per-sender cap bounds only the **queued** (future / nonce-gapped) sub-pool, aligned with geth's `AccountQueue`: transactions whose nonce is beyond the sender's contiguous executable run from the on-chain nonce. `BlockchainOptions::max_queued_txs_per_account` (default `DEFAULT_MAX_QUEUED_TXS_PER_ACCOUNT = 64`, `--mempool.max-queued-txs-per-account`). Executable (contiguous-nonce) transactions are **never** capped, so a single high-throughput sender is unaffected. Exceeding the cap yields `MaxQueuedTxsPerAccountExceeded { sender, count, limit }`.

The check runs inside `Mempool::add_transaction` under the same write lock as the insertion — the queued count is computed atomically with the insert, so two concurrent submissions from one sender cannot both pass a stale count and race past the cap. A `QueuedCap { account_nonce, max }` value, computed by `validate_transaction` (which holds the sender's on-chain nonce), is threaded through. Replacements are exempt: a same-`(sender, nonce)` replace doesn't grow the queue.

## Insertion Path

`Mempool::add_transaction(hash, sender, transaction, frame_reservation, queued_cap)` runs under the write lock in this order:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                       Mempool::add_transaction                           │
├─────────────────────────────────────────────────────────────────────────┤
│  acquire write lock                                                      │
│   │                                                                      │
│   ├── per-sender queued cap (only future-nonce txs; atomic w/ insert)    │
│   │     if is_future && queued_count ≥ max → MaxQueuedTxsPerAccount…      │
│   │                                                                      │
│   ├── frame-tx (EIP-8141) gating: one-pending-per-sender + locked        │
│   │     paymaster availability / non-canonical-limit re-check, then      │
│   │     remove the tx occupying (sender, nonce) if replacing             │
│   │                                                                      │
│   ├── prune txs_order if len > mempool_prune_threshold                   │
│   │                                                                      │
│   ├── eviction: blob → remove_worst_blob_transaction if over blob cap;   │
│   │             regular → remove_oldest_regular_transaction if at cap,    │
│   │             then push hash onto txs_order                            │
│   │                                                                      │
│   ├── insert (sender, nonce) → hash into txs_by_sender_nonce             │
│   ├── insert hash → MempoolTransaction into transaction_pool             │
│   ├── insert hash into broadcast_pool; drop from alternates              │
│   └── frame-tx: record pending_frame_tx_by_sender + paymaster reserves   │
│  drop write lock                                                         │
│  bump tx_seq; tx_added.notify_waiters()  ── wakes the payload builder    │
└─────────────────────────────────────────────────────────────────────────┘
```

For blob transactions, `add_blobs_bundle` is called immediately before `add_transaction` so the sidecar is queryable by the time the payload builder is notified.

## Eviction

Regular and blob transactions are evicted by separate policies:

- **Regular (`remove_oldest_regular_transaction`)** — strictly **FIFO** by insertion order: pop from the front of `txs_order` until `regular_tx_count() < max_mempool_size`. No tip- or sender-fairness consideration (a known limitation — see [Open work](#known-limitations--open-work)).
- **Blob (`remove_worst_blob_transaction`)** — the blob sub-pool is evicted by *least includability*, not FIFO: the dropped tx maximizes `(per-sender nonce offset, Reverse(max_fee_per_blob_gas))` — i.e. the deepest-in-its-own-queue blob, ties broken by lowest blob fee — so the next-includable low-nonce blob is never dropped for a high-nonce one.

`txs_order` is a tombstoned queue: removing a transaction via any path (eviction, replacement, block inclusion, explicit removal) leaves a stale hash that is filtered at pop time. When its length crosses `mempool_prune_threshold` (`max + max/2`), the next `add_transaction` compacts it in place.

## Frame Transactions (EIP-8141)

Frame transactions carry a validation prefix and may be sponsored by a paymaster. The mempool holds extra per-sender and per-paymaster state to admit them safely:

- **One pending frame tx per sender** — `pending_frame_tx_by_sender` enforces a single in-flight frame tx per sender; the check and insert are atomic under the write lock.
- **Paymaster reservation accounting** — `reserved_pending_cost` sums each paymaster's reserved max-cost across pending frame txs so concurrent sponsored txs can't collectively overdraw it (`FrameTxPaymasterUnderfunded`); `noncanonical_paymaster_pending` bounds the number of pending frame txs a non-canonical paymaster may sponsor (`FrameTxNonCanonicalPaymasterLimit`); `frame_tx_paymaster` records each tx's reservation so removal (eviction / inclusion / reorg) releases it exactly once.

The unlocked checks in `validate_transaction` (availability, non-canonical limit) are a pre-filter; the authoritative re-check runs under the write lock in `add_transaction`, matching the pattern used by the per-sender gates.

## P2P Propagation

Three P2P paths interact with the mempool:

**Periodic broadcast.** `TxBroadcaster` (`crates/networking/p2p/tx_broadcaster.rs`) runs an actor on a `--p2p.tx-broadcasting-interval`-millisecond tick. Each tick it snapshots `broadcast_pool` via `Mempool::get_txs_for_broadcast`, sends full transaction bodies to ~`sqrt(peers)` peers and `NewPooledTransactionHashes` announcements to the rest, then calls `Mempool::remove_broadcasted_txs` to clear the set. Blob (EIP-4844) and frame (EIP-8141) transactions are announced by hash only; privileged transactions are filtered out. Per-peer deduplication is tracked with a broadcast record keyed by hash and a peer bitset.

**New-peer hash dump.** On a fresh RLPx connection, `send_all_pooled_tx_hashes` (`crates/networking/p2p/rlpx/connection/server.rs`) sends the node's known pooled-transaction hashes to the new peer. It reads them via `Mempool::get_all_txs_by_sender`, flattens, and skips privileged transactions.

**GetPooledTransactions responses.** `GetPooledTransactions::handle` (`crates/networking/p2p/rlpx/eth/transactions.rs`) serves requested hashes through `Blockchain::get_p2p_transaction_by_hash`, which reads the pooled transaction (and its blobs bundle for blob txs) and returns a `not found` path for missing hashes and for privileged transactions (which are not served over P2P).

Incoming gossip flows the other way: `Mempool::reserve_unknown_hashes` filters incoming `NewPooledTransactionHashes` against `transaction_pool` and `in_flight_txs` in one locked pass, marking unknown hashes in-flight so concurrent peer handlers don't issue duplicate `GetPooledTransactions` requests. `clear_in_flight_txs` clears the marker when the response arrives or the connection drops; `alternates` records fallback announcers to retry against.

## Operator-Facing CLI

Mempool-related flags in `cmd/ethrex/cli.rs`:

| Flag | Env | Default | Controls |
|------|-----|---------|----------|
| `--mempool.maxsize` | `ETHREX_MEMPOOL_MAX_SIZE` | `10_000` | Cap on the regular pool — eviction starts at this size. |
| `--mempool.max-queued-txs-per-account` | `ETHREX_MEMPOOL_MAX_QUEUED_TXS_PER_ACCOUNT` | `64` (`DEFAULT_MAX_QUEUED_TXS_PER_ACCOUNT`) | Per-sender cap on queued (future-nonce) transactions; executable txs are never capped. |
| `--mempool.gap-admit-occupancy-threshold` | `ETHREX_MEMPOOL_GAP_ADMIT_OCCUPANCY_THRESHOLD` | `90` (`DEFAULT_GAP_ADMIT_OCCUPANCY_THRESHOLD`) | Occupancy percentage (`0..=100`) at/above which gapped-nonce txs are rejected; `100` disables the gate. |
| `--p2p.tx-broadcasting-interval` | `ETHREX_P2P_TX_BROADCASTING_INTERVAL` | `1000` ms (`BROADCAST_INTERVAL_MS`) | Period of the `TxBroadcaster` actor tick. |

## Error Taxonomy

`MempoolError` (`crates/blockchain/error.rs`). Selected variants:

| Variant | Triggered when |
|---------|---------------|
| `L2OnlyTransactionType` | An L2-only tx type (`FeeToken` / `PrivilegedL2`) reaches an L1 node's admission. |
| `TxSizeExceeded { actual, limit }` | Canonical wire-encoded size exceeds `MAX_TX_SIZE` (non-blob) or the blob wrapper cap. |
| `TxMaxInitCodeSizeError` | Contract-creation init code exceeds the fork's initcode cap. |
| `TxMaxGasLimitExceededError(hash, limit)` | EIP-7825: `tx.gas_limit` exceeds the post-Osaka cap. |
| `TxGasLimitExceededError` | `tx.gas_limit > header.gas_limit`. |
| `TxTipAboveFeeCapError` | `max_priority_fee_per_gas > max_fee_per_gas`. |
| `Eip7702TxPreFork` / `EmptyAuthorizationList` | Type-4 tx before Prague / with an empty authorization list. |
| `TxIntrinsicGasCostAboveLimitError` / `TxGasOverflowError` / `IntrinsicGasError(_)` | Intrinsic gas exceeds the limit / overflow / computation error. |
| `TxBlobBaseFeeTooLowError` | `max_fee_per_blob_gas < MIN_BASE_FEE_PER_BLOB_GAS`. |
| `NonceTooLow` | `tx.nonce < account.nonce` or `nonce == u64::MAX`. |
| `SenderIsContract` | EIP-3607: sender has non-empty code that isn't a valid EIP-7702 delegation. |
| `NotEnoughBalance` | Single-tx cost exceeds balance, or sender absent from state. |
| `InsufficientCumulativeBalance { required, available }` | Sum of the sender's pending-tx costs plus this one exceeds balance. |
| `UnderpricedReplacement` | RBF candidate doesn't strictly out-bid the in-pool tx on all fee fields. |
| `InvalidChainId(expected)` | `tx.chain_id` set and doesn't match the configured chain. |
| `GapAdmissionDeniedUnderPressure { occupancy_pct, nonce_gap }` | Non-replacement gapped-nonce tx while occupancy ≥ threshold. |
| `MaxQueuedTxsPerAccountExceeded { sender, count, limit }` | Per-sender queued (future-nonce) cap would be exceeded. |
| `BlobsBundleError(_)` / `BlobTxNoBlobsBundle` | KZG/sidecar validation failed / blob tx submitted to a non-blob entry point. |
| `InvalidPooledTxType(_)` / `InvalidPooledTxSize` / `RequestedPooledTxNotFound` | `PooledTransactions` response type/size mismatch, or an unrequested tx. |
| `FrameTx*` (many) | EIP-8141 frame-tx admission failures (fork, expiry, signature, prefix, paymaster). |
| `InvalidTxSender(_)` / `StoreError(_)` / `NoBlockHeaderError` | Signature recovery failed / storage error / missing latest header. |

## Known Limitations / Open Work

The mempool today is a single combined pool with FIFO regular eviction and the admission checks above. Areas with hardening work in flight or planned (tracked in their own PRs, which carry their own documentation):

- **Tip-aware regular eviction.** Regular eviction is arrival-order FIFO; a high-tip transaction can be evicted for a low-tip one. Heap/tip-aware eviction is in flight (#6607).
- **RBF percentage fee bump.** RBF currently accepts any strict fee increase; a configurable minimum-percentage bump (and cross-type replacement rejection) is in flight (#6601).
- **Minimum priority-fee floor at admission** (#6604).
- **Non-propagating local transactions** (`--mempool.private`) (#6576).
- **Local-vs-P2P origin threading** through admission (#6608).
- **Periodic re-validation / sweep** of stale or dormant pending transactions (#6610).
- **Single combined pool.** Blob and non-blob transactions share `transaction_pool` with separate caps and eviction, but there is no full pending/queued sub-pool separation.

## Related Documentation

- [System Overview](./overview.md) — where the mempool sits inside the node.
- [Block Execution Pipeline](./block_execution.md) — how the payload builder consumes pending transactions.
