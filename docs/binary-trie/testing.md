# Binary Trie Backend — Testing Strategy

## Layers of testing

1. **Unit tests** — per-module correctness of `ethrex-binary-trie` internals.
2. **Test vectors** — cross-implementation checks against the EIP-7864 Python reference.
3. **StateBackend conformance** — `BinaryBackend` as a `StateReader`/`StateCommitter`.
4. **Transition integration tests** — overlay read/write/tombstone/code-chunk/restart/reorg semantics on `TransitionBackend`.
5. **Manual smoke tests** — running against mainnet. Documented in `operational.md`. Not part of CI.

EF blockchain / state tests are out of scope for this PR.

## Test vectors

### Source

The reference Python generator is on branch `eip-7864-plan` at `crates/common/binary_trie/testgen/generate_vectors.py`. Note the source path uses an underscore (`binary_trie`) — that branch predates the workspace's hyphen convention for new crates and uses a flat layout without `src/`. Ported into `crates/common/binary-trie/testgen/generate_vectors.py` in this PR (hyphenated per Rust workspace convention).

The generator writes all JSON files directly into `crates/common/binary-trie/testgen/` (the script does NOT emit to stdout). Files produced:

- `test_vectors.json` — original raw-insert vectors (Phase 1)
- `vectors_accounts.json`, `vectors_storage.json`, `vectors_codechunk.json`, `vectors_negative.json` — Phase 2 suites

A `test_vectors.json` entry has the shape:

```json
{
  "name": "two_addresses_distinct_stems",
  "inserts": [
    {"key": "<64-hex 32-byte key>", "value": "<64-hex 32-byte value>"}
  ],
  "expected_root": "<64-hex 32-byte root>",
  "note": "(optional)"
}
```

Each Phase 2 vector file has its own schema; the authoritative source is the corresponding `#[serde(deny_unknown_fields)]` struct in `tests/test_vectors.rs`.

### How to regenerate

```bash
# From crate root:
python3 crates/common/binary-trie/testgen/generate_vectors.py

# From testgen/:
cd crates/common/binary-trie/testgen && python3 generate_vectors.py
```

Either form writes the 5 JSON files into `testgen/` in place. Requires `blake3` Python package (`pip install blake3`). The generator is deterministic (PRNG seed `0xDEADBEEF42`); re-running must produce byte-identical output.

### Coverage expected

Phase 1 vectors (required to land):

- Empty trie (expected_root = `[0u8; 32]`)
- Single BASIC_DATA leaf
- Single CODE_HASH leaf
- BASIC_DATA + CODE_HASH on same stem
- Two addresses with distinct stems
- Two addresses with stems sharing a prefix (forces `InternalNode` splits at varying depths)
- Storage slot in header range (slot < 64)
- Storage slot in main range (slot >= 256)
- Storage slot exactly at boundary (slot == CODE_OFFSET - HEADER_STORAGE_OFFSET)
- Contract with 1 chunk of code
- Contract with code that crosses PUSH data boundaries
- Contract with exactly 791 chunks (max contract size)
- SELFDESTRUCT tombstone (after setting a stem, remove it; verify root matches "stem never existed")
- Update a previously-set value (old leaf replaced, not appended)

Phase 2 vectors (non-blocking; can land after Phase 3):
- 50 addresses at random, interleaved ops, stress-test stem sharing
- Account with 100 storage slots, spread across header and main ranges
- Sequential deploy + selfdestruct + redeploy of the same address

### Running vectors in Rust

```bash
cargo test -p ethrex-binary-trie --test test_vectors
```

Each JSON entry is executed as a separate `#[test]`. Failure output includes the description, the diverging hash, the expected hash, and the operation index that triggered the divergence.

## Unit tests

Per-module (all in `crates/common/binary-trie/`):

