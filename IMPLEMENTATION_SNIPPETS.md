# Implementation Snippets: Ready-to-Use Optimization Patterns

## 1. Two-Pointer Merge for Sorted Vectors (Reth #21098)

**File**: `crates/trie/common/src/utils.rs`  
**Applicability**: State aggregation, trie update merging  
**Complexity**: Medium  
**Estimated Impact**: 20-30% on merge operations  

```rust
/// Extend a sorted vector with another sorted vector using 2-pointer merge.
/// Values from `other` take precedence for duplicate keys.
pub(crate) fn extend_sorted_vec<K, V>(target: &mut Vec<(K, V)>, other: &[(K, V)])
where
    K: Clone + Ord,
    V: Clone,
{
    if other.is_empty() {
        return;
    }

    if target.is_empty() {
        target.extend_from_slice(other);
        return;
    }

    // Fast path: non-overlapping ranges - just append
    if target.last().map(|(k, _)| k) < other.first().map(|(k, _)| k) {
        target.extend_from_slice(other);
        return;
    }

    // Move ownership of target to avoid cloning owned elements
    let left = core::mem::take(target);
    let mut out = Vec::with_capacity(left.len() + other.len());

    let mut a = left.into_iter().peekable();
    let mut b = other.iter().peekable();

    while let (Some(aa), Some(bb)) = (a.peek(), b.peek()) {
        match aa.0.cmp(&bb.0) {
            Ordering::Less => {
                out.push(a.next().unwrap());
            }
            Ordering::Greater => {
                out.push(b.next().unwrap().clone());
            }
            Ordering::Equal => {
                // `other` takes precedence for duplicate keys
                let (k, _) = a.next().unwrap();
                out.push((k, b.next().unwrap().1.clone()));
            }
        }
    }

    // Drain remaining: `a` moves, `b` clones
    out.extend(a);
    out.extend(b.cloned());

    *target = out;
}
```

**Performance**: O(n+m) vs O(n log n) sort. ~10-20x faster for large n,m.

**Usage**:
```rust
let mut state = vec![(addr1, val1), (addr2, val2)];
let updates = vec![(addr1, val_new), (addr3, val3)];
extend_sorted_vec(&mut state, &updates);
// Result: vec![(addr1, val_new), (addr2, val2), (addr3, val3)]
```

---

## 2. K-Way Merge Batch (Reth #21080)

**File**: `crates/engine/tree/src/tree/payload_validator.rs`  
**Applicability**: Multiple block state aggregation  
**Complexity**: High  
**Estimated Impact**: 10-20% on multi-block scenarios  

```rust
/// Threshold for switching from extend_ref loop to merge_batch
const MERGE_BATCH_THRESHOLD: usize = 64;

/// Aggregates in-memory blocks into sorted state using k-way merge
fn merge_overlay_trie_input(blocks: &[ExecutedBlock<N>]) -> TrieInputSorted {
    if blocks.is_empty() {
        return TrieInputSorted::default();
    }

    // Single block: return Arc directly without cloning
    if blocks.len() == 1 {
        let data = blocks[0].trie_data();
        return TrieInputSorted {
            state: Arc::clone(&data.hashed_state),
            nodes: Arc::clone(&data.trie_updates),
            prefix_sets: Default::default(),
        };
    }

    if blocks.len() < MERGE_BATCH_THRESHOLD {
        // Small k: extend_ref loop is faster (better cache locality)
        // Iterate oldest->newest so newer values override older ones
        let mut blocks_iter = blocks.iter().rev();
        let first = blocks_iter.next().expect("blocks is non-empty");
        let data = first.trie_data();

        let mut state = Arc::clone(&data.hashed_state);
        let mut nodes = Arc::clone(&data.trie_updates);
        let state_mut = Arc::make_mut(&mut state);
        let nodes_mut = Arc::make_mut(&mut nodes);

        for block in blocks_iter {
            let data = block.trie_data();
            state_mut.extend_ref(data.hashed_state.as_ref());
            nodes_mut.extend_ref(data.trie_updates.as_ref());
        }

        TrieInputSorted { state, nodes, prefix_sets: Default::default() }
    } else {
        // Large k: merge_batch is faster (O(n log k) via k-way merge)
        let trie_data: Vec<_> = blocks.iter().map(|b| b.trie_data()).collect();

        let merged_state = HashedPostStateSorted::merge_batch(
            trie_data.iter().map(|d| d.hashed_state.as_ref()),
        );
        let merged_nodes =
            TrieUpdatesSorted::merge_batch(trie_data.iter().map(|d| d.trie_updates.as_ref()));

        TrieInputSorted {
            state: Arc::new(merged_state),
            nodes: Arc::new(merged_nodes),
            prefix_sets: Default::default(),
        }
    }
}
```

