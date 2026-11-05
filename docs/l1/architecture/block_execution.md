# Block Execution Pipeline

This document describes how ethrex validates and executes blocks, from receiving a block to committing state changes.

## Overview

Block execution in ethrex follows the Ethereum specification closely. The pipeline handles:

1. Block header validation
2. System-level operations (beacon root contract, block hash storage)
3. Transaction execution
4. Withdrawal processing
5. Request extraction (post-Prague)
6. State root verification

## Entry Points

Blocks enter the execution pipeline through two main paths:

### 1. P2P Sync (`Syncer`)

During synchronization, blocks are fetched from peers and executed in batches:

```rust
// crates/networking/p2p/sync.rs
Syncer::add_blocks() → Blockchain::add_blocks_in_batch() → execute each block
```

### 2. Engine API (`engine_newPayloadV{1,2,3}`)

Post-Merge, the consensus client sends new blocks via the Engine API:

```rust
// crates/networking/rpc/engine/payload.rs
NewPayloadV3::handle() → Blockchain::add_block() → execute block
```

## Block Header Validation

Before executing a block, its header is validated:

```rust
// crates/blockchain/blockchain.rs
fn validate_header(header: &BlockHeader, parent: &BlockHeader) -> Result<()>
```

### Validation Checks

| Check | Description |
|-------|-------------|
| Parent hash | Must match parent block's hash |
| Block number | Must be parent.number + 1 |
| Timestamp | Must be > parent.timestamp |
| Gas limit | Must be within bounds of parent (EIP-1559) |
| Base fee | Must match calculated value (EIP-1559) |
| Difficulty | Must be 0 (post-Merge) |
| Nonce | Must be 0 (post-Merge) |
| Ommers hash | Must be empty hash (post-Merge) |
| Withdrawals root | Must match if Shanghai activated |
| Blob gas fields | Must be present if Cancun activated |
| Requests hash | Must match if Prague activated |

## Execution Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Block Execution                               │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  1. ┌────────────────────────────────────────────────────────────┐  │
│     │             System Operations (post-Cancun)                 │  │
│     │  • Store beacon block root (EIP-4788)                       │  │
│     │  • Store parent block hash (EIP-2935)                       │  │
│     └────────────────────────────────────────────────────────────┘  │
│                              │                                       │
│                              ▼                                       │
│  2. ┌────────────────────────────────────────────────────────────┐  │
│     │              Transaction Execution                          │  │
│     │  For each transaction:                                      │  │
│     │  • Validate signature and nonce                             │  │
│     │  • Check sender balance                                     │  │
│     │  • Execute in EVM                                           │  │
│     │  • Apply gas refunds                                        │  │
│     │  • Update account states                                    │  │
│     │  • Generate receipt                                         │  │
│     └────────────────────────────────────────────────────────────┘  │
│                              │                                       │
│                              ▼                                       │
│  3. ┌────────────────────────────────────────────────────────────┐  │
│     │              Withdrawal Processing (post-Shanghai)          │  │
│     │  For each withdrawal:                                       │  │
│     │  • Credit validator address with withdrawal amount          │  │
│     └────────────────────────────────────────────────────────────┘  │
│                              │                                       │
│                              ▼                                       │
│  4. ┌────────────────────────────────────────────────────────────┐  │
│     │              Request Extraction (post-Prague)               │  │
│     │  • Deposit requests from logs                               │  │
│     │  • Withdrawal requests from system contract                 │  │
│     │  • Consolidation requests from system contract              │  │
│     └────────────────────────────────────────────────────────────┘  │
│                              │                                       │
│                              ▼                                       │
│  5. ┌────────────────────────────────────────────────────────────┐  │
│     │                  State Finalization                         │  │
│     │  • Compute state root from account updates                  │  │
│     │  • Verify against header.state_root                         │  │
│     │  • Commit changes to storage                                │  │
│     └────────────────────────────────────────────────────────────┘  │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

## Transaction Execution

