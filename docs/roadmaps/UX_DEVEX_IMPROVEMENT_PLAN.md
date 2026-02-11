# ethrex UX/DevEx Improvement Plan

## Executive Summary

This plan identifies **50+ actionable improvements** across 4 categories to enhance the experience for node operators running ethrex in production. The analysis covers:

1. Understanding node state at a glance
2. Monitoring sync progress and health
3. Diagnosing issues quickly
4. Operating nodes with confidence
5. Configuring nodes correctly

**Current State:** ethrex has strong technical foundations but significant gaps in error handling, observability, and operator tooling that create friction.

**Estimated Total Effort:** 14-16 developer-weeks (10-12 week timeline for 2 developers)

---

## Key Findings by Category

| Category | Issues Found | Severity | Impact |
|----------|-------------|----------|--------|
| Error Handling | 150+ problems | Mixed (5 Critical, 2 High, rest Medium) | Node crashes, hours lost debugging |
| Node Operator UX | 15 gaps | High | Can't monitor/diagnose issues |
| Configuration | 21 missing env vars | Medium | Container deployment friction |
| Documentation | 4 major gaps | Medium | Onboarding delays |

---

## Category 1: Error Handling (Priority: CRITICAL)

### 1.1 Remove Production Panics

**Problem:** 7+ `panic!()` calls in non-initialization production code paths with no context. Node crashes are unrecoverable.

**Critical Locations:**

| File | Line | Context | Severity |
|------|------|---------|----------|
| `crates/storage/utils.rs` | 28 | Invalid ChainDataIndex casting | Critical |
| `crates/storage/utils.rs` | 58 | Invalid SnapStateIndex casting | Critical |
| `crates/storage/layering.rs` | 94 | State cycle detection | Critical |
| `crates/networking/p2p/sync/storage_healing.rs` | 444 | Node response validation | Critical |
| `crates/networking/p2p/sync/state_healing.rs` | 397 | Parent node existence check | Critical |
| `crates/networking/p2p/peer_handler.rs` | 1333 | Account hash zero check | High |
| `cmd/ethrex/cli.rs` | 862 | Database error on export ("Internal DB Error") | High |

**Note:** The `panic!` at `cmd/ethrex/initializers.rs:247-250` (dev mode without feature flag) is covered in Section 3.3 with other initialization panics.

**Note:** Panics in `crates/common/trie/node/extension.rs` (lines 367, 395) and `crates/common/trie/node/leaf.rs` (line 234) are in `#[test]` functions and are not production issues. The `unreachable!()` at `cmd/ethrex/initializers.rs:580` is covered in Section 1.2.

**Solution:** Convert panics to `Result` types with descriptive errors.

**Effort:** 3-4 days
**Breaking:** Yes (function signatures change)

---

### 1.2 Replace unreachable!() in Network Code

**Problem:** 40 `unreachable!()` calls in discovery and related code assume perfect protocol compliance. Network code should handle unexpected messages gracefully.

**DiscoveryV5 Peer Table** (`crates/networking/p2p/discv5/peer_table.rs`):
- Lines: 399, 416, 424, 432, 440, 449, 458, 471, 484, 500, 515, 536, 557, 573, 591, 603, 637, 645, 666
- All in message matching patterns for peer table operations

**DiscoveryV4 Peer Table** (`crates/networking/p2p/discv4/peer_table.rs`):
- Lines: 364, 381, 389, 397, 405, 414, 423, 436, 449, 464, 506, 522, 539, 551, 578, 586, 607
- Same pattern as V5

**Additional locations:**
- `cmd/ethrex/initializers.rs:580` - Genesis block missing after store
- `crates/networking/p2p/discv4/server.rs:621` - Unmatched peer table response
- `crates/blockchain/blockchain.rs:577` - Root node type assertion
- `crates/common/trie/verify_range.rs:285` - Trie node reference type

**Solution:** Replace with proper error variants or log warnings for unexpected messages.

**Effort:** 2-3 days
**Breaking:** No (internal)

---

### 1.3 Fix Context-Discarding Error Handling

**Problem:** 100+ `map_err(|_| ...)` calls discard the underlying error, losing root cause information.

**Storage Module** (`crates/storage/`):

| File | Lines | Issue |
|------|-------|-------|
| `backend/in_memory.rs` | 30, 89, 106, 122, 153, 168 | "Failed to acquire lock" - no read/write distinction |
| `store.rs` | 522, 914, 928, 942, 971, 1109, 1967, 2596 | "Invalid BlockNumber bytes" - repeated 8x |
| `store.rs` | 681, 707, 729, 778 | Account code cache lock - generic |
| `store.rs` | 2430, 2463, 2488, 2528, 2649, 2699, 2704, 2708, 2769, 2844, 2874, 2903 | Trie cache locks - 12 locations |

**Blockchain Module** (`crates/blockchain/`):

| File | Lines | Issue |
|------|-------|-------|
| `blockchain.rs` | 65 | "Failed to join task" |
| `blockchain.rs` | 114-115 | "Failed to convert payload" |
| `blockchain.rs` | 893, 1077, 1222 | "Parent state not found" - 3x |
| `blockchain.rs` | 904-905, 1086-1087, 1342-1343 | "Failed to lock state trie witness" - 3x |
| `blockchain.rs` | 914-915, 1233-1234 | "Failed to get root state node" - 2x |
| `blockchain.rs` | 1065-1066, 1330-1331 | "Failed to lock storage trie witness" - 2x |
| `blockchain.rs` | 1541-1542, 2264 | "Fee config lock poisoned" - 2x |
| `tracing.rs` | 76, 161-162 | "Unexpected Runtime Error" / "Tracing timeout" |
| `vm.rs` | 102 | "LockError" - generic |