**Key Points**:
- Threshold of 64 blocks empirically determined
- Below threshold: sequential extend (cache-friendly)
- Above threshold: k-way merge (optimal complexity)

---

## 3. Binary Search in Forward Cursor (Reth #21049)

**File**: `crates/trie/trie/src/forward_cursor.rs`  
**Applicability**: Trie traversal, proof generation  
**Complexity**: Medium  
**Estimated Impact**: 5-10% on sequential traversal  

```rust
/// Threshold for remaining entries above which binary search is used
const BINARY_SEARCH_THRESHOLD: usize = 64;

impl<K, V> ForwardInMemoryCursor<'_, K, V>
where
    K: Ord + Clone,
    V: Clone,
{
    /// Advances cursor while predicate returns true
    /// Uses binary search for large remaining slices (>= 64), linear for small ones
    fn advance_while(&mut self, predicate: impl Fn(&K) -> bool) -> Option<(K, V)> {
        let remaining = self.entries.len().saturating_sub(self.idx);
        if remaining >= BINARY_SEARCH_THRESHOLD {
            // Binary search for large slices
            let slice = &self.entries[self.idx..];
            let pos = slice.partition_point(|(k, _)| predicate(k));
            self.idx += pos;
        } else {
            // Linear scan for small slices (better cache locality)
            while self.current().is_some_and(|(k, _)| predicate(k)) {
                self.next();
            }
        }
        self.current().cloned()
    }
}
```

---

## 4. Selective Bloom Filter & Compression (Reth #21310)

**File**: `crates/storage/provider/src/providers/rocksdb/provider.rs`  
**Applicability**: Database configuration tuning  
**Complexity**: Low  
**Estimated Impact**: 5-15% I/O improvement on specific tables  

```rust
/// Creates optimized column family options for high-hit-rate tables
fn tx_hash_numbers_column_family_options(cache: &Cache) -> Options {
    let mut table_options = BlockBasedOptions::default();
    table_options.set_block_size(DEFAULT_BLOCK_SIZE);
    table_options.set_cache_index_and_filter_blocks(true);
    table_options.set_pin_l0_filter_and_index_blocks_in_cache(true);
    table_options.set_block_cache(cache);
    
    // Disable bloom filter: every lookup expects a hit
    // Bloom filters only help when checking non-existent keys
    // (bloom filter has 100% false positive rate for hits)

    let mut cf_options = Options::default();
    cf_options.set_block_based_table_factory(&table_options);
    cf_options.set_level_compaction_dynamic_level_bytes(true);
    
    // Disable compression: B256 keys are incompressible hashes
    // TxNumber values are varint-encoded u64 (few bytes)
    // Compression wastes CPU cycles for zero space savings
    cf_options.set_compression_type(DBCompressionType::None);
    cf_options.set_bottommost_compression_type(DBCompressionType::None);

    cf_options
}

/// Configuration logic in builder
pub fn add_table_column_families(&mut self) {
    self.column_families
        .iter()
        .map(|name| {
            let cf_options = if name == tables::TransactionHashNumbers::NAME {
                // Use optimized options for this table
                Self::tx_hash_numbers_column_family_options(&self.block_cache)
            } else {
                // Default options for other tables
                Self::default_column_family_options(&self.block_cache)
            };
            ColumnFamilyDescriptor::new(name.clone(), cf_options)
        })
        .collect()
}
```