Each transaction goes through the following steps:

### 1. Pre-Execution Validation

```rust
// crates/blockchain/validate.rs
fn validate_transaction(tx: &Transaction, header: &BlockHeader) -> Result<()>
```

- Signature recovery and validation
- Nonce check (must match account nonce)
- Gas limit check (must be <= block gas remaining)
- Balance check (must cover `gas_limit * gas_price + value`)
- Intrinsic gas calculation
- EIP-2930 access list validation
- EIP-4844 blob validation (if applicable)

### 2. EVM Execution

```rust
// crates/vm/levm/src/vm.rs
VM::execute() → Result<ExecutionReport>
```

The EVM executes the transaction bytecode:

1. **Contract Call**: Execute target contract code
2. **Contract Creation**: Deploy new contract, execute constructor
3. **Transfer**: Simple value transfer (no code execution)

During execution:
- Opcodes are decoded and executed
- Gas is consumed for each operation
- State changes are tracked (but not committed)
- Logs are collected
- Errors revert all changes

### 3. Post-Execution

After EVM execution:

```rust
// crates/vm/levm/src/vm.rs
fn finalize_transaction() -> Receipt
```

- Calculate gas refund (max 1/5 of gas used, post-London)
- Credit coinbase with priority fee
- Generate receipt with logs and status
- Update cumulative gas used

## State Management

### Account Updates

State changes are tracked as `AccountUpdate` structs:

```rust
pub struct AccountUpdate {
    pub address: Address,
    pub removed: bool,
    pub info: Option<AccountInfo>,       // balance, nonce, code_hash
    pub code: Option<Bytes>,             // bytecode if changed
    pub added_storage: HashMap<H256, U256>,
}
```

### State Root Computation

After all transactions execute:

```rust
// crates/storage/store.rs
Store::apply_account_updates_batch(parent_hash, updates) -> StateTrieHash
```

This is one of the two merkleization backends (the other is used by `add_block_pipeline`):

1. Load parent state trie
2. Apply each account update to the trie
3. For accounts with storage changes, update storage tries
4. Compute new state root
5. Verify it matches `header.state_root`

### Sharded State Trie

```rust
// crates/blockchain/sharded_trie.rs
ShardedStateTrie::open(store, parent_state_root)
```

`ShardedStateTrie` splits the state trie into 16 shards by the first nibble of
the hashed address and keeps a storage trie open per touched account. A storage
trie first written with 2048 or more slots is split in 16 the same way. Work on
the shards runs on scoped threads, never on the blockchain's merkle pool, whose
16 workers must all run together.

- `apply` takes absolute writes per account (the last write to a field wins).
- `root()` recomputes the root re-hashing only the paths written since the
  previous call, so it can be called repeatedly on the same tree.
- `finish()` consumes the tree and returns the root plus the trie nodes to
  persist. It is only valid on a tree that never ran `root()`.

