# Mempool

This document describes how ethrex stores, validates, and propagates pending transactions. It covers the state as it stands on `main`, plus admission and propagation behavior added by the six in-flight mempool-hardening PRs (#6576, #6599, #6600, #6601, #6603, #6604). It will be updated as those PRs merge.

## Overview

The mempool is the single in-memory pool of pending transactions held by the node. It is the entry point for every transaction that the node knows about — local RPC submissions and gossip from peers — and the source the payload builder pulls from when constructing a new block.

```
                   ┌─────────────────────────┐         ┌─────────────────────────┐
                   │   JSON-RPC ingress      │         │   P2P ingress           │
                   │  eth_sendRawTransaction │         │  Transactions /         │
                   │                         │         │  PooledTransactions     │
                   └────────────┬────────────┘         └────────────┬────────────┘
                                │                                   │
                                │  add_local_*_to_pool              │  add_*_to_pool
                                ▼                                   ▼
                   ┌──────────────────────────────────────────────────────────────┐
                   │           Blockchain::validate_transaction                    │
                   │   (size, init code, chain id, sender slots, nonce, EIP-3607, │
                   │    balance, min-tip floor, RBF check, …)                      │
                   └──────────────────────────────┬───────────────────────────────┘
                                                  │
                                                  ▼
                   ┌──────────────────────────────────────────────────────────────┐
                   │                          Mempool                              │
                   │  transaction_pool · broadcast_pool · private_pool · …         │
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

- **RPC ingress** — `eth_sendRawTransaction` calls `Blockchain::add_local_transaction_to_pool` / `add_local_blob_transaction_to_pool` (via #6576). These honor `BlockchainOptions::private_mempool`.
- **P2P ingress** — incoming `Transactions` / `PooledTransactions` messages call `Blockchain::add_transaction_to_pool` / `add_blob_transaction_to_pool`, which always broadcast.
- **Block-building source** — `Blockchain::build_payload` calls `Mempool::filter_transactions` to pull pending transactions grouped by sender and sorted by nonce.
- **Block-removal source** — when fork-choice promotes a new canonical head, `Blockchain::remove_block_transactions_from_pool` evicts every included transaction (`crates/networking/rpc/engine/fork_choice.rs:292`).

## Data Structures

The pool is a single `Mempool` value (`crates/blockchain/mempool.rs:113`) protected by one `RwLock<MempoolInner>` plus a `tokio::sync::Notify` used to wake payload builders on insertions.

### `MempoolInner`

`crates/blockchain/mempool.rs:27`

| Field | Type | Purpose |
|-------|------|---------|
| `transaction_pool` | `FxHashMap<H256, MempoolTransaction>` | The authoritative set of pooled transactions, keyed by hash. |
| `broadcast_pool` | `FxHashSet<H256>` | Hashes queued for the next P2P broadcast tick. Drained by `tx_broadcaster`. |
| `private_pool` | `FxHashSet<H256>` | Hashes admitted with `--mempool.private` set; must not be propagated to peers via any P2P path (via #6576). |
| `blobs_bundle_pool` | `FxHashMap<H256, BlobsBundle>` | EIP-4844 sidecars, keyed by their owning transaction hash. |
| `blobs_bundle_by_versioned_hash` | `FxHashMap<H256, FxHashMap<H256, usize>>` | Reverse index from blob versioned hash to `(tx_hash, position)`. |
| `in_flight_txs` | `FxHashSet<H256>` | Hashes for which a `GetPooledTransactions` request has been sent but no response has arrived. Prevents duplicate requests across peers. |
| `txs_by_sender_nonce` | `BTreeMap<(H160, u64), H256>` | Sorted sender-nonce index used for nonce lookups, RBF, and the per-sender slot cap. |
| `txs_order` | `VecDeque<H256>` | FIFO queue of insertion order. Drives eviction when the pool is full. |
| `max_mempool_size` | `usize` | Hard cap on `transaction_pool.len()`. Set from `--mempool.maxsize`. |
| `mempool_prune_threshold` | `usize` | Length threshold at which `txs_order` is compacted to remove tombstones. Set to `1.5 × max_mempool_size`. |

### `MempoolTransaction`

Defined in `crates/common/types/mempool.rs`, this wraps a `Transaction` together with the recovered sender address and insertion timestamp. The pool only stores `MempoolTransaction`s — never raw transactions — so the sender doesn't need to be recovered again on every query.

## Admission Validation

Every transaction entering the pool — through RPC or P2P — passes through `Blockchain::validate_transaction` (`crates/blockchain/blockchain.rs:2417`). The function returns `Ok(Some(hash))` if the transaction replaces an existing one at the same `(sender, nonce)`, `Ok(None)` if it is a new entry, or `Err(MempoolError::*)` on rejection. Checks run in this order:

```
┌──────────────────────────────────────────────────────────────────────────┐
│                  Blockchain::validate_transaction                         │
├──────────────────────────────────────────────────────────────────────────┤
│  1. Privileged-tx short-circuit (L2 only)            → Ok(None)           │
│  2. Per-tx wire-size cap (non-blob)                  → TxSizeExceeded     │  #6599
│  3. Init code size cap (Shanghai+, Amsterdam adj.)   → TxMaxInitCodeSize  │
│  4. Post-Osaka gas-limit cap (EIP-7825)              → TxMaxGasLimitExc.. │
│  5. tx.gas_limit ≤ header.gas_limit                  → TxGasLimitExceeded │
│  6. max_priority_fee ≤ max_fee_per_gas               → TxTipAboveFeeCap   │
│  7. Tip-cap floor                                    → TipBelowMinimum   │  #6604
│  8. Intrinsic gas ≤ tx.gas_limit                     → TxIntrinsicGas…    │
│  9. max_fee_per_blob_gas ≥ MIN_BASE_FEE_PER_BLOB_GAS → TxBlobBaseFeeTooLow│
│ 10. Nonce ≥ account.nonce  &&  nonce ≠ u64::MAX      → NonceTooLow        │
│ 11. EIP-3607 contract-sender check (with 7702 exc.)  → SenderIsContract  │  #6600
│ 12. tx.cost_without_base_fee ≤ account.balance       → NotEnoughBalance   │
│ 13. RBF: find_tx_to_replace                          → UnderpricedRepl…  │  #6601
│                                                        ReplacementType…  │
│ 14. tx.chain_id matches config.chain_id (if set)     → InvalidChainId    │
│ (per-sender slot cap is enforced inside add_transaction, post-validation) │
└──────────────────────────────────────────────────────────────────────────┘
```

A few notes on individual checks:

**Privileged short-circuit.** `Transaction::PrivilegedL2Transaction` returns `Ok(None)` before any other check. These transactions are produced by the L2 sequencer and bypass mempool admission.

**Per-tx wire-size cap (via #6599).** Non-blob transactions are bounded by `MAX_TX_SIZE = 128 KiB` against `Transaction::encode_canonical_len()`. Blob transactions are bounded by `MAX_BLOB_TX_SIZE = 1 MiB` enforced in `add_blob_transaction_to_pool` against the wire wrapper — `Transaction::encode_canonical_len() + BlobsBundle.length()` — since ethrex stores the core transaction and the sidecar in separate structs. The previous `MAX_TRANSACTION_DATA_SIZE` calldata-only check is removed; the encoded-size cap is strictly tighter for non-blob transactions.

**Init code size.** Active from Shanghai. Limit is `MAX_INITCODE_SIZE = 48 KiB` (`2 × MAX_CODE_SIZE`); from Amsterdam onward (EIP-7954) it becomes `AMSTERDAM_MAX_INITCODE_SIZE = 64 KiB`.

**Chain id.** Only checked if the transaction declares one (legacy unsigned-by-chain-id transactions are accepted). Mismatch with `ChainConfig::chain_id` rejects.

**Per-sender pending-tx cap (via #6603).** `BlockchainOptions::max_pending_txs_per_account` (default 16, `DEFAULT_MAX_PENDING_TXS_PER_ACCOUNT`). The check happens inside `Mempool::add_transaction` under the write lock used for the insertion — `txs_by_sender_nonce.range((sender, 0)..=(sender, u64::MAX)).count()` is computed atomically with the insert. Replacement candidates at an existing `(sender, nonce)` bypass the cap because the caller removes the old transaction first, leaving the post-removal count one below the limit.

**Nonce lookup.** Reads `account.nonce` from storage at the latest block. Rejects with `NonceTooLow` for `nonce < account.nonce` or `nonce == u64::MAX`.

**EIP-3607 (via #6600).** Senders with non-empty `code_hash` are rejected with `SenderIsContract`, except when their code is a valid EIP-7702 delegation designation — exactly `EIP7702_DELEGATED_CODE_LEN = 23` bytes prefixed with `0xef0100`. The check uses a length-based fast path: code-metadata length is consulted first, and the full bytecode is only fetched when the length matches the delegation shape.

**Balance check.** `tx.cost_without_base_fee()` covers `gas_limit × max_fee_per_gas + value` plus `blob_gas × max_fee_per_blob_gas` for blob transactions (the blob-gas contribution was added by the already-merged #6509). Senders not present in state are rejected — they cannot possibly fund the transaction.

**Tip-cap floor (via #6604).** `BlockchainOptions::min_tip_wei` (default `DEFAULT_MIN_TIP_WEI = 1`). The check compares the raw tip cap — `max_priority_fee_per_gas` for typed transactions, `gas_price` for legacy — against the configured floor. The decision is independent of the current base fee: a transaction that paid the floor at admission isn't reclassified later as base fee oscillates. A floor of `0` disables the check.

**Cumulative-balance check.** Not present on main. The static comment in `blockchain.rs` lists it as future work (point 4 in the `SOME VALIDATIONS THAT WE COULD INCLUDE` block above `validate_transaction`). When a sender has multiple pending transactions in the pool, ethrex currently checks only the new transaction's cost against the on-chain balance — not the sum across all pooled transactions plus the new one.

## Replacement by Fee (RBF)

`Mempool::find_tx_to_replace` (`crates/blockchain/mempool.rs:454`) decides whether a new transaction at an existing `(sender, nonce)` is accepted as a replacement. The semantics on main today is "any strict-greater bump on the appropriate fee fields". The behavior added by #6601 tightens this:

**Type-change rejection.** Replacing a transaction with a different `Transaction` variant — legacy by 1559, blob by non-blob, etc. — is rejected via `std::mem::discriminant` comparison and surfaces `MempoolError::ReplacementTypeMismatch`. Cross-type replacement skews accounting (blob vs non-blob slot reservation) under a single combined pool.

**Percentage fee bump.** Each applicable fee field must increase by at least the configured percentage compared to the in-pool transaction:

| Transaction type | Fee fields that must bump | Default bump |
|------------------|--------------------------|--------------|
| Legacy | `gas_price` | 10% |
| EIP-2930 / EIP-1559 / EIP-7702 / fee-token | `max_fee_per_gas`, `max_priority_fee_per_gas` | 10% |
| EIP-4844 blob | `max_fee_per_gas`, `max_priority_fee_per_gas`, `max_fee_per_blob_gas` | 100% |

Bump arithmetic uses `u128` checked multiplication: an overflow in `existing × (100 + bump) / 100` rejects the replacement rather than silently saturating. A bump of `0` collapses to "new ≥ existing".

The two thresholds are configurable via `BlockchainOptions::price_bump_percent` and `BlockchainOptions::blob_price_bump_percent`, exposed as `--mempool.price-bump` and `--mempool.blob-price-bump`. Defaults are `DEFAULT_PRICE_BUMP_PERCENT = 10` and `DEFAULT_BLOB_PRICE_BUMP_PERCENT = 100`.

## Insertion Path

`Mempool::add_transaction` (`crates/blockchain/mempool.rs:145`) runs under the inner write lock in this order:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                       Mempool::add_transaction                           │
├─────────────────────────────────────────────────────────────────────────┤
│  acquire write lock                                                      │
│   │                                                                      │
│   ├── per-sender slot count (atomic with insertion)             #6603    │
│   │     count = txs_by_sender_nonce.range((sender, ..)).count()          │
│   │     if count ≥ max_pending_txs_per_account                          │
│   │       → MaxPendingTxsPerAccountExceeded                              │
│   │                                                                      │
│   ├── prune order queue if txs_order.len() > prune_threshold             │
│   │     retain only hashes still present in transaction_pool             │
│   │                                                                      │
│   ├── evict oldest if transaction_pool.len() ≥ max_mempool_size          │
│   │     pop from txs_order until under cap; remove from all indices      │
│   │                                                                      │
│   ├── push hash onto txs_order                                           │
│   ├── insert (sender, nonce) → hash into txs_by_sender_nonce             │
│   ├── insert hash → MempoolTransaction into transaction_pool             │
│   └── insert hash into broadcast_pool (or private_pool for local-private)│
│  drop write lock                                                         │
│  tx_added.notify_waiters()  ── wakes the payload builder                 │
└─────────────────────────────────────────────────────────────────────────┘
```

The per-sender slot check (via #6603) sits inside the same write lock that performs the insertion. Concurrent submissions cannot both pass a stale check and race past the cap.

The private-pool routing (via #6576) is selected by the caller through a sibling method `add_transaction_no_broadcast`; both share a private `add_transaction_inner` so the only behavioral difference is which set the hash is inserted into. RPC entry points (`add_local_transaction_to_pool`, `add_local_blob_transaction_to_pool`) consult `BlockchainOptions::private_mempool` and route accordingly; the P2P entry points (`add_transaction_to_pool`, `add_blob_transaction_to_pool`) always broadcast.

For blob transactions, `add_blobs_bundle` is called immediately before `add_transaction` so the sidecar is queryable by the time the payload builder is notified.

## Eviction

When `transaction_pool.len()` reaches `max_mempool_size`, `MempoolInner::remove_oldest_transaction` (`crates/blockchain/mempool.rs:95`) pops hashes from the front of `txs_order` and removes each from the pool until capacity is freed. Eviction is strictly FIFO by insertion order — no tip-based or sender-fairness consideration. This is a known limitation; tip-based or heap-based eviction is an open work item (see [Known limitations](#known-limitations--open-work)).

`txs_order` is a tombstoned queue: removing a transaction via any path (eviction, RBF replacement, block inclusion, explicit `remove_transaction`) leaves a stale hash in the queue, which is filtered out at pop time. When the queue length crosses `mempool_prune_threshold = 1.5 × max_mempool_size`, the next `add_transaction` call compacts it in place.

## P2P Propagation

Three P2P paths interact with the mempool:

**Periodic broadcast.** `TxBroadcaster` (`crates/networking/p2p/tx_broadcaster.rs`) runs an actor with a `--p2p.tx-broadcasting-interval`-millisecond tick. On each tick it calls `Mempool::get_txs_for_broadcast` to take a snapshot of every hash currently in `broadcast_pool`, sends full transactions to `√peers` peers, sends `NewPooledTransactionHashes` to the rest, then calls `Mempool::remove_broadcasted_txs` to clear the hashes from the broadcast set. Per-peer deduplication is tracked with a `BroadcastRecord` keyed by transaction hash and a `PeerMask` bitset.

**New-peer pooled-hash dump.** On a fresh RLPx connection, `send_all_pooled_tx_hashes` (`crates/networking/p2p/rlpx/connection/server.rs:762`) sends every broadcast-eligible mempool hash to the new peer. The implementation calls `Mempool::get_txs_for_new_peer_dump` (added by #6576 follow-up), which takes a single read lock and returns the broadcast-eligible snapshot in one pass, skipping privileged transactions and anything in `private_pool`.

**GetPooledTransactions responses.** `GetPooledTransactions::handle` (`crates/networking/p2p/rlpx/eth/transactions.rs:220`) serves requested hashes through `Blockchain::get_p2p_transaction_by_hash` (`crates/blockchain/blockchain.rs:2539`). With #6576, this method first checks `Mempool::is_private`: a peer-requested hash sitting in `private_pool` returns the same `not found` error path as a missing hash, which the spec for `GetPooledTransactions` explicitly allows.

Incoming gossip is handled in the opposite direction — `Mempool::reserve_unknown_hashes` filters incoming `NewPooledTransactionHashes` against `transaction_pool` and `in_flight_txs` in a single locked pass, marking the unknown ones as in-flight so concurrent peer handlers don't issue duplicate `GetPooledTransactions` requests. When the response arrives (or the connection drops), `clear_in_flight_txs` clears the marker.

## Private Mempool (`--mempool.private`)

Via #6576. When `BlockchainOptions::private_mempool` is `true`:

- Transactions submitted through this node's RPC (`eth_sendRawTransaction`) enter the pool through `add_transaction_no_broadcast`. Their hash goes into `private_pool` instead of `broadcast_pool`.
- The payload builder reads from `transaction_pool` directly, so private transactions are eligible for inclusion in locally-built blocks.
- `tx_broadcaster` reads only `broadcast_pool` — private transactions are never gossiped.
- `send_all_pooled_tx_hashes` skips `private_pool` entries, so a peer connecting after a private transaction was admitted does not learn its hash.
- `GetPooledTransactions` responses for a private hash return `not found`.

P2P-received transactions are unaffected — `add_transaction_to_pool` (the P2P path) always broadcasts. If the same transaction arrives via gossip after being submitted locally, the duplicate-hash short-circuit in `add_transaction_to_pool_inner` cannot retroactively un-broadcast it; an operator-visible `warn!` is logged in that case.

## Operator-Facing CLI

All mempool-related flags live in `cmd/ethrex/cli.rs`. Defaults are pinned to named constants — no inline literals.

| Flag | Env | Default | Controls |
|------|-----|---------|----------|
| `--mempool.maxsize` | `ETHREX_MEMPOOL_MAX_SIZE` | `10_000` | Cap on `transaction_pool.len()` — eviction starts at this size. |
| `--mempool.private` | `ETHREX_MEMPOOL_PRIVATE` | `false` | Keep RPC-submitted transactions out of P2P propagation (via #6576). |
| `--mempool.price-bump` | `ETHREX_MEMPOOL_PRICE_BUMP` | `10` | Minimum fee bump (percent) to replace a non-blob transaction (via #6601). |
| `--mempool.blob-price-bump` | `ETHREX_MEMPOOL_BLOB_PRICE_BUMP` | `100` | Minimum fee bump (percent) to replace an EIP-4844 blob transaction (via #6601). |
| `--mempool.max-pending-txs-per-account` | `ETHREX_MEMPOOL_MAX_PENDING_TXS_PER_ACCOUNT` | `16` | Per-sender pending-transaction cap (via #6603). |
| `--mempool.min-tip` | `ETHREX_MEMPOOL_MIN_TIP` | `1` | Minimum tip cap (wei) at admission; `0` disables the floor (via #6604). |
| `--p2p.tx-broadcasting-interval` | `ETHREX_P2P_TX_BROADCASTING_INTERVAL` | `1000` ms | Period of the `TxBroadcaster` actor tick. |

See [Admission Validation](#admission-validation), [Replacement by Fee (RBF)](#replacement-by-fee-rbf), [Private Mempool](#private-mempool---mempoolprivate), and [P2P Propagation](#p2p-propagation) for the sections that consume each setting.

## Error Taxonomy

`MempoolError` (`crates/blockchain/error.rs:74`). Variants in italics are introduced by the in-flight PRs.

| Variant | Triggered when |
|---------|---------------|
| `NoBlockHeaderError` | The latest block header is not in storage. |
| `StoreError(_)` | Underlying storage lookup failed. |
| `BlobsBundleError(_)` | KZG / sidecar validation failed for an EIP-4844 transaction. |
| `TxMaxInitCodeSizeError` | Contract-creation transaction's init code exceeds the fork's initcode cap. |
| `TxMaxDataSizeError` | (To be removed by #6599 in favor of `TxSizeExceeded`.) Non-creation transaction's calldata exceeds the legacy `MAX_TRANSACTION_DATA_SIZE`. |
| *`TxSizeExceeded { actual, limit }`* (#6599) | Canonical wire-encoded size exceeds `MAX_TX_SIZE` (non-blob) or `MAX_BLOB_TX_SIZE` (wrapper). |
| *`SenderIsContract`* (#6600) | EIP-3607: sender has non-empty code that is not a valid EIP-7702 delegation. |
| `TxGasLimitExceededError` | `tx.gas_limit > header.gas_limit`. |
| `TxMaxGasLimitExceededError(hash, limit)` | EIP-7825: `tx.gas_limit > POST_OSAKA_GAS_LIMIT_CAP`. |
| `TxGasOverflowError` | Intrinsic-gas computation overflowed `u64`. |
| `TxTipAboveFeeCapError` | `max_priority_fee_per_gas > max_fee_per_gas`. |
| *`TipBelowMinimum { actual, limit }`* (#6604) | Raw tip cap below `min_tip_wei` admission floor. |
| `TxIntrinsicGasCostAboveLimitError` | Intrinsic gas exceeds `tx.gas_limit`. |
| `TxBlobBaseFeeTooLowError` | `max_fee_per_blob_gas < MIN_BASE_FEE_PER_BLOB_GAS`. |
| `BlobTxNoBlobsBundle` | EIP-4844 transaction submitted to a non-blob entry point. |
| `NonceTooLow` | `tx.nonce < account.nonce` or `nonce == u64::MAX`. |
| `InvalidNonce` | Reserved (currently unused on the admission path). |
| `InvalidChainId(expected)` | `tx.chain_id` set and doesn't match the configured chain. |
| `NotEnoughBalance` | `cost_without_base_fee` exceeds sender balance, or sender absent from state. |
| `InvalidTxGasvalues` | `cost_without_base_fee` overflowed. |
| `InvalidPooledTxType(expected)` | `PooledTransactions` response type doesn't match the announced type. |
| `InvalidPooledTxSize` | `PooledTransactions` response size doesn't match the announced size. |
| `RequestedPooledTxNotFound` | A `PooledTransactions` response contains a transaction we didn't request. |
| `InvalidTxSender(_)` | Signature recovery failed. |
| `UnderpricedReplacement` | RBF fee bump below the configured percentage (via #6601). |
| *`ReplacementTypeMismatch`* (#6601) | RBF candidate is a different `Transaction` variant from the in-pool entry. |
| *`MaxPendingTxsPerAccountExceeded { count, limit }`* (#6603) | Per-sender cap would be exceeded by the new admission. |

## Known Limitations / Open Work

The mempool today is a single FIFO-evicting pool with the admission checks listed above. The following are explicit gaps on `main`:

- **No cumulative-balance check.** When a sender has multiple pending transactions, only the new one's cost is compared against the on-chain balance.
- **FIFO eviction is not tip-aware.** A high-tip transaction can be evicted to make room for a low-tip one. Heap-based or tip-aware eviction is open work.
- **No sweep tasks.** There is no periodic re-validation of pending transactions against current state (balance changes, nonce gaps, base-fee changes).
- **No first-class local-origin marker on main.** `--mempool.private` (via #6576) introduces routing between local-RPC and P2P entry points, but the broader concept of local-origin preference (e.g. eviction preference, propagation lag) is not yet implemented.
- **Single combined pool.** Blob and non-blob transactions share a single `transaction_pool`; per-type sub-pools and their accounting are not implemented. The type-change rejection in #6601 partly mitigates this for RBF specifically.
- **No dynamic per-sender accounting beyond slot count.** Per-sender gas / value accounting is not tracked.

These items are tracked separately from this document and will be folded in as work lands.

## Related Documentation

- [System Overview](./overview.md) — where the mempool sits inside the node.
- [Block Execution Pipeline](./block_execution.md) — how the payload builder consumes pending transactions.