**Solution:** Replace with typed error variants that preserve the original error:
```rust
// Before
.map_err(|_| StoreError::Custom("Failed to acquire lock".to_string()))

// After
.map_err(|e| StoreError::LockError {
    operation: "read account state",
    source: e.to_string(),
})
```

**Effort:** 2-3 days
**Breaking:** No

---

### 1.4 Improve Error Type Definitions

**Problem:** Error enums have variants with no context.

**Storage Layer** (`crates/storage/error.rs:8-9`):
```rust
#[error("DecodeError")]
DecodeError,  // No context about what failed to decode
```

**RLP Layer** (`crates/common/rlp/error.rs`):
```rust
#[error("InvalidLength")]
InvalidLength,  // No expected vs actual
#[error("MalformedData")]
MalformedData,  // Too generic
#[error("MalformedBoolean")]
MalformedBoolean,  // No context
#[error("UnexpectedList")]
UnexpectedList,  // No context about expected type
#[error("UnexpectedString")]
UnexpectedString,  // No context about expected type
```

**Networking Layer** (`crates/networking/p2p/rlpx/error.rs`):
```rust
#[error("No matching capabilities")]
NoMatchingCapabilities,  // Which capabilities were available?
#[error("Invalid message length")]
InvalidMessageLength,  // Expected vs actual?
#[error("Invalid peer id")]
InvalidPeerId,  // What makes it invalid?
```

**Existing TODOs confirm this is known:**
- `crates/storage/error.rs:5` - `// TODO improve errors`
- `crates/common/rlp/error.rs:3, 24` - `// TODO: improve errors`
- `crates/networking/p2p/rlpx/error.rs:21` - `// TODO improve errors`

**Solution:** Add context to error variants:
```rust
#[error("Failed to decode {item_type}: {reason}")]
DecodeError {
    item_type: &'static str,
    reason: String,
}
```

**Effort:** 2-3 days
**Breaking:** Yes (error enum changes)

---

### 1.5 Fix RPC Error Mapping Bug

**Problem:** `InvalidPayloadAttributes` returns wrong message "Invalid forkchoice state" (copy-paste error).

**File:** `crates/networking/rpc/utils.rs:150-154`

**Effort:** 30 minutes
**Breaking:** No

---

### 1.6 Address Known Error Handling TODOs

**Problem:** 11 TODO comments indicate incomplete error handling in critical paths.

| File | Line | TODO |
|------|------|------|
| `crates/networking/rpc/utils.rs` | 170-171 | "Actually return different errors for each case" |
| `crates/networking/p2p/sync/storage_healing.rs` | 296 | "if we have a store error we should stop" |
| `crates/networking/p2p/sync/storage_healing.rs` | 369 | "add error handling" |
| `crates/networking/p2p/sync/state_healing.rs` | 256 | "check errors to determine whether current block is stale" |
| `crates/networking/p2p/sync/state_healing.rs` | 265 | "add error handling" |
| `crates/networking/p2p/types.rs` | 324-325 | "decode as optional to ignore errors, should return error" |
| `crates/networking/p2p/peer_handler.rs` | 103 | "Better error handling" |
| `crates/networking/p2p/peer_handler.rs` | 692 | "check the error type and handle it properly" |
| `crates/networking/p2p/rlpx/connection/server.rs` | 775 | "build proper matching between error types and disconnect reasons" |
| `crates/common/trie/node/leaf.rs` | 96 | "handle override case (error?)" |
| `crates/common/trie/node/branch.rs` | 140 | "handle override case (error?)" |

**Solution:** Address each TODO systematically.

**Effort:** 3-4 days
**Breaking:** Varies

---

### 1.7 Improve Logging Consistency

**Problem:** Log messages often lack context:
```rust
warn!("Failed to update latest fcu head for syncing")  // Why?
error!("No account storage found, this shouldn't happen")  // Which account?
```

**Solution:** Add structured fields to all log statements:
```rust
warn!(err = ?e, block = %block_hash, "Failed to update FCU head");
error!(account = %address, block = %block_num, "Account storage not found");
```

**Effort:** 2-3 days
**Breaking:** No

---

## Category 2: Node Operator UX (Priority: HIGH)

### 2.1 Enhance Existing Startup Banner with Configuration Summary

**Opportunity:** ethrex already has a startup banner with clean INFO-level log output. The existing format is visually appealing and should be preserved. The improvement is to extend the banner with additional configuration details so operators can confirm their setup at a glance.

**Current** (`cmd/ethrex/ethrex.rs:142`):
```
INFO ethrex version: ethrex/0.1.0
```

**Proposed Enhancement:** Add configuration details as additional INFO-level log lines following the existing style:
```
INFO ethrex version: ethrex/0.1.0
INFO Network:    Mainnet (chain ID 1)
INFO Datadir:    ~/.ethrex
INFO Sync Mode:  Snap
INFO HTTP RPC:   http://0.0.0.0:8545
INFO Auth RPC:   http://127.0.0.1:8551
INFO Metrics:    http://0.0.0.0:9090
INFO P2P:        0.0.0.0:30303
```

**Files:** `cmd/ethrex/ethrex.rs`, `cmd/ethrex/initializers.rs`

**Effort:** 4-6 hours
**Breaking:** No

---

### 2.2 Add CLI Status Command

**Problem:** No way to query node status from CLI. Must use RPC HTTP calls or parse logs.

**Current CLI Subcommands** (`cmd/ethrex/cli.rs:382-460`):
- `removedb` - Remove database
- `import` - Import blocks from file
- `import-bench` - Benchmark import
- `export` - Export blocks to file
- `compute-state-root` - Compute genesis state root

No `status` command exists.

**Solution:** Add `ethrex status` command that queries local RPC:
```
Sync Status:    SYNCING (45.2%)
Current Block:  8,234,567
Target Block:   18,200,000
Sync Phase:     Downloading Storage Ranges
Peers:          12 / 50 (8 snap-capable)
Database:       287 GB
Uptime:         2h 14m
```

