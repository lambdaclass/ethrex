# EncodedTrie Optimization Plan for ZisK

> **Goal:** Bridge the performance gap between ZisK (~5% improvement) and SP1 (~13% improvement) from the EncodedTrie implementation.
>
> **Target:** Achieve ~9-10% total improvement on ZisK (from 525M → ~472M steps)
>
> **Baseline:** Block 24271094, ZisK profile `stats_24271094_20260120_102715.txt`

## Executive Summary

The EncodedTrie eliminated rkyv deserialization but introduced new memcpy overhead. This plan addresses four specific regressions:

| Optimization | Expected Gain | Complexity | Priority |
|--------------|---------------|------------|----------|
| 1. Direct H256 in decode_child | ~8M steps (-1.6%) | Low | P0 |
| 2. Remove Nibbles::clone in encode_extension | ~2M steps (-0.4%) | Low | P0 |
| 3. Inline NodeHash in encode_branch | ~3M steps (-0.6%) | Low | P1 |
| 4. Cache storage authentication | ~10-15M steps (-2-3%) | Medium | P1 |

**Total expected improvement:** 23-28M additional steps saved

---

## Optimization 1: Direct H256 Construction in decode_child

### Problem

`decode_child()` calls `NodeHash::from_slice()` which calls `H256::from_slice()` for 32-byte hashes. This triggers an expensive memcpy.

**Evidence from ZisK profile:**
- Before EncodedTrie: 18,571 calls, 838,858 steps
- After EncodedTrie: 68,825 calls, 9,555,506 steps (**+1040%**)

### Location

**File:** `crates/common/trie/rlp.rs:154-160`

### Current Code

```rust
pub fn decode_child(rlp: &[u8]) -> NodeHash {
    match decode_bytes(rlp) {
        Ok((hash, &[])) if hash.len() == 32 => NodeHash::from_slice(hash),
        Ok((&[], &[])) => NodeHash::default(),
        _ => NodeHash::from_slice(rlp),
    }
}
```

### Proposed Code

```rust
use ethereum_types::H256;

pub fn decode_child(rlp: &[u8]) -> NodeHash {
    match decode_bytes(rlp) {
        Ok((hash, &[])) if hash.len() == 32 => {
            // Direct H256 construction to avoid memcpy from H256::from_slice
            // SAFETY: We verified hash.len() == 32
            let arr: [u8; 32] = hash.try_into().expect("length checked above");
            NodeHash::Hashed(H256(arr))
        }
        Ok((&[], &[])) => NodeHash::default(),
        _ => NodeHash::from_slice(rlp),
    }
}
```

### Alternative (Zero-Copy)

If the hash slice lifetime permits, consider:

```rust
pub fn decode_child(rlp: &[u8]) -> NodeHash {
    match decode_bytes(rlp) {
        Ok((hash, &[])) if hash.len() == 32 => {
            // Use pointer cast for zero-copy when slice is aligned
            let ptr = hash.as_ptr() as *const [u8; 32];
            // SAFETY: We verified length is 32, alignment is guaranteed for u8
            NodeHash::Hashed(H256(unsafe { *ptr }))
        }
        Ok((&[], &[])) => NodeHash::default(),
        _ => NodeHash::from_slice(rlp),
    }
}
```

### Success Criteria

- [ ] `H256::from_slice` calls reduced from 68,825 to <20,000
- [ ] `H256::from_slice` steps reduced from 9.5M to <1M
- [ ] All existing tests pass
- [ ] `make test` passes for `ethrex-trie`

### Verification

```bash
# Run ZisK profile and check H256::from_slice in memcpy callers
cd scripts/zkvm-bench
./profile-zisk.sh

# Grep for H256::from_slice in output
grep "H256::from_slice" profiles/zisk/stats_*.txt
```

---

## Optimization 2: Remove Nibbles::clone in encode_extension

### Problem

`encode_extension()` takes `Nibbles` by value, causing clones at call sites. The clone triggers memcpy for the internal `Vec<u8>`.

### Locations

**File:** `crates/common/trie/encodedtrie.rs`

Call sites:
- Line 700: `encode_extension(prefix, child_hash)` - prefix is from `get_extension_data` (borrowed)
- Line 706: `encode_extension(prefix.clone(), child_hash)` - explicit clone

### Current Code

