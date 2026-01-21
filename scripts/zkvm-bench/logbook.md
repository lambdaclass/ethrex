# zkVM Optimization Logbook

This logbook tracks all optimization attempts, their results, and key learnings.

## Summary Table

| Date       | ID | Description                                      | Impact (Steps)      | Result      | Commit  |
|------------|----|--------------------------------------------------|---------------------|-------------|---------|
| 2026-01-20 | 01 | Direct H256 construction in decode_child         | -0.15% (-756k)      | ✅ Kept      | f67f856 |
| 2026-01-20 | 02 | Remove Nibbles::clone in encode_extension        | +0.00% (+208)       | ✅ Kept      | 486db83 |
| 2026-01-20 | 03 | Inline NodeHash encoding in encode_branch        | +0.06% (+315k)      | ❌ Reverted  | 06894fd |
| 2026-01-20 | 04 | Cache Storage Authentication Results             | N/A                 | ⏸️ Exists   | N/A     |
| 2026-01-20 | 05 | Avoid NodeType Clone in authenticate/hash        | -0.79% (-3.9M)      | ✅ Kept      | 7e1e94a |
| 2026-01-20 | 06 | Inline H256 RLP encoding (skip trait overhead)   | -0.71% (-3.54M)     | ✅ Kept      | e1906b2 |
| 2026-01-20 | 07 | Specialized get_two_encoded_items                | -0.19% (-967k)      | ✅ Kept      | 81600cb |

## Detailed Entries

### 07. Specialized get_two_encoded_items for Leaf/Extension
- **Date:** 2026-01-20
- **Goal:** Avoid Vec allocation when decoding leaf/extension nodes which always have exactly 2 items.
- **Change:** Added `get_two_encoded_items()` returning `(&[u8], &[u8])` tuple instead of `Vec<&[u8]>`. Updated all callers in `get_leaf_data`, `get_extension_data`, `get_extension_encoded_child_hash`, and `authenticate::recursive` (extension path).
- **Results:**
  - Total steps: 498,003,967 → 497,037,298 (-0.19%, -967k steps)
  - memcpy cost: -0.45%
  - `hash::recursive` cost: -0.10%
- **Key Insight:** The original `get_encoded_items()` allocated Vec with capacity 17 on every call, even for 2-item nodes (85%+ of calls based on mainnet trie analysis showing leaf/extension node dominance).
- **Profile:** `stats_20260120_160507_get_two_encoded_items.txt`

### 06. Inline H256 RLP Encoding (Skip Trait Overhead)
- **Date:** 2026-01-20
- **Goal:** Reduce overhead from RLPEncode trait calls when encoding H256 hashes in branch node encoding.
- **Change:** Modified `encode_branch` in `encodedtrie.rs` and `BranchNode::encode`/`encode_to_vec` in `rlp.rs` to directly emit `0xa0` prefix + 32 bytes instead of calling `hash.0.encode(&mut buf)`.
- **Results:**
  - Total steps: 501,544,194 → 498,003,967 (-0.71%, -3.54M steps)
  - `hash::recursive` cost: -1.33%
  - `authenticate::recursive` cost: -2.09%
  - memcpy calls: 805,839 → 789,487 (-16,352 calls)
  - memcpy cost: 11,495M → 11,222M (-2.4%)
- **Key Insight:** Unlike #03 which tried byte-by-byte loops (regression), this keeps `put_slice`/`extend_from_slice` for the 32-byte hash while avoiding the RLPEncode trait dispatch overhead.
- **Profile:** `stats_20260120_160004_inline_h256_encoding.txt`

### 05. Avoid NodeType Clone in authenticate/hash
- **Date:** 2026-01-20
- **Goal:** Reduce `memcpy` overhead by avoiding cloning `NodeType` enum (which can be large) during recursive traversal in `authenticate` and `hash` functions.
- **Change:** Modified `authenticate()` and `hash()` in `crates/common/trie/encodedtrie.rs` to inspect `node_type` by reference. Extracted child indices upfront to avoid borrowing conflicts.
- **Results:** 
  - Total steps reduced by 3,980,560 (-0.79%).
  - `memcpy` calls reduced by 16,352.
  - `authenticate::recursive` cost reduced by 2.09%.
- **Report:** [20260120_avoid_nodetype_clone.md](reports/20260120_avoid_nodetype_clone.md)

### 04. Cache Storage Authentication Results
- **Date:** 2026-01-20
- **Goal:** Avoid re-authenticating storage tries.
- **Analysis:** Found that caching is already implemented via `verified_storage_roots` in `GuestProgramState`. Profile confirmed ratio of ~7.27 nodes authenticated per trie call, indicating caching is working.
- **Status:** Not Implemented (Already exists).

### 03. Inline NodeHash Encoding in encode_branch
- **Date:** 2026-01-20
- **Goal:** Avoid `put_slice` overhead by using byte-by-byte `push`.
- **Change:** Replaced `buf.put_slice` with manual loop in `encode_branch`.
- **Results:** Regression (+0.06% steps). `put_slice` is likely using optimized `memcpy` or SIMD which is faster than a manual loop for this size.
- **Status:** Reverted.

### 02. Remove Nibbles::clone in encode_extension
- **Date:** 2026-01-20
- **Goal:** Avoid cloning `Nibbles` vector.
- **Change:** Changed `encode_extension` to take `&Nibbles` instead of `Nibbles`.
- **Results:** Negligible impact (+208 steps). The clone was not in a hot path or was optimized away.
- **Status:** Kept (good practice).

### 01. Direct H256 Construction in decode_child
- **Date:** 2026-01-20
- **Goal:** Avoid `H256::from_slice` memcpy overhead.
- **Change:** Changed `decode_child` to construct `H256` directly from byte array.
- **Results:** -756,778 steps (-0.15%). `H256::from_slice` was fully eliminated from profile.
- **Status:** Kept.
