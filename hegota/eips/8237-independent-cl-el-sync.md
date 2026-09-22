# EIP-8237: Independent CL/EL Sync
- **Layer:** CL (with EL header + Engine API changes)
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acde/235, 2026-04-23)
- **Authors:** M. Kalinin, Potuz, Toni Wahrstätter
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8237) · [discussion](https://ethereum-magicians.org/t/eip-8237-independent-cl-el-sync/28331)
- **Prior fork history:** none

## TL;DR
Under ePBS (EIP-7732), payloads travel separately from beacon blocks, but a CL doing range sync still has to download every payload envelope to check its block hash and execution requests against the bid. This EIP replaces `execution_requests_root` in the `ExecutionPayloadBid` with a SHA256 accumulator (`partial_header_hash`) that chains across blocks, so the CL can verify an entire synced range in one pass once the EL catches up — without downloading payloads or round-tripping the engine during sync.

## What it changes
- **`ExecutionPayloadBid` (EIP-7732):** `execution_requests_root` is removed.
- **`BeaconBlockBody`:** new field `partial_header_hash: Hash32`.
- **`ExecutionPayload` and EL `Header`:** new field `partial_header_hash: Hash32`, covered by `block_hash`.
- **Accumulator function:** `SHA256(parent_hash || prev_randao || gas_limit || timestamp || withdrawals || slot_number || execution_requests)` — raw byte concatenation, little-endian u64s, list fields as raw byte concatenation. Plain SHA256 (not SSZ `hash_tree_root`) so the EL can compute it without SSZ infrastructure. Trade-off: not Merkle-provable.
- **CL verification:** in `verify_execution_payload_envelope` and `process_parent_execution_payload`, the old `hash_tree_root(requests) == bid.execution_requests_root` check is replaced by recomputing the accumulator from bid fields + `state.payload_expected_withdrawals` and comparing against `envelope.payload.partial_header_hash` / `block.body.partial_header_hash`.
- **EL verification:** the EL MUST independently recompute the accumulator (it knows all inputs) and assert it matches the payload's `partial_header_hash`.
- **Engine API:** new method to fetch the accumulator value by block hash, so that when the CL and EL accumulators diverge after a long range sync, the CL can bisect to find the divergence point.

## Motivation
With ePBS, CL range sync could in principle fetch only beacon blocks — big bandwidth and latency savings, no engine interaction. But without payloads the CL cannot verify `block_hash` or execution requests against the bid. A cross-block chained accumulator defers all payload verification: the CL syncs cheaply, and once the EL catches up both sides compare one 32-byte value covering the whole range; any drift is detected (though only localized via the per-block-hash engine query).

## Dependencies & related EIPs
- **Depends on EIP-7732** (ePBS, Glamsterdam): modifies `ExecutionPayloadBid` / payload envelopes introduced there.
- Interacts with **EIP-7928** (BALs, Glamsterdam) and **EIP-7843** (`slot_number` in payload) — the accumulator covers withdrawals, slot number, and execution requests.
- Synergizes with **EIP-8379** (top-up sync): both reduce EL involvement during sync; 8379 handles block delivery, 8237 handles verification deferral.
- Related to **EIP-8383** (CL block retention): both target cheap(er) sync and history serving.

## Impact on ethrex / client teams
Not CL-only: ethrex must add `partial_header_hash` to the block header type (`crates/common/types/`), compute and verify the SHA256 accumulator during payload validation in `crates/blockchain`, and implement the new engine API method in `crates/networking/rpc/engine` to serve accumulator values by block hash. The accumulator inputs (parent hash, randao, gas limit, timestamp, withdrawals, slot number, execution requests) are all available in existing block-processing code, so this is mostly plumbing. **Effort: small-to-medium**, gated on ePBS/EIP-7732 landing first.

## Open questions & controversies
- SHA256 accumulator is deliberately not Merkle-provable — acceptable for a binding commitment, but limits future proof-based uses.
- Verification failure is detected only after the EL catches up; localizing the bad block requires the extra engine query (bisection).
- Spec is young (Draft, April 2026) and tightly coupled to the still-evolving Gloas/ePBS spec — container layouts may shift.
