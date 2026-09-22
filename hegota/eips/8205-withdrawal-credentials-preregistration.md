# EIP-8205: Withdrawal credentials preregistration
- **Layer:** CL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — presented at acdc/177, 2026-04-16 (champions: George Avsetsin, Greg Koumoutsos)
- **Authors:** George Avsetsin, Dmitry Gusakov, Greg Koumoutsos, Eugene Mamin
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8205) · [discussion](https://ethereum-magicians.org/t/eip-8205-withdrawal-credentials-preregistration/28084)
- **Prior fork history:** none

## TL;DR
Fixes the deposit front-running vulnerability in delegated staking: a node operator holding the BLS key can deposit 1 ETH first with its own withdrawal credentials and hijack the staking protocol's much larger deposit (first-deposit-wins). This EIP lets the key holder preregister — via an EL system contract with a BLS signature — which withdrawal credentials a pubkey must use; the CL then silently rejects any mismatched deposit. Fully opt-in; deposits without preregistration are unchanged.

## What it changes
- New EIP-7685 request type (TBD) carrying `pubkey` (48B) ++ `withdrawal_credentials` (32B) ++ `signature` (96B) = 176-byte records, from a new EL system contract (address TBD) following the EIP-7002/7251 queue + exponential-fee + end-of-block system-call pattern (target 1/block, max 4 dequeued/payload).
- CL: new `validator_preregistrations` list in `BeaconState` (cap `PREREGISTRATIONS_LIMIT = 2^19`, worst case ~46 MB); records expire after `PREREGISTRATION_EXPIRY_SLOTS = 2^18` (~36 days) and are garbage-collected in epoch processing.
- BLS signature verified on CL only, under new `DOMAIN_PREREGISTRATION` (0x11000000) using `GENESIS_FORK_VERSION` so signatures are fork-agnostic (valid across all forks; chain separation via `genesis_validators_root`).
- `process_deposit_request` (EIP-6110) modified: with an active preregistration, a deposit is accepted only if withdrawal credentials match AND the deposit BLS signature is valid (prevents invalid-sig griefing from consuming the preregistration); mismatched deposits are silently rejected and the ETH is burned in the deposit contract — permanent rejection makes front-running economically self-defeating.
- Requests are applied from `parent_execution_requests` (EIP-7732 ePBS: a payload's requests are carried in the next block), after deposit requests; expiry is evaluated against the parent payload's slot so a deposit is checked against bindings active when its payload was created.
- Staking protocols verify preregistration on-chain via EIP-4788 proofs (plus EIP-7843 SLOTNUM to check the absolute `expiry_slot`), possibly atomically with the deposit tx.
- A system call with non-empty calldata disables the queue (EXCESS_INHIBITOR) — reserved for a future upgrade; unused under this EIP.

## Motivation
At least a third of staked ETH flows through delegated architectures (liquid staking, SaaS). Every such protocol has independently built application-layer defenses (bond-based pre-deposits, guardian committees) against deposit front-running. A single protocol-level binding removes that duplicated infrastructure, locked capital, and off-chain trust assumptions.

## Dependencies & related EIPs
- Requires EIP-6110 (in-protocol deposits, Pectra), EIP-7685 (requests framework, Pectra), and EIP-7732 (ePBS — the spec is written against `parent_execution_requests` from Glamsterdam).
- Reuses the system-contract pattern of EIP-7002/7251; verification flows rely on EIP-4788 (beacon root) and EIP-7843 (SLOTNUM opcode).
- Cluster relations: same author group as EIP-8148 (custom sweep threshold) — both add EL request types; EIP-8365 adopts the same silent permanent-rejection semantics for 0x00 deposits, explicitly citing 8205. Mentions EIP-8282 builder deposits (builder deposits bypass preregistration).

## Impact on ethrex / client teams
- Medium for ethrex. EL work mirrors EIP-8148: new system contract in `crates/vm/system_contracts.rs`, end-of-block system call in `crates/vm/backends/levm/mod.rs`, request extraction and ordering into the EIP-7685 requests list (block validation + `crates/blockchain/payload.rs`), `requestsHash` commitment changes.
- The system-call "disable on non-empty calldata" path must be implemented exactly (currently unreachable but consensus-relevant).
- BLS verification, state list, expiry, and deposit enforcement are all CL-side; ethrex needs none of that.
- Contract bytecode and deployment address are TBD — cannot be finalized until those land.

## Open questions & controversies
- Draft maturity: predeploy address, request type number, bytecode, test vectors, and reference implementation all TBD.
- The permanent-burn semantics for mismatched deposits is deliberate but harsh — accidental mismatch loses funds irrecoverably; mitigated by BLS signing step + EIP-4788 verification flow, but relies on protocols integrating correctly.
- Race caveats: if an attacker's deposit lands before the preregistration, first-deposit-wins still applies — protocols must verify preregistration atomically (or after finality), which adds integration complexity.
- Preregistration signatures never expire cryptographically (only the on-chain record does); a leaked signed message can re-establish the binding forever — operators must treat them as permanent commitments.
- Depends on ePBS (EIP-7732) having shipped, tying its schedule to Glamsterdam.
