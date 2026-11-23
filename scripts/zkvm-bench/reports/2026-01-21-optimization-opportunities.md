# zkVM Optimization Opportunities Analysis

**Date:** 2026-01-21
**Branch:** zerocopy_trie
**Baseline:** Main (699.7M steps) vs Latest zerocopy (663.2M steps)
**Current Improvement:** -5.2% (-36.5M steps)

---

## Executive Summary

This report analyzes five optimization opportunities for the ethrex zkVM guest program, with precise performance estimates based on profiling data from block 23769082.

| Priority | Optimization | Steps Saved | % Gain | Cost Saved | Effort |
|----------|-------------|-------------|--------|------------|--------|
| **P0** | rkyv Zero-Copy | 35-40M | 5.3-6.0% | 4-5B | High |
| **P1** | Code Hash Pre-computation | 25-30M | 3.8-4.5% | 4.5-5.5B | Medium |
| **P2** | fake_exponential Lookup Table | 25-30M | 3.8-4.5% | 5-6B | Medium |
| **P3** | Array-based get_encoded_items | 9-15M | 1.4-2.3% | 0.8-1.2B | Low |
| **P4** | Lazy Trie Encoding | 5-10M | 0.8-1.5% | 0.5-1B | Medium |

**Total Potential Gain:** 99-125M steps (14.9-18.8% additional reduction)

---

## Current Profile Breakdown (663.2M steps total)

### Top Functions by Steps

| Function | Steps | % | Calls | Steps/Call |
|----------|-------|---|-------|------------|
| `hash::recursive` | 196.0M | 29.5% | 10,182 | 19,249 |
| memcpy | 153.0M | 23.1% | 1,001,396 | 153 |
| `authenticate::recursive` | 136.8M | 20.6% | 12,681 | 10,787 |
| `tiny_keccak::update` | 82.7M | 12.5% | 38,024 | 2,175 |
| `fake_exponential` (via get_base_fee) | 32.9M | 5.0% | 511 | 64,290 |
| `Code::from_bytecode` | 33.1M | 5.0% | 279 | 118,654 |
| `get_encoded_items` | 29.6M | 4.5% | 17,173 | 1,724 |
| rkyv deserialize | 22.5M | 3.4% | 1 | 22,518,369 |

### Top Functions by Cost

| Function | Cost | % | Primary Bottleneck |
|----------|------|---|-------------------|
| `hash::recursive` | 23.8B | 31.8% | Keccak hashing |
| `authenticate::recursive` | 21.3B | 28.5% | Keccak + RLP decode |
| `tiny_keccak::update` | 15.8B | 21.2% | Keccak precompile |
| memcpy | 14.1B | 18.8% | Memory operations |
| keccak precompile | 13.0B | 17.4% | (expected) |
| `fake_exponential` | 6.1B | 8.2% | U256::div_mod |
| `Code::from_bytecode` | 5.9B | 7.9% | Keccak hashing |

---

## P0: rkyv Zero-Copy Deserialization

### Current State

**Problem:** Full deserialization of `ExecutionWitness` at startup copies all data.

```
Profile Data:
- ArchivedProgramInput::deserialize: 22,518,369 steps (3.4%)
- ArchivedExecutionWitness::deserialize: 21,408,466 steps (3.2%)
- BTreeMap visit_raw: 81,363,584 steps (implicit overlap)
- Total rkyv-related: ~44M steps (6.6%)
```

**Entry Points:**
- `crates/l2/prover/src/guest_program/src/zisk/src/main.rs:12`
- Uses `rkyv::from_bytes::<ProgramInput, Error>(&input)`

### Technical Details

**Current Flow:**
```
Serialized bytes → rkyv::from_bytes() → ExecutionWitness → TryFrom → GuestProgramState
                         ↑                                    ↑
              Full deserialization              Copies all Vec<Vec<u8>> fields
```

**Fields with Allocation Overhead:**

| Field | Type | rkyv Wrapper | Zero-Copy? |
|-------|------|--------------|------------|
| `codes` | `Vec<Vec<u8>>` | `VecVecWrapper` | **No** - getter clones |
| `block_headers_bytes` | `Vec<Vec<u8>>` | `VecVecWrapper` | **No** - getter clones |
| `storage_tries` | `BTreeMap<Address, EncodedTrie>` | `MapKV` | Partial |
| `state_trie` | `Option<EncodedTrie>` | Identity | Yes |
| `first_block_number` | `u64` | None | Yes |