**Implementation:** Query `eth_syncing`, `admin_peers`, `admin_nodeInfo` and format output.

**Files:** `cmd/ethrex/cli.rs` (new subcommand)

**Effort:** 2-3 days
**Breaking:** No

---

### 2.3 Expose Sync Progress as Prometheus Metrics

**Problem:** Sync progress exists only in console logs, not in Prometheus. Operators can't build dashboards or alerts.

**Currently Tracked Internally** (`crates/networking/p2p/metrics.rs:57-97`) but NOT exposed to Prometheus:

| Internal Field | Type | Description |
|----------------|------|-------------|
| `sync_head_block` | AtomicU64 | Target sync block |
| `sync_head_hash` | H256 | Target sync hash |
| `current_step` | Enum | Current sync phase |
| `downloaded_account_tries` | AtomicU64 | Account tries downloaded |
| `account_tries_inserted` | AtomicU64 | Account tries in DB |
| `storage_leaves_downloaded` | IntCounter | Storage leaves downloaded |
| `storage_leaves_inserted` | IntCounter | Storage leaves in DB |
| `global_state_trie_leafs_healed` | AtomicU64 | State trie leaves healed |
| `global_storage_tries_leafs_healed` | AtomicU64 | Storage trie leaves healed |
| `bytecodes_to_download` | AtomicU64 | Expected bytecodes |
| `downloaded_bytecodes` | AtomicU64 | Bytecodes downloaded |

**Current Sync Phases** (`CurrentStepValue` enum, lines 117-128):
- `DownloadingHeaders`
- `RequestingAccountRanges` / `InsertingAccountRanges`
- `RequestingStorageRanges` / `InsertingStorageRanges`
- `RequestingBytecodes`
- `HealingState` / `HealingStorage`

**Confirmed by** `docs/internal/l1/metrics_coverage_gap_analysis.md`:
> "Current counters live only in logs via `periodically_show_peer_stats_during_syncing`"

**Solution:** Add to `crates/blockchain/metrics/`:
```rust
ethrex_sync_stage: StringGauge,           // "headers", "accounts", etc.
ethrex_sync_target_block: IntGauge,
ethrex_sync_progress_percent: Gauge,
ethrex_sync_accounts_downloaded: IntGauge,
ethrex_sync_storage_downloaded: IntGauge,
ethrex_sync_bytecodes_downloaded: IntGauge,
ethrex_sync_state_healed: IntGauge,
ethrex_sync_fkv_calculation_progress: Gauge,  // FKV (Flat Key-Value) calculation progress
```

**Files:**
- `crates/blockchain/metrics/` (new sync.rs)
- `crates/networking/p2p/network.rs` (wire updates)

**Effort:** 2-3 days
**Breaking:** No

---

### 2.4 Wire L1 Mempool Metrics

**Problem:** Mempool metrics exist but L1 never sets them. `mempool_tx_count` always shows 0.

**Existing Metrics** (`crates/blockchain/metrics/transactions.rs:28-118`):
- `mempool_tx_count{type}` - IntGaugeVec with "blob"/"regular" labels
- `transactions_per_second` - Gauge

These are only called from L2 sequencer (`crates/l2/sequencer/`), not L1.

**Solution:** Call `METRICS_TX.set_mempool_tx_count()` when transactions are added/removed in L1 mempool.

**Files:** `crates/blockchain/mempool.rs`

**Effort:** 4-6 hours
**Breaking:** No

---

### 2.5 Add Block Import Failure Metrics

**Problem:** No metrics for failed block imports, reorg depth, or chain reversions.

**Solution:** Add metrics:
```rust
ethrex_block_import_failures: IntCounterVec,  // by reason
ethrex_reorg_count: IntCounter,
ethrex_reorg_depth: Histogram,
```

**Files:** `crates/blockchain/metrics/blocks.rs`

**Effort:** 1 day
**Breaking:** No

---

### 2.6 Add Health Status Metrics

**Problem:** Critical state not easily queryable via Prometheus.

**Solution:** Add metrics:
```rust
ethrex_node_synced: IntGauge,              // 0 or 1
ethrex_sync_mode: StringGauge,             // "full", "snap"
ethrex_latest_block_base_fee: Gauge,
ethrex_finalized_block: IntGauge,
ethrex_safe_block: IntGauge,
```

**Files:** `crates/blockchain/metrics/blocks.rs`

**Effort:** 4-6 hours
**Breaking:** No

---

### 2.7 Improve Shutdown Messages

**Problem:** Minimal shutdown feedback.

**Current** (`cmd/ethrex/ethrex.rs:41-50`):
```
INFO: Server shut down started...
INFO: Storing config at {path}...
INFO: Server shutting down!
```

