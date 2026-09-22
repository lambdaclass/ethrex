# EIP-8188: Last-Written Block for Accounts and Slots
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) (acde/237, 2026-05-21)
- **Authors:** Wei Han Ng, Amirul Ashraf, Guillaume Ballet, Maria Silva, Gary Rong, Carlos Perez, Jochem Brouwer
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8188) · [discussion](https://ethereum-magicians.org/t/eip8188-state-tiering-by-write-age/28234)
- **Prior fork history:** none

## TL;DR
Adds a consensus-level `last_written_block` field to the RLP encoding of every account and storage slot, updated on writes and never on reads. This gives every client the same consensus-verified signal of which state is actively churning, enabling storage tiering (hot/cold) and restart prefetching. No gas changes; pricing on top of the metadata is left to a separate proposal.

## What it changes
- Account encoding: `RLP([nonce, balance, storageRoot, codeHash, last_written_block])`. Legacy 4-element accounts decode with `last_written_block = 0`.
- Slot encoding: `RLP([value, last_written_block])` (slots become a list; legacy bare bytestrings decode with field = 0 — the two are distinguishable by RLP prefix 0x80+ vs 0xc0+).
- Update rule: set to current `block_number` on mutation — SSTORE value changes, slot creation (account field also updated, since storageRoot changes), balance transfers (sender+receiver), nonce increments, new accounts/slots. SELFDESTRUCT follows balance-transfer rules per EIP-6780. No-op writes (SSTORE same value) do nothing. Rewrites within one block are idempotent.
- Reads (SLOAD, BALANCE, EXTCODE*, non-mutating calls) MUST NOT update it — required for STATICCALL correctness (EIP-214) since the field affects the state root.
- Reverts restore the field exactly like balance/nonce/storage.
- Cost: 5 bytes per account, 6 per slot (4-byte block number + RLP framing); worst case ~10.8 GB extra state at current sizes (360M accounts, 1.5B slots).

## Motivation
The protocol has no record of when state was last mutated; clients approximate write-recency with local caches/snapshots that differ across nodes. A consensus-visible write-age field yields a deterministic hot/cold partition of state: the mutable tier can be sized for write throughput (small, dense commit working set — less compaction/page-rewrite churn per block), the cold tier for density and read throughput, and a restarting node can prefetch the recent working set instead of relearning it organically.

## Dependencies & related EIPs
- Requires/interacts with **EIP-8037** (state creation pricing): the extra 5/6 bytes per entry mean `STATE_BYTES_PER_NEW_ACCOUNT` should rise 120→125 and `STATE_BYTES_PER_STORAGE_SET` 64→70, else new state is underpriced relative to encoded size.
- Foundation for a future in-protocol state-tiering gas schedule (separate, not-yet-final proposal) — this EIP deliberately ships only the metadata.
- Authors overlap with **EIP-8268** (CPerezz); both are state/trie-structure work from the statelessness/storage community.

## Impact on ethrex / client teams
Deep storage-layer change: account/slot RLP encoding and decoding in `crates/common` and the trie/database layer in `crates/storage` (both backends), state-root computation (leaf hashes change), plus write-path bookkeeping in the EVM backends (`crates/vm/levm`, `crates/vm/backends`) including revert-journal handling of the new field. Snap sync, witnesses, and `eth_getProof` must handle the new leaf format. The enabled optimizations (tiered storage, startup prefetch) are optional follow-ups. Effort: large.

## Open questions & controversies
- ~10.8 GB worst-case state growth for metadata with no direct protocol-level payoff — the benefit is client-internal optimization, which some may see as not worth consensus-enforcing.
- Only writes update the field: read-hot/write-cold state (e.g. popular price feeds) still looks inactive, limiting the signal.
- Couples a state-format change to a pricing design (EIP-8037 adjustments) that must land in sync.
- The real payoff (differentiated gas costs by tier) is explicitly out of scope and unproven.