- `node.rs` — `StemNode::get/set/remove`, `InternalNode` child navigation, `stem_bit` correctness for all 8 bit positions of all 31 stem bytes.
- `trie.rs` — `insert`, `insert_multi`, `get`, `get_shared`, `remove` on a fresh `NodeStore`.
- `hash.rs` — BLAKE3 determinism, round-trip tests.
- `key_mapping.rs` — `get_tree_key` against Python-generated pairs, `chunkify_code` against hand-computed vectors, `pack_basic_data`/`unpack_basic_data` round-trips.
- `merkle.rs` — `merkle_hash_64`, `merkelize` on single-leaf, two-leaf, 256-leaf stems.
- `node_store.rs` — allocation, serialization, disk round-trip via an in-memory backend.
- `proof.rs` — `get_proof` + `verify` on presence, same-stem absence, different-stem absence, empty-child absence. Tamper with each response field and assert `verify` returns false.
- `state.rs` — `apply_account_update` semantics including removal + tombstone, `take_block_diffs` correctness.
- `layer_cache.rs` — put/get/commit semantics with tombstone framing.
- `witness.rs` — recorder correctness on a scripted sequence.

## StateBackend conformance

`crates/common/binary-trie/tests/state_backend.rs` drives `BinaryBackend` through `StateReader` + `StateCommitter`:

- `account` returns `None` for unset addresses
- `account` returns the expected `AccountInfo` after `update_accounts`
- `storage` returns `H256::zero()` for unset slots
- `storage` returns the expected value after `update_storage`
- `code` returns `None` for unset, the expected bytecode for set via `update_accounts` with `code: Some(_)`
- `hash` returns the same root as `commit` followed by a fresh reader at the committed root
- `commit` returns a `MerkleOutput` with `NodeUpdates::Binary { node_diffs, deleted_stems }` containing exactly the changed leaves

Each test mirrors the analogous MPT conformance test in `ethrex-trie` for side-by-side comparison.

## Transition integration tests

`crates/storage/tests/transition.rs` (new file) drives a real `Store` through:

1. **Read before write**: seed MPT with accounts A and B; activate transition; read A from the `TransitionBackend`; confirm fallback to MPT succeeds.
2. **Write + read**: write A in overlay; confirm subsequent read sees the new value, not MPT's.
3. **Partial stem invariant**: write only `balance` on MPT-resident account A; confirm all four BASIC_DATA sub-leaves + CODE_HASH are present in overlay after the write (copy-on-write).
4. **SELFDESTRUCT tombstone**: activate transition; SELFDESTRUCT an MPT-resident account A; confirm subsequent read returns `None` (not A's pre-switch state).
5. **Code chunk write + hash read**: deploy new contract post-switch; confirm `code(addr, hash)` returns the bytecode (from `AccountCodes`); confirm binary trie also has the chunks (via direct `BinaryBackend` inspection).
6. **Pre-switch code read**: EXTCODECOPY on an MPT-resident contract post-switch; confirm bytecode is returned (from `AccountCodes`, populated pre-switch).
7. **Layer cache + restart**: apply writes to overlay; kill process (simulated drop); restart from the same `Store`; confirm committed state is preserved, uncommitted cache layers are lost (consistent with MPT behavior).
8. **Switch metadata persistence**: activate transition; shut down; restart; confirm `TransitionBackend` is reconstructed with the same `(switch_block, frozen_mpt_root, binary_root)`.
9. **Reorg under 128 blocks, post-switch**: apply a reorg of depth 100 crossing blocks after the switch; confirm overlay state is correctly rolled back via the binary layer cache.

Each test is `#[test]` against an in-memory `Store`.

## Benchmarks

Optional, added in Phase 10:

- `build_block_benchmark` with and without transition. Regression budget: binary trie should be within 10× of MPT single-threaded merkleization for a 100-stem block. (Not apples-to-apples — BLAKE3 is faster than keccak per byte, but stems are individually hashed vs. MPT's fanout. Profile output informs future optimization.)
- `storage_hot_loop_benchmark`: a tight SLOAD loop on a 10k-slot contract, measuring overlay hit/miss composition.

## Running the whole binary-trie test suite

```bash
cargo test -p ethrex-binary-trie
cargo test -p ethrex-storage --test transition
```

Total runtime budget: under 30 seconds on a modern laptop, so the suite is trivially CI-friendly.
