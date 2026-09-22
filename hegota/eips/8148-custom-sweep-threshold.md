# EIP-8148: Custom sweep threshold for validators
- **Layer:** CL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — presented at acdc/179, 2026-05-28 (champions: Dmitry Gusakov, Greg Koumoutsos)
- **Authors:** Dmitry Gusakov, Dmitry Chernukhin, Greg Koumoutsos, Manu
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8148) · [discussion](https://ethereum-magicians.org/t/eip-8148-custom-sweep-threshold-for-validators/27669)
- **Prior fork history:** none

## TL;DR
Lets validators with compounding (0x02) withdrawal credentials set a custom balance threshold above which excess rewards are automatically swept to their withdrawal address, instead of waiting for the 2,048 ETH max effective balance. The goal is to make 0x02 migration attractive to solo stakers and operators who currently stay on 0x01 for liquidity reasons — effectively allowing validator "size" anywhere from 32 to 2,048 ETH.

## What it changes
- New EIP-7685 request type `SET_SWEEP_THRESHOLD_REQUEST_TYPE = 0x03` with fields `source_address`, `validator_pubkey`, `threshold` (uint64), produced by a new EL system contract (address TBD).
- The system contract follows the exact EIP-7002/7251 pattern: in-contract FIFO queue (3 slots per entry), EIP-1559-style fee via `fake_exponential` (target 2/block, max dequeued 16/block), end-of-block system call from `SYSTEM_ADDRESS` (0xffff…fffe) with a dedicated 30M gas limit excluded from block gas accounting; missing code or a failing call invalidates the block.
- CL: new `validator_sweep_thresholds` mapping in `BeaconState` (deliberately not added to the phase-0 `Validator` container); new `process_set_sweep_threshold_request` applied from execution requests; sweep logic (`is_partially_withdrawable_validator`, `get_validators_sweep_withdrawals`) uses the custom threshold; `process_effective_balance_updates` caps effective balance at the threshold.
- Threshold must be ≥ current balance (prevents using it for instant withdrawals — partial withdrawals stay the path for that) and a multiple of 1 ETH (`EFFECTIVE_BALANCE_INCREMENT`).
- Initial threshold can be encoded in bytes 10–11 of 0x02 withdrawal credentials at deposit time; existing 0x02 validators default to `MAX_EFFECTIVE_BALANCE_ELECTRA` (2,048 ETH); 0x00/0x01 validators get threshold 0.
- Requests are processed immediately on dequeue (no CL-side queue, unlike partial withdrawals).

## Motivation
Migration to 0x02 compounding credentials has been very slow: stakers don't want rewards locked until a 2,048 ETH balance, and manual partial withdrawals require user transactions plus an unpredictable wait in the shared exit queue. A custom sweep threshold gives automatic, predictable reward liquidity at whatever size the staker chooses, and is also groundwork for future MAX_EB changes or removal.

## Dependencies & related EIPs
- Requires EIP-7251 (0x02 compounding credentials / MAX_EB increase, Pectra) and EIP-7685 (execution requests framework, Pectra).
- Same system-contract pattern as EIP-7002 (triggerable withdrawals) and EIP-7251 consolidation requests.
- Cluster relations: same author group as EIP-8205 (preregistration) — both add EL request types for staking UX; EIP-8205 also modifies `process_deposit_request`, no direct conflict. EIP-8148's threshold interacts with the balance machinery that EIP-8365/8367 only touch for 0x00 validators — no overlap.

## Impact on ethrex / client teams
- Medium for ethrex. EL work: add the new system contract to `crates/vm/system_contracts.rs` (ethrex already implements the 7002/7251 pattern there), execute the end-of-block system call in `crates/vm/backends/levm/mod.rs`, extract the new request type into the requests list (block validation + payload building in `crates/blockchain/payload.rs`), and include it in the `requestsHash` commitment.
- New request type touches Engine API payload validation surface and the `ExecutionRequests` encoding, plus devnet testing via `test/tests/levm/requests_*` patterns.
- The heavy logic (sweep predicate, state mapping) is CL-side; ethrex only needs correct request plumbing and system-call semantics.
- Note the contract bytecode is provided in the spec (geas/sys-asm), but the predeploy address is still TBD.

## Open questions & controversies
- Draft maturity: `SET_SWEEP_THRESHOLD_REQUEST_PREDEPLOY_ADDRESS`, deployment tx, and full CL spec (lives in `specs/_features/eip8148/`) are TBD.
- Encoding an initial threshold into bytes 10–11 of withdrawal credentials gives previously-meaningless bytes consensus meaning — backwards-compatible in practice but a subtle new interpretation of a published field.
- Some debate on whether this is worth new EL surface (system contract + request type) for a staking-UX improvement, vs. waiting for simpler sweep-cycle improvements; also interacts with any future MAX_EB redesign.
