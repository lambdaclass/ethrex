# EIP-8141: Frame Transaction
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Scheduled (SFI, acde/244, 2026-08-27) — was headliner candidate (not selected)
- **Authors:** Vitalik Buterin, lightclient, Felix Lange, Yoav Weiss, Alex Forshtat, Dror Tirosh, Shahaf Nacson, Derek Chiang, Toni Wahrstätter, Stavros Vlachakis
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8141) · [discussion](https://ethereum-magicians.org/t/frame-transaction/27617)
- **Prior fork history:** Considered for Hegota at acde/233 (2026-03-26); no earlier fork entries

## TL;DR
A new EIP-2718 transaction type (`0x06`) that replaces ECDSA-bound validity with a sequence of "frames" — contract calls that validate the transaction, approve gas payment, and execute user operations. It delivers native account abstraction: arbitrary signature schemes (post-quantum path), gas sponsorship without relayers, native batching, and key rotation. This is the biggest single EL change proposed for Hegota and was a headliner candidate.

## What it changes
- **New tx type `FRAME_TX_TYPE = 0x06`**, RLP payload `[chain_id, nonce, sender, frames, signatures, fees, blob_versioned_hashes]`. Each frame is `[mode, flags, target, limits, value, data]`; `limits` is a two-dimensional `[execution, state]` gas budget (EIP-8037 state gas). Max 64 frames per tx.
- **Frame modes:** `DEFAULT` (caller = ENTRY_POINT `0xaa`), `VERIFY` (STATICCALL semantics, revert ⇒ whole tx invalid), `SENDER` (caller = tx.sender, requires prior execution approval). Optional `deploy`-style first frame can install sender code before validation. ATOMIC_BATCH flag groups consecutive non-VERIFY frames into all-or-nothing batches.
- **New opcode `APPROVE` (0xaa):** called by a VERIFY frame's resolved target; `APPROVE_PAYMENT` (0x1) sets the tx `payer`, collects `max_cost`, and increments the sender nonce; `APPROVE_EXECUTION` (0x2) sets `sender_approved`, unlocking SENDER frames. Approval scope must be declared in frame flags.
- **New introspection opcodes (frame txs only):** `TXPARAM` (0xb0, tx fields incl. canonical sig hash and `state_gas_left`), `FRAMEDATALOAD` (0xb1), `FRAMEDATACOPY` (0xb2), `FRAMEPARAM` (0xb3, per-frame fields + status/gas_used of completed frames), `SIGPARAM` (0xb4), `SIGDATACOPY` (0xb5, ARBITRARY sigs only).
- **Signatures:** outer `signatures` list with schemes SECP256K1, P256, ARBITRARY; protocol-validated pre-execution (ecrecover / P256VERIFY, charged 2800/6700 gas). Canonical sig hash elides raw bytes of empty-`msg` signatures (future aggregation). Default code lets plain EOAs use frame txs with one secp256k1 signature.
- **Receipts change:** `[cumulative_gas_used, payer, [frame_receipt, ...]]` with per-frame `status`/`gas_used [execution, state]`/logs; new status 0x2 for batch-skipped frames; no tx-level status.
- **Gas:** intrinsic 12000 + 475/frame + per-token data costs (EIP-7976) + sig verification + value costs; EIP-7825 tx cap applies to intrinsic + execution budgets; calldata floor (EIP-7623) compared per EIP-8037 rules with `gas_used = execution + state`. Blob support reuses EIP-4844/7594 wrapping.
- **Mempool policy (ERC-7562-derived, no staking/reputation):** validation prefix must match one of 4 recognized shapes (self-relay, deploy, canonical paymaster, deploy+paymaster); `MAX_VERIFY_GAS = 100k`, `MAX_VERIFY_STATE_GAS = 500k`; banned opcodes and storage restrictions during validation; canonical paymaster reservation accounting; one pending frame tx per sender. `EXPIRY_VERIFIER` predeploy at `0x8141` gives txs an expiry timestamp.
- **ORIGIN** returns the frame caller (ENTRY_POINT or sender), not the tx origin. EIP-3607 restriction lifted for frame txs (sender may have code).

## Motivation
Realizes native account abstraction: accounts become "address with code". Provides the off-ramp from ECDSA to post-quantum signature schemes, native key rotation, batching without wrapper contracts, and gas sponsorship / ERC-20 fee payment without ERC-4337's separate mempool, bundlers, and EntryPoint contract. Also simplifies smart accounts and hardens the protocol's lowest-common-denominator account model.

## Dependencies & related EIPs
- **Depends on:** EIP-8037 (two-dimensional gas — itself a Hegota candidate), EIP-7623/7976 (calldata floor), EIP-7825 (tx gas cap), EIP-7702 (delegation semantics), EIP-7708 (transfer logs), EIP-7778 (refund/block accounting), EIP-2780, EIP-4844, EIP-7594, EIP-3529, EIP-3607 (explicitly relaxed for frame txs). Also references EIP-7843 (SLOTNUM, Hegota candidate) in mempool banned-opcode rules and EIP-7997 (deterministic deployer) for the canonical deploy flow.
- **Amended by (in-cluster):** EIP-8250 (keyed nonces replace the `nonce` field), EIP-8272 (recent-root references appended to payload), EIP-7906 (adds POST_TX frame mode). All three are deltas against this spec.
- **Alternative-to / supersedes-in-spirit:** ERC-4337 infrastructure and EIP-7701-style AA proposals; competes with "EVMification" approaches (EIP-8200, also Hegota candidate) as a different path to similar goals. Builds on EIP-7702 (Pectra) as a stepping stone.
- **EIP-8151 (in-cluster) interacts:** ecRecover restriction matters most for accounts migrated away from ECDSA, which frame txs enable.

## Impact on ethrex / client teams
Large — the biggest item in this cluster. Touches: tx types/decoding (`crates/common/types/transaction.rs`), EVM interpreter (new opcodes; `crates/vm/levm/src/opcode_handlers/frame_tx.rs` already exists with 717 lines, plus ~4k lines of eip8141 tests — this repo already has a devnet implementation), block validation/execution (frame loop, atomic-batch journaling, two-pool gas accounting, new receipt encoding and receipts root), tx pool (validation-prefix simulation, banned-opcode tracing, paymaster reservations, one-pending-per-sender, revalidation indexing), networking (receipt encoding in `Receipts` message, EIP-7594-style blob sidecar wrapping for frame txs), Engine API/RPC (new receipt shape, payer field, per-frame statuses; gas estimation becomes two-dimensional). Note the local implementation still has `FRAME_TX_INTRINSIC_COST = 15000` while the spec now says 12000 — the spec is actively drifting, so keeping pace is real ongoing cost. Effort: **large**, though partially done.

## Open questions & controversies
- Spec is Draft and churning (constants like intrinsic cost changed between the devnet implementation and current spec; signature-hash and paymaster details have moved repeatedly).
- Public mempool DoS surface is the main concern: validation is arbitrary EVM code, so ERC-7562-style trace rules, paymaster reservation accounting, and dependency revalidation are consensus-adjacent policy that all clients must implement compatibly.
- ORIGIN semantic change and EIP-3607 relaxation touch long-standing invariants; some tooling assumes `tx.origin == sender`.
- New receipt format breaks explorers/indexers; no tx-level status is a UX regression for tooling.
- Was a headliner candidate but not selected as headliner; still scheduled SFI. Debate continues over shipping it vs. smaller AA increments, and interaction with EIP-8037 (state gas) means it cannot ship alone if 8037 is dropped.
