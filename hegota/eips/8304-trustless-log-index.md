# EIP-8304: Trustless log and transaction index
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) (acde/239, 2026-06-18)
- **Authors:** Zsolt Felföldi
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8304) · [discussion](https://ethereum-magicians.org/t/eip-8304-trustless-log-and-transaction-index/28824)
- **Prior fork history:** none

## TL;DR
Every block builds small binary-tree-hashed "index tables" of its transactions, logs (address + all 4 topics), and parent block hash; table roots are committed to a system contract (EIP-4788-style). Larger tables are merged asynchronously from smaller ones across 5 levels (1/4/16/64/256 blocks). This makes log/tx lookups provable: a provider's answer to a filter query can be verified against state, fixing the saturated and useless header bloom filters.

## What it changes
- **Index tables**: ordered lists of typed entries (block, transaction, log.address, log.topics[0..3]) with fixed binary encodings (38–50 bytes: type id, content, position info), lexicographically ordered; table root = root of an SSZ `List[Hash32, entry_count]` of SHA2-256 entry hashes.
- **Levels**: TABLE_SIZES = [1, 4, 16, 64, 256], each level's roots stored in a 1024-entry ring buffer in the index contract (~last 2^18 ≈ 262k blocks covered in-protocol). Level-0 tables added right after tx processing; higher levels with `table_size // 4` block delay so merging is asynchronous, off the block-processing critical path.
- **Index contract**: EIP-4788-style system contract (called as `SYSTEM_ADDRESS 0xffff…fffe` at end of block processing, gas-limit 30M, not counted against block gas, fails silently if no code). `set()` stores `table_root` at `table_size * 1024 + (first_block // table_size) % 1024`; `get()` serves proofs. Deployed à la EIP-4788 via synthetic address.
- **Indexing rules**: block entries added with one-block delay (table for block N contains the block entry of N-1). Genesis gets no entry.
- Requires **EIP-4788** conventions; optional **EIP-8141** extension adds a 2-byte frame index to position info. No change to header bloom filters; no LOG gas cost increase claimed.

## Motivation
Per-block bloom filters are saturated and practically useless for content lookup, so users querying logs must trust RPC providers — information asymmetry that worsens as throughput grows. Provable index queries restore trustless end-user access, enable contracts to consume proven logs from other chains running the same index (cross-chain messaging via log query proofs), and longer-term could let logs act as a cheap, expirable alternative form of state. Big-history tables (millions of blocks) are left to external provers committing ZKP-verified roots to a separate (out-of-scope) history index contract.

## Dependencies & related EIPs
- Requires **EIP-4788** (beacon roots system contract pattern, Cancun) for the system-contract call convention.
- Optional integration with **EIP-8141** (frame transactions, Hegota candidate): adds frame index to entry position encoding; most useful if receipts become SSZ/tree-hashed.
- Mentions **EIP-7708** (ETH transfer logs) as a future source of additional entry data.
- Longer-term synergy with ZK batched pre-checks and stateless witnesses (proof verification batched in recursive ZKPs).
- A successor in spirit to earlier log-index proposals (e.g. EIP-7668 was a bloom-filter-era attempt); this replaces rather than repairs blooms.

## Impact on ethrex / client teams
Block-processing pipeline: after executing transactions, build the level-0 index table from the block + receipts, hash it (SHA2-256 + SSZ list root — note: not keccak/RLP), and issue the system call; asynchronously merge lower-level tables into higher-level ones on the delayed schedule and commit those roots. Touches `crates/blockchain` (payload/block processing), system-contract call plumbing (existing 4788-style path), plus new persistent storage for tables (they must be regenerable from the last ~319 blocks after sync). RPC filter serving (`eth_getLogs`) could later use it for provable answers, but that's optional. Effort: medium-large (new per-block data structure, new hashing path, contract deployment, sync bootstrap rule).

## Open questions & controversies
- `INDEX_CONTRACT_ADDRESS` and deployment transaction parameters still TBD; entry encodings and gas analysis are early-draft.
- Every node takes on per-block index construction and hashing — claimed cheap, but unbenchmarked across clients.
- Only ~262k blocks indexed in-protocol; older history relies on out-of-protocol ZKP provers and a history index contract that doesn't exist yet — the trustless story for old logs is aspirational.
- Typical query proofs combine 30–50 table proofs — sizeable; proof UX for wallets/explorers is unproven.
- Branch-poisoning concern acknowledged for the hot system-contract storage paths (deemed low-risk).