**Root Cause:** `VecVecWrapper` at `crates/common/rkyv_utils.rs:15-30`:
```rust
fn vec_vec_to_vec(vec_vec: &[Vec<u8>]) -> Vec<Vec<u8>> {
    vec_vec.iter().map(|b| b.to_vec()).collect()  // CLONES all data
}
```

### Optimization Approach

**Option A: Direct Archived Access (Recommended)**
```rust
// Before
let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();

// After
let archived = rkyv::access::<ArchivedProgramInput, _>(&input).unwrap();
// Access fields directly from archived without full deserialization
```

**Option B: Custom ArchiveWith for Vec<Vec<u8>>**
```rust
// Return &[&[u8]] instead of Vec<Vec<u8>>
impl ArchiveWith<Vec<Vec<u8>>> for ZeroCopyVecVec {
    fn resolve_with(...) -> Self::Resolver { ... }
}
```

### Performance Estimate

| Metric | Current | Optimized | Savings |
|--------|---------|-----------|---------|
| Steps | 44M | 5-10M | **35-40M (79-88%)** |
| Cost | 7.2B | 1-2B | **5-6B** |
| % Total | 6.6% | 0.8-1.5% | **5.3-6.0%** |

**Calculation:**
- rkyv deserialize steps: 22.5M + 21.4M = 43.9M
- Zero-copy eliminates ~80% of copies
- Savings: 43.9M × 0.80 = 35.1M steps

### Implementation Complexity

- **Effort:** High (2-3 days)
- **Risk:** Medium (requires careful API changes)
- **Files to modify:**
  - `crates/common/rkyv_utils.rs` - new zero-copy wrappers
  - `crates/common/types/block_execution_witness.rs` - TryFrom changes
  - `crates/l2/prover/src/guest_program/src/*/main.rs` - entry point

---

## P1: Code Hash Pre-computation

### Current State

**Problem:** `Code::from_bytecode` computes Keccak-256 hash every time, even when hash is already known.

```
Profile Data:
- Code::from_bytecode: 33,104,148 steps (5.0%), 5,942,240,065 cost (7.9%)
- Called 279 times
- Average: 118,654 steps per call
- Keccak dominates (~90% of cost)
```

**Location:** `crates/common/types/account.rs:52-59`
```rust
pub fn from_bytecode(code: Bytes) -> Self {
    let jump_targets = Self::compute_jump_targets(&code);
    Self {
        hash: keccak(code.as_ref()),  // EXPENSIVE: ~100k+ steps
        bytecode: code,
        jump_targets,
    }
}
```

### Call Sites Analysis

**High-frequency (per block):**
1. `block_execution_witness.rs:159` - Witness deserialization (279 calls in profile)
2. `execution_handlers.rs:68` - Contract creation (CREATE/CREATE2)
3. `gen_db.rs:55` - VM database initialization

**Low-frequency:**
- Genesis initialization
- P2P sync (already uses unchecked variant)

### Technical Details

The witness already contains bytecode, but NOT the pre-computed hash:

```rust
// crates/common/types/block_execution_witness.rs:156-162
let codes_hashed = codes
    .into_iter()
    .map(|code| {
        let code = Code::from_bytecode(code.into());  // Hash computed HERE
        (code.hash, code)
    })
    .collect();
```

### Optimization Approach

**Option A: Store code hashes in witness (Recommended)**

Modify `ExecutionWitness` to include pre-computed hashes:
```rust
pub struct ExecutionWitness {
    pub codes: Vec<Vec<u8>>,
    pub code_hashes: Vec<H256>,  // NEW: pre-computed by host
    // ...
}

// Guest uses unchecked variant:
let code = Code::from_bytecode_unchecked(bytecode, hash);
```

**Option B: Cache in GuestProgramState**
```rust
struct CodeCache {
    cache: HashMap<H256, Code>,
}

fn get_or_compute_code(&mut self, bytecode: &[u8]) -> &Code {
    let hash = keccak(bytecode);  // Still computed once
    self.cache.entry(hash).or_insert_with(|| Code::from_bytecode_unchecked(...))
}
```

### Performance Estimate

