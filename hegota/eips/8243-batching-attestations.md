# EIP-8243: Batching Attestations at Source
- **Layer:** CL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) (acdc/180, 2026-06-11)
- **Authors:** Raúl Kripalani, Toni Wahrstätter, Mikhail Kalinin
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8243) · [discussion](https://ethereum-magicians.org/t/eip-8243-batching-attestations-at-source/28606)
- **Prior fork history:** none

## TL;DR
Lets an operator whose validators share a slot committee publish one pre-aggregated `BatchAttestation` instead of N individual ones. Each member pre-signs a "batch seal" authorizing a designated batcher; the batcher signs the composition. The on-chain attestation container and block validity rules are completely unchanged — only gossip encoding changes.

## What it changes
- **New containers:**
  - `BatchSealPreimage {slot, committee_index, batcher}` — each batched validator signs this under new `DOMAIN_BATCH_ATTESTER` (0x0B000000). Deliberately NOT bound to `aggregation_bits`, so seals can be pre-signed at epoch start and tolerate validator-client failures.
  - `BatcherPreimage {slot, committee_index, aggregation_bits}` — signed by the batcher under `DOMAIN_BATCHER` (0x0B0000FF) to close over the exact composition.
  - `BatchAttestation` = committee_index + aggregation_bits + data + **standard BLS aggregate signature over `data`** (identical to today's aggregate) + `batcher` + gossip-only `batch_seal` and `batcher_signature` (discarded after validation).
  - `WireAttestation = Union[SingleAttestation, BatchAttestation]`, SSZ-serialized with a 1-byte selector (0x00 / 0x01).
- **P2P:** `beacon_attestation_{subnet_id}` topic message type changes from `SingleAttestation` to `WireAttestation` — a hard-fork-boundary change (pre/post-fork serializations are incompatible).
- **Dedup principle:** accept a message iff it carries ≥1 previously unseen vote for the duty; per-(slot, committee, data_root) caches track `seen_attesters` and `seen_batchers` (first batch per batcher only). This bounds spam by committee size and makes the naive O(2^k) subset-rebundling attack non-propagating.
- **State transition:** none. Aggregators strip the seal/batcher signature and feed the batch into the existing aggregation pipeline; `process_attestation` unchanged.
- **No new slashing conditions** (signature theft and non-consensual inclusion are structurally impossible; equivocation evidence exists but slashing is left as future work).

## Motivation
~1M validators produce ~31k attestation messages per slot across 64 subnets; large operators re-send the same vote many times. EIP-7251 (balance consolidation) reduced validator count only slowly because it's opt-in. Batching gets consolidation-like network savings with zero operator action. Holding traffic constant, the savings could enable fewer subnets/committees and shorter epochs (faster finality). It also creates a k-anonymity primitive: any attester can route its vote through a consenting co-committee batcher. Not a substitute for an eventual validator cap.

## Dependencies & related EIPs
- **Synergizes with:** EIP-8334 (this cluster, bundled propagation) — 8334's rationale explicitly names 8243 as a composable next step; 8243 cuts message count at source, 8334 cuts bytes on the wire.
- **Related:** EIP-7251 (MaxEB, Pectra) as the opt-in alternative for reducing attestation volume; enablers for shorter epochs / faster finality research; complements EIP-8198 (quick slots, this cluster) which needs lower attestation overhead to shrink slot time.
- **Independent of** FOCIL (EIP-7805) technically, though both are attester-duty changes competing for CL review bandwidth in Hegota.

## Impact on ethrex / client teams
- **None for ethrex** — pure CL change (gossip encoding, BLS seal verification, dedup caches). On-chain containers and block validity unchanged, so no EL touchpoint, no Engine API change.
- For CL teams: small-medium — new containers/domains, union deserialization on a hot gossip path, two extra aggregate BLS verifications per batch, new dedup cache. Validator-client integration (seal pre-signing, batcher selection, fallback) is most of the work.
- Effort for ethrex: **none**.

## Open questions & controversies
- Privacy trade-off: batches reveal which validators are co-located under one operator (partially mitigated by the cross-operator k-anonymity use case, which needs off-protocol coordination).
- DVT/pooled-staking integration not yet validated.
- Batcher failover relies on pre-signed fallback seals — operator policy, not protocol.
- Same-batcher equivocation produces slashable-grade evidence but slashing is deliberately not enabled.
- Requires simultaneous upgrade at fork boundary (wire format change), unlike the negotiated rollout of EIP-8334.
