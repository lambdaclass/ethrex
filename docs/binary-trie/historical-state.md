# Binary Trie: State Architecture and Scope

## Current ethrex State Architecture

ethrex uses a layered state architecture, NOT a pure content-addressed MPT:

- **Flat Key-Value (FKV)**: O(1) lookups for accounts and storage, keyed by
  `keccak(address)` / `keccak(address) + keccak(slot)`. This is what LEVM reads
  from during execution. Fast, flat, no trie traversals.
- **In-memory diff layers** (~128 recent blocks): Accumulate state changes per block.
  Support branching for reorgs. Values are flushed to the disk layer periodically.
- **Path-based MPT**: Used exclusively for merkleization (computing state roots) and
  proof generation. NOT used for execution reads. Nodes are path-based (not
  content-addressed), so old state is NOT preserved -- ethrex does not support
  historical state queries via MPT.
- **Store (RocksDB)**: Blocks, receipts, headers, code, and the FKV disk layer.

## What the Binary Trie Changes

The binary trie (EIP-7864) replaces **only the merkleization backend**:

```
                    BEFORE                          AFTER
Execution reads:    FKV (unchanged)                 FKV (unchanged)
State writes:       FKV + MPT trie nodes            FKV + Binary Trie nodes
Merkleization:      MPT (16-way, keccak)            Binary Trie (binary, blake3)
State root:         MPT root                        Binary Trie root
Proofs:             MPT proofs (eth_getProof)        Binary Trie proofs (TBD)
```

The key architectural difference of the binary trie is that it uses a **unified trie**
for accounts, storage, AND code -- all in one tree with a flat 32-byte key space. This
changes the write path (one trie insert per field instead of separate account/storage
tries) but does NOT require changing the read path.

### What stays the same

- FKV for execution reads (LEVM never touches the trie)
- In-memory diff layers for recent state and reorg support
- Store for blocks, receipts, headers, code storage
- The diff layer flush mechanism (128-block threshold)

### What changes

- Merkleization: MPT merkleizer thread replaced with binary trie updates
- State root computation: binary trie root instead of MPT root
- Node caching: binary trie's NodeStore (dirty/warm/clean) replaces MPT's TrieLayerCache
- Write path: account updates applied to binary trie (unified trie for account+storage+code)
- Proof generation: binary trie proofs (sibling hashes) instead of MPT proofs

### What is explicitly out of scope

- **Historical state queries**: ethrex does not support these today (MPT is path-based,
  old state is pruned). The binary trie should not add this capability.
- **Periodic trie snapshots**: Not needed for the merkleization replacement.
- **Persisted diff layers to disk**: The in-memory diff layers (~128 blocks) are
  sufficient, matching current behavior. Persisting diffs for historical queries is a
  separate feature.
- **FKV key space change**: The FKV stays keccak-keyed. The binary trie uses its own
  key space internally. They are decoupled.

## Binary Trie Write Path

The binary trie introduces a unified trie where accounts, storage, and code all live in
the same tree. Each piece of state maps to a 32-byte tree key:

- `basic_data(address)` -- packed nonce, balance, code_size
- `code_hash(address)` -- the keccak256 of the contract bytecode
- `code_chunk(address, index)` -- 31-byte chunks of contract bytecode
- `storage_slot(address, slot)` -- individual storage values

This means a single account update that changes balance + 3 storage slots results in
4 trie inserts (1 basic_data + 3 storage slots), all in the same tree. In MPT, this
would be 1 account trie update + 3 storage trie updates across 2 separate tries.

The unified tree simplifies merkleization (one root, one proof path) but makes writes
more granular (more individual node updates per block).

## Binary Trie vs MPT: Comparison

| Dimension | Binary Trie (EIP-7864) | MPT (current ethrex) |
|-----------|----------------------|---------------------|
| **Tree structure** | Binary (2 children per node) | 16-way branching |
| **Hash function** | Blake3 | Keccak256 |
| **Key space** | Unified (accounts + storage + code in one tree) | Separate account trie + per-account storage tries |
| **Node types** | InternalNode (73 bytes) + StemNode (~450 bytes) | Branch (up to 532 bytes) + Extension + Leaf |
| **Proof size** | ~25 sibling hashes (~800 bytes) | ~8-10 nodes (variable, often 1-4 KB) |
| **Merkleization workers** | Single-threaded (fast O(depth) inserts) | 16-worker parallel merkleizer |
| **ZK-friendliness** | Binary + blake3 (circuit-friendly) | 16-way + keccak (expensive in circuits) |
| **Execution reads** | Not used (FKV handles reads) | Not used (FKV handles reads) |

## Open Questions

1. **eth_getProof format**: Binary trie proofs are sibling hash arrays, not RLP-encoded
   MPT nodes. The RPC response format needs to be defined. Options: return binary trie
   proofs in a new format, or return an error until a format is standardized.

2. **FKV key space**: Currently keccak-based. If the FKV eventually moves to binary trie
   keys, reads could be served directly from the trie's leaf values without a separate
   flat index. This is a future optimization, not needed for the initial replacement.

## Implementation Status

### Core (in scope)

- **Interior Mutability**: Concurrent reads on NodeStore for warmer+executor pipeline.
- **Diff-Layer Flush**: Deferred flush with 128-block threshold. Matches current behavior.
- **Binary Trie Proofs**: get_proof implementation. Needed for eth_getProof.
- **BinaryTrieState in Blockchain**: Genesis init, startup integration.
- **Replace Merkleization**: Core change -- binary trie updates instead of MPT merkleizer.
- **Full MPT Removal**: Removed merkleizer thread, empty trie data to Store.
- **Startup/Recovery**: Binary trie checkpoint replay, shutdown flush.

### Extended (may need scope review)

These go beyond the minimal merkleization replacement. Whether they stay depends on
whether the binary trie serves as the sole state backend or only as a merkleization
engine alongside FKV:

- **Replace StoreVmDatabase with BinaryTrieVmDb**: LEVM reads from the binary trie
  instead of FKV/Store. If FKV remains the read path, this should be reverted.

- **Replace RPC State Reads**: eth_getBalance, eth_getCode, etc. read from
  BinaryTrieState. If FKV remains the read path, this should be reverted.

- **Persisted Diff Layers**: On-disk diff persistence for historical state queries.
  ethrex does not support historical state today. If this capability is not needed,
  the persist_diff calls and disk walk code can be removed. In-memory diff layers
  (~128 blocks) should remain regardless.

- **Witness Generation**: BinaryTrieWitness with trie reconstruction for pre-state
  proofs. Keep if binary trie proofs are needed for the experiment. Remove if not.
