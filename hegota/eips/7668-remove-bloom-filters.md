# EIP-7668: Remove bloom filters
- **Layer:** EL
- **EIP status:** Stagnant
- **Hegota status:** Proposed (CFI) — acde/242, 2026-07-30 (champion: Jochem Brouwer)
- **Authors:** Vitalik Buterin
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7668) · [discussion](https://ethereum-magicians.org/t/eip-7653-remove-bloom-filters/19447)
- **Prior fork history:** Declined from Fusaka; Proposed for Glamsterdam then Declined (acde/227, 2026-01-05)

## TL;DR
Require the logs bloom in both the block header and transaction receipts to be empty (0 bytes). Bloom filters were designed to let light clients and dapps cheaply scan history for relevant logs, but in practice they are too slow and too noisy at current gas limits, and virtually everyone queries logs through centralized indexers instead.

## What it changes
- Block-header `logsBloom` field: must be empty (0 bytes) after the fork — the field is not removed from the header RLP yet, only required to be empty.
- Receipt `logsBloom` field: likewise required to be empty.
- Gas costs of LOG opcodes are intentionally NOT reduced (hashing costs still matter for zk-EVM provers even though the bloom externality goes away).
- A future EIP could remove the fields entirely; this one is the minimally-disruptive step.
- Breaks any application still relying on bloom filters for log filtering (few, given the high false-positive rate today).

## Motivation
The bloom mechanism never delivered on its intent: history queries via node RPC are far too slow, so dapps use extra-protocol centralized services (indexers). Keeping blooms forces every client to compute and store data nobody uses. Removing them simplifies the protocol and nudges the ecosystem toward decentralized provable log indexes (ZK-SNARK/IVC-based).

## Dependencies & related EIPs
- None required. Standalone consensus change.
- Synergizes with general header-cleanup candidates; note EIP-7709 (blockhash-related) and receipt-format cleanups are in the same "protocol simplification" bucket but are independent.
- Declined twice already (Fusaka, Glamsterdam) — its recurring re-proposal is driven by devnet/test simplification arguments.

## Impact on ethrex / client teams
- Trivial-to-small. Bloom computation/validation lives in `crates/common/validation.rs` (header bloom check), `crates/common/types/receipt.rs`, and payload building in `crates/blockchain/payload.rs`; the change is to write/expect empty blooms post-fork instead of computing `logs_bloom`.
- RPC: `eth_getLogs` filtering paths in `crates/networking/rpc/eth/logs.rs` may currently use header blooms as a fast pre-filter — would need to scan receipts directly (slower for naive implementations, hence the "dapps should use indexers" rationale).
- Engine API and devp2p: header field stays present (empty), so wire formats are unchanged. zk/guest-program code (`crates/guest-program`) no longer needs bloom computation.
- Existing tests reference blooms (`test/tests/blockchain/logs_bloom_tests.rs`, `logs_bloom_validation_tests.rs`) and need fork-gating.

## Open questions & controversies
- Declined for two consecutive forks — Glamsterdam ACDE decided against it (acde/227), partly because it breaks log-filtering UX without a shipped decentralized alternative, and the client-side savings are modest.
- Pushback: keeping an always-empty field in the header is seen by some as the worst of both worlds (breaking consumers but not actually simplifying the wire format).
- Spec is Stagnant and minimal; no test vectors.
