# EIP-7805: Fork-choice enforced Inclusion Lists (FOCIL)
- **Layer:** CL (with significant EL/Engine API surface)
- **EIP status:** Draft
- **Hegota status:** Scheduled (SFI) — HEADLINER
- **Authors:** Thomas Thiery, Francesco D'Amato, Julian Ma, Barnabé Monnot, Terence Tsao, Jacob Kaufmann, Jihoon Song
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7805) · [discussion](https://ethereum-magicians.org/t/eip-7805-committee-based-fork-choice-enforced-inclusion-lists-focil/21578)
- **Prior fork history:** Glamsterdam headliner candidate (presented acdc/158, 2025-05-29), Declined from Glamsterdam (acdc/171, 2025-12-11); Hegota headliner proposal 2026-01-27, presented acdc/174, Scheduled (acdc/175, 2026-02-19)

## TL;DR
FOCIL lets a committee of 16 validators per slot publish "inclusion lists" (ILs) of mempool transactions; attesters of the next slot only vote for a block if it contains all IL transactions (or proves they can't be included). This hard-codes censorship resistance into the fork choice instead of relying on altruistic local block building. It is the confirmed Hegota headliner (SFI).

## What it changes
- **New gossip objects:** `InclusionList {slot, validator_index, inclusion_list_committee_root, transactions}` (max 8 KiB of RLP txs) and `SignedInclusionList`, broadcast on a new global topic by the per-slot IL committee (`IL_COMMITTEE_SIZE = 16`, domain `0x0C000000`).
- **Timeline (12s slot):** slot N t=0–8s committee members build/gossip ILs from their mempool view; t=9s view-freeze deadline (validators stop storing new ILs, keep forwarding); t=11s builder freezes its IL view and asks the EL to update the payload; slot N+1 t=0 proposer broadcasts block; t=4 attesters vote only if the payload satisfies their stored ILs.
- **IL satisfaction rule (conditional, anywhere-in-block):** every tx from stored ILs must be in the payload, UNLESS it doesn't fit remaining gas or fails nonce/balance checks when appended at the end of the payload.
- **EL check:** after executing all payload txs, for each IL tx not in the payload: skip if `T.gas > gas_left`; else check nonce/balance of sender against post-state; if valid, return `INCLUSION_LIST_UNSATISFIED`. Block stays valid, but the CL won't attest to it.
- **Engine API:** new `engine_getInclusionListV1`; `engine_forkchoiceUpdated` payload attributes extended to carry the IL for block building; `engine_newPayload` extended with IL transactions parameter, returning `INCLUSION_LIST_UNSATISFIED` when unsatisfied.
- **Fork choice:** store ILs before view freeze; >1 IL from the same committee member = equivocation → ignore all that member's ILs; attestation conditioned on IL satisfaction.
- **P2P:** new global topic for `SignedInclusionList`, new Req/Resp for fetching ILs by committee index; max 2 ILs per member forwarded (equivocation bound).

## Motivation
MEV-Boost/PBS concentrated block building in a few sophisticated builders, degrading censorship resistance; today CR relies on local builders which conflicts with performance/incentives. FOCIL improves on EIP-7547 (forward ILs) by being same-slot (no 1-slot delay), committee-based (harder to bribe/extort), fork-choice enforced (cannot be bypassed), and unconditional-fee-free (no new incentive mechanism; 1-out-of-N honesty among IL committee suffices).

## Dependencies & related EIPs
- Successor/alternative to EIP-7547 (inclusion lists, never shipped).
- Interacts with ePBS (EIP-7732, Glamsterdam): builders must incorporate ILs; payload-reveal timing vs. IL checks is a known tension.
- EIP-8369 (this cluster) defines VOPS eligibility profiles for which txs FOCIL can enforce (esp. AA frame txs from EIP-8141) as a future extension.
- Synergizes with 6-second slots / shorter-slot proposals; sensitive to any slot-duration change (EIP-8198, this cluster) because its deadlines are baked into the slot timeline.
- Competes in spirit with BRAID/APS-style designs but is the chosen headliner.

## Impact on ethrex / client teams
- **Engine API (`crates/networking/rpc/engine/`):** implement `engine_getInclusionListV1`, extend `forkchoiceUpdated` payload attributes and `newPayload` with IL data + `INCLUSION_LIST_UNSATISFIED` status. Medium.
- **Block validation (`crates/blockchain/blockchain.rs`, `crates/vm`):** post-execution IL check — for each missing IL tx, nonce/balance check against post-state + gas-left check. Needs access to sender account state after execution; straightforward given existing state access. Small-medium.
- **Block production (`crates/blockchain/payload.rs`, mempool):** IL-building strategy from the local mempool (8 KiB cap), and payload construction that appends IL txs; spec warns of a naive O(n²) re-validation loop and suggests tracking nonce/balance of IL-sender EOAs during building.
- **ethrex L1 is EL-only**, so IL committee gossip, fork-choice enforcement, and attester logic are CL client work; ethrex's touchpoints are Engine API + validation + payload building.
- Overall: **medium-large** for ethrex (mostly Engine API + payload building/validation paths); large for CL teams.

## Open questions & controversies
- Exact timings (t=9s freeze, t=11s builder freeze) marked as subject to tests/benchmarks.
- No incentives for IL committee members — relies on altruism; debated.
- Interaction with AA (EIP-7702/EIP-8141) txs whose validity depends on arbitrary state — the end-of-payload nonce/balance check is insufficient there; EIP-8369 scopes what's enforceable.
- Tension with ePBS timing and with any slot-time reduction (deadlines would compress).
- Builder compliance is enforcement-by-fork-choice, not validity: blocks that ignore ILs are valid but orphaned in practice — social/liveness edge cases around split views of ILs.
