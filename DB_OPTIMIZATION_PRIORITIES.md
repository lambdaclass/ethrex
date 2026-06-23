# Database Optimization Priorities

This document categorizes all DB-related pending items from the roadmap based on whether they require database resyncing and their potential performance impact.

## Items Requiring Schema Modification (Require Resyncing)

These items modify existing database tables/schema and require a full resync:

| Section | Item | Priority | Description |
|---------|------|----------|-------------|
| IO | **Canonical tx index** | 1 | Add a canonical-tx index table or DUPSORT layout for O(1) lookups (currently O(k) prefix scans) |
| IO | **Split hot vs cold data** | 2 | Geth "freezer/ancients" pattern - store recent state in fast KV store, push old bodies/receipts to append-only ancient store |
| New Features | **Archive node** | — | Allow archive node mode - changes storage requirements/schema |
| New Features | **Pre merge blocks** | — | Be able to process pre merge blocks - requires schema changes to support different block formats |

---

## Items NOT Requiring Schema Modification (No Resync)

These items are optimizations and configurations that don't require resyncing, sorted by potential performance impact:

### High Impact (Most Likely to Improve Performance)

1. **Add Block Cache (RocksDB)** (#5935, P0)
   - Currently relying only on OS page cache. Explicit block cache is fundamental for RocksDB performance
   - Also try row cache
   - **Why high impact**: Block cache is one of the most important RocksDB features for read performance

2. **Use Two-Level Index (RocksDB)** (#5936, P0)
   - Use Two-Level Index with Partitioned Filters
   - **Why high impact**: Significantly reduces memory overhead and improves cache efficiency for large datasets

3. **Use multiget on trie traversal** (#4949, P1)
   - Using multiget on trie traversal might reduce read time
   - **Why high impact**: Batching reads during trie traversal can dramatically reduce I/O latency

4. **Bulk reads for block bodies** (P1)
   - Implement `multi_get` for `get_block_bodies` and `get_block_bodies_by_hash` which currently loop over per-key reads
   - Location: `crates/storage/store.rs:388-454`
   - **Why high impact**: Substantial improvement for batch operations, reduces round-trips

5. **Enable unordered writes for State (RocksDB)** (#5937, P0)
   - For `ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES cf_opts.set_unordered_write(true);`
   - Faster writes when we don't need strict ordering
   - **Why high impact**: Can significantly speed up write-heavy operations during sync

6. **Toggle compaction during sync** (P2)
   - Disable RocksDB compaction during snap sync for higher write throughput, then compact after
   - Nethermind pattern: Wire `disable_compaction/enable_compaction` into sync stages
   - **Why high impact**: Proven pattern from other clients, can dramatically improve sync performance

7. **Memory-Mapped Reads (RocksDB)** (#5943, P0)
   - Can be an improvement on high-RAM systems
   - **Why high impact**: Significant improvement by bypassing kernel page cache on systems with sufficient RAM

### Medium-High Impact

8. **Page caching + readahead** (#5940, P0)
   - Use for trie iteration, sync operations
   - **Why medium-high**: Reduces random I/O by prefetching related data

9. **Reduce trie cache Mutex contention** (P1)
   - `trie_cache` is behind `Arc<Mutex<Arc<TrieLayerCache>>>`
   - Use `ArcSwap` or `RwLock` for lock-free reads
   - Location: `crates/storage/store.rs:159,1360`
   - **Why medium-high**: High-frequency access point, lock contention can be significant bottleneck

10. **Reduce LatestBlockHeaderCache contention** (P1)
    - `LatestBlockHeaderCache` uses Mutex for every read
    - Use `ArcSwap` for atomic pointer swaps
    - Location: `crates/storage/store.rs:2880-2894`
    - **Why medium-high**: Accessed on every read operation

11. **Increase Bloom Filter (RocksDB)** (#5938, P0)
    - Change and benchmark higher bits per key for state tables
    - **Why medium-high**: Reduces unnecessary disk reads by improving filter accuracy

12. **Use Bytes/Arc in trie layer cache** (P2)
    - Trie layer cache clones `Vec<u8>` values on every read
    - Use `Bytes` or `Arc<[u8]>` to reduce allocations
    - Location: `crates/storage/layering.rs:57,63`
    - **Why medium-high**: Reduces allocations in hot path

13. **Optimize for Point Lookups (RocksDB)** (#5941, P0)
    - Adds hash index inside FlatKeyValue for faster point lookups
    - **Why medium-high**: Faster for common lookup patterns

### Medium Impact

14. **Consider LZ4 for State Tables (RocksDB)** (#5939, P0)
    - Trades CPU for smaller DB and potentially better cache utilization
    - **Why medium**: Depends on CPU vs I/O bottleneck and workload characteristics

15. **Increase layers commit threshold** (#5944, P0)
    - For read-heavy workloads with plenty of RAM
    - **Why medium**: Reduces write amplification but only beneficial in specific scenarios

16. **Configurable cache budgets** (P2)
    - Expose cache split for DB/trie/snapshot as runtime config
    - Currently hardcoded in ethrex
    - **Why medium**: Allows tuning for specific hardware but requires user knowledge

17. **Benchmark bloom filter** (#5946, P1)
    - Review trie layer's bloom filter, remove it or test other libraries/configurations
    - **Why medium**: May remove overhead if not beneficial, but needs measurement

18. **Modify block size (RocksDB)** (#5942, P0)
    - Benchmark different block size configurations
    - **Why medium**: Workload dependent, requires benchmarking to determine optimal value

### Lower Impact (But Still Useful)

19. **Remove locks** (#5945, P1)
    - Check if there are still some unnecessary locks, e.g. in the VM we have one
    - **Why lower**: Limited scope, only affects specific components

20. **geth db migration tooling** (P0, In Progress)
    - As we don't support pre-merge blocks we need a tool to migrate other client's DB to ours at a specific block
    - **Why lower**: More of a feature than performance improvement, enables compatibility

21. **Migrations** (P4)
    - Add DB Migration mechanism for ethrex upgrades
    - **Why lower**: Infrastructure for future changes, not direct performance improvement

---

## Summary

- **4 items** require DB schema changes and resyncing
- **21 items** are DB-related optimizations/configurations that don't require resyncing
- Most high-priority (P0) DB work focuses on RocksDB tuning and configuration

### Top Recommended Actions (No Resync Required)

The top 7 items provide the most significant performance improvements:

1. Add Block Cache (RocksDB) - fundamental for read performance
2. Use Two-Level Index (RocksDB) - reduces memory overhead
3. Use multiget on trie traversal - reduces I/O latency
4. Bulk reads for block bodies - improves batch operations
5. Enable unordered writes for State - speeds up writes during sync
6. Toggle compaction during sync - proven pattern for sync performance
7. Memory-Mapped Reads - significant improvement on high-RAM systems

### Key Observation

The explicit block cache (#5935) is likely the **single biggest win** since the system is currently relying only on OS page cache, which is a fundamental RocksDB optimization.
