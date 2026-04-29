### Review: `implement-polygon` — Blockchain, RPC, Types, Config, Storage

**Scope:** 20 files reviewed across crates/blockchain/, crates/networking/rpc/, crates/common/types/, crates/common/config/, crates/storage/.

**Commits:** 120+ commits on `implement-polygon` branch (see `git log --oneline main...HEAD`).

---

#### Issues

**[CORRECTNESS] [critical]** `crates/networking/rpc/bor/mod.rs:151,183-213` — `bor_getRootHash` produces wrong Merkle root (3 independent divergences from Bor)

> The entire `compute_root_hash` function and its caller diverge from the Bor reference implementation in three fundamental ways:
>
> **(a) Wrong leaf values.** ethrex uses `keccak256(block_hash)` as each leaf (line 190-192). Bor uses `keccak256(appendBytes32(Number.Bytes(), Time.Bytes(), TxHash.Bytes(), ReceiptHash.Bytes()))` — four header fields, each right-padded to 32 bytes. See `/tmp/bor/consensus/bor/api.go:379-384`. These produce entirely different leaf hashes.
>
> **(b) Wrong tree structure.** ethrex promotes the last element as-is when the level has odd count (line 206-208). Bor pads the leaf array to `nextPowerOfTwo(length)` with zero-filled `[32]byte` entries (`api.go:356`, `merkle.go:14-28`), then uses `go-merkle` with `DisableHashLeaves: true`. For 3 blocks: Bor root = `keccak(keccak(h0||h1) || keccak(h2||ZERO))`, ethrex root = `keccak(keccak(h0||h1) || h2)`.
>
> **(c) Missing `MaxCheckpointLength` validation.** Bor rejects ranges > 2^15 blocks (`api.go:323`). ethrex has no upper bound — an attacker can request the full chain length, causing unbounded sequential storage reads.
>
> **Verified against:** `/tmp/bor/consensus/bor/api.go:280-400`, `/tmp/bor/consensus/bor/merkle.go:3-52`.
>
> **USER IMPACT:** When the Polygon checkpoint contract or Heimdall calls `bor_getRootHash`, the returned root hash will never match Bor nodes' values, making checkpoint verification impossible. Any checkpoint submitted using this node's root hash will be rejected.

**[CORRECTNESS] [critical]** `crates/blockchain/blockchain.rs:500-539` — Unconditional `warn!`-level balance dump fires on every Polygon block

> The block at line 501 `if matches!(self.options.r#type, BlockchainType::Polygon)` has **no mismatch guard** — it runs on every successfully executed Polygon block. It calls `tx.sender(&NativeCrypto)` (ecrecover) for every transaction (line 505), builds a `HashMap<Address, U256>` from all account updates (line 523-526), and emits one `warn!` log per key address (line 530-537). For a typical 100-tx Polygon block, this adds 100 ecrecover operations + ~5 warn! log lines per block, every 2 seconds.
>
> This is development debugging scaffolding that was never gated or removed. It belongs behind `cfg(debug_assertions)` or `tracing::debug!` at minimum.
>
> **USER IMPACT:** Every Polygon block in production produces spurious `warn!`-level log lines and wastes CPU on unnecessary ecrecover operations, flooding operator dashboards and making real warnings invisible.

**[CORRECTNESS] [critical]** `crates/storage/store.rs:1790-1809` — Unconditional `warn!`-level logging for every account state update

> Two `tracing::warn!` calls fire for **every** account update in `apply_account_updates_from_trie_batch`:
> - Line 1790: "Storage root update" — emits for every account with storage changes
> - Line 1801: "Account state inserted into trie" — emits for **every** account update, with full RLP hex encoding (`hex::encode(&encoded_account)`)
>
> On a typical Polygon block touching 200 accounts with 500 storage slots, this produces 700+ warn-level log lines per block, each including hex-encoded data. This is not gated on any error condition.
>
> **USER IMPACT:** Every block during sync floods logs with hundreds of warn-level messages including RLP hex dumps, making log storage expensive and real warnings invisible.

**[SECURITY] [important]** `crates/blockchain/blockchain.rs:3117-3121` — `polygon_pending_blocks` buffer has no eviction policy

