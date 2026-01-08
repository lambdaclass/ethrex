# Sync State Machine

This document describes the synchronization algorithms implemented in ethrex, including full sync and snap sync.

## Overview

ethrex supports two synchronization modes:

| Mode | Description | Use Case |
|------|-------------|----------|
| **Full Sync** | Downloads and executes every block | Maximum security, slower |
| **Snap Sync** | Downloads state directly, executes recent blocks | Faster initial sync |

## Sync Manager Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        SyncManager                               │
│  • Receives sync targets from Engine API / P2P                   │
│  • Tracks current sync mode (Full / Snap)                        │
│  • Coordinates Syncer for actual sync work                       │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                          Syncer                                  │
│  • Executes sync cycles                                          │
│  • Manages peer connections via PeerHandler                      │
│  • Handles both full and snap sync algorithms                    │
└─────────────────────────────────────────────────────────────────┘
```

## Sync Triggers

Synchronization is triggered by:

1. **Engine API**: `engine_forkchoiceUpdated` with unknown head hash
2. **P2P**: Receiving block announcements for unknown blocks
3. **Startup**: When local chain is behind network

```rust
// crates/networking/rpc/engine/fork_choice.rs
match apply_fork_choice(...) {
    Err(InvalidForkChoice::Syncing) => {
        syncer.sync_to_head(fork_choice_state.head_block_hash);
        // Return SYNCING status to consensus client
    }
}
```

## Full Sync Algorithm

Full sync downloads blocks from the network and executes each one to reconstruct the state.

### State Machine

```
                    ┌─────────────────┐
                    │   START SYNC    │
                    └────────┬────────┘
                             │
                             ▼
                    ┌─────────────────┐
         ┌─────────│  Request Headers │◄─────────────┐
         │         └────────┬────────┘              │
         │                  │                        │
         │                  ▼                        │
         │         ┌─────────────────┐              │
         │         │ Validate Headers│              │
         │         └────────┬────────┘              │
         │                  │                        │
         │                  ▼                        │
         │         ┌─────────────────┐              │
         │         │ Found Canonical │──No──────────┘
         │         │   Ancestor?     │
         │         └────────┬────────┘
         │                  │ Yes
         │                  ▼
         │         ┌─────────────────┐
         │         │  Request Bodies │◄─────────────┐
         │         └────────┬────────┘              │
         │                  │                        │
         │                  ▼                        │
         │         ┌─────────────────┐              │
         │         │ Execute Batch   │              │
         │         │ (1024 blocks)   │              │
         │         └────────┬────────┘              │
         │                  │                        │
         │                  ▼                        │
         │         ┌─────────────────┐              │
         │         │  More Blocks?   │──Yes─────────┘
         │         └────────┬────────┘
         │                  │ No
         │                  ▼
         │         ┌─────────────────┐
         └─Error───│   SYNC DONE     │
                   └─────────────────┘