**Solution:** Display shutdown summary:
```
INFO: ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
INFO:   GRACEFUL SHUTDOWN
INFO: ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
INFO: Disconnecting 12 peers
INFO: Saving node configuration
INFO: Closing database
INFO: 
INFO: Final State:
INFO:   Last Block:      #18,523,456
INFO:   Session Time:    2h 14m 30s
INFO:   Blocks Synced:   1,234,567
INFO: ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**Files:** `cmd/ethrex/ethrex.rs`

**Effort:** 4-6 hours
**Breaking:** No

---

### 2.8 Add Database Health Metrics

**Problem:** Only `datadir_size_bytes` is tracked (Linux only). Missing I/O and performance metrics.

**Current** (`crates/blockchain/metrics/process.rs:29-57`):
- `datadir_size_bytes` - Requires datadir path, Linux process collector

**Solution:** Add metrics:
```rust
ethrex_db_read_bytes_total: Counter,
ethrex_db_write_bytes_total: Counter,
ethrex_db_read_latency_seconds: Histogram,
ethrex_db_write_latency_seconds: Histogram,
```

**Caution:** Latency histograms on every DB read/write add timing overhead on a hot path. Consider sampling (e.g., 1-in-100 operations) or making latency histograms opt-in via a `--metrics.detailed` flag. Byte counters are low-overhead and safe to always enable.

**Files:** `crates/storage/store.rs`, `crates/blockchain/metrics/`

**Effort:** 2-3 days
**Breaking:** No

---

### 2.9 Add Peer Health Metrics

**Problem:** Limited peer visibility.

**Current** (`crates/blockchain/metrics/p2p.rs:24-62`):
- `ethrex_p2p_peer_count` - Total peers
- `ethrex_p2p_peer_clients` - By client type
- `ethrex_p2p_disconnections` - By reason and client

**Missing:**
```rust
ethrex_p2p_connections_total: CounterVec,    // by direction (in/out)
ethrex_p2p_bytes_received_total: Counter,
ethrex_p2p_bytes_sent_total: Counter,
ethrex_p2p_snap_capable_peers: Gauge,
ethrex_p2p_handshake_failures: CounterVec,  // by reason
```

**Files:** `crates/blockchain/metrics/p2p.rs`

**Effort:** 1-2 days
**Breaking:** No

---

### 2.10 Add Network Diagnostics Command

**Problem:** Hard to diagnose P2P connectivity issues.

**Solution:** Add `ethrex net diagnose` command:
```
Network Diagnostics
━━━━━━━━━━━━━━━━━━━
P2P Port (30303):     Listening
Discovery Port:       Listening
Bootnodes Reachable:  Connected to 3/4 bootnodes
NAT Type:             Symmetric NAT (may limit inbound)
Public IP:            203.0.113.42
Local ENR:            enr:-...
```

**Files:** `cmd/ethrex/cli.rs` (new subcommand)

**Effort:** 2-3 days
**Breaking:** No

---

### 2.11 Add Database Management Commands

**Problem:** Limited database tooling. Only `removedb` exists.

**Solution:** Add commands:
- `ethrex db stats` - Show database statistics (size by table, record counts)
- `ethrex db compact` - Compact database (RocksDB)
- `ethrex db verify` - Verify database integrity

**Files:** `cmd/ethrex/cli.rs` (new subcommands)

**Effort:** 1 week
**Breaking:** No

---

### 2.12 Provide Grafana Dashboard Templates

**Problem:** Metrics exist but dashboards are incomplete.

**Current:** `metrics/provisioning/grafana/dashboards/common_dashboards/ethrex_l1_perf.json` exists but doesn't include sync progress.

**Solution:** Provide additional Grafana dashboard JSON files:
- Sync progress dashboard (using new sync metrics)
- P2P network dashboard
- Error rates dashboard

**Files:** `metrics/provisioning/grafana/dashboards/`

**Effort:** 2-3 days
**Breaking:** No

---

### 2.13 Provide Systemd Service Files

**Problem:** No ready-to-use service files for production deployment.

**Solution:** Provide:
```ini
# contrib/systemd/ethrex.service
[Unit]
Description=ethrex Ethereum Execution Client
After=network.target

