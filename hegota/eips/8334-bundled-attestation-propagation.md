# EIP-8334: Bundled Attestation Propagation
- **Layer:** CL
- **EIP status:** Draft (PR #11905 pending, not yet merged on eips.ethereum.org)
- **Hegota status:** Proposed (CFI) (acdc/183, 2026-07-23)
- **Authors:** Sukun Tarachandani, Raul Kripalani
- **Links:** [spec PR](https://github.com/ethereum/EIPs/pull/11905) · [discussion](https://ethereum-magicians.org/t/eip-8334-bundled-attestation-propagation/29008)
- **Prior fork history:** none

## TL;DR
Replaces per-attestation gossip messages with bundles that state the shared `AttestationData` once and list `(validator_index, signature)` pairs, transported via the gossipsub partial-messages extension. Peers reconcile which attestations they hold by validator index instead of per-message hashes. Simulations claim ~50% bandwidth savings and up to 30% lower latency. Deployable between forks — no hard fork required.

## What it changes
- **New containers:** `AttestationBundle {committee_index, attestation_data, attester_indices, signatures}` (max `MAX_ATTESTATIONS_PER_BUNDLE = 50`, one committee + one shared attestation data per bundle, no duplicate validators) and `CommitteeAttestationPartsMetadata {committee_index, available, requests}` — data-free advertisements/requests keyed by global validator index.
- **Transport:** gossipsub partial-messages extension on `beacon_attestation_{subnet_id}`; group identifier = slot (LE uint64). Nodes eagerly push bundles to mesh peers every ~20ms tick; metadata advertisements go only to gossip (non-mesh) peers, which request what they lack via the `requests` field.
- **Dedup rule:** before validation, dedup identity MUST include attestation data + signature (not `(slot, validator_index)` alone, to avoid an invalid signature suppressing a later valid one); after a validator's attestation for a slot validates, further attestations from it may be dropped as replay/equivocation.
- **Validation:** each attestation in a bundle is validated individually under the existing `beacon_attestation` gossip conditions; over-size bundles are rejected.
- **Backwards compatibility:** support is negotiated via the partial-messages extension; non-upgraded nodes keep exchanging `SingleAttestation`. Not signature aggregation — signatures stay separate; the savings come from data dedup.

## Motivation
Attestation and aggregate propagation dominate time-to-finality. Today every slot gossips ~31k near-identical messages, each duplicating the same `AttestationData`, and gossipsub pays IHAVE/IWANT hash overhead per message. Deduplicating shared data cuts bandwidth ~50%, which in turn allows aggregating more attestations per committee — a prerequisite for faster finality (e.g. decoupled-consensus designs). Chosen over a fully custom protocol (which could shave another 10–20% tail latency) because it reuses gossipsub machinery and rolls out without a fork.

## Dependencies & related EIPs
- **Synergizes with:** EIP-8243 (this cluster, batching at source) — explicitly named as composable ("with some modifications"); both attack attestation bandwidth from different angles (8334: propagation transport; 8243: message count at source).
- **Related:** EIP-8136 (cell-level deltas) — the partial-messages extension was designed for it; 8334 would be its second production user. Faster-finality / decoupled-consensus research is the downstream consumer.
- **Conflicts/alternative to:** a custom attestation protocol (rejected for complexity); bitmap-based reconciliation (deferred to a later change).

## Impact on ethrex / client teams
- **None for ethrex** — pure CL networking (gossipsub topics, partial messages). ethrex is an EL client; its devp2p stack is untouched.
- For CL teams: medium — new gossipsub extension usage, bundle encode/decode, dedup cache semantics, peer scoring adjustments (must not favor partial-message peers, downscore invalid bundles).
- Effort for ethrex: **none/trivial**.

## Open questions & controversies
- Spec not yet merged into the EIPs repo (PR #11905 open at time of writing); minor maturity risk.
- Depends on the gossipsub partial-messages extension being production-proven (only prior user is EIP-8136 cell deltas).
- Bitmaps would reconcile more efficiently than validator-index lists but were deferred — possible follow-up change.
- Attestation retention/expiry left to consensus specs; objects expected to change again under Decoupled Consensus.