**Analysis Framework** (apply per table):
```
For each MDBX table:
1. Measure hit rate:
   - If hit_rate > 95%: disable bloom filters
   - If hit_rate < 50%: keep bloom filters

2. Measure key compressibility:
   - If key is hash/random: disable compression
   - If key is structured: profile compression ratio

3. Measure compression CPU cost:
   - If CPU cost > space savings: disable
   - If space savings > CPU cost: keep
```

---

## 5. ZSTD Compression Upgrade (Reth: c4df4e3035)

**File**: `crates/storage/provider/src/providers/rocksdb/provider.rs`  
**Applicability**: Database compression strategy  
**Complexity**: Low  
**Estimated Impact**: 8-10% size reduction, same performance  

```rust
impl RocksDBBuilder {
    fn default_column_family_options(cache: &Cache) -> Options {
        let mut options = Options::default();
        
        // Compression configuration
        options.set_bottommost_compression_type(DBCompressionType::Zstd);
        options.set_bottommost_zstd_max_train_bytes(0, true);
        
        // Switch from LZ4 to ZSTD for all levels
        // Benchmarks show 9% size reduction with no performance penalty
        // - LZ4: 266GB output, 32.9 min migration
        // - ZSTD: 215GB output, 27.8 min migration
        options.set_compression_type(DBCompressionType::Zstd);
        options.set_compaction_pri(CompactionPri::MinOverlappingRatio);
        
        // ... rest of configuration
        options
    }
}
```

**Migration Notes**:
- ZSTD provides better compression ratio (9%) than LZ4
- No CPU penalty observed in migration time
- Recommended for new deployments
- Migration for existing DB: ~30 min for 266GB dataset

---

## 6. Fixed-Size Execution Cache (Reth #21128)

**File**: `crates/engine/tree/src/tree/cached_state.rs`  
**Applicability**: State caching during block execution  
**Complexity**: Medium  
**Estimated Impact**: 10-20% memory reduction, bounded latency  

```rust
use lru::LruCache;

pub struct ExecutionCache<N: Network> {
    // Replace unbounded HashMap with bounded LRU
    cache: LruCache<BlockHash, CachedState<N>>,
    stats: CacheStats,
}

impl<N: Network> ExecutionCache<N> {
    /// Create cache with fixed size (empirically tuned)
    pub fn new(max_cached_blocks: usize) -> Self {
        let cache = LruCache::new(
            std::num::NonZeroUsize::new(max_cached_blocks)
                .expect("max_cached_blocks must be > 0")
        );
        
        Self {
            cache,
            stats: CacheStats::default(),
        }
    }

    pub fn insert(&mut self, hash: BlockHash, state: CachedState<N>) {
        self.cache.put(hash, state);
        self.stats.inserts += 1;
    }

    pub fn get(&mut self, hash: &BlockHash) -> Option<&CachedState<N>> {
        self.cache.get(hash).inspect(|_| {
            self.stats.hits += 1;
        })
    }
}
```

**Tuning**: Set `max_cached_blocks` based on:
- Available memory
- Block execution time (cache hit saves ~50-100ms)
- Typical reorg depth (minimum N+1 for reorg safety)

---

## 7. Lazy Overlay Computation (Reth #21133)

**File**: `crates/engine/tree/src/tree/payload_processor/mod.rs`  
**Applicability**: Deferred proof generation  
**Complexity**: Medium  
**Estimated Impact**: 5-10% on non-proof paths  

```rust
pub struct LazyOverlay<N: Network> {
    state: Arc<HashedPostState>,
    trie_updates: Option<Arc<TrieUpdates>>,
    computed: Option<Arc<TrieOverlay>>,
}

impl<N: Network> LazyOverlay<N> {
    /// Create lazy overlay without computing trie overlay
    pub fn new(state: Arc<HashedPostState>) -> Self {
        Self {
            state,
            trie_updates: None,
            computed: None,
        }
    }

    /// Compute overlay only when actually accessed
    pub fn get_or_compute(&mut self) -> Arc<TrieOverlay> {
        if let Some(computed) = &self.computed {
            return Arc::clone(computed);
        }

        // Expensive computation deferred until actually needed
        let overlay = Arc::new(
            TrieOverlay::from_hashed_state(&self.state)
        );
        self.computed = Some(Arc::clone(&overlay));
        overlay
    }
}
```