[Service]
Type=simple
User=ethrex
ExecStart=/usr/local/bin/ethrex --network mainnet --datadir /var/lib/ethrex
Restart=on-failure
RestartSec=5
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
```

**Files:** New `contrib/systemd/ethrex.service`

**Effort:** 4-6 hours
**Breaking:** No

---

### 2.14 Interactive REPL for Node State Inspection

**Problem:** Operators have no interactive way to inspect node state. Diagnosing issues requires crafting RPC calls manually or writing scripts. Other clients (e.g., geth's JavaScript console) provide a REPL that lets operators query accounts, storage, blocks, and chain state interactively.

**Solution:** Provide a REPL (read-eval-print loop) that works both locally (connecting to a running node via IPC or RPC) and remotely (connecting over HTTP/WS). The REPL should support:
- Querying account balances, nonces, and code
- Inspecting block headers, transactions, and receipts
- Browsing storage slots
- Checking sync status and peer info
- Scriptable via command history and piping

**Effort:** 1-2 weeks
**Breaking:** No

---

### 2.15 State and Block Composition Analysis

**Problem:** Operators and researchers have no built-in way to analyze the composition of on-chain state or blocks. Understanding what percentage of state consists of ERC20 tokens, bridge contracts, DeFi protocols, etc. — or what types of transactions fill blocks — requires external tooling.

**Solution:** Provide a tool or command (e.g., `ethrex analyze`) that generates reports on:
- State composition: breakdown of accounts by type (EOA vs contract), classification of known contract types (ERC20, ERC721, bridge, DeFi, etc.)
- Block composition: transaction types, gas usage by category, blob usage
- Output formats: human-readable summary, CSV, or JSON for further analysis

**Effort:** 1-2 weeks
**Breaking:** No

---

## Category 3: Configuration and Setup (Priority: MEDIUM)

### 3.1 Expand Environment Variable Coverage

**Problem:** 21 CLI options have no environment variable support, making containerized deployment harder.

**Missing Environment Variables:**

| CLI Option | Location | Impact | Priority |
|-----------|----------|--------|----------|
| `--bootnodes` | cli.rs:64 | Can't override in containers | High |
| `--syncmode` | cli.rs:85 | Can't select sync mode | High |
| `--dev` | cli.rs:109-115 | Can't enable dev mode | High |
| `--authrpc.addr` | cli.rs:195-201 | Auth RPC binding | High |
| `--authrpc.port` | cli.rs:203-209 | Auth RPC port | High |
| `--authrpc.jwtsecret` | cli.rs:211-217 | JWT file path | High |
| `--p2p.addr` | cli.rs:221-226 | P2P address | Medium |
| `--p2p.port` | cli.rs:228-234 | P2P port | Medium |
| `--p2p.disabled` | cli.rs:219 | Disable P2P | Medium |
| `--p2p.target-peers` | cli.rs:252-258 | Peer target | Medium |
| `--p2p.lookup-interval` | cli.rs:260-266 | Discovery timing | Low |
| `--p2p.tx-broadcasting-interval` | cli.rs:244-250 | Broadcast timing | Low |
| `--discovery.port` | cli.rs:236-242 | Discovery port | Medium |
| `--builder.extra-data` | cli.rs:268-274 | Block extra data | Low |
| `--builder.gas-limit` | cli.rs:276-282 | Gas limit | Medium |
| `--builder.max-blobs` | cli.rs:284-290 | Blob limit | Low |
| `--mempool.maxsize` | cli.rs:141-147 | Mempool size | Medium |
| `--log.dir` | cli.rs:134-139 | Log directory | Medium |
| `--log.color` | cli.rs:126-132 | Log coloring | Low |
| `--metrics.addr` | cli.rs:87-92 | Metrics binding | Medium |
| `--precompute-witnesses` | cli.rs:292-298 | Witness generation | Low |

**Already Supported** (for reference):
- `ETHREX_NETWORK`, `ETHREX_DATADIR`, `ETHREX_HTTP_ADDR`, `ETHREX_HTTP_PORT`
- `ETHREX_ENABLE_WS`, `ETHREX_WS_ADDR`, `ETHREX_WS_PORT`
- `ETHREX_METRICS_PORT`, `ETHREX_LOG_LEVEL`

**Solution:** Add env vars for High/Medium priority CLI options using the existing `ETHREX_<OPTION>` pattern. Low-priority options (e.g., `--builder.extra-data`, `--p2p.lookup-interval`) can be deferred.

**Files:** `cmd/ethrex/cli.rs`

**Effort:** 1-2 days
**Breaking:** No

---

### 3.2 Improve Network Detection Errors

**Problem:** Unknown network names silently become file paths. Typo "hoddi" instead of "hoodi" fails with confusing file-not-found error.

**Current Behavior** (`crates/common/config/networks.rs:53-64`):
```rust
impl From<&str> for Network {
    fn from(s: &str) -> Self {
        match s {
            "hoodi" => PublicNetwork(PublicNetwork::Hoodi),
            "holesky" => PublicNetwork(PublicNetwork::Holesky),
            "mainnet" => PublicNetwork(PublicNetwork::Mainnet),
            "sepolia" => PublicNetwork(PublicNetwork::Sepolia),
            _ => GenesisPath(PathBuf::from(s)),  // Silent fallback!
        }
    }
}
```

**Solution:** Validate network names and provide suggestions:
```
Error: Unknown network 'hoddi'

Did you mean one of these?
  - hoodi
  - holesky

Available networks: mainnet, sepolia, holesky, hoodi

Or provide a path to a genesis.json file.
```

**Files:** `crates/common/config/networks.rs`

**Effort:** 4 hours
**Breaking:** No

---

### 3.3 Replace Initialization Panics with Results

**Problem:** Initialization code panics on errors instead of returning Results.

**Panic Locations:**

| File | Line | Trigger | Message |
|------|------|---------|---------|
| `initializers.rs` | 80 | Log dir creation fails | expect() |
| `initializers.rs` | 94 | Log file open fails | expect() |
| `initializers.rs` | 112 | Tracing subscriber setup | expect() |
| `initializers.rs` | 247-250 | Dev mode without feature | panic!("Build with dev feature") |
| `initializers.rs` | 333 | Secret key parsing | expect() |
| `initializers.rs` | 340, 344 | Key file I/O | expect() |
| `initializers.rs` | 351, 355, 358 | Port parsing | expect() |
| `initializers.rs` | 399, 404, 409 | Socket address parsing | expect() |
| `initializers.rs` | 417 | SYNC_BLOCK_NUM env parse | expect() |
| `utils.rs` | 51, 54, 57 | JWT file I/O | expect() |
| `utils.rs` | 69-70 | Chain file I/O | expect() |
| `utils.rs` | 96 | Home directory detection | expect() |
| `utils.rs` | 109 | Non-directory datadir | panic!("not a directory") |
| `utils.rs` | 112 | Datadir creation | expect() |

**Solution:** Convert to `Result` types with helpful error messages:
```rust
// Before
let port: u16 = port_str.parse().expect("Invalid port");

// After
let port: u16 = port_str.parse()
    .map_err(|e| ConfigError::InvalidPort {
        value: port_str.to_string(),
        hint: "Port must be a number between 1-65535",
        source: e.to_string(),
    })?;
```

**Files:**
- `cmd/ethrex/utils.rs`
- `cmd/ethrex/initializers.rs`

**Effort:** 1-2 days
**Breaking:** No (internal)

---

### 3.4 Add Genesis Validation

**Problem:** Malformed genesis files cause cryptic errors later during block execution.

**Current Validation** (`crates/common/types/genesis.rs:70-100`):
- Checks post-merge fork configuration (but only warns, doesn't reject)
- Checks blob schedule presence (but only warns)
- JSON schema validation via serde

**Note:** Known public networks already embed genesis via `include_str!` and `Store::add_initial_state()` verifies hash consistency against the database. Genesis validation is primarily useful for **custom genesis files** provided via `--network <path>`.

**Missing Validation (for custom genesis files):**
- Chain ID consistency check
- Gas limit sanity check (e.g., > 0, < reasonable max)
- Account state (`alloc`) validation
- Extra data size validation
- Block parameter validation (nonce, difficulty)
- Cross-fork compatibility checks

**Solution:** Add validation for custom genesis files:
```rust
pub fn validate_genesis(genesis: &Genesis) -> Result<(), GenesisError> {
    if genesis.gas_limit == 0 {
        return Err(GenesisError::InvalidGasLimit("Gas limit cannot be 0"));
    }
    // ... more checks for custom genesis files
}
```

**Files:** `crates/common/types/genesis.rs`

**Effort:** 2 days
**Breaking:** No

---

### 3.5 Create .env.example

**Problem:** Environment variables are scattered across CLI help. No single reference.

**Solution:** Create `.env.example` in repository root:
```bash
# ethrex Environment Variables
# Copy to .env and customize