| Metric | Current | Optimized | Savings |
|--------|---------|-----------|---------|
| Steps | 33.1M | 5-8M | **25-28M (75-85%)** |
| Cost | 5.9B | 0.5-1B | **4.5-5.5B** |
| % Total | 5.0% | 0.8-1.2% | **3.8-4.2%** |

**Calculation:**
- 279 calls × 118,654 steps = 33.1M steps
- Keccak accounts for ~90% = 29.8M steps
- Pre-computed hashes eliminate keccak: -29.8M steps
- Remaining (jump_targets): ~3.3M steps
- Net savings: ~25-28M steps

### Implementation Complexity

- **Effort:** Medium (1 day)
- **Risk:** Low (additive change)
- **Files to modify:**
  - `crates/common/types/block_execution_witness.rs` - add `code_hashes` field
  - Host-side witness generation - compute hashes
  - `crates/common/types/account.rs` - use unchecked path

---

## P2: fake_exponential Lookup Table

### Current State

**Problem:** `fake_exponential` uses Taylor series with expensive U256 division.

```
Profile Data:
- get_base_fee_per_blob_gas: 32,852,190 steps (5.0%), 3,046,660,183 cost (4.1%)
- fake_exponential (direct): 6,119,576,846 cost (8.2%), 1,027 calls
- U256::div_mod: 3,534,089,418 cost (4.7%), 62,957 calls
- Total related cost: ~9-12B (12-16%)
```

**Location:** `crates/common/types/block.rs:452-500`

### Algorithm Analysis

```rust
pub fn fake_exponential(factor: U256, numerator: U256, denominator: u64) -> U256 {
    // Taylor series: factor * e^(numerator/denominator)
    while !numerator_accum.is_zero() {
        output += numerator_accum;
        numerator_accum = (numerator_accum * numerator) / denominator_by_i;  // EXPENSIVE
        denominator_by_i += denominator;
    }
    output / denominator
}
```

**Per-call cost breakdown:**
- Average 10-20 loop iterations
- Each iteration: 1 U256 mul + 1 U256 div
- U256 div is ~1,000-2,000 cycles in zkVM

### Why 1,027 Calls?

1. Per-transaction blob gas calculation: ~511 txs
2. RPC receipt generation: ~500+ calls
3. Block validation: ~15 calls

### Optimization Approach

**Option A: Lookup Table (Recommended)**

For EIP-4844, denominator is fixed at 3,338,477. Pre-compute results:

```rust
// Pre-computed for BLOB_BASE_FEE_UPDATE_FRACTION = 3,338,477
const BLOB_FEE_TABLE: [(u64, U256); 32] = [
    (0, U256::from(1)),           // 1 wei base
    (3_338_477, U256::from(3)),   // e^1 ≈ 2.718
    (6_676_954, U256::from(7)),   // e^2 ≈ 7.389
    // ... exponential growth
];

pub fn calculate_base_fee_per_blob_gas_fast(excess: u64) -> U256 {
    // Binary search + linear interpolation
    // O(log n) instead of O(iterations × div_cost)
}
```

**Option B: Fixed-point arithmetic**
```rust
// Use i128 instead of U256 for intermediate calculations
// Valid since numerator < 400M (EIP-4844 limit)
fn fake_exponential_i128(factor: u64, numerator: u64, denom: u64) -> u64
```

### Performance Estimate

| Metric | Current | Optimized | Savings |
|--------|---------|-----------|---------|
| Steps | ~35M | 2-5M | **30-33M (85-95%)** |
| Cost | 9-12B | 0.5-1B | **8-11B** |
| % Total | 5.3% | 0.3-0.8% | **4.5-5.0%** |

**Calculation:**
- 1,027 calls × ~34,000 steps/call = 35M steps
- Lookup table: ~2,000 steps/call (binary search + interpolation)
- Savings: 35M - 2M = 33M steps

### Implementation Complexity

- **Effort:** Medium (1-2 days)
- **Risk:** Low (can validate against current impl)
- **Files to modify:**
  - `crates/common/types/block.rs` - add lookup table + fast path
  - `crates/common/types/constants.rs` - table constants

---

## P3: Array-based get_encoded_items for Branch Nodes

### Current State

**Problem:** `get_encoded_items` allocates `Vec::with_capacity(17)` for every call.