**Use Case**:
```rust
// Before: Always compute overlay (even if not needed)
let overlay = TrieOverlay::from_hashed_state(&state); // expensive
process_payload(&payload); // may not use overlay

// After: Compute only when needed
let mut lazy = LazyOverlay::new(Arc::new(state)); // cheap
process_payload(&payload);
if let Some(proof_required) = payload.proof_requirement {
    let overlay = lazy.get_or_compute(); // expensive, but only if needed
}
```

---

## 8. Storage Key Optimization (Nethermind #10241)

**Language**: C#  
**File**: `src/Nethermind/Nethermind.State/StorageTree.cs`  
**Applicability**: Static storage key lookup  
**Complexity**: Medium  
**Estimated Impact**: 5-15% on storage access patterns  

```csharp
// Before: FrozenDictionary with byte[] values (heap allocation per lookup)
private static readonly FrozenDictionary<UInt256, byte[]> Lookup = CreateLookup();

private static FrozenDictionary<UInt256, byte[]> CreateLookup()
{
    Dictionary<UInt256, byte[]> lookup = new Dictionary<UInt256, byte[]>(LookupSize);
    for (int i = 0; i < LookupSize; i++)
    {
        UInt256 index = (UInt256)i;
        index.ToBigEndian(buffer);
        lookup[index] = Keccak.Compute(buffer).BytesToArray(); // heap allocation
    }
    return lookup.ToFrozenDictionary();
}

// After: ValueHash256 array (stack value types, zero-copy)
private static readonly ValueHash256[] Lookup = CreateLookup();

private static ValueHash256[] CreateLookup()
{
    const int LookupSize = 1024;
    ValueHash256[] lookup = new ValueHash256[LookupSize]; // stack values
    
    for (int i = 0; i < lookup.Length; i++)
    {
        UInt256 index = new UInt256((uint)i);
        index.ToBigEndian(buffer);
        lookup[i] = ValueKeccak.Compute(buffer); // value type, no heap
    }
    
    return lookup;
}

// Usage becomes direct array access (no dictionary lookup)
[SkipLocalsInit]
public static void ComputeKeyWithLookup(in UInt256 index, ref ValueHash256 key)
{
    ValueHash256[] lookup = Lookup;
    ulong u0 = index.u0;
    
    if (index.IsUint64 && u0 < (uint)lookup.Length)
    {
        // Direct array indexing via pointer arithmetic
        key = Unsafe.Add(ref MemoryMarshal.GetArrayDataReference(lookup), (nuint)u0);
        return;
    }
    
    // Fallback: compute key
    ComputeKey(in index, out key);
}
```

**Key Optimizations**:
1. **Value types instead of heap references**: `ValueHash256[]` instead of `byte[]`
2. **Direct memory access**: `MemoryMarshal.GetArrayDataReference` + `Unsafe.Add`
3. **SkipLocalsInit attribute**: Skip zero-initialization of locals on hot paths
4. **Static lookup table**: Pre-computed deterministic keys

---

## Benchmarking Template (Reth Pattern)

```bash
# Run benchmark with reth-bench
reth bench --profile-gas-limit-ramp \
    --from-block 19000000 \
    --to-block 19000100 \
    --target-gas-limit 30000000

# Compare before/after:
# - Ggas/s (throughput)
# - p50/p99 latency (variance)
# - Memory usage
# - Disk I/O patterns
```

---

## Integration Checklist

When adopting patterns to Ethrex:

- [ ] Profile current performance baseline
- [ ] Identify hot paths (state merge, trie traversal, proof generation)
- [ ] Apply optimization (binary search, k-way merge, etc.)
- [ ] Benchmark improvement (target: 10%+ for high-priority items)
- [ ] Check for correctness (especially ordering, precedence rules)
- [ ] Measure memory impact
- [ ] Document threshold/tuning constants

