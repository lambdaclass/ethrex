# Binary Trie: State Architecture

## Overview

ethrex uses a layered state architecture with clear separation between the read path
(optimized for speed) and the merkleization path (optimized for cryptographic integrity).

```
                          ┌─────────────────────┐
                          │   Block Execution    │
                          │       (LEVM)         │
                          └──────────┬──────────┘
                                     │ reads
                          ┌──────────▼──────────┐
                          │       Store          │
                          │  (single interface)  │
                          └──────────┬──────────┘
                                     │reads
                             ┌───────▼───────┐
                             │     FKV       │
                             │  (RocksDB)    │
                             │   O(1) get    │
                             └───────────────┘

                          ┌─────────────────────┐
                          │    Binary Trie       │
                          │  (merkleization +    │
                          │   proof generation)  │
                          └─────────────────────┘
```

## Layers

### Flat Key-Value Store (FKV)

The FKV is the primary disk-backed read path for account state and storage values.
It stores denormalized, ready-to-read values in flat RocksDB tables:

- `ACCOUNT_FLATKEYVALUE`: `keccak(address) → RLP(AccountState)`
- `STORAGE_FLATKEYVALUE`: `keccak(address) || keccak(slot) → RLP(value)`

Every account update writes to the FKV as part of the block's write transaction.
The FKV is always current -- there is no background scan or lazy population.

Reads are O(1): a single RocksDB point get by key. No trie traversal.

The FKV key space (`keccak(address)`) is independent of the binary trie's internal
key space (`tree_key(address)`). This decoupling means the trie structure can change
without affecting the read path.

### Binary Trie

The binary trie (EIP-7864) is used exclusively for:

1. **Merkleization**: Computing the state root after each block. Account updates are
   applied to the trie, and `state_root()` returns the root hash.
2. **Proof generation**: `get_proof(key)` returns sibling hashes from root to leaf
   for any state key. Used by `eth_getProof` and witness generation.

The binary trie is NOT used for execution reads. LEVM never traverses the trie.

The trie has its own node cache (dirty/warm/clean) that keeps hot nodes in memory
for efficient tree operations during merkleization. These caches serve the trie's
internal needs and are separate from the FKV read cache.

### Store (RocksDB)

Store is the single interface for all state access. It holds:

- Block headers, bodies, receipts
- Contract code (`ACCOUNT_CODES` table)
- FKV tables (account state and storage)
- Binary trie nodes (managed by NodeStore)

All reads go through Store. Store reads directly from the FKV.
Callers never interact with the binary trie or FKV directly.

## Read Path

```
Store.get_account_info(block_number, address)
  │
  ├─ resolve block_number → block_hash
  │
  └─ read from ACCOUNT_FLATKEYVALUE
       → key: keccak(address)
       → single RocksDB get, O(1)
```

Storage reads follow the same pattern via `STORAGE_FLATKEYVALUE`.

Code reads use `ACCOUNT_CODES` table keyed by code hash.

## Write Path

```
Block execution produces Vec<AccountUpdate>
  │
  ├─ Write to FKV tables (in the block's write transaction)
  │    → ACCOUNT_FLATKEYVALUE: keccak(address) → RLP(AccountState)
  │    → STORAGE_FLATKEYVALUE: keccak(address) || keccak(slot) → RLP(value)
  │
  ├─ Write to binary trie (apply_account_update)
  │    → Updates trie nodes for merkleization
  │    → state_root() computes binary trie root
  │
  └─ Write code to ACCOUNT_CODES table
```

## Merkleization Path

```
After block execution:
  │
  ├─ apply_account_update(update)
  │    → Inserts/updates leaves in the binary trie
  │    → basic_data, code_hash, code_chunks, storage_slot keys
  │
  ├─ state_root()
  │    → Computes hashes bottom-up (blake3)
  │    → Caches hashes in nodes for incremental reuse
  │    → Returns 32-byte root
  │
  └─ flush_if_needed(block_number, block_hash)
       → Every ~128 blocks, persist dirty nodes to disk
       → Rotate node caches (dirty → warm → clean)
```

## Binary Trie Structure

The binary trie uses a unified key space for all state types:

| State type | Tree key | Notes |
|-----------|----------|-------|
| Account basic data | `tree_key(address, 0)` | Packed: nonce, balance, code_size |
| Account code hash | `tree_key(address, 1)` | keccak256 of bytecode |
| Code chunks | `tree_key(address, 2+i)` | 31-byte chunks of bytecode |
| Storage slots | `tree_key(address, 64+slot)` | Individual storage values |

All state lives in one tree. No separate account trie and per-account storage tries.
This simplifies merkleization (one root) and proof generation (one proof path per key).

Node types:
- **InternalNode** (73 bytes): Two children (left/right) + cached hash
- **StemNode** (~450 bytes avg): 31-byte stem + up to 256 leaf values in a sparse map

## Binary Trie vs MPT

| Dimension | Binary Trie | MPT |
|-----------|------------|-----|
| Tree structure | Binary (2 children) | 16-way branching |
| Hash function | Blake3 | Keccak256 |
| Key space | Unified (accounts + storage + code) | Separate account + storage tries |
| Proof size | ~25 sibling hashes (~800 bytes) | ~8-10 RLP nodes (1-4 KB) |
| Merkleization | Single-threaded, O(depth) per key | 16-worker parallel |
| ZK-friendliness | Binary + blake3 (circuit-friendly) | 16-way + keccak (expensive) |
| Execution reads | Not used (FKV) | Not used (FKV) |
| Node size | InternalNode 73B, StemNode ~450B | Branch up to 532B |
