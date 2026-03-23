# Binary Trie State Architecture: Changes from MPT

## What changed

This branch replaces the MPT (Merkle Patricia Trie) with an EIP-7864 binary trie for
state root computation and proof generation. The read path (FKV) and write path (Store)
are unchanged. The binary trie is a drop-in replacement for the MPT's merkleization role.

## Architecture comparison

### Main branch (MPT)

```
Pipeline (3 concurrent threads):
  [warmer]     prefetches state into cache
  [executor]   runs LEVM, sends Vec<AccountUpdate> batches via channel -->
  [merkleizer] calls handle_merkleization() -- applies to MPT (16-shard parallel),
               computes state root, returns AccountUpdatesList

Non-pipeline (add_block):
  execute_block() -> updates
  Store.apply_account_updates_batch(&updates) -> AccountUpdatesList
    (internally calls handle_merkleization + flush)

Both paths then:
  store_block(block, account_updates_list, result)
    +--> FKV tables (account state, storage)
    +--> ACCOUNT_CODES table
    +--> TrieLayerCache (in-memory node cache, flush every ~128 blocks)
           --> persist to ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES

Reads:
  StoreVmDatabase --> FKV tables (O(1) RocksDB gets)
```

### This branch (Binary Trie)

```
Pipeline (3 concurrent threads):                                    [SAME STRUCTURE]
  [warmer]     prefetches state into cache                          [UNCHANGED]
  [executor]   runs LEVM, sends Vec<AccountUpdate> batches -->      [UNCHANGED]
  [merkleizer] calls handle_merkleization() -- applies to binary    [SAME FUNCTION]
               trie, computes state root, returns AccountUpdatesList

Non-pipeline (add_block):                                           [SAME STRUCTURE]
  execute_block() -> updates
  Store.apply_account_updates_batch(&updates) -> AccountUpdatesList [SAME CALL SITE]
    (internally calls handle_merkleization + flush)

Both paths then:
  store_block(block, account_updates_list, result)                  [SAME CALL SITE]
    +--> FKV tables (account state, storage)                        [UNCHANGED]
    +--> ACCOUNT_CODES table                                        [UNCHANGED]
    +--> flush_binary_trie_if_needed (every ~128 blocks)
           --> persist to BINARY_TRIE_NODES CF

Reads:
  StoreVmDatabase --> FKV tables (O(1) RocksDB gets)               [UNCHANGED]
```

The pipeline structure is identical: warmer, executor, and merkleizer threads run
concurrently. The merkleizer applies trie updates as batches arrive from the executor,
overlapping merkleization with execution. The only change is the merkleizer's internals
(MPT 16-shard parallel -> binary trie single-tree incremental). Both `BinaryTrieState`
and `TrieLayerCache` live on `Store`, not on `Blockchain`.

## What was removed

| MPT component | Replacement |
|---|---|
| `ACCOUNT_TRIE_NODES` table | `BINARY_TRIE_NODES` CF |
| `STORAGE_TRIE_NODES` table | Not needed (unified tree, no per-account storage tries) |
| `TrieLayerCache` (in-memory trie node diff layers) | `NodeStore` dirty/warm/clean tiers (same role, different node format) |
| 16-shard parallel MPT merkleization | Binary trie `apply_account_update` (single tree, incremental) |
| `handle_merkleization` / `handle_merkleization_bal` | Replaced by `Store.handle_merkleization()` which applies updates via `BinaryTrieState.apply_account_update()`. |
| `BranchNode[16]` root assembly | Binary trie `state_root()` |
| MPT body of `apply_account_updates_batch()` | Binary trie body. Same function name, same call sites. |
| `AccountUpdatesList` MPT fields (`state_trie_hash`, `state_updates`, `storage_updates`) | Replaced with `code_updates` + `flat_updates`. MPT fields moved to `MptUpdatesList` for EF tests. |
| RLP node encoding + keccak hashing | Raw concatenation + blake3 |

## What was NOT changed