> `buffer_polygon_pending_block` inserts blocks into an unbounded `HashMap<H256, Block>` keyed by `parent_hash` with no size cap, no TTL, and no eviction. The only removal path is `take_polygon_pending_block` (which requires the parent to be processed). A malicious peer can send `NewBlock` messages with distinct unknown parent hashes. Each block (~5-10 KB) creates a new entry. 100,000 spam blocks = ~500 MB-1 GB heap growth with no recovery mechanism.
>
> **USER IMPACT:** A malicious P2P peer can exhaust node memory by broadcasting blocks with non-existent parents, causing an OOM kill of the ethrex process.

**[CORRECTNESS] [important]** `crates/blockchain/blockchain.rs:3348-3371` — `warn!`-level debug dump on every sprint-start block

> The `StateSyncTransaction` debug dump at lines 3348-3371 fires on every block that contains a `StateSyncTransaction` (every sprint-start block — roughly every 16 blocks on Amoy, every 64 on mainnet). It calls `first_tx.encode_to_vec()` (full RLP encode), allocates a hex string character-by-character, and collects event IDs into a Vec. All at `warn!` level with no feature flag guard.
>
> **USER IMPACT:** Every sprint-start block emits spurious warn-level log messages with hex dumps of the StateSyncTransaction, adding noise to production logs.

**[ARCHITECTURE] [important]** `crates/common/config/networks.rs` — Ethereum L1 network variants removed from `PublicNetwork`

> The `PublicNetwork` enum lost `Mainnet`, `Holesky`, `Sepolia`, and `Hoodi` variants, replaced entirely by `Polygon` and `Amoy`. This means:
> - `Network::default()` returns `Polygon` instead of `Mainnet`
> - `TryFrom<u64>` fails for chain IDs 1, 17000, 11155111, 560048
> - `From<&str>` no longer recognizes `"mainnet"`, `"holesky"`, etc. (falls through to `GenesisPath`)
>
> The genesis files still exist in the repo, and users can pass `--network /path/to/genesis.json`, so this is not a complete loss of L1 support. However, it breaks the named-network convenience and changes the default behavior of the client. This is a merge-time concern — the branch should add Polygon/Amoy to the existing enum rather than replacing it when merging to main.
>
> **USER IMPACT:** Running this branch without `--network` defaults to Polygon instead of Ethereum mainnet. Passing `--network mainnet` creates a `GenesisPath("mainnet")` which fails if no file named "mainnet" exists.

**[STYLE] [important]** `crates/blockchain/blockchain.rs` (multiple sites) — Extensive debug logging left at `warn!` level throughout

> Beyond the specific findings above, the branch contains pervasive `warn!`-level diagnostic logging across the blockchain execution path:
> - Lines 428-483: Receipts root mismatch dump (per-receipt, per-log detail) — correctly error-gated but at `warn!` level
> - Lines 700-778: Pipeline receipts root mismatch dump — correctly error-gated but duplicates the above
> - Lines 3357-3371: StateSyncTransaction hex dump — unconditional on sprint-start blocks
> - Lines 500-539: Balance dump — unconditional on every block
>
> Additionally, `store.rs:1790-1809` has unconditional `warn!` for every storage/account update.
>
> All of these should be either removed, moved behind `cfg(debug_assertions)`, downgraded to `trace!`/`debug!`, or gated behind a `--polygon-debug` flag before merging.

---

#### Positive Observations

- `crates/common/types/polygon_fee_config.rs` — **Clean type design.** The `PolygonFeeConfig` struct clearly separates the three Polygon fee roles (burnt contract, coinbase, author) with accurate doc comments. Placement in `ethrex-common` is correct given the dependency graph (avoids circular dependency with `ethrex-polygon`).

- `crates/common/types/transaction.rs` — **Thorough StateSyncTransaction implementation.** The `StateSyncTransaction` type correctly handles RLP encoding/decoding, type byte `0x7f`, zero gas/value/nonce semantics, and canonical encoding. The test suite (12 tests) covers roundtrip encoding, type identification, and hash determinism.

