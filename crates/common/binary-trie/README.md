## Ethrex-Binary-Trie

Implementation of the [EIP-8297](https://eips.ethereum.org/EIPS/eip-8297)
Partitioned Binary Tree: the binary trie proposed to replace the Merkle
Patricia Trie as Ethereum's state commitment.

The crate has two halves:

- `trie` — the raw tree: a compressed binary radix trie mapping
  prefix-free variable-length keys to 32-byte values, committing to its
  contents with BLAKE3 over tagged leaf/branch preimages
  (`blake3(0x00 ‖ key ‖ value)`, `blake3(0x01 ‖ prefix ‖ left ‖ right)`)
  up to a single root. The empty tree's root is the 32-zero-byte
  sentinel `EMPTY_TRIE_ROOT`.
- `embedding` — the state embedding: how accounts, storage and code map
  onto tree keys and values. Zone-prefixed keys, per-account header
  stems (basic data, code hash, first 64 storage slots, first 128 code
  chunks under one stem), overflow storage/code key derivation
  (`get_tree_key_for_storage_slot`, `get_tree_key_for_code_chunk`),
  code chunking (`chunkify_code`) and basic-data packing
  (`encode_basic_data`).

### The trie

`trie::BinaryTrie` is incremental and insertion-based. Its canonical
structure is insertion-order independent: any insertion order over the
same key/value set yields the same root. Correctness is pinned by the
spec conformance vectors rather than by a second in-crate
implementation.

### Storage

The trie is storage-backed. `trie::BinaryTrieDB` is the node store —
`get(path)` / `put_batch`, keyed by a node's bit path from the root, the
same path-keyed single-version model the MPT's `TrieDB` uses — and
`trie::InMemoryBinaryTrieDB` is the implementation this crate ships;
the RocksDB one belongs to `crates/storage`. A child is either loaded
or the hash of a subtree still in the database, so `open` costs
nothing, `root()` reads nothing (a stored reference already is its
hash), and reads and inserts load only the nodes on their path.
`commit()` writes the loaded nodes back and returns the root.

### Test vectors

`tests/vectors/binary_trie_vectors.json` is **vendored** from the EELS
reference implementation, which owns the generator and its schema
documentation:

    ethereum/execution-specs, branch projects/binary-trie
    tests/binary_trie/vectors/

**Vendored from:** `projects/binary-trie` @ `f8986dca`

### Non-goals (today)

No hash caching and no dirty tracking (`commit` rewrites every loaded
node), no deletion, no proofs, no fork wiring. These are deferred to
the state-commitment integration work.

### Spec discrepancy

`encode_basic_data` packs `code_size` as 4 bytes at offset 4, following
the EELS branch this crate is ported from; EIP-7864 specifies a 3-byte
field at offset 5. The conformance fixture pins the EELS choice —
revisit when EIP-8297's final layout lands.