Import of blocks with a block access list (Amsterdam and later, through
`add_block_pipeline`) merkleizes on it one-shot (`apply`, then `finish()`).
Payload sealing uses `root()` (see [Pre-sealing](#pre-sealing)).

## Payload Building

When ethrex acts as a block producer (validator), it builds payloads:

```rust
// crates/blockchain/payload.rs
Blockchain::build_payload(template: Block) -> PayloadBuildResult
```

### Building Process

1. **Fetch transactions** from mempool, filtered by:
   - Base fee (must afford current base fee)
   - Blob fee (for EIP-4844 transactions)
   - Nonce ordering (consecutive nonces per sender)

2. **Order transactions** by effective tip (highest first)

3. **Execute transactions** until:
   - Block gas limit reached
   - No more valid transactions
   - Blob limit reached (for blob transactions)

4. **Finalize block**:
   - Apply withdrawals
   - Extract requests
   - Compute state root
   - Compute receipts root
   - Generate logs bloom

### Payload Rebuilding

Payloads are rebuilt as new transactions arrive, until `getPayload` arrives or
the slot ends (12 s from the forkchoice update that started the build):

```rust
// crates/blockchain/payload.rs
Blockchain::build_payload_loop(payload, cancel_token)
```

The loop keeps the most valuable build (block value, then gas used) and leaves
it unsealed; `getPayload` seals it once.

### Pre-sealing

The state root is the costly part of a seal. To keep it off `getPayload`, the
held build owns a `PreSealer` (`crates/blockchain/preseal.rs`): a worker thread
with a `ShardedStateTrie` opened at the parent state, kept equal to the held
build's state.

- After a build, the loop reads its diff from the VM with `state_diff()`, which
  leaves the VM usable for the seal.
- When a rebuild beats the held build, the PreSealer moves to it and rebases
  the tree by diff: it writes only the keys whose value differs between the two
  builds, and resets keys only the old build touched to their parent value. A
  burst of new best builds coalesces into one rebase.
- On `getPayload` the seal runs the post-transaction phase (withdrawals, request
  system calls) and sends the final updates to the worker, which applies the
  difference and returns the root. `getPayload` pays for the rest of an
  in-flight rebase plus the last change, not for a full state root.
- If the worker fails, the seal logs a warning and computes the root in one
  shot from the final updates: a broken pre-seal costs time, never a wrong root.

Builds outside the loop (L2 block production, explicit-transaction builds) have
no PreSealer and compute the root in one shot on a `ShardedStateTrie`.

### Payload Lifecycle

- At most 4 entries are kept (`MAX_PAYLOADS`). Registering a build first drops
  entries whose build already ended, then evicts the oldest if still full.
- A build not requested within its slot is dropped with its tree, and a later
  `getPayload` for its id answers `UnknownPayload` (-38001).
- A running build's tree keeps every key touched by any of the slot's best
  builds loaded until `getPayload`, the timeout or eviction.

## Error Handling

Block execution can fail for various reasons:

| Error | Cause | Recovery |
|-------|-------|----------|
| `InvalidBlock::InvalidStateRoot` | Computed state root doesn't match header | Reject block |
| `InvalidBlock::InvalidGasUsed` | Gas used doesn't match header | Reject block |
| `InvalidBlock::InvalidTransaction` | Transaction validation failed | Reject block |
| `EvmError::OutOfGas` | Transaction ran out of gas | Revert transaction, continue block |
| `EvmError::InvalidOpcode` | Unknown opcode encountered | Revert transaction, continue block |

## Performance Considerations

### Batch Execution

During sync, blocks are executed in batches (default 1024 blocks):

```rust
// crates/networking/p2p/sync.rs
const EXECUTE_BATCH_SIZE: usize = 1024;
```

This reduces database commits and improves throughput.

### Parallel Trie Operations

Storage trie updates can be parallelized across accounts:

```rust
// Uses rayon for parallel iteration
account_updates.par_iter().map(|update| update_storage_trie(update))
```

### State Caching

The EVM maintains a cache of accessed accounts and storage slots to minimize database reads during execution.

## Hard Fork Handling

Block execution adapts based on the active hard fork:

```rust
// crates/common/types/chain_config.rs
impl ChainConfig {
    pub fn fork(&self, timestamp: u64) -> Fork { ... }
    pub fn is_cancun_activated(&self, timestamp: u64) -> bool { ... }
    pub fn is_prague_activated(&self, timestamp: u64) -> bool { ... }
}
```

Each fork may introduce:
- New opcodes (e.g., `PUSH0` in Shanghai)
- New precompiles (e.g., point evaluation in Cancun)
- New system contracts (e.g., beacon root contract in Cancun)
- Changed gas costs
- New transaction types

## Related Documentation

- [LEVM Documentation](../../vm/levm/debug.md) - EVM implementation details
- [Sync State Machine](./sync_state_machine.md) - How blocks flow during sync
- [Crate Map](./crate_map.md) - Overview of involved crates
