# EIP-8369: VOPS Profiles for FOCIL Eligibility
- **Layer:** CL (conceptually; defines EL validity surfaces)
- **EIP status:** Draft (Informational)
- **Hegota status:** Proposed (CFI) (acdc/184, 2026-08-06)
- **Authors:** Thomas Thiery
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8369) · [discussion](https://ethereum-magicians.org/t/eip-8369-vops-profiles-for-focil-eligibility/29298)
- **Prior fork history:** none

## TL;DR
An Informational reference model that draws the line between transactions FOCIL can cheaply enforce and those it can't. Ordinary (EOA) transactions keep FOCIL's end-of-payload omission check; EIP-8141 frame transactions get a weaker, index-based check bounded by a fixed "AA-VOPS" state surface. It sets no consensus rules itself — a future Standards Track extension to EIP-7805 must restate everything binding.

## What it changes
- Defines two **VOPS (validity-only partial statelessness) profiles**:
  - **Profile 1 (Base VOPS):** legacy / EIP-2930 / EIP-1559 / EIP-7702 txs. Validity = structural validity + signature + chain_id + EIP-3607 sender check (7702 delegation counts as valid EOA) + fee validity (`max_fee >= base_fee`, `priority <= max_fee`). Omission justified only if the tx doesn't fit remaining gas or nonce/balance fails at end of payload — i.e. today's EIP-7805 rule. Needs only (address, nonce, balance, codeFlag) per account; no code execution.
  - **Profile 2 (FOCIL AA-VOPS):** EIP-8141 frame txs with empty `blob_versioned_hashes`, matching one of four validation-prefix shapes (`self_verify`, `deploy|self_verify`, `only_verify|pay`, `deploy|only_verify|pay`). Validation may read only: VOPS account fields of sender/payer, the first `AA_VOPS_SLOT_COUNT` storage slots of sender and payer, EIP-8250 keyed-nonce state, EIP-8272 recent-root entries, and code/codeHash reached during validation.
- **Index-based omission check:** builders commit a claimed insertion index for each omitted Profile-2 tx (missing/malformed index defaults to end of payload); attesters reconstruct state at that index using the EIP-7928 BAL plus parent VOPS state and replay the validation prefix.
- **VERIFY budget:** static per-IL budget `MAX_VERIFY_GAS_PER_IL` (candidate 2²⁰ ≈ 1.05M gas; ≈16.8M gas/slot across 16 includers) and `MAX_VERIFY_GAS_PER_TX`, bounding signature + validation-prefix replay cost. Budget debiting is ordered to bound work from invalid signatures.
- **Exclusions:** blob txs (EIP-4844 and frame txs with blobs), out-of-surface storage reads, non-conforming shapes, over-budget txs — all may be in ILs but are not enforced.
- **Mempool admission ≠ FOCIL eligibility:** public mempool keeps EIP-8141's sender-only storage rule; Profile 2 allows payer slots too, so some eligible txs need custom mempools/direct submission.
- Reserved: future Profile 3 for witness-backed external storage reads.

## Motivation
FOCIL's omission check is only viable if every attester can cheaply and deterministically decide "was this omission justified?". For plain txs that's nonce/balance; for AA frame txs validity can depend on near-arbitrary state, making naive checks quadratic or unverifiable. A single shared eligibility boundary prevents nodes agreeing on an IL but disagreeing on enforcement. The claimed-index design replaces an earlier O(n²) append loop, at the cost of a weaker guarantee (tx must be valid at every claimable index).

## Dependencies & related EIPs
- **Depends on:** EIP-7805 (FOCIL, this cluster) — it is an eligibility extension of it; EIP-8141 (frame txs), EIP-8250 (keyed nonces), EIP-8272 (recent roots) must be live for Profile 2; EIP-7928 (BALs, Glamsterdam) for attester state reconstruction; EIP-7843 (slot number in block).
- **Builds on:** EIP-1559/2718/2930/3607/7702/4844 tx typing for profile classification.
- **Tension with:** ePBS (EIP-7732, Glamsterdam) — payloads revealed after beacon attestations means omission checks must move post-reveal or Profile 2 stays disabled under ePBS.
- Future path: builder omission proofs once zkEVM proofs are mandatory.

## Impact on ethrex / client teams
- Informational, so **no immediate changes**. But it is a preview of the EL work a FOCIL-AA extension would require:
  - Maintaining VOPS-style state surfaces (low storage slots, keyed nonces, recent roots, code corpus) — new storage/sync surface.
  - Attester-side state reconstruction at an arbitrary tx index from BAL + deterministic replay of EIP-8141 validation prefixes — significant new EVM replay machinery with strict perf budgets vs. the attestation deadline.
  - ethrex already has in-progress EIP-8141 frame-tx code (`crates/vm/levm/src/opcode_handlers/frame_tx.rs`, `test/tests/levm/eip8141_tests.rs`) and BAL support (`crates/common/types/block_access_list.rs`), which are the primitives this would build on.
- Effort if enforced: **large** (EL+CL co-design, benchmarks pending).

## Open questions & controversies
- Everything is non-binding; constants (`AA_VOPS_SLOT_COUNT` 2–4, VERIFY caps) are candidates pending full-pipeline benchmarks.
- Profile 2 guarantee is strictly weaker than Profile 1 (builder picks the index; position-dependent txs get less protection).
- ePBS incompatibility unresolved (post-reveal omission checks).
- BAL correctness is currently only established by executing the payload; using BALs for omission checks before execution proofs exist needs care.
- Storage growth of the globally-held surface is bounded by shape, not size.