```
Profile Data:
- get_encoded_items: 29,598,391 steps (4.5%), 2,501,118,431 cost (3.3%)
- Called 17,173 times
- Average: 1,724 steps per call
```

**Location:** `crates/common/trie/encodedtrie.rs:891-904`
```rust
pub fn get_encoded_items(&self, index: usize) -> Result<Vec<&[u8]>, RLPDecodeError> {
    let mut rlp_items = Vec::with_capacity(17);  // ALLOCATION
    while !decoder.is_done() && rlp_items.len() < 17 {
        // ...
        rlp_items.push(item);
    }
    Ok(rlp_items)
}
```

### Call Sites

1. `authenticate::recursive` line 654 - Branch node validation
2. `hash::recursive` line 752 - Branch node hashing (only when pruned)

Both always need exactly 17 items for branch nodes.

### Optimization Approach

Add specialized function returning fixed-size array:

```rust
/// Gets exactly 17 encoded items for branch nodes (no Vec allocation)
#[inline]
pub fn get_branch_encoded_items(&self, index: usize) -> Result<[&[u8]; 17], RLPDecodeError> {
    let node = &self.nodes[index];
    let encoded_range = node.encoded_range.expect("no encoded range");
    let data = &self.encoded_data[encoded_range.0..encoded_range.1];

    let mut items: [&[u8]; 17] = [&[]; 17];
    let mut decoder = Decoder::new(data)?;

    for i in 0..17 {
        let (item, new_decoder) = decoder.get_encoded_item_ref()?;
        items[i] = item;
        decoder = new_decoder;
    }
    Ok(items)
}
```

### Performance Estimate

| Metric | Current | Optimized | Savings |
|--------|---------|-----------|---------|
| Steps | 29.6M | 15-20M | **9-15M (30-50%)** |
| Cost | 2.5B | 1.3-1.7B | **0.8-1.2B** |
| % Total | 4.5% | 2.3-3.0% | **1.4-2.2%** |

**Calculation:**
- Vec allocation overhead: ~500-800 steps per call
- 17,173 calls × 600 steps = 10.3M steps from allocation
- Additional memcpy for Vec growth: ~3-5M steps
- Conservative estimate: 30-50% reduction

### Implementation Complexity

- **Effort:** Low (2-4 hours)
- **Risk:** Very low (additive change)
- **Files to modify:**
  - `crates/common/trie/encodedtrie.rs` - add `get_branch_encoded_items`
  - Update callers in `authenticate` and `hash`

---

## P4: Lazy Trie Encoding

### Current State

**Problem:** Trie nodes are re-encoded on every modification before hashing.

```
Profile Data:
- insert_inner: 25,803,904 steps (3.9%), 2,351,741,990 cost (3.1%)
- Called 12,489 times
- encode_branch/encode_leaf/encode_extension called during hash()
```

**Current Flow:**
```
insert() → modify node → [encode later in hash()]
                              ↑
                    Re-encodes ALL modified nodes
```

### Technical Details

In `hash::recursive` at line 711-780:
```rust
NodeType::Leaf { partial, value } => {
    if partial.is_some() || value.is_some() {
        // Re-encode with new values
        let (partial, value) = trie.get_leaf_data(index)?;
        let encoded = encode_leaf(&partial, value);  // ALLOCATION + ENCODING
        Some(NodeHash::from_encoded(&encoded))
    }
}
```

### Optimization Approach

**Option A: Batch encoding at hash time**

Currently already somewhat optimized - only encodes when hash() is called.

**Option B: Incremental hash updates**

For small modifications, update hash incrementally:
```rust
// If only value changed, can recompute hash faster
fn update_leaf_hash(old_hash: NodeHash, old_value: &[u8], new_value: &[u8]) -> NodeHash
```

**Option C: Cache encoded nodes**

Store encoded representation alongside node data:
```rust
pub struct Node {
    pub node_type: NodeType,
    pub encoded_range: Option<(usize, usize)>,
    pub cached_encoding: Option<Vec<u8>>,  // NEW: avoid re-encoding
}
```

### Performance Estimate

| Metric | Current | Optimized | Savings |
|--------|---------|-----------|---------|
| Steps | 12-15M (encoding) | 7-10M | **5-8M (33-50%)** |
| Cost | 1-1.5B | 0.5-0.8B | **0.5-0.8B** |
| % Total | 1.8-2.3% | 1.0-1.5% | **0.8-1.2%** |