| Component | Notes |
|---|---|
| LEVM | Zero code changes. The EVM is completely unaware of the trie backend. |
| `StoreVmDatabase` | Still the sole VM read path, reads from FKV. LEVM only touches FKV, never the trie. |
| FKV tables (`ACCOUNT_FLATKEYVALUE`, `STORAGE_FLATKEYVALUE`) | Intact. Updated every block, O(1) reads. The tables, key format, and read logic are identical to main. |
| FKV write path | FKV writes happen in `store_block()` on both branches. The data written is the same: `keccak(address) -> AccountState` and `keccak(address) \|\| keccak(slot) -> value`. |
| `ACCOUNT_CODES` table | Code stored by hash, read by VM. Unchanged. |
| Block/header/receipt storage | Unchanged |
| `Store` interface | Still the single entry point for all state access. `BinaryTrieState` lives on Store (same as `TrieLayerCache` on main). |
| Pipeline structure | 3 concurrent threads: warmer, executor, merkleizer. Same structure, merkleizer internals changed. |
| `Blockchain` struct | Identical to main: `storage`, `mempool`, `is_synced`, `options`, `payloads`. No trie state. |
| Transaction pool, p2p, RPC layer | Unchanged |
| Consensus validation (except state root) | Unchanged |

### Not added

- No historical state diffs beyond the in-memory node cache window
- No periodic state snapshots

## Key differences: Binary Trie vs MPT

| Aspect | MPT | Binary Trie |
|---|---|---|
| Tree structure | 16-way branching (BranchNode, ExtensionNode, LeafNode) | Binary (InternalNode with left/right, StemNode with 256 leaves) |
| Key space | Separate account trie + per-account storage tries | Single unified tree for accounts, storage, and code chunks |
| Hash function | Keccak-256 | Blake3 |
| Node encoding | RLP before hashing | Raw concatenation (`hash(left \|\| right)`) |
| Merkleization | 16 parallel shard workers | Pipelined merkleizer thread, incremental (only rehash dirty paths) |
| Proof size | ~8-10 RLP nodes (1-4 KB) | ~25 sibling hashes (~800 bytes) |
| ZK-friendliness | Poor (RLP + keccak hard to prove) | Good (binary + blake3 circuit-friendly) |
| State root in headers | MPT root (consensus-valid) | Binary trie root (NOT validated against header, header has MPT root) |
| Flush strategy | `TrieLayerCache` chains layers by state root hash | `NodeStore` uses stable NodeIds, dirty/warm/clean rotation |

## Binary trie key mapping

All state is mapped into 32-byte keys in a single tree:

```
tree_key(address, tree_index, sub_index):
  stem = blake3(zero_pad(address) || big_endian(tree_index))[:31]
  key  = stem || sub_index
```

| State type | tree_index | sub_index |
|---|---|---|
| Account basic_data | 0 | 0 |
| Account code_hash | 0 | 1 |
| Header storage slots (0-63) | 0 | 64 + slot |
| Code chunks | (128 + chunk_id) / 256 | (128 + chunk_id) % 256 |
| Main storage slots (>= 64) | (2^248 + slot) / 256 | (2^248 + slot) % 256 |

The basic_data leaf packs version (1B), code_size (3B), nonce (8B), balance (16B) into 32 bytes.

## State root handling

The binary trie produces a different state root than the MPT. Block headers on the
canonical chain contain MPT state roots. This branch:

- Skips `state_root == header.state_root` validation
- Computes and logs the binary trie root per block
- Equivalence between binary trie and MPT roots is proven externally by the zkVM
  proving layer (recursive verification, not direct mathematical proof)

## NodeStore: trie node persistence

The `NodeStore` replaces `TrieLayerCache` for managing in-memory trie nodes:

| `TrieLayerCache` (MPT) | `NodeStore` (Binary Trie) |
|---|---|
| Content-addressed nodes (keyed by hash) | Stable NodeIds (u64, monotonically allocated) |
| Layers chained by state root hash | Three tiers: dirty, warm, clean (LRU) |
| Bloom filter for layer lookup | Direct HashMap lookup by NodeId |
| Background flush via trie worker thread | `flush_if_needed()` called inline after block |

Flush cycle (every ~128 blocks):
1. Write dirty nodes + metadata to `BINARY_TRIE_NODES` via atomic `WriteBatch`
2. Rotate: dirty -> warm -> evict old warm to clean LRU
3. Clean LRU evicts to disk on capacity