```rust
// Line 886-896
fn encode_extension(path: Nibbles, child: NodeHash) -> Vec<u8> {
    // Pre-allocate: RLP overhead (3-5 bytes) + compact path + child hash (up to 33 bytes)
    let compact = path.encode_compact();
    let estimated_size = 5 + compact.len() + 33;
    let mut buf = Vec::with_capacity(estimated_size);
    let mut encoder = Encoder::new(&mut buf);
    encoder = encoder.encode_bytes(&compact);
    encoder = child.encode(encoder);
    encoder.finish();
    buf
}
```

### Proposed Code

```rust
// Line 886-896
fn encode_extension(path: &Nibbles, child: NodeHash) -> Vec<u8> {
    // Pre-allocate: RLP overhead (3-5 bytes) + compact path + child hash (up to 33 bytes)
    let compact = path.encode_compact();
    let estimated_size = 5 + compact.len() + 33;
    let mut buf = Vec::with_capacity(estimated_size);
    let mut encoder = Encoder::new(&mut buf);
    encoder = encoder.encode_bytes(&compact);
    encoder = child.encode(encoder);
    encoder.finish();
    buf
}
```

**Update call sites:**

```rust
// Line 700 - already has a borrow, just add &
let encoded = encode_extension(&prefix, child_hash);

// Line 706 - remove clone
let encoded = encode_extension(prefix, child_hash);
```

Wait - line 706 has `prefix.clone()` where `prefix` is `&Nibbles`. We need to check if prefix is `Some(prefix)` pattern matched.

### Detailed Call Site Analysis

```rust
// Line 688-708
NodeType::Extension {
    prefix,        // This is Option<Nibbles> from the pattern match
    child_index,
} => match (prefix, child_index) {
    (None, None) => {
        trie.hashes[index] = trie.hash_encoded_data(index).map(Some)?;
    }
    (_, Some(child_index)) => {
        recursive(trie, *child_index)?;
        let child_hash = trie.get_hash(*child_index)?;
        let prefix = trie.get_extension_data(index)?;  // Returns Nibbles (owned)
        let encoded = encode_extension(prefix, child_hash);  // Can pass owned
        trie.hashes[index] = Some(NodeHash::from_encoded(&encoded));
    }
    (Some(prefix), None) => {
        let child_hash = trie.get_extension_encoded_child_hash(index)?;
        let encoded = encode_extension(prefix.clone(), child_hash);  // prefix is &Nibbles here
        trie.hashes[index] = Some(NodeHash::from_encoded(&encoded));
    }
},
```

**Corrected proposed changes:**

```rust
// Change function signature
fn encode_extension(path: &Nibbles, child: NodeHash) -> Vec<u8> { ... }

// Line 700 - prefix is Nibbles (owned from get_extension_data)
let encoded = encode_extension(&prefix, child_hash);

// Line 706 - prefix is &Nibbles (from pattern match Some(prefix))
let encoded = encode_extension(prefix, child_hash);  // Remove .clone()
```

### Success Criteria

- [ ] No `Nibbles::clone` calls from encode_extension path
- [ ] Memcpy steps from `EncodedTrie::hash::recursive` reduced by ~2M
- [ ] All existing tests pass

### Verification

```bash
# Check for Nibbles clone in profile
grep -i "nibbles.*clone" profiles/zisk/stats_*.txt

# Run trie tests
cargo test -p ethrex-trie
```

---

## Optimization 3: Inline NodeHash Encoding in encode_branch

### Problem

`encode_branch` uses `buf.put_slice()` for inline NodeHash values, which calls memcpy even for small slices (< 32 bytes).

**Evidence from ZisK profile:**
- `encode_branch`: 82,746 calls, 11,029,548 memcpy steps

### Location

**File:** `crates/common/trie/encodedtrie.rs:898-924`

### Current Code

```rust
fn encode_branch(children: [Option<NodeHash>; 16]) -> Vec<u8> {
    // ... payload_len calculation ...

    let mut buf: Vec<u8> = Vec::with_capacity(payload_len + 3);
    encode_length(payload_len, &mut buf);

    for child in children.iter() {
        let Some(child) = child else {
            buf.put_u8(RLP_NULL);
            continue;
        };
        match child {
            NodeHash::Hashed(hash) => hash.0.encode(&mut buf),
            NodeHash::Inline((_, 0)) => buf.put_u8(RLP_NULL),
            NodeHash::Inline((encoded, len)) => buf.put_slice(&encoded[..*len as usize]),  // memcpy here
        }
    }
    buf.put_u8(RLP_NULL);
    buf
}
```

### Proposed Code