# Network Configuration
ETHREX_NETWORK=mainnet
ETHREX_DATADIR=~/.ethrex

# RPC Configuration
ETHREX_HTTP_ADDR=0.0.0.0
ETHREX_HTTP_PORT=8545
ETHREX_ENABLE_WS=false
ETHREX_WS_ADDR=0.0.0.0
ETHREX_WS_PORT=8546

# Metrics
ETHREX_METRICS_PORT=9090

# Logging
ETHREX_LOG_LEVEL=info
```

**Effort:** 2 hours
**Breaking:** No

---

### 3.6 Add HTTP Health Endpoint and Docker Health Checks

**Problem:** `docker-compose.yml` has no health checks, and the RPC server has no `/health` endpoint to check against.

**Solution (Part 1 — Health Endpoint):** Add a `/health` endpoint to the RPC server that returns HTTP 200 when the node is running (and optionally indicates sync status in the response body). This is a prerequisite for Docker health checks and load balancer integration.

**Files:** `crates/networking/rpc/` (new handler)

**Solution (Part 2 — Docker Health Checks):** Add healthcheck configuration:
```yaml
services:
  ethrex:
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8545/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 60s
```

**Files:** `tooling/sync/docker-compose.yml`, `docker-compose.yaml`

**Effort:** 1 day (endpoint + compose config)
**Breaking:** No

---

### 3.7 Add Log Rotation Configuration

**Problem:** No built-in log rotation. Logs can fill disk.

**Solution:**
- Add `--log.max-size` option (e.g., 100MB)
- Add `--log.max-files` option (e.g., 5 rotated files)
- Support structured JSON output (`--log.format json`)
- Document integration with logrotate/journald

**Files:** `cmd/ethrex/initializers.rs`, `cmd/ethrex/cli.rs`

**Effort:** 2 days
**Breaking:** No

---

### 3.8 Add Nix Support

**Problem:** No Nix flake or derivation exists for building ethrex. Developers using NixOS or nix-based workflows cannot easily build, develop, or deploy ethrex. This creates friction for a significant portion of the systems programming community and blocks reproducible build guarantees.

**Current State:** Building ethrex requires manually installing Rust toolchain, system dependencies (e.g., libclang for RocksDB, openssl), and managing feature flags. There is no `flake.nix`, `default.nix`, or `shell.nix` in the repository.

**Solution:** Add Nix flake support with:

1. **`flake.nix`** at repository root providing:
   - `packages.default` — Build ethrex with default features
   - `packages.ethrex-dev` — Build with `dev` feature enabled
   - `devShells.default` — Development shell with Rust toolchain, cargo, clippy, rustfmt, and system dependencies (libclang, openssl, pkg-config)

2. **Development shell** should include:
   - Rust toolchain (pinned via `rust-toolchain.toml` or oxalica overlay)
   - System libraries required by RocksDB (libclang, snappy, lz4, zstd)
   - OpenSSL and pkg-config
   - Optional: cargo-nextest, cargo-audit, and other dev tools

3. **NixOS module** (stretch goal):
   - Systemd service configuration via NixOS module
   - Declarative configuration for network, datadir, RPC settings
   - Integrates with Section 2.13 (Systemd Service Files)

**Example usage:**
```bash
# Build ethrex
nix build

# Enter development shell
nix develop

# Run directly
nix run .#ethrex -- --network mainnet