```

### Algorithm Details

```rust
// crates/networking/p2p/sync.rs
async fn sync_cycle_full(sync_head: H256, store: Store) -> Result<()>
```

1. **Find Chain Link**
   - Request headers backwards from sync_head
   - Stop when reaching a canonical block (already known)
   - This identifies the fork point

2. **Store Headers**
   - Save all new headers to temporary storage
   - Headers are stored in batches during download

3. **Download Bodies**
   - Request bodies for stored headers
   - Match bodies to headers by hash
   - Maximum 64 bodies per request

4. **Execute Blocks**
   - Execute in batches of 1024 blocks
   - Each block is fully validated and executed
   - State is committed after each batch

5. **Update Fork Choice**
   - After all blocks executed, update canonical chain
   - Set new head, safe, and finalized blocks

### Key Constants

```rust
const EXECUTE_BATCH_SIZE: usize = 1024;      // Blocks per execution batch
const MAX_BLOCK_BODIES_TO_REQUEST: usize = 64; // Bodies per request
```

## Snap Sync Algorithm

Snap sync downloads state directly from peers instead of executing all historical blocks.

### State Machine

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SNAP SYNC STATE MACHINE                            │
└─────────────────────────────────────────────────────────────────────────────┘

    ┌──────────────┐
    │  START SNAP  │
    │    SYNC      │
    └──────┬───────┘
           │
           ▼
    ┌──────────────┐     ┌─────────────────────────────────────────────────────┐
    │   Download   │     │  Download headers to find sync head                  │
    │   Headers    │────▶│  Store hashes for later body download               │
    └──────┬───────┘     └─────────────────────────────────────────────────────┘
           │
           ▼
    ┌──────────────┐     ┌─────────────────────────────────────────────────────┐
    │ Select Pivot │────▶│  Choose recent block as pivot (must not be stale)   │
    │    Block     │     │  Pivot block is target for state download           │
    └──────┬───────┘     └─────────────────────────────────────────────────────┘
           │
           ▼
    ┌──────────────┐     ┌─────────────────────────────────────────────────────┐
    │  Download    │────▶│  Request account ranges via SNAP protocol           │
    │  Accounts    │     │  Store account states to disk as snapshots          │
    └──────┬───────┘     └─────────────────────────────────────────────────────┘
           │
           ▼
    ┌──────────────┐     ┌─────────────────────────────────────────────────────┐
    │   Insert     │────▶│  Build account trie from downloaded leaves          │
    │  Accounts    │     │  Identify accounts with non-empty storage           │
    └──────┬───────┘     └─────────────────────────────────────────────────────┘
           │
           ▼
    ┌──────────────┐     ┌─────────────────────────────────────────────────────┐
    │  Download    │────▶│  For each account with storage:                     │
    │  Storage     │     │  Request storage ranges and build storage tries.    │
    │              │     │  Includes a healing loop to fix state trie changes. │
    └──────┬───────┘     └─────────────────────────────────────────────────────┘
           │
           ▼
    ┌──────────────┐     ┌─────────────────────────────────────────────────────┐
    │    Heal      │────▶│  Heal state trie (fill missing nodes)               │
    │    Tries     │     │  Heal storage tries for modified accounts           │
    └──────┬───────┘     └─────────────────────────────────────────────────────┘
           │
           ▼
    ┌──────────────┐     ┌─────────────────────────────────────────────────────┐
    │  Download    │────▶│  Download bytecode for all contract accounts        │
    │  Bytecode    │     │  Match by code hash                                 │
    └──────┬───────┘     └─────────────────────────────────────────────────────┘
           │
           ▼
    ┌──────────────┐
    │  SNAP SYNC   │
    │   COMPLETE   │
    └──────┬───────┘
           │
           ▼
    ┌──────────────┐     ┌─────────────────────────────────────────────────────┐
    │   Switch to  │────▶│  Execute recent blocks from pivot to head           │
    │  Full Sync   │     │  Continue with full sync for new blocks             │
    └──────────────┘     └─────────────────────────────────────────────────────┘
```

### Phase 1: Header Download

Download all block headers from current head to sync target:

```rust
// crates/networking/p2p/sync.rs
async fn sync_cycle_snap(sync_head: H256, store: Store) -> Result<()>
```