```rust
fn encode_branch(children: [Option<NodeHash>; 16]) -> Vec<u8> {
    // ... payload_len calculation unchanged ...

    let mut buf: Vec<u8> = Vec::with_capacity(payload_len + 3);
    encode_length(payload_len, &mut buf);

    for child in children.iter() {
        let Some(child) = child else {
            buf.push(RLP_NULL);  // push instead of put_u8
            continue;
        };
        match child {
            NodeHash::Hashed(hash) => hash.0.encode(&mut buf),
            NodeHash::Inline((_, 0)) => buf.push(RLP_NULL),
            NodeHash::Inline((encoded, len)) => {
                // Inline byte-by-byte copy to avoid memcpy overhead for small slices
                // Inline nodes are always < 32 bytes, typically 1-20 bytes
                let len = *len as usize;
                buf.reserve(len);
                for i in 0..len {
                    buf.push(encoded[i]);
                }
            }
        }
    }
    buf.push(RLP_NULL);
    buf
}
```

### Alternative: extend_from_slice with reserve

```rust
NodeHash::Inline((encoded, len)) => {
    let len = *len as usize;
    // extend_from_slice may be optimized better than put_slice
    // when the compiler can see the small size
    buf.extend_from_slice(&encoded[..len]);
}
```

### Benchmark Both Approaches

Create a micro-benchmark to determine which is faster in ZisK:

```rust
#[cfg(test)]
mod bench {
    use super::*;

    #[test]
    fn bench_inline_encoding() {
        let encoded = [1u8; 31];
        let len = 20u8;

        // Approach 1: put_slice
        let mut buf1 = Vec::with_capacity(100);
        for _ in 0..10000 {
            buf1.clear();
            buf1.put_slice(&encoded[..len as usize]);
        }

        // Approach 2: byte-by-byte
        let mut buf2 = Vec::with_capacity(100);
        for _ in 0..10000 {
            buf2.clear();
            for i in 0..len as usize {
                buf2.push(encoded[i]);
            }
        }

        // Approach 3: extend_from_slice
        let mut buf3 = Vec::with_capacity(100);
        for _ in 0..10000 {
            buf3.clear();
            buf3.extend_from_slice(&encoded[..len as usize]);
        }
    }
}
```

### Success Criteria

- [ ] `encode_branch` memcpy steps reduced from 11M to <5M
- [ ] No performance regression in non-zkVM builds
- [ ] All existing tests pass

### Verification

```bash
# Profile and check encode_branch
grep "encode_branch" profiles/zisk/stats_*.txt
```

---

## Optimization 4: Cache Storage Authentication Results

### Problem

Storage slot access during VM execution repeatedly authenticates the same trie paths. Each `get_storage_slot` call triggers `authenticate::recursive` traversing overlapping paths.

**Evidence from ZisK profile:**
- `get_storage_slot`: 1,468 calls, 41,147,193 steps (8.20% of total)
- `authenticate::recursive`: 10,665 calls, 119,898,430 steps

Many storage accesses are to the same contract, traversing the same account trie path multiple times.

### Location

**File:** `crates/common/types/block_execution_witness.rs`

### Current Flow

```
get_storage_slot(address, key)
  → get_valid_storage_trie(address)
    → storage_tries.get(address)
    → trie.authenticate()  // Full authentication every time
  → trie.get(key)
```

### Proposed Flow

```
get_storage_slot(address, key)
  → get_valid_storage_trie(address)
    → authenticated_storage_tries.get(address)  // Check cache first
    → IF NOT CACHED:
        → storage_tries.get(address)
        → trie.authenticate()
        → authenticated_storage_tries.insert(address, trie)
    → RETURN cached authenticated trie
  → trie.get(key)
```

### Implementation

**Add to `GuestProgramState` struct:**

```rust
pub struct GuestProgramState {
    // ... existing fields ...

    /// Cache of storage tries that have already been authenticated.
    /// Key is the account address, value is the authenticated storage trie.
    #[serde(skip)]
    authenticated_storage_tries: HashMap<Address, EncodedTrie>,
}
```

**Modify `get_valid_storage_trie`:**

```rust
impl GuestProgramState {
    pub fn get_valid_storage_trie(
        &mut self,
        address: &Address,
    ) -> Result<&EncodedTrie, StatelessValidationError> {
        // Check cache first
        if self.authenticated_storage_tries.contains_key(address) {
            return Ok(self.authenticated_storage_tries.get(address).unwrap());
        }

        // Get and authenticate the storage trie
        let storage_trie = self
            .storage_tries
            .get_mut(address)
            .ok_or(StatelessValidationError::StorageTrieNotFound(*address))?;

        // Authenticate
        let root = storage_trie.authenticate()
            .map_err(|e| StatelessValidationError::InvalidStorageTrie(e.to_string()))?;

        // Verify root matches expected
        let expected_root = self.get_account_storage_root(address)?;
        if root.finalize() != expected_root {
            return Err(StatelessValidationError::InvalidStorageRoot {
                address: *address,
                expected: expected_root,
                got: root.finalize(),
            });
        }

        // Cache the authenticated trie (clone or move depending on usage pattern)
        self.authenticated_storage_tries.insert(*address, storage_trie.clone());

        Ok(self.authenticated_storage_tries.get(address).unwrap())
    }
}
```