# On NixOS (stretch goal)
services.ethrex = {
  enable = true;
  network = "mainnet";
  datadir = "/var/lib/ethrex";
  http.port = 8545;
};
```

**Files:**
- New `flake.nix` at repository root
- New `flake.lock` (auto-generated)
- New `nix/` directory for module and overlays (if NixOS module is pursued)

**Effort:** 3-4 days (flake + dev shell), 2-3 days additional for NixOS module
**Breaking:** No

---

## Category 4: Documentation (Priority: CRITICAL)

### 4.1 Storage/Database API Reference

**Problem:** No documentation on how to query state, read accounts, or access the trie.

**Current:** `docs/developers/l1/storage_api.md` covers:
- Three access types (StorageReadView, StorageWriteBatch, StorageLockedView)
- Minimal interface philosophy

**Missing:**
- Schema documentation (what data is stored where)
- Code examples for common queries
- Performance characteristics
- Recovery procedures

**Solution:** Expand documentation with query examples and schema docs.

**Effort:** 2 days
**Files affected:** `docs/developers/l1/storage_api.md`

---

### 4.2 Production Deployment Guide

**Problem:** Multiple deployment paths documented in fragments. No unified production checklist.

**Current Docs:**
- `docs/l1/running/` - Basic L1 setup
- `docs/l2/deployment/` - L2 deployment modes
- `docs/getting-started/hardware_requirements.md` - Hardware specs

**Missing:**
- Hardware sizing recommendations (detailed by use case)
- Network configuration guide (firewalls, ports)
- Database tuning guide (RocksDB options)
- Monitoring and alerting setup (beyond basic Prometheus)
- HA/failover guidance
- Backup/restore procedures
- Performance tuning guide

**Solution:** Create comprehensive deployment guide.

**Effort:** 3 days
**Files affected:** New `docs/deployment/production-guide.md`

---

### 4.3 Crate-Level README Files

**Problem:** Critical crates have no README.

**Missing READMEs:**

| Crate | Purpose | Priority |
|-------|---------|----------|
| `crates/common/` | Common types and utilities | High |
| `crates/common/rlp/` | RLP encoding | Medium |
| `crates/vm/` | VM top-level | High |
| `crates/l2/` | L2 stack (entire directory) | Critical |
| `crates/l2/prover/` | Prover coordination | Critical |
| `crates/l2/storage/` | L2 storage | High |
| `crates/l2/common/` | L2 common utilities | Medium |

**Have READMEs (for reference):**
- `crates/blockchain/`, `crates/storage/`, `crates/networking/p2p/`, `crates/networking/rpc/`
- `crates/vm/levm/`, `crates/common/trie/`, `crates/guest-program/`

**Solution:** Add README.md to each major crate.

**Effort:** 2-3 days
**Files affected:** 7+ new README files

---

### 4.4 Troubleshooting Guide

**Problem:** No documentation for common operator issues.

**Solution:** Create guide covering:
- Node won't start (common causes)
- Sync stuck (diagnosis steps)
- High memory usage
- Peer connection issues
- Database corruption recovery
- Performance tuning

**Effort:** 2 days
**Files affected:** New `docs/operators/troubleshooting.md`

---

## Implementation Roadmap

> **Team:** 2 developers working in parallel
> **Timeline:** 10-12 weeks (includes buffer for review cycles and testing)
> **Priority:** Stability first, then sync monitoring + container deployment

---

### Parallel Tracks Overview

| Week | Track A (Stability) | Track B (Ops & Config) |
|------|---------------------|------------------------|
| 1 | 1.1 Remove Panics | 2.3 Sync Progress Metrics |
| 2 | 1.1 continued + 1.4 Error Types + 1.5 RPC bug | 2.3 continued + 3.5 .env.example |
| 3 | 1.3 Context-Discarding Errors | 3.1 Env Var Coverage |
| 4 | 1.2 Replace unreachable!() | 2.6 Health Status Metrics + 2.4 Mempool |
| 5 | 1.6 Address Error TODOs | 2.1 Startup Banner + 2.7 Shutdown + 4.4 Troubleshooting |
| 6 | 1.7 Improve Logging | 2.2 CLI Status Command |
| 7 | 3.3 Replace Init Panics | 3.6 Health Endpoint + Docker + 3.2 Network Detection |
| 8 | (buffer / stretch goals) | 4.2 Deployment Guide |
| 9-10 | Dashboards, docs & polish | Dashboards, docs & polish |

---

### Week 1-2: Foundation (Batch All Breaking Error Changes)

**Track A - Stability:**
| Item | Effort | Why First |
|------|--------|-----------|
| **1.1 Remove Production Panics** | 3-4 days | Primary goal - eliminate crashes |
| **1.4 Improve Error Types** | 2-3 days | Batch with 1.1 to consolidate breaking changes |
| **1.5 Fix RPC Error Mapping Bug** | 30 min | Quick win while in error code |

**Track B - Sync Visibility:**
| Item | Effort | Why First |
|------|--------|-----------|
| **2.3 Sync Progress Metrics** | 2-3 days | Top pain point - operators flying blind |
| **3.5 Create .env.example** | 2 hours | Quick win for container users |

**Week 1-2 Deliverables:**
- Zero panics in critical sync/storage paths
- Error enums have context fields (single wave of breaking changes)
- Prometheus metrics: `ethrex_sync_stage`, `ethrex_sync_target_block`, `ethrex_sync_progress_percent`
- `.env.example` file in repo root

---

### Week 3-4: Error Context + Config

**Track A - Stability:**
| Item | Effort | Why Now |
|------|--------|---------|
| **1.3 Fix Context-Discarding Errors** | 2-3 days | Debugging experience |
| **1.2 Replace unreachable!()** | 2-3 days | Network code stability (verify each arm first) |

**Track B - Container Deployment:**
| Item | Effort | Why Now |
|------|--------|---------|
| **3.1 Env Var Coverage (High/Medium)** | 1-2 days | Container deployment blocker |
| **2.6 Health Status Metrics** | 4-6 hours | Operators need synced/not-synced indicator |
| **2.4 Wire L1 Mempool Metrics** | 4-6 hours | Quick win, metrics already exist |

**Week 3-4 Deliverables:**
- High/Medium priority CLI options have env var equivalents (~15-20 options)
- Health metrics: `ethrex_node_synced`, `ethrex_sync_mode`
- `unreachable!()` arms audited; those reachable via malformed input replaced with error handling
- Errors include root cause information

---

### Week 5-6: Error TODOs + Operator UX + Troubleshooting

**Track A - Stability:**
| Item | Effort | Why Now |
|------|--------|---------|
| **1.6 Address Error TODOs** | 3-4 days | Close known gaps |
| **1.7 Improve Logging** | 2-3 days | Structured log fields |

**Track B - Operator Experience + Critical Docs:**
| Item | Effort | Why Now |
|------|--------|---------|
| **2.1 Enhance Startup Banner** | 4-6 hours | Operators see config at launch |
| **2.7 Shutdown Messages** | 4-6 hours | Clean shutdown feedback |
| **2.2 CLI Status Command** | 2-3 days | Query node without RPC tools |
| **4.4 Troubleshooting Guide** | 2 days | Critical - helps operators |

**Week 5-6 Deliverables:**
- `ethrex status` command works
- Startup shows network, datadir, ports in banner
- Shutdown shows session summary
- All known error TODOs addressed
- Logs have structured fields
- Troubleshooting guide for common operator issues

---

### Week 7-8: Init Stability + Monitoring + Deployment Guide

**Track A - Stability:**
| Item | Effort | Why Now |
|------|--------|---------|
| **3.3 Replace Init Panics** | 1-2 days | Graceful startup failures |

**Track B - Monitoring + Critical Docs:**
| Item | Effort | Why Now |
|------|--------|---------|
| **3.6 Health Endpoint + Docker Health Checks** | 1 day | Container orchestration (endpoint first, then compose) |
| **3.2 Improve Network Detection** | 4 hours | Better error on typos |
| **4.2 Production Deployment Guide** | 3 days | Critical - needed for adoption |

**Week 7-8 Deliverables:**
- `/health` endpoint available on RPC server
- Docker health checks in compose files
- Startup errors don't panic, return Results
- Production deployment guide with sizing, tuning, and HA guidance

---

### Week 9-10: Dashboards, Docs & Polish

**Both Tracks - Documentation & Monitoring:**
| Item | Effort | Priority |
|------|--------|----------|
| **2.12 Grafana Dashboards** | 2-3 days | Medium - visualize new metrics |
| **2.13 Systemd Service Files** | 4-6 hours | Medium - easy production setup |
| **4.3 Crate-Level READMEs** | 2-3 days | Medium - developer onboarding |

**Stretch Goals (if time permits):**
| Item | Effort | Impact |
|------|--------|--------|
| 2.5 Block Import Failure Metrics | 1 day | Medium |
| 2.8 Database Health Metrics | 2-3 days | Medium |
| 2.9 Peer Health Metrics | 1-2 days | Medium |
| 3.4 Genesis Validation | 2 days | Medium |
| 3.7 Log Rotation | 2 days | Medium |
| 3.8 Nix Support (flake + dev shell) | 3-4 days | Medium |

---

### Deferred Items (Future Work)

These items are valuable but lower priority given current goals:

| Item | Effort | Reason Deferred |
|------|--------|-----------------|
| 2.10 Network Diagnostics Command | 2-3 days | Nice-to-have, not blocking |
| 2.11 Database Management Commands | 1 week | Large effort, can wait |
| 2.14 Interactive REPL for Node State Inspection | 1-2 weeks | Large effort, nice-to-have |
| 2.15 State and Block Composition Analysis | 1-2 weeks | Large effort, research-oriented |
| 4.1 Storage API Reference | 2 days | Developer docs, not operator |

---

### Testing Strategy

Each category of change requires corresponding verification:

- **Error type changes (1.1, 1.3, 1.4):** Unit tests for new error variants ensuring context fields are populated. Existing tests must continue to pass after signature changes.
- **unreachable!() replacements (1.2):** Before replacing, verify whether each arm is genuinely dead code (dispatched by message type) or reachable via malformed input. Only add error handling where the arm can actually be hit.
- **Metrics (2.3–2.9):** Integration tests that verify metrics are registered and updated during sync/block import. Can use the existing test harness.
- **CLI commands (2.2):** End-to-end test that starts a node, runs `ethrex status`, and verifies output format.
- **Configuration (3.1–3.3):** Unit tests for env var parsing and validation error messages.

---

## Effort Summary (Revised)

| Phase | Weeks | Track A | Track B |
|-------|-------|---------|---------|
| Foundation | 1-2 | Panics, error types, RPC bug (all breaking changes) | Sync metrics, .env |
| Error + Config | 3-4 | Context errors, unreachable | Env vars, health metrics |
| TODOs + UX + Troubleshooting | 5-6 | Error TODOs, logging | Banner, status cmd, troubleshooting guide |
| Init + Monitoring + Deployment | 7-8 | Init panics | Health endpoint, Docker, deployment guide |
| Dashboards + Polish | 9-10 | Shared | Dashboards, systemd, crate READMEs |

**Total Effort:** 14-16 developer-weeks (2 developers)
**Timeline:** 10-12 weeks (includes buffer for review cycles and testing)

---

## Success Metrics

**Stability (Primary Goal):**
1. Zero panics in production code paths (sync, storage, networking)
2. Errors in critical paths (sync, storage, networking) include actionable context

**Sync Monitoring (Pain Point #1):**
3. Operators can see sync progress in Prometheus/Grafana
4. `ethrex status` command shows sync phase, progress %, target block

**Container Deployment (Pain Point #2):**
5. High/Medium priority CLI options (~15-20) configurable via environment variables
6. Docker health checks available in compose files (with `/health` endpoint)

**Timeline Milestones:**
| Week | Milestone |
|------|-----------|
| 2 | Panics removed, error types improved, sync metrics in Prometheus |
| 4 | Env vars added (High/Medium), health metrics working |
| 6 | `ethrex status` command, startup/shutdown banners, troubleshooting guide |
| 8 | Health endpoint, Docker health checks, production deployment guide |
| 10 | Grafana dashboards, remaining documentation complete |

---

## Related Documents

- Metrics Gap Analysis: `docs/internal/l1/metrics_coverage_gap_analysis.md`
- Current Dashboards: `docs/developers/l1/dashboards.md`
---

## Appendix: Detailed Issue Counts

### Error Handling (150+)
- **Panics:** 7 non-init production locations + 14 initialization expect/panic locations (3 additional are test-only)
- **Unreachable:** 40 locations (DiscV4/V5 peer tables + additional)
- **Context-discarding map_err:** 100+ locations
- **TODOs about error handling:** 11 locations

### Node Operator UX (15)
1. Startup banner lacks configuration details
2. No CLI status command
3. Sync progress not in Prometheus
4. Mempool metrics not wired for L1
5. No block import failure metrics
6. No health status metrics
7. Minimal shutdown feedback
8. No database I/O metrics
9. Limited peer health metrics
10. Limited database commands
11. No network diagnostics
12. No systemd service files
13. Incomplete Grafana dashboards
14. No interactive REPL for node state inspection
15. No state/block composition analysis tooling

### Configuration (22)
21 CLI options without environment variable equivalents + no Nix support

### Documentation (4)
1. Storage API schema undocumented
2. No production deployment guide
3. Missing crate READMEs (7 crates)
4. No troubleshooting guide
