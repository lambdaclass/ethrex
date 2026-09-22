# Hegotá reference for ethrex

Research snapshot: **2026-09-08**. This reference covers **65 numbered proposals tracked for Hegotá by Forkcast**, of which **58 pass its ranking filter**. It records mechanisms, dependencies, interactions, draft gaps, and ethrex implications. **No ethrex tiers have been assigned.**

Start with the [proposal catalog and profiles](eips.md). Use the [dependency guide](dependencies.md) when comparing proposals and the [ethrex code map](ethrex.md) when assessing implementation work. [knowledge.json](knowledge.json) contains the same profiles with structured metadata and an empty `ethrex_tier` for each proposal. [sources.json](sources.json) records immutable specification versions and hashes; [dependencies.json](dependencies.json) preserves the declared dependency graph.

## Scope and source rules

The roster comes from the [Forkcast EIP feed](https://forkcast.org/api/eips.json), generated at `2026-09-08T16:44:20.099Z`. Ranking eligibility follows the captured [Forkcast filter](https://github.com/ethereum/forkcast/blob/1591931556093b664684f5a291e46aecc38ea259/src/domain/eips/rankableEips.ts): a Hegotá relationship whose latest stage is Proposed or Considered, excluding headliners and Informational EIPs. This is a reproducible snapshot, not a claim that the list cannot change.

| Outside the 58-item ranking board | Reason | Still covered here because… |
| --- | --- | --- |
| [7805](eips.md#eip-7805) FOCIL | Scheduled; headliner | It changes the context for transaction eligibility and censorship resistance. |
| [8141](eips.md#eip-8141) Frames | Scheduled | Several rankable proposals amend it; ethrex already has an older implementation. |
| [8173](eips.md#eip-8173) Control-flow foundations | Informational | It explains the rationale behind control-flow changes. |
| [8369](eips.md#eip-8369) VOPS profiles | Informational | It explores the bridge between FOCIL and programmable transaction validity. |
| [8383](eips.md#eip-8383) CL retention | Informational | It affects storage defaults and synchronization assumptions. |
| [8105](eips.md#eip-8105) Encrypted mempool | Withdrawn for Hegotá | It provides context for later encrypted-ordering designs. |
| [8184](eips.md#eip-8184) LUCID | Declined for Hegotá | It connects encrypted ordering, inclusion lists, and future Frames extensions. |

Document maturity and fork inclusion are separate: an EIP can be Draft and Scheduled simultaneously. The latest status-history entry matters: 7807 was reproposed after an earlier decline. A Networking stage on a prerequisite can describe a rollout independent of a hard fork.

The captured [Hegotá meta EIP](https://eips.ethereum.org/EIPS/eip-8081) lists 52 of these numbered proposals, so it is insufficient as the sole inventory. Forkcast additionally tracks 8105, 8184, 8219, 8334, 8341, 8358, 8359, 8365, 8367, 8374, 8375, 8379, and 8383. Conversely, contextual proposals such as 7973 and 8058 have only declined Glamsterdam relationships in this feed, and 8268 has no fork relationship; they are not added to the Hegotá count.

All 65 primary specifications were read from versioned sources. Fifty-six match Ethereum/EIPs commit `991d932f52a56477753cd9f62114b842cd77275c`; nine were available through open EIPs pull requests and are pinned to their respective head commits. The profile links use those immutable versions. Source hashes were checked against the downloaded contents. Linked assets and reference implementations have not all been audited; reading a proposal is not a conformance or security certification.

For mechanism claims, prefer the specification over Forkcast summaries or third-party ratings. Where an EIP explicitly delegates to another normative repository, check both and record disagreements. Implementation implications and review questions in this reference are analysis, not statements from an ethrex team decision.

## A useful reading sequence

These groups organize context; their order is not a ranking.

1. **Establish the fork baseline.** Read 7732 ePBS, 7928 BALs, and the jointly specified 2780/8037/8038 gas changes. Include 7778 block accounting and 7843 SLOTNUM. Forkcast places these in Glamsterdam, so they should not be counted as new Hegotá candidates.
2. **Understand Frames and inclusion lists together.** Read 8141 and 7805, then 8250 keyed nonces, 8272 recent roots, 7906 assertions, and informational 8369. Separate transaction consensus validity, public relay policy, and mandatory inclusion eligibility.
3. **Compare account-control mechanisms.** Read 7819 factory-controlled delegation, 7851 self-controlled delegation, 8298 ordinary-code replacement, and 8151 ecRecover changes. Their ownership and key-retirement guarantees differ.
4. **Compose gas changes before assessing their effects.** Read 3298, 8131 → 8279, 8358, 8368/8372, and 8374; evaluate 7923 memory accounting as a separate resource model. A cheaper operation does not automatically make all gas-sensitive code compatible.
5. **Follow data from production to validation.** Read 7862 versus 8341, then 8146, 8142, 8025, and 8371. Compare 8237 and 8379 synchronization architectures. Treat 7807 as a broad encoding change across these paths.
6. **Review smaller EVM and networking proposals on their own terms.** The profiles cover opcode cleanup/additions, 7666 → 8200 EVMification, ML-DSA verification, and 8077/8094 propagation.
7. **Check the CL proposals for hidden EL work.** 8148 and 8205 introduce EL request-contract work; 8375 changes EL fee accounting. Then compare validator lifecycle, rewards, attestations, randomness, checkpoints, and slot timing, including 8365 → 8367.
8. **Read privacy separately from encrypted ordering.** 8182 keeps transfers inside a shielded pool; 8105/8184 conceal transactions before ordering and reveal them later. These are different guarantees.

## Findings to keep in mind

- **Frames is an existing implementation that needs version reconciliation.** Current 8141 uses nested fee fields, explicit execution/state budgets, two-dimensional frame receipts, and SIGDATACOPY. The inspected ethrex revision uses older representations. See the evidence in the [code map](ethrex.md).
- **VOPS is not a complete AA inclusion implementation.** Current 8369 is Informational and leaves binding enforcement/encoding work to a later standards-track extension. Its recent-root description also needs alignment with revised 8272.
- **Optional proofs do not currently replace execution.** The 8025 specification section retains re-execution; its linked CL code still calls the execution engine. Its summary and pinned EL/CL sources disagree on some public-input details.
- **Several widely repeated descriptions are stale.** Current 3298 retains write-reversal refunds, 8188 adds write metadata rather than SSTORE repricing, 8198 specifies eight-second slots, and 8272 uses a canonical frame rather than a new envelope field.
- **Composition has concrete unresolved details.** Examples include 8141/8250 encoding, 8141/8374 warming, 8115/8375 fee timing, 8321/8375 domain allocation, and gas drafts using different account-write prices. The [interaction table](dependencies.md#interactions-that-need-explicit-composition) explains each.

When recording a later judgment, keep the draft revision, expected benefit, compatibility evidence, ethrex work, and unresolved dependency beside the tier. A proposal’s missing constants, incomplete code, or contradictory wording should remain visible independently of whether its underlying design is desirable. Recheck the live roster and changed specs before submitting the finished tier list.
