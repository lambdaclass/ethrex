# EIP-8333: Align Checkpoint with Epoch Boundary Block
- **Layer:** CL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acdc/184, 2026-08-06
- **Authors:** Cayman, Nico Flaig, Lodekeeper, twoeths
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8333) · [discussion](https://ethereum-magicians.org/t/eip-8333-align-checkpoint-with-epoch-boundary-block/29003)
- **Prior fork history:** none

## TL;DR
Moves the FFG checkpoint anchor for epoch N from the **first block of epoch N** to the **last block before epoch N** (the epoch boundary block). The checkpoint epoch itself is unchanged — only which block root a checkpoint maps to. This fixes a long-standing off-by-one (consensus-specs issues 2174/652) that splits first-slot attesters' target votes whenever the epoch's first block is late, costing them target rewards; it also makes "epoch N finalized" actually mean the whole epoch is finalized.

## What it changes
- New helper `get_checkpoint_slot(epoch)`: from the fork epoch on, returns `compute_start_slot_at_epoch(epoch) - 1`; pre-fork epochs and genesis keep the old anchoring.
- New accessor `get_checkpoint_root(state, epoch)` = block root at the checkpoint slot (most recent block at or before it — falls back to an earlier block if trailing slots are empty). Replaces `get_block_root(state, epoch)` wherever targets/checkpoints resolve:
  - `get_attestation_participation_flag_indices` (matching-target check),
  - `weigh_justification_and_finalization` (justified checkpoint roots),
  - fork choice `get_checkpoint_block` (ancestor at the checkpoint slot) — consumers `on_attestation`, `filter_block_tree`, and gossip target checks need no further change.
  - Honest validator guide: FFG target root uses `get_checkpoint_root`; the first-slot special case is removed.
- Fork transition rule: pre-activation epochs must resolve under the old rule or block import and finality checks break; attestations with pre-activation target epochs stay valid and rewarded.
- No container changes, no slashing-condition changes, no gossip format changes.

## Motivation
Today the first slot's committee must vote on a target block proposed in the same slot, so a late first block splits votes (~1/32 of FFG weight exposed every epoch, visible on mainnet as target-miss concentration in slot 0 of each epoch). Anchoring to the boundary block fixes the target before the epoch begins, giving the first committee a full extra slot of propagation margin. It also aligns FFG checkpoints with the shuffling-dependent-root anchor already used for duty lookahead, and makes finality reports from explorers/tooling accurate per epoch.

## Dependencies & related EIPs
- **Depends on:** nothing (self-contained CL change).
- **Synergizes with:** fork-choice hygiene generally; orthogonal to the rest of the cluster.
- No interaction with EL, Engine API, or ePBS.

## Impact on ethrex / client teams
- None for ethrex (CL-only; the spec states explicitly "no changes to the execution layer").
- CL client work: touch the two state-transition call sites, one fork-choice function, validator guide, plus careful fork-transition handling for pre-activation checkpoints. Checkpoint-sync providers/consumers and tooling that assume the old anchor must be updated.
- Rough effort: **small** (but the fork-transition edge cases need care).

## Open questions & controversies
- A maliciously late *boundary* block can still split the next epoch's first-slot votes — the same attack as today but strictly harder (margin grows from sub-slot to over a full slot).
- Tooling/checkpoint-sync assumptions about the old anchor are a real migration cost.
- Otherwise uncontroversial: framed by the authors as fixing an off-by-one rather than a design change.