- Request headers in batches
- Store header hashes for later use
- Identify pivot block (recent block whose state we'll download)

### Phase 2: Pivot Selection

The pivot block must be:
- Recent enough to have state available on peers
- Not "stale" (older than SNAP_LIMIT * 12 seconds)

```rust
// crates/networking/p2p/sync.rs
fn block_is_stale(header: &BlockHeader) -> bool {
    calculate_staleness_timestamp(header.timestamp) < current_unix_time()
}

const SNAP_LIMIT: usize = 128; // Blocks before pivot is considered stale
```

If the pivot becomes stale during sync, a new pivot is selected:

```rust
async fn update_pivot(block_number: u64, ...) -> Result<BlockHeader>
```

### Phase 3: Account Download

Download all account states at the pivot block:

```rust
// Uses SNAP protocol GetAccountRange messages
peers.request_account_range(start_hash, end_hash, snapshot_dir, pivot_header)
```

- Accounts are saved to disk as RLP-encoded snapshots
- Each snapshot file contains a batch of (hash, account_state) pairs
- Process tracks code hashes for later bytecode download

### Phase 4: Account Trie Construction

Build the account state trie from downloaded leaves:

```rust
async fn insert_accounts(store, storage_accounts, snapshots_dir, ...) -> (H256, accounts_with_storage)
```

For RocksDB backend:
- Ingest snapshot files directly via SST ingestion
- Build trie using sorted insertion algorithm
- Track accounts with non-empty storage root

### Phase 5: Storage Download

For each account with storage, download storage slots:

```rust
peers.request_storage_ranges(storage_accounts, snapshots_dir, chunk_index, pivot_header)
```

- Multiple accounts can be requested per message
- Large accounts are downloaded in chunks
- "Big accounts" (>4096 slots) are marked for healing instead

### Phase 6: Trie Healing

State may have changed while downloading. Healing fixes inconsistencies:

**State Trie Healing:**
```rust
async fn heal_state_trie_wrap(state_root, store, peers, deadline, ...) -> bool
```
- Walk trie from root
- Request missing nodes from peers
- Fill in gaps caused by state changes

**Storage Trie Healing:**
```rust
async fn heal_storage_trie(state_root, accounts, peers, store, ...) -> bool
```
- For each account marked for healing
- Request missing storage trie nodes
- Verify storage roots match account state

### Phase 7: Bytecode Download

Download contract bytecode:

```rust
peers.request_bytecodes(&code_hashes)
```

- Code hashes collected during account download
- Bytecode downloaded in chunks (50,000 per batch)
- Verified by hashing and comparing to code_hash

### Phase 8: Transition to Full Sync

After snap sync completes:
1. Store pivot block body
2. Update fork choice to pivot
3. Switch sync mode to Full
4. Execute any remaining blocks normally

## P2P Protocols Used

### eth/68 Protocol

Used for block header and body download:

| Message | Purpose |
|---------|---------|
| `GetBlockHeaders` | Request headers by number or hash |
| `BlockHeaders` | Response with headers |
| `GetBlockBodies` | Request bodies by hash |
| `BlockBodies` | Response with bodies |

### snap/1 Protocol

Used for state download during snap sync:

| Message | Purpose |
|---------|---------|
| `GetAccountRange` | Request accounts in hash range |
| `AccountRange` | Response with accounts and proof |
| `GetStorageRanges` | Request storage for accounts |
| `StorageRanges` | Response with storage and proofs |
| `GetByteCodes` | Request bytecode by hash |
| `ByteCodes` | Response with bytecode |
| `GetTrieNodes` | Request specific trie nodes |
| `TrieNodes` | Response with nodes |

## Error Recovery

### Recoverable Errors

These errors cause sync to retry:
- Peer disconnection
- Invalid response from peer
- Timeout waiting for response
- Database errors (transient)

### Non-Recoverable Errors

These errors cause sync to abort with warning:
- Snapshot file corruption
- Database corruption
- State root mismatch after healing

```rust
// crates/networking/p2p/sync.rs
impl SyncError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            SyncError::Chain(_) | SyncError::Store(_) | ... => true,
            SyncError::CorruptDB | SyncError::SnapshotDecodeError(_) | ... => false,
        }
    }
}
```

## Performance Optimizations

### Parallel Operations

- Account trie insertion uses Rayon for parallelism
- Storage tries built in parallel across accounts
- Bytecode downloads are batched

### Disk I/O

- Snapshot files written in batches to reduce writes
- RocksDB SST ingestion for fast account loading
- Temporary directories cleaned up after sync

### Network

- Multiple peers used concurrently
- Peer scoring based on response time and validity
- Automatic peer rotation for failed requests

## Metrics

Sync progress is tracked via metrics:

```rust
// crates/networking/p2p/metrics.rs
METRICS.account_tries_inserted     // Accounts added to trie
METRICS.storage_leaves_inserted    // Storage slots added
METRICS.current_step               // Current sync phase
METRICS.sync_head_hash             // Current sync target
```

## Configuration

| Option | Description | Default |
|--------|-------------|---------|
| `--syncmode` | Sync mode (`full` or `snap`) | `snap` |
| `EXECUTE_BATCH_SIZE` | Blocks per batch (env var) | 1024 |
| `MIN_FULL_BLOCKS` | Min blocks to full sync in snap mode | 10,000 |

## Related Documentation

- [Snap Sync Internals](../fundamentals/snap_sync.md) - Detailed snap sync documentation
- [Block Execution Pipeline](./block_execution.md) - How blocks are executed
- [Networking](../fundamentals/networking.md) - P2P protocol details

> **Note:** For comprehensive snap sync documentation, see [Snap Sync Internals](../fundamentals/snap_sync.md).