- `crates/blockchain/blockchain.rs:execute_polygon_system_calls` — **Correct Bor Finalize order.** The function correctly executes commitSpan before commitState, matches Bor's `Finalize` ordering, properly collects logs into a single state sync receipt, and handles the post-Rio skip of commitSpan.

- `crates/networking/rpc/rpc.rs:699-704` — **Engine namespace correctly disabled for Polygon.** The `map_authrpc_requests` function returns `MethodNotFound` for engine_ namespace calls on Polygon networks, preventing CL-related RPCs from being served when there's no beacon chain.

- `crates/blockchain/blockchain.rs:apply_polygon_block_alloc` — **Correct block alloc implementation.** Matches Bor's `changeContractCodeIfNeeded` semantics: sets code unconditionally, sets balance only when zero.

---

#### Style Notes (lower priority)

- `crates/networking/rpc/bor/mod.rs:61` — `bor_getAuthor` formats the address as `format!("{signer:?}")` which uses Rust's `Debug` formatter producing `0x1234…abcd` — this matches the expected hex format for addresses, but `format!("{signer:#x}")` or `serde_json::to_value(signer)` would be more explicit.

- `crates/networking/rpc/bor/mod.rs:65-116` — Four stub RPC handlers (`bor_getSnapshot`, `bor_getSignersAtHash`, `bor_getCurrentValidators`, `bor_getCurrentProposer`) return `RpcErr::Internal` (JSON-RPC -32603) instead of `RpcErr::MethodNotFound` (-32601). Clients may retry -32603 errors thinking they're transient.

- `crates/networking/rpc/rpc.rs:905-907` — `is_polygon_network` uses magic numbers `137` and `80002`. Named constants `POLYGON_MAINNET_CHAIN_ID` and `AMOY_CHAIN_ID` exist in `networks.rs`, or `BlockchainType::Polygon` from `context.blockchain.options.r#type` could be used.

- `crates/blockchain/blockchain.rs` (4 sites) — The `PolygonFeeConfig` construction pattern (`recover_signer + bor_config_for_chain + PolygonFeeConfig { ... }`) is duplicated at lines ~375, ~1730, ~2385, ~2400. A helper function would eliminate duplication and centralize the `unwrap_or(coinbase)` fallback policy.

- `crates/storage/store.rs:986` — `rollback_latest_block_number` is defined but has zero call sites in the workspace. Dead code.

#### Unverified / Needs Human Check

- `crates/storage/store.rs:1421-1428` — **Canonical block hash double-write.** `store_block_updates` now writes to `CANONICAL_BLOCK_HASHES` (number→hash). On L1, this mapping is also set by `fork_choice_updated`. On Polygon (no Engine API), this is the only write path — correct. But if this code is reached on L1 (it's not gated on Polygon), it could conflict with or duplicate `fork_choice_updated` writes. Needs human verification of whether `store_block_updates` is called on L1 blocks. (Confidence: medium)

#### Findings Rejected During Verification

- **"block_in_place panics on current-thread runtime"** — `tokio::task::block_in_place()` in `verify_bor_header` does require a multi-thread runtime, but the ethrex binary always uses `#[tokio::main]` (multi-thread default). No `current_thread` usage found anywhere in the codebase. The panic path is structurally unreachable in all deployed configurations.

- **"Warn-level diagnostic logging at 428-483, 700-778 fires unconditionally"** — The receipts-root and gas-used diagnostic blocks at these lines ARE correctly gated behind mismatch conditions (`computed_root != expected_root`, `gas_used != header.gas_used`). They're at the wrong log level (`warn!` vs `debug!`) but they don't fire on every block. The unconditional balance dump at 500-539 is a separate, confirmed issue.

---

#### Summary
- 7 issues (3 critical, 3 important, 1 important/style)
- 5 positive observations
- 2 findings rejected as false positives
- Overall: The `bor_getRootHash` Merkle root computation is fundamentally wrong and must be rewritten to match Bor's leaf formula and tree structure. The pervasive `warn!`-level debugging scaffolding (unconditional balance dumps, storage update logging, sprint-boundary hex dumps) must be removed or downgraded before merge. The pending-blocks buffer needs eviction bounds. The core block validation, fee distribution, system call execution, and transaction type handling are well-implemented and match the Bor reference.
