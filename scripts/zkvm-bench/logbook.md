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
| 2026-01-22 | 08 | Array-based get_branch_encoded_items             | +0.52% (+3.4M)      | ❌ Reverted  | 05729a9 |
| 2026-01-22 | 09 | Pre-compute code hashes on host                  | N/A                 | 🚫 Invalid  | N/A     |
| 2026-01-22 | 10 | i128 fast path for fake_exponential              | -6.68% (-42.9M)     | ✅ Kept      | 38c1aae |
| 2026-01-22 | 11 | Block-level base blob fee caching                | -7.86% (-50.4M)     | ✅ Kept      | 40885a4 |

## Detailed Entries

### 11. Block-Level Base Blob Fee Caching
- **Date:** 2026-01-22
- **Goal:** Replace i128 conditional compilation approach with block-level caching to reduce redundant `fake_exponential` calls from ~1536 per block to 1.
- **Change:**
  - Removed `fake_exponential_i128()` and all `#[cfg(any(feature = "zisk", feature = "sp1", feature = "risc0"))]` conditional compilation
  - Modified `calculate_blob_gas_cost()` to accept pre-computed `base_blob_fee_per_gas` from Environment (eliminates ~768 calls/block)
  - Compute `base_blob_fee_per_gas` once in `execute_block()`/`execute_block_pipeline()` before transaction loop (eliminates ~768 calls/block)
  - Thread the value through `setup_env()`, `execute_tx()`, and `execute_tx_in_block()` functions
  - Updated all call sites: `trace_tx_calls`, `warm_block`, `rerun_block`, Evm wrapper
- **Results:**
  - Total steps: 641,671,652 → 591,264,620 (-50.4M steps, **-7.86%**)
  - Total cost: 72.8B → 68.1B (-4.7B, **-6.42%**)
  - `LEVM::execute_tx` (382 calls): 38.9B → 34.3B (**-11.96%**)
  - `LEVM::execute_block`: 41.2B → 36.6B (**-11.33%**)
  - `VM::execute` (386 calls): 35.1B → 32.8B (**-6.70%**)
  - memcpy calls: 921,521 → 832,937 (-88,584 calls, **-9.6%**)
  - memcpy cost: 13.5B → 12.6B (-900M, **-6.7%**)
  - Net code change: **-62 lines** (removed 130, added 68)
