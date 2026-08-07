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
  stems (basic data, the code hash *or* an EIP-7702 delegation
  indicator, and the first 64 storage slots under one stem), overflow
  storage and content-addressed code key derivation
  (`get_tree_key_for_storage_slot`, `get_tree_key_for_code_chunk`),
  code chunking (`chunkify_code`) and basic-data packing
  (`encode_basic_data`). Code is not keyed by account: every chunk
  lives in the code zone addressed by code hash, so contracts with
  identical bytecode share their chunk leaves.

### The trie

`trie::BinaryTrie` is incremental: it inserts, updates and removes in
place. Its canonical structure is update-order independent — any
sequence of insertions and removals arriving at the same key/value set
yields the same root. Insertion splits a node in two when keys diverge
inside its prefix; removal is the inverse, collapsing the parent branch
of the removed leaf into its surviving sibling, which absorbs the bits
the branch consumed. Correctness is pinned by the spec conformance
vectors rather than by a second in-crate implementation.

`BinaryTrie::from_sorted_leaves` builds a whole trie at once from
leaves already in bit order — which, for prefix-free keys, is plain
ascending byte order. Because the structure is canonical, sorted input
determines it: one bottom-up fold builds every node in its final shape,
instead of descending from the root once per key and splitting branches
that a later key splits again. It is what genesis wants, and later
snapshot import. Input is validated, not trusted: keys out of order or
repeated are rejected. The build is in-memory — nothing is written
until `commit()` — so it suits allocs and snapshots that fit in memory
rather than an unbounded stream.

### Storage

The trie is storage-backed. `trie::BinaryTrieDB` is the node store —
`get(path)` / `put_batch`, keyed by a node's bit path from the root, the
same path-keyed single-version model the MPT's `TrieDB` uses — and
`trie::InMemoryBinaryTrieDB` is the implementation this crate ships;
the RocksDB one belongs to `crates/storage`. A child is either loaded
or the hash of a subtree still in the database, so `open` costs
nothing, `root()` reads nothing (a stored reference already is its
hash), and reads and inserts load only the nodes on their path.

A loaded node also caches its own hash and tracks whether the
database's copy of it is stale. `root()` therefore hashes each node at
most once, and `commit()` writes only the nodes that changed —
committing an unchanged trie writes nothing at all. An update clears
both, on every node from the root down to what it changed, so neither
cache can outlive the subtree it describes.

A removal leaves nodes behind in the store: the removed leaf, and the
collapsed branch's surviving child, which moves up one level. `commit()`
carries those paths in the same batch as empty-valued entries, which
the backend deletes — the tombstone convention the MPT's `TrieDB`
already uses, so `BinaryTrieDB` needs no removal method. Nothing below
the survivor moves: it absorbs exactly the bits its old path loses, so
the paths of its whole subtree are unchanged and none of it is
rewritten.

### Test vectors

`tests/vectors/binary_trie_vectors.json` is **vendored** from the EELS
reference implementation, which owns the generator and its schema
documentation:

    ethereum/execution-specs, branch projects/binary-trie
    tests/binary_trie/vectors/

**Vendored from:** `projects/binary-trie` @ `ad84d306`

### Non-goals (today)

No proofs, no fork wiring. These are deferred to the state-commitment
integration work.

### Spec discrepancy

`encode_basic_data` packs `code_size` as 4 bytes at offset 4, following
the EELS branch this crate is ported from; EIP-7864 specifies a 3-byte
field at offset 5. The conformance fixture pins the EELS choice —
revisit when EIP-8297's final layout lands.