**Calculation:**
- Encoding cost is ~30% of insert_inner + hash operations
- Caching eliminates redundant re-encoding
- Conservative estimate due to complexity

### Implementation Complexity

- **Effort:** Medium (1-2 days)
- **Risk:** Medium (memory overhead, cache invalidation)
- **Files to modify:**
  - `crates/common/trie/encodedtrie.rs` - Node struct, hash(), insert()

---

## Comparison: Main Baseline vs Optimized

### Current State

| Metric | Main | zerocopy_trie | Diff |
|--------|------|---------------|------|
| Total Steps | 699.7M | 663.2M | -5.2% |
| memcpy calls | 1,199,137 | 1,001,396 | -16.5% |
| Trie hash cost | 21.7B | 23.8B* | +9.5% |

*Note: zerocopy uses EncodedTrie which has different cost profile

### Projected After All Optimizations

| Optimization | Steps Saved | New Total | Cumulative % |
|--------------|-------------|-----------|--------------|
| Baseline (zerocopy) | — | 663.2M | 0% |
| + rkyv zero-copy | -37M | 626.2M | -5.6% |
| + Code hash cache | -27M | 599.2M | -9.6% |
| + fake_exp lookup | -31M | 568.2M | -14.3% |
| + Array branch items | -12M | 556.2M | -16.1% |
| + Lazy encoding | -6M | 550.2M | -17.0% |

**Final projected: 550M steps (vs 663M current, vs 700M main)**

---

## Implementation Roadmap

### Phase 1: Quick Wins (1-2 days)

1. **Array-based get_branch_encoded_items** - P3
   - Effort: 2-4 hours
   - Impact: 1.4-2.2%
   - Risk: Very low

2. **Code hash pre-computation** - P1
   - Effort: 1 day
   - Impact: 3.8-4.2%
   - Risk: Low

### Phase 2: Medium Effort (3-5 days)

3. **fake_exponential lookup table** - P2
   - Effort: 1-2 days
   - Impact: 4.5-5.0%
   - Risk: Low

4. **Lazy trie encoding** - P4
   - Effort: 1-2 days
   - Impact: 0.8-1.2%
   - Risk: Medium

### Phase 3: High Effort (1-2 weeks)

5. **rkyv zero-copy deserialization** - P0
   - Effort: 2-3 days (implementation) + 2-3 days (testing)
   - Impact: 5.3-6.0%
   - Risk: Medium (API changes)

---

## Validation Strategy

### Per-Optimization Benchmarks

```bash
# Before each optimization
make -C scripts/zkvm-bench profile ZKVM=zisk BLOCK=23769082 TITLE="before_opt_name"

# After implementation
make -C scripts/zkvm-bench profile ZKVM=zisk BLOCK=23769082 TITLE="after_opt_name"

# Compare
make -C scripts/zkvm-bench compare \
  BASELINE=scripts/zkvm-bench/profiles/zisk/stats_*_before_opt_name.txt \
  CURRENT=scripts/zkvm-bench/profiles/zisk/stats_*_after_opt_name.txt
```

### Regression Tests

1. Run existing proptests: `cargo test -p ethrex-trie`
2. Run EF tests: `make test-blockchain`
3. Cross-validate with main branch hash outputs

### Metrics to Track

| Metric | Target |
|--------|--------|
| Total steps | -15% from current |
| memcpy calls | -20% from current |
| Hash accuracy | 100% match |
| EF test pass rate | 100% |

---

## Appendix: Profile Data Sources

- **Latest profile:** `stats_20260121_122544_put_node_encoded_fix.txt`
- **Main baseline:** `stats_20260121_110625_main_baseline.txt`
- **Block:** 23769082 (mainnet)
- **ZisK version:** 0.15.0

### Key Files Referenced

| File | Purpose |
|------|---------|
| `crates/common/trie/encodedtrie.rs` | EncodedTrie implementation |
| `crates/common/types/block_execution_witness.rs` | Witness deserialization |
| `crates/common/types/block.rs` | fake_exponential |
| `crates/common/types/account.rs` | Code::from_bytecode |
| `crates/common/rkyv_utils.rs` | rkyv wrappers |
| `crates/l2/prover/src/guest_program/src/zisk/src/main.rs` | Guest entry point |
