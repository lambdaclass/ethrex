# EIP-8383: Reduce CL Block Retention Window
- **Layer:** CL
- **EIP status:** Draft (Informational; pending as PR ethereum/EIPs#12188, not yet merged — spec read from the PR)
- **Hegota status:** Proposed (CFI, acdc/185, 2026-08-20)
- **Authors:** Kevaundray Wedderburn
- **Links:** [spec (PR)](https://github.com/ethereum/EIPs/pull/12188/files) · [discussion](https://ethereum-magicians.org/t/eip-8383-reduce-cl-block-retention-window/29449)
- **Prior fork history:** none

## TL;DR
Cuts the CL's mandatory beacon-block serving window (`MIN_EPOCHS_FOR_BLOCK_REQUESTS`) from 33,024 epochs (~5 months) to 8,192 epochs (~36 days). The old value came from a worst-case weak-subjectivity calculation with `MAX_SAFETY_DECAY = 100`, while checkpoint sync actually runs with `SAFETY_DECAY = 10`, giving a real weak-subjectivity bound of ~3,532 epochs. No hard fork required — it's a config/serving-expectation change.

## What it changes
- Sets `MIN_EPOCHS_FOR_BLOCK_REQUESTS = 8192`; replaces `compute_min_epochs_for_block_requests()` with a constant return.
- All block-serving and backfill requirements that reference that function apply with the new value.
- 8,192 chosen as the next power of two above 4,096 (smallest power of two above the 3,532-epoch WS bound), as a conservative first step.
- Test expectations: consensus-spec tests must assert max WS period under mainnet policy < 8,192 epochs; requests for blocks older than the window MAY return `ResourceUnavailable` without penalty to the serving peer.
- Non-finality guidance: during long non-finality, clients (EL and CL) SHOULD retain all unfinalized blocks, decoupling history retention from the WS period.
- Notably states: some CLs reconstruct historical execution payloads from the EL, so the CL retention window acts as a *lower bound* on how much payload history a history-expiring EL should store.

## Motivation
The current window forces a checkpoint-synced node to backfill almost five months of blocks just to serve peers, even if its checkpoint is hours old — wasted bandwidth, disk, and sync time. Blocks older than the WS bound don't make an outdated checkpoint safe anyway (you'd need a newer checkpoint regardless).

## Dependencies & related EIPs
- **No hard dependency**; standalone config change, Informational type.
- Interacts with **EIP-8379** (top-up sync): the CL retention window caps how far back top-up delivery can reach before the EL must snap-sync.
- Interacts with **EIP-8237** (independent CL/EL sync): lighter retention complements payload-free CL range sync.
- Adjacent to EL history-expiry work (EIP-4444 lineage; the candidate list's history-related proposals): the EIP explicitly positions the CL window as a lower bound for EL payload retention.
- Note: **EIP-8146** independently sets BAL sidecar retention at 3,533 epochs (matching EIP-7928's window) — a different, shorter window.

## Impact on ethrex / client teams
Essentially **none/minimal for ethrex (CL-only, no fork)**. The one EL touchpoint is the soft guidance that a history-expiring EL should keep at least as much payload history as the CL block window (i.e. ~36 days after this change, down from ~5 months) so CLs can reconstruct payloads. ethrex currently keeps full history by default; any future pruning feature would take this bound into account. **Effort: trivial** (awareness only, unless/until ethrex implements history expiry).

## Open questions & controversies
- Mixed deployment: old clients may request blocks in the 8,192–33,024 epoch gap and get `ResourceUnavailable`; the EIP argues this is harmless since some nodes already serve no history.
- Choosing 8,192 is admittedly arbitrary ("conservative initial reduction"); some may push for 4,096.
- Invariant (`WS period < retention window`) must be re-checked if weak-subjectivity parameters ever change.
- Being Informational rather than Core, it's a coordination/serving-expectations document more than a consensus change.