- **Why it worked:** The base blob fee per gas is constant for all transactions in a block (determined by `parent_excess_blob_gas` and `base_fee_update_fraction`). Computing it once and reusing eliminates massive redundancy. Additionally, this is architecturally correct: block-level constants should be computed at block level.
- **Key Insight:** The i128 approach (#10) still computed the value ~1536 times per block with conditional compilation complexity. This block-level approach is simpler, more maintainable, and more efficient. **Always consider call frequency reduction before micro-optimizations.**
- **Comparison with #10:**
  - i128 approach: -6.68%, conditional compilation, still ~1536 calls/block
  - Block-level: **-7.86%**, no conditional compilation, **1 call/block**
  - This supersedes #10 with better results and cleaner code
- **Status:** Kept (exceeds target, supersedes #10).
- **Profile:** Block 24291039, baseline: `stats_20260122_115717_f5524c2d7_baseline.txt`, optimized: `stats_20260122_154857_40885a49d_p2_block_level.txt`
- **Branch:** opt/p2-fake-exp
- **Commits:** 40885a49d (implementation), ef5f48343 (logbook)
- **PR:** #6001

### 10. i128 Fast Path for fake_exponential (EIP-4844 Blob Gas)
- **Date:** 2026-01-22
- **Goal:** Optimize EIP-4844 blob gas calculation by replacing U256 arithmetic with i128 for zkVM contexts.
- **Change:**
  - Added `fake_exponential_i128()` using native i128 arithmetic instead of U256
  - Replaced U256 version in `calculate_base_fee_per_blob_gas()` in `block.rs` (block-level, 1 call/block)
  - Replaced U256 version in `get_base_fee_per_blob_gas()` in `levm/utils.rs` (tx-level, ~769 calls/block)
  - Gated with `#[cfg(any(feature = "zisk", feature = "sp1", feature = "risc0"))]`
  - Added feature flags `zisk`, `sp1` to `ethrex-common/Cargo.toml`
- **Results:**
  - Total steps: 641,671,652 → 598,786,008 (-42.9M steps, **-6.68%**)
  - LEVM execute_tx: **-9.98%** (main call site with 769 calls)
  - memcpy calls: 921,521 → 833,469 (-88,052 calls, -9.6%)
  - memcpy cost: 13.5B → 12.6B (-900M)
  - Function disappeared from profile (fell below threshold), was consuming 50M+ steps before
- **Why it worked:** i128 native arithmetic is much faster than U256 checked operations. The values fit comfortably in i128 for realistic blob gas (factor=1, numerator<100M, denominator~8.8M). Main impact was from levm's transaction-level calls.
- **Key Insight:** Always check **all call sites** when optimizing a function. The block-level call gave minimal improvement (~55k steps), but the transaction-level call in levm provided the bulk of the savings.
- **Status:** Kept (exceeds target).
- **Profile:** Block 24291039, baseline: `stats_20260122_115717_f5524c2d7_baseline.txt`, optimized: `stats_20260122_124944_38c1aaeda_p2_fake_exp_final.txt`
- **Branch:** opt/p2-fake-exp
- **Commits:** 00b1c471e (block.rs), a683dd30e (Cargo.toml), 38c1aaeda (levm/utils.rs)

### 09. Pre-compute Code Hashes on Host
- **Date:** 2026-01-22
- **Goal:** Avoid keccak256 hashing of contract bytecode in guest by pre-computing on host.
- **Analysis:** **This optimization is fundamentally insecure.** The zkVM proof must cryptographically prove `keccak256(code) == account.code_hash` to bind execution to actual on-chain code. Pre-computing the hash on the host would allow an attacker to:
  1. Provide malicious bytecode that doesn't match real on-chain account
  2. Provide fake pre-computed hash
  3. Generate "valid" proof of execution with **wrong code**
- **Key Insight:** Not all host-side pre-computation is valid. Cryptographic verification that binds witness data to on-chain state must happen in the guest.
- **Status:** Not Implemented (Security issue - breaks proof soundness).

### 08. Array-based get_branch_encoded_items
- **Date:** 2026-01-22
- **Goal:** Avoid Vec allocation when decoding branch nodes by using fixed-size array `[&[u8]; 17]`.
- **Change:** Added `get_branch_encoded_items()` returning `[&[u8]; 17]` instead of `Vec<&[u8]>` for branch nodes. Updated call sites in `authenticate::recursive` and `hash::recursive`.
- **Results:**
  - Total steps: 641,671,652 → 645,029,609 (+0.52%, +3.4M steps) **REGRESSION**
  - authenticate cost: +2.29%
  - memcpy calls: 921,521 → 955,939 (+34,418 calls, +3.7%)
  - memcpy cost: 13.5B → 14.0B (+0.5B)
- **Why it failed:** The fixed-size array `[&[u8]; 17]` initialization and filling is less efficient than Vec in this context. The array likely incurs stack copying overhead that outweighs the heap allocation cost of Vec. The compiler optimizes Vec::with_capacity(17) better than manual array initialization.
- **Key Insight:** Not all heap allocations are bad in zkVM context. Vec's allocator overhead can be less than stack array copying when the data structure is passed around or stored temporarily.
- **Status:** Reverted.
- **Profile:** Block 24291039, baseline: `stats_20260122_115717_f5524c2d7_baseline.txt`, optimized: `stats_20260122_120222_05729a9bc_p3_branch_items.txt`
- **Branch:** opt/p3-branch-items

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