### Alternative: Lazy Authentication with Memoization

Instead of authenticating the full trie upfront, authenticate paths lazily and memoize:

```rust
impl EncodedTrie {
    /// Get a value, authenticating the path lazily
    pub fn get_authenticated(
        &mut self,
        path: &[u8],
        expected_root: H256,
    ) -> Result<Option<&[u8]>, EncodedTrieError> {
        // Authenticate only nodes on the access path
        // Cache authentication results in self.hashes
        // This is already partially implemented - just need to expose it
        ...
    }
}
```

### Success Criteria

- [ ] `authenticate::recursive` total calls reduced by 50%+
- [ ] `get_storage_slot` steps reduced from 41M to <25M
- [ ] Storage root verification still passes
- [ ] No correctness regressions in EF tests

### Verification

```bash
# Run EF tests
cd tooling/ef_tests
cargo run --release

# Profile and check authenticate calls
grep "authenticate" profiles/zisk/stats_*.txt
```

---

## Implementation Order

### Phase 1: Low-Hanging Fruit (P0)

1. **Optimization 1: decode_child** - 1 file, ~20 lines changed
2. **Optimization 2: encode_extension signature** - 1 file, ~5 lines changed

**Expected gain:** ~10M steps
**Time estimate:** 1-2 hours
**Risk:** Low

### Phase 2: Medium Effort (P1)

3. **Optimization 3: encode_branch inline** - 1 file, ~10 lines changed
4. **Optimization 4: Storage cache** - 1 file, ~50 lines changed

**Expected gain:** ~13-18M steps
**Time estimate:** 3-4 hours
**Risk:** Medium (need to verify correctness)

---

## Testing Plan

### Unit Tests

```bash
# Run all trie tests
cargo test -p ethrex-trie

# Run block execution tests
cargo test -p ethrex-common --lib block_execution

# Run VM tests
cargo test -p ethrex-vm
```

### Integration Tests

```bash
# EF tests (blockchain tests)
cd tooling/ef_tests
cargo run --release -- blockchain

# L2 prover tests
cargo test -p ethrex-l2-prover
```

### ZisK Profiling

```bash
cd scripts/zkvm-bench

# Build guest program
./build-zisk.sh

# Run profile
./profile-zisk.sh

# Compare with baseline
diff profiles/zisk/baseline.txt profiles/zisk/stats_*.txt
```

### Regression Criteria

A change is acceptable if:
1. All unit tests pass
2. All EF blockchain tests pass
3. ZisK total steps decrease or stay the same
4. No new functions appear in top 25 cost functions unless expected

---

## Rollback Plan

Each optimization is independent. If an optimization causes issues:

1. Revert the specific commit
2. Re-run ZisK profile to confirm baseline restored
3. Investigate root cause before re-attempting

---

## Appendix: Profile Comparison Commands

```bash
# Generate baseline profile (before changes)
cd scripts/zkvm-bench
./profile-zisk.sh
cp profiles/zisk/stats_*.txt profiles/zisk/baseline_opt.txt

# After implementing optimizations, generate new profile
./profile-zisk.sh

# Compare key metrics
echo "=== Steps Comparison ==="
grep "^STEPS" profiles/zisk/baseline_opt.txt profiles/zisk/stats_*.txt

echo "=== memcpy Comparison ==="
grep "memcpy" profiles/zisk/baseline_opt.txt profiles/zisk/stats_*.txt

echo "=== H256::from_slice Comparison ==="
grep "H256::from_slice" profiles/zisk/baseline_opt.txt profiles/zisk/stats_*.txt
```

---

## References

- ZisK Baseline Profile: `scripts/zkvm-bench/profiles/zisk/baseline.txt`
- Post-EncodedTrie Profile: `scripts/zkvm-bench/profiles/zisk/stats_24271094_20260120_102715.txt`
- SP1 Comparison Tool: `scripts/zkvm-bench/parse_sp1_profile.py`
- Context Document: `~/Personal/contexts/lambdaclass-ethrex.md`
