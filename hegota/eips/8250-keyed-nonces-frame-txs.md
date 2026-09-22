# EIP-8250: Keyed Nonces for Frame Transactions
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acde/240, 2026-07-02)
- **Authors:** Thomas Thiery, Toni Wahrstätter, lightclient, Vitalik Buterin
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8250) · [discussion](https://ethereum-magicians.org/t/eip-8250-keyed-nonces-for-frame-transactions/28437)
- **Prior fork history:** none

## TL;DR
Replaces the single linear `nonce` field in EIP-8141 frame transactions with `(nonce_keys, nonce_seq)`: up to 16 independent nonce domains, each with its own protocol-managed sequence stored in a `NONCE_MANAGER` system contract. Key `[0]` aliases the legacy account nonce; non-zero keys give replay-independent sequences, so many unrelated users can share one sender (privacy protocols, relayers, session keys) without one stuck tx blocking all others.

## What it changes
- **Payload delta on 8141:** `nonce` is replaced by `nonce_keys` (sorted list of 1–16 uint256s, canonical RLP ints) + `nonce_seq` (uint64). Both priced as calldata under EIP-7623/8141 accounting. Key 0 may only appear alone (`[0]` = legacy nonce).
- **Nonce state:** for non-zero keys, sequence stored in protocol-managed storage of `NONCE_MANAGER` at `0x...8250`, slot = `keccak256(pad32(sender) || bytes32(key))`. Ordinary calls to the contract revert (`0x60006000fd`); only the protocol writes. First use creates the slot, surcharged at EIP-8037 state-gas price (`STATE_BYTES_PER_STORAGE_SET * CPSB` ≈ 97,920 state gas); slots are never deleted.
- **Validity:** `nonce_seq` must equal the current sequence of **every** selected key, checked pre-execution against actual pre-state (like the existing nonce check). Overlapping key sets therefore serialize; disjoint sets are replay-independent.
- **Consumption:** replaces 8141's nonce increment inside the payment-scoped `APPROVE` transition — atomic with fee collection, persists through later frame reverts and atomic-batch rollback. `[0]` still increments (not sets) the legacy nonce, preserving CREATE/CREATE2 interactions.
- **Introspection:** new `TXPARAM` indices — `0x0D` pre-state legacy nonce, `0x0E` key count, `0x0F` `nonce_keys_hash`, `0x10` first key. `TXPARAM(0x01)` now returns `nonce_seq`.
- **Activation:** system contract initialized at fork boundary (create or set code/nonce, preserve balance); reorg-safe; pre-fork frame txs die at the boundary.
- **Mempool:** pending identity becomes `(sender, nonce_keys, nonce_seq)`; revalidation triggers on any selected key's sequence change. The one-pending-frame-tx-per-sender public mempool rule from 8141 is kept, but the protocol-level obstacle to relaxing it is removed.

## Motivation
Privacy protocols route many users through one shared sender; with a single linear nonce, one included tx invalidates every other pending withdrawal. Same pain for session keys and relayer senders. Tying keyed-nonce consumption to the payment-approval step also gives single-use keys (nullifiers) an atomic spent-once guarantee that survives later frame reverts — something a contract-managed nullifier table inside a STATICCALL-ish VERIFY frame cannot provide.

## Dependencies & related EIPs
- **Depends on (hard):** EIP-8141 (delta against it; must activate at or after), EIP-8037 (state gas pricing for slot creation), EIP-7623 (calldata pricing of new fields).
- **In-cluster:** synergizes with EIP-8272 (recent roots) — together they target the privacy-spend use case: 8272 supplies provable recent tree roots, 8250 supplies nullifier-friendly nonce domains. Compatible with 7906 (orthogonal POST_TX mode).
- **Related prior art:** ERC-4337's key/sequence packed nonce (this EIP generalizes it: full 32-byte keys, explicit 64-bit sequence).

## Impact on ethrex / client teams
Medium (on top of 8141). New payload decoding/validation for the frame tx type (`crates/common/types/transaction.rs`), a new system contract with fork-boundary activation and reorg handling, changes to the APPROVE payment transition (nonce-set consumption + first-use state-gas charge), four new TXPARAM params, and mempool dependency tracking keyed on `(sender, key)` pairs instead of `(sender, nonce)`. Stateless/zkVM guest programs must handle the system-contract storage reads as protocol bookkeeping (excluded from EIP-2929 access lists and EIP-2200 pricing). Cancellation-by-same-nonce semantics change for keyed txs — RPC/wallet-facing subtlety. Effort: **medium**; inapplicable without 8141.

## Open questions & controversies
- Permanent state growth: every used key occupies a slot forever, priced only by first-write state gas (~98k gas at current CPSB); block-level creation rate bounded but lifetime cost debated.
- Nonce keys are visible in the payload — key-domain choice is not private, which weakens the privacy narrative (metadata leak, noted in the spec).
- Shared `nonce_seq` across all selected keys is a deliberate simplification; applications needing independent sequences per key are explicitly out of scope, leaving possible future extension debt.
- `TXPARAM(0x01)` semantics change (no longer necessarily the legacy nonce) — verifier contracts written against 8141 alone may need updates; fork-boundary invalidation of pending 8141 txs if activated later.
