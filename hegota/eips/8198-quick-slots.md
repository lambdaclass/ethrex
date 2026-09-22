# EIP-8198: Quick Slots
- **Layer:** CL (small EL touchpoint: gas/blob scaling at fork block)
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) (acdc/184, 2026-08-06)
- **Authors:** Carl Beekhuizen
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8198) · [discussion](https://ethereum-magicians.org/t/eip-8198-quick-slots/28057)
- **Prior fork history:** none

## TL;DR
Makes `SLOT_DURATION_MS` a runtime CL configuration instead of a compile-time constant, then uses that to cut slots from 12s to 8s, scaling gas and blob limits proportionally so throughput per second is unchanged. Delivered in three phases: timing-infrastructure refactor, CL performance characterization, then iterative slot reduction. Requires EIP-7892 (blob-parameter-only forks) for the blob schedule entry.

## What it changes
- **Phase 1 (infrastructure):** remove the hardcoded 12s assumption across clients — background task scheduling, timing constants, `compute_time_at_slot(...)`, fork transition logic all derive from runtime config. Intra-slot deadlines (attestation at 4s, etc.) become basis points of `SLOT_DURATION_MS` and scale automatically.
- **Phase 2:** systematic CL perf characterization (devnets/benchmarks analogous to EL bloat-nets/perf-nets) — blob propagation, aggregation capacity, block building times.
- **Phase 3 (this fork):** `SLOT_DURATION_MS` 12,000 → 8,000 at fork epoch (8s is a placeholder pending phase 2), with constant adjustments to preserve wall-clock/economic invariants:
  - `BASE_REWARD_FACTOR` 64 → 42 (annualized issuance; ~1.6% under-issuance from truncation).
  - `INACTIVITY_PENALTY_QUOTIENT_BELLATRIX` 16,777,216 → 37,748,736 (quadratic in epoch count).
  - Blob/data-column sidecar retention 4,096 → 6,144 epochs (preserve wall-clock window).
  - Churn: `CHURN_LIMIT_QUOTIENT` 65,536 → 98,304; per-epoch activation/exit limits scaled to preserve the weak subjectivity period.
  - `SLOTS_PER_EPOCH` stays 32 → epochs shrink to ~4.3 min; finality improves ~13 → ~8.5 min "for free".
- **EL gas limit:** the first block at/after the fork timestamp MUST set `fork_gas_limit = parent_gas_limit * 8000 // 12000`, bypassing the normal ±1/1024 voting rule for that one block; normal voting resumes after. Preserves gas-per-second.
- **Blobs:** new `BLOB_SCHEDULE` entry at the fork epoch with `new_max_blobs = old_max_blobs * 8000 // 12000` (per EIP-7892 machinery).

## Motivation
12s slots are perceptible in payments, deposits, every interaction. Shorter slots also: cut DEX arbitrage losses (~sqrt of inter-block time; 12→8s ≈ 18% reduction), shrink builders' free option under ePBS (fewer empty blocks), reduce the need for preconfirmation protocols, and speed up based rollups and L2 interop that inherit L1 block time. The infrastructure-first framing means even if 8s proves infeasible, clients end up cleaner and future slot changes become config updates.

## Dependencies & related EIPs
- **Requires:** EIP-7892 (blob parameter only hardforks / BLOB_SCHEDULE) for the blob schedule adjustment.
- **Tensions with this cluster:** FOCIL (EIP-7805) has a tight intra-slot IL timeline (t=8s build, t=9s freeze, t=11s builder freeze) that would compress under 8s slots — co-scheduling both in Hegota needs timing analysis; attestation bandwidth EIPs 8243/8334 are natural enablers (less attestation traffic per second needed).
- **Related:** ePBS (EIP-7732, Glamsterdam) — shorter slots mitigate its empty-block problem; PeerDAS (EIP-7594, Fusaka) propagation margins are a phase-2 unknown.

## Impact on ethrex / client teams
- **EL impact is small and well-scoped:** the one-time fork-block gas limit rule (header validation in `crates/blockchain` / fork transition logic) and consuming the new `BLOB_SCHEDULE` entry. ethrex has no slot clock of its own (grep shows slot-duration references only as timing heuristics in `crates/blockchain/payload.rs` / `prewarm.rs`), so no deep EL refactor.
- Second-order effects: block time assumptions in RPC UX (`eth_getBlockByNumber` polling cadence in wallets/tooling, not ethrex itself), metrics dashboards, and any L2 sequencer logic in `crates/l2` that assumes 12s L1 (worth auditing: `crates/l2/sequencer/l1_watcher.rs`).
- The heavy lifting is on CL clients (timing refactor across the whole codebase) and on joint perf characterization.
- Effort for ethrex: **small** (fork-block gas rule + blob schedule + L2 audit).

## Open questions & controversies
- The final number is explicitly undecided — 8s is a placeholder; phase 2 could conclude 12s is optimal (fallback = keep 12s, keep the cleanup).
- Shrinks propagation/validation/aggregation windows; validator hardware and home-staker bandwidth are the constraint. Peak per-payload bandwidth is unaffected (gas/blob per block scale down), but per-second overheads (attestation aggregation, fork choice) rise.
- Interaction with FOCIL's deadlines in the same fork is unresolved.
- Applications/tooling assuming 12s blocks need updates; weak-subjectivity preserved only via the churn scaling (needs review).
- Competes for Hegota scope with the FOCIL headliner; non-headliner timing changes have historically been contentious (see prior 6s-slot debates).
