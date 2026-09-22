# EIP-8146: Block Access List Sidecars
- **Layer:** CL (with Engine API changes)
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acde/240, 2026-07-02)
- **Authors:** Toni Wahrstätter, Raúl Kripalani
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8146) · [discussion](https://ethereum-magicians.org/t/eip-8146-block-access-list-sidecars/27757)
- **Prior fork history:** none

## TL;DR
Takes the EIP-7928 block access list out of the execution payload and ships it as its own gossip sidecar, committed in the ePBS bid via the existing header `block_access_list_hash`. Because a BAL is roughly as big as the rest of the block (~72 KiB at 60M gas), this halves the critical-path object and — delivered ≥1s before the payload — lets ELs prefetch state and start the post-state root before execution begins.

## What it changes
- **`ExecutionPayload`:** the `block_access_list` field added by EIP-7928 is removed; BAL construction/validation and the header hash stay as in 7928.
- **`ExecutionPayloadBid`:** new field `block_access_list_hash = keccak256(rlp(BAL))` — reuses the header commitment, no new hashing scheme, no sidecar signature.
- **New `BlockAccessListSidecar` container** (`beacon_block_root`, `slot`, opaque RLP BAL bytes, max 8 MiB) on a new global gossip topic `block_access_list_sidecar`, verified by `keccak256(sidecar.bal) == bid.block_access_list_hash`.
- **Req/resp:** `BlockAccessListSidecarsByRoot` and `ByRange` v1; serving window `MIN_EPOCHS_FOR_BLOCK_ACCESS_LIST_SIDECARS_REQUESTS = 3533` epochs (matches EIP-7928 retention).
- **PTC:** `PayloadAttestationData` gains `block_access_list_present`; members vote true only if a valid sidecar arrived ≥ `BLOCK_ACCESS_LIST_LEAD_TIME` (1s) before the attestation deadline. Signal only, no penalty; payload processing hard-gates on local BAL availability (`on_execution_payload_envelope` asserts the BAL is stored; early envelopes are cached, not dropped).
- **Engine API:**
  - `engine_getPayloadV6`: returns `blockAccessList` as a top-level field, not inside the payload.
  - `engine_notifyBlockAccessListV1(blockAccessList, blockHash)`: CL MUST call it before `engine_newPayloadV5`, as soon as the sidecar verifies; the EL stores the BAL, prefetches declared accounts/slots, and MAY start the post-state root. Not a validity check.
  - `engine_newPayloadV5`: BAL field removed; EL pairs payload with the previously delivered BAL and validates per EIP-7928.
- **Builder duties:** publish the sidecar as early as possible, MUST NOT delay it until envelope reveal.

## Motivation
- The BAL is a state diff, not transactions — nodes that only advance state need the diff alone (separation of concerns).
- Removing it from the envelope halves critical-path block size, headroom that gas-limit increases need.
- Early delivery creates a guaranteed prefetch/post-state-root window (`t1 - t0` in the spec's timeline).
- EIP-7805 inclusion-list builders can use the early BAL's post-values to drop transactions the pending block already invalidates.
- Unlike the payload envelope, early BAL release is safe from same-slot unbundling: it carries no signed transactions.

## Dependencies & related EIPs
- **Requires EIP-7732** (ePBS, Glamsterdam — bid, PTC, envelope flow) and **EIP-7928** (BALs, Glamsterdam — the object being sidecarred).
- Synergizes with **EIP-7805** (FOCIL): early BAL improves inclusion-list quality.
- Tension with **EIP-8142** (Block-in-Blobs): 8146 moves the BAL off-payload while 8142 republishes payload+BAL in blobs; co-design needed if both ship.
- Complements **EIP-8379** (top-up sync): BAL-based fast-forward is one of 8379's state-advance mechanisms; sidecars make BALs retrievable independently.
- Retention window (3533 epochs) is independent of the **EIP-8383** block window (8192).

## Impact on ethrex / client teams
Engine API additions: implement `engine_notifyBlockAccessListV1`, version up getPayload/newPayload, and split BAL handling out of the payload path. ethrex already has BAL types, hashing, validation and prefetch-relevant plumbing from its EIP-7928 implementation (`crates/common/types/block_access_list.rs`, `crates/networking/rpc/engine/payload.rs`, rlpx BAL messages), so the work is reshaping delivery, not new semantics. A bounded cache for unmatched BALs is needed. **Effort: small-medium** on the EL side; bulk of the spec is CL (gossip, PTC, fork choice).

## Open questions & controversies
- Withholding: a builder can withhold the sidecar and brick its own payload (payload can't validate without the BAL) — enforced economically, but adds a third availability signal for the PTC to track.
- Adds a gossip topic, engine method, and retention requirement — cumulative networking surface.
- Early BAL exposure leaks coarse in-slot activity (touched contracts, balance moves) before the payload; judged acceptable, but it's a real information leak.
- Spec is Draft and tied to moving Gloas containers (`getPayloadV6`/`newPayloadV5` numbering may shift).
