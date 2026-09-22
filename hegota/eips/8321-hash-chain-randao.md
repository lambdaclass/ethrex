# EIP-8321: Hash-Chain RANDAO
- **Layer:** CL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acdc/183, 2026-07-23
- **Authors:** Kevaundray Wedderburn, Benedikt Wagner, Tom Wambsgans, Justin Drake, Thomas Coratger
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8321) · [discussion](https://ethereum-magicians.org/t/eip-8321-hash-chain-randao/28942)
- **Prior fork history:** none

## TL;DR
Replaces the BLS-signature RANDAO reveal with a hash-chain commit-reveal: each validator registers the tip of a hash chain once, then reveals the next preimage per proposed block. Motivation is post-quantum readiness — RANDAO's grind resistance relies on BLS signature uniqueness, which neither survives a quantum adversary recovering keys nor holds in the planned hash-based (lean/XMSS) signature scheme. Unregistered validators keep using the legacy BLS path, so there is no cutover deadline.

## What it changes
- Validator generates a chain `c_i = blake3("HASH_CHAIN_RANDAO" + c_{i-1})` from a random 32-byte secret `c_0` (recommended n ≥ 2^16 links, ~centuries of proposals), registers the tip `c_n`, and reveals links in reverse order, one per proposed block.
- New beacon operation `SignedRandaoCommitmentRegistration` (BLS-signed, `DOMAIN_RANDAO_COMMITMENT_REGISTRATION = 0x0F000000`), capped at 128/block, valid only while the validator is unregistered, activated after `COMMITMENT_REGISTRATION_DELAY = 3` epochs (≥ `MIN_SEED_LOOKAHEAD + 2`) so registration can't be ground against known duties. One-time only — a lost chain or bad commitment means exit and re-enter.
- `BeaconBlockBody` gains `hash_chain_reveal: Bytes32` and `randao_commitment_registrations`; `randao_reveal` becomes transitional (must be the G2 point at infinity once registered).
- `BeaconState` gains `randao_commitments: List[Bytes32, VALIDATOR_REGISTRY_LIMIT]` (zero = unregistered) and `pending_randao_commitments` (EIP-7916 progressive list).
- Modified `process_randao`: if the proposer is registered, verify `blake3(DST + reveal) == commitment`, fold the raw reveal into the mix via a **hash accumulator** `mix = blake3(mix + reveal)` (not XOR, to kill a copied-commitment cancellation attack), and store the reveal as the new commitment. Legacy path unchanged otherwise.
- All hashing introduced uses **BLAKE3** (fast in software, efficient in binary-field proof systems), not the consensus `hash` helper.
- New gossip topic `randao_commitment_registration` with first-seen-per-validator dedup. Legacy BLS path and the registration op are meant to be sunset in the eventual PQ fork (which sets commitments at deposit time).

## Motivation
A CRQC can recover a BLS key and predict every future RANDAO contribution — leaking the proposer schedule and amplifying bias (any k slots in the mixing period become usable for reveal-or-withhold, not just k consecutive tail slots). Hash chains need only collision resistance (anti-grinding) and preimage resistance (unpredictability), both believed quantum-secure. This is an incremental step: block signatures and the registration signature stay BLS; the point is to remove RANDAO's structural dependence on signature uniqueness before the PQ fork swaps signature schemes.

## Dependencies & related EIPs
- **Depends on:** EIP-7916 (progressive lists, for the pending-commitments queue).
- **Builds toward:** the post-quantum consensus signature fork (lean/XMSS signatures), which this EIP is designed to unblock — XMSS is grindable, so RANDAO must not depend on signature uniqueness.
- **EL touchpoint:** EIP-4399 PREVRANDAO is unaffected — `randao_mixes` remains a 32-byte per-block accumulator, only the provenance of contributions changes.

## Impact on ethrex / client teams
- None for ethrex (CL-only). The RANDAO mix consumed via `prev_randao` in the Engine API / PREVRANDAO opcode keeps identical format and semantics.
- Rough effort: none (EL); moderate CL work (new op, state fields, epoch processing, gossip).

## Open questions & controversies
- A commitment can never be updated: lost/mistyped chain forces exit and re-entry; the chain secret needs signing-key-grade custody (and must not be derived from it, since the chain eventually reveals its seed).
- Reveals in orphaned blocks remain valid future contributions (predictable, but RANDAO tolerates that).
- The transition is incomplete by design — BLS remains for block signatures and registrations until the PQ fork.
- During the transition, a CRQC attacker could predict remaining legacy reveals and grind its one-time registration; acknowledged in the spec.
