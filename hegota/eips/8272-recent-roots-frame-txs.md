# EIP-8272: Recent Roots for Frame Transactions
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acde/240, 2026-07-02)
- **Authors:** Thomas Thiery, Vitalik Buterin, Toni Wahrstätter, lightclient
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8272) · [discussion](https://ethereum-magicians.org/t/eip-8272-recent-roots-for-frame-transactions/28621)
- **Prior fork history:** none

## TL;DR
Lets EIP-8141 frame transactions declare up to 16 `(source_id, slot, root)` references in the signed envelope, which clients verify against a `RECENT_ROOT` system contract **before** any frame executes. This gives validation code access to recent application roots (e.g. privacy commitment-tree roots) without reading mutable third-party storage during validation — the thing ERC-7562-style mempool rules forbid.

## What it changes
- **New system contract** at `0x...8272`: accepts a single 64-byte write call `(salt, root)`; `source_id = keccak256(msg.sender || salt)`. Each source has a ring buffer of `RECENT_ROOT_LENGTH = 8192` slots indexed by beacon slot number; stored entry is a domain-separated hash committing to `(source_id, slot, root)`. Direct calls only (DELEGATECALL/CALLCODE cannot write); last write in a slot wins; no read operation exposed.
- **Payload delta on 8141:** appends `recent_root_references = [[source_id, slot, root], ...]` (max 16) after `blob_versioned_hashes`; covered by the canonical signature hash; priced as calldata plus per-reference intrinsic gas (one access-list address + storage key + two keccaks per reference).
- **Reference validity (pre-execution, tx pre-state):** reference valid iff `1 <= current_slot - slot <= 8191` and the stored entry hash matches. Any invalid reference makes the whole transaction (and a block containing it) invalid. `current_slot` comes from the EIP-7843 `slotNumber` field, **not** derived from timestamp. Current-slot writes are never referenceable, so in-progress writes can't invalidate pending txs.
- **Access-list integration:** each valid reference warms `RECENT_ROOT_ADDRESS` + its storage key (gas accounting only).
- **Introspection:** `TXPARAM(0x0F)` = reference count; new opcode `RECENTROOTREFLOAD` (0xB5, 3 gas) reads `source_id`/`slot`/`root` of a declared reference — usable in any frame mode including VERIFY.
- **Activation:** fork-boundary system-contract creation/update, reorg-aware; pre-fork frame txs invalidated.
- **Mempool policy:** nodes admit only txs whose references are valid against current head; evict on expiry (`current_slot - slot >= 8192`), recheck on head change/slot advance/reorg; SHOULD index pending txs by declared reference and expiry slot.

## Motivation
EIP-8141 validation must not read mutable application storage — one changed cell could mass-invalidate pending transactions. But privacy protocols need to prove spends against a recent tree root, and wallets against authorization roots. Declaring the root in the signed envelope, verified against a rolling system-contract buffer, provides this dependency in a bounded, pre-execution-checkable way: each reference names exactly one system-contract storage key.

## Dependencies & related EIPs
- **Depends on (hard):** EIP-8141 (payload/frame delta; must activate at or after), EIP-7843 (SLOTNUM / slot in the block header — `current_slot` source; 7843 is also a Hegota candidate), EIP-7623 (calldata pricing).
- **In-cluster:** synergizes with EIP-8250 (keyed nonces) — jointly the privacy-spend stack: 8272 carries provable roots, 8250 carries nullifier-derived nonce domains. Independent of 7906.
- **Related:** pattern is similar to EIP-2935/4788 system-contract ring buffers (history storage), extended to application-defined roots.

## Impact on ethrex / client teams
Medium (on top of 8141). New system contract with fork-boundary activation; new payload field + static validity rules; a pre-execution reference-check step in block validation and tx-pool admission (up to 16 storage reads + hashing per tx); one new cheap opcode and one TXPARAM index; Engine API dependency on the `slotNumber` field from EIP-7843 being available at execution; mempool needs reference/expiry indexing and slot-advance eviction. Stateless guest programs must include the touched system-contract slots in witnesses. Effort: **medium**; inapplicable without 8141 (and awkward without 7843).

## Open questions & controversies
- Root writes have **no inclusion guarantee** — applications must run their own publication path/redundant sources; usability depends on off-protocol incentives.
- Permanent storage growth per root source (8192 slots each), priced only by the writes that create it; spec flags a possible future per-source registration surcharge.
- `source_id` is chain-independent — cross-chain proofs must bind the chain domain themselves; replay confusion risk for bridges.
- Adds a second system contract with an irregular fork-boundary activation; the fork config must verify the address is empty at activation or the payload is invalid.
- Ties EL execution validity to beacon slot numbering (via 7843), a subtle coupling between layers that needs care on reorgs and non-mainnet chains.
