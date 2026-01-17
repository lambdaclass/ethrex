# Snap Sync Benchmarks

This file tracks snap sync performance across commits on Hoodi testnet.

## Benchmark Format
| Commit | Date | Headers | Accounts DL | Accounts Insert | Storage DL | Storage Insert | Healing | Bytecodes | Total | Notes |

## Benchmarks


### 4eb0d4737 - 2026-01-17 17:07
**Hoodi Testnet Snap Sync Results:**

| Phase | Downloaded | Time | Rate |
|-------|-----------|------|------|
| Headers | 2,051,027 | ~2 min | ~1M/min |
| Accounts DL | 17,635,865 | 31 sec | 569k/sec |
| Accounts Insert | 17,635,865 | 49 sec | 360k/sec |
| Storage DL | 239,290,740 | 4:24 | 906k/sec |
| Storage Insert | 239,279,002 | 8:12 | 486k/sec |
| Healing | - | 26 sec | - |
| Bytecodes | 1,402,487 | 3:40 | 6.4k/sec |
| **Total State DL** | - | **~19 min** | - |

**Optimizations in this commit:**
- Parallel file reading/decoding with rayon
- Parallel sort for trie computation
- Increased chunk counts (800→1200)
- Increased MAX_RESPONSE_BYTES (1MB→2MB)
- Reduced BYTECODE_CHUNK_SIZE (50k→10k)
- Increased NODE_BATCH_SIZE (800→1500)
- Increased channel sizes (1000→2000-3000)

**Notes:** Sync cycle errors after state download during block execution (Insufficient account funds)
