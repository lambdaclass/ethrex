# EIP-8015: Remove `deposit` and `eth1data` fields
- **Layer:** CL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acdc/183, 2026-07-23 (champion: Etan Kissling)
- **Authors:** Terence, Etan Kissling
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8015) · [discussion](https://ethereum-magicians.org/t/eip-8015-remove-legacy-deposit-and-eth1data-fields/25401)
- **Prior fork history:** none

## TL;DR
Deletes the legacy `eth1_data` and `deposits` fields from `BeaconBlockBody` and the `eth1_data`, `eth1_data_votes`, `eth1_deposit_index`, and `deposit_requests_start_index` fields from `BeaconState`, once EIP-6110 in-protocol deposits have fully finalized. Also removes the eth1 voting machinery proposers still run. Pure consensus-layer cleanup of the pre-EIP-6110 deposit pipeline.

## What it changes
- `BeaconBlockBody` loses `eth1_data` and `deposits`; expressed via EIP-7688 ProgressiveContainer `active_fields` bits, so generalized indices of remaining fields are unchanged.
- `BeaconState` loses `eth1_data`, `eth1_data_votes`, `eth1_deposit_index`, `deposit_requests_start_index`.
- Block processing: removes `process_eth1_data()` and the `len(body.deposits) == 0` assertion.
- Epoch processing: removes `process_eth1_data_reset()`.
- Honest validator spec: proposers stop polling the execution chain for eth1 votes.
- Activation condition: only when `state.eth1_deposit_index == state.deposit_requests_start_index` (EIP-6110 fully finalized) AND Glamsterdam (EIP-7773) is in effect.
- Historical blocks/states keep old schemas; clients must retain both schemas for historical data.

## Motivation
EIP-6110 moved deposits into the EL requests framework (EIP-7685), making CL proposer deposit voting obsolete. The transition coexists until finalization completes; after that, the fields are permanently unused but still carried in every block and state. Removing them deletes dead validation logic and technical debt, and stops proposers polling the EL for votes nobody consumes.

## Dependencies & related EIPs
- Requires EIP-6110 (in-protocol deposits, shipped in Pectra), EIP-7688 (progressive containers — the mechanism that makes field removal clean), EIP-7773 (Glamsterdam meta — i.e. can only ship after Glamsterdam).
- The spec's BeaconBlockBody already assumes ePBS (EIP-7732, `signed_execution_payload_bid`, `parent_execution_requests`) — i.e. it is written against the post-Glamsterdam container.
- Cluster relations: complementary cleanup with EIP-7668 (bloom removal) on the EL side; the staking EIPs (8148, 8205) extend the ExecutionRequests container that 8015's spec text already includes.

## Impact on ethrex / client teams
- None/minimal for ethrex as an EL client (CL-only). Block body and state containers are CL structures; validation of deposits and eth1 voting is CL logic.
- Indirect touchpoint: proposers stopping eth1 polling slightly changes CL→EL RPC traffic patterns, but no EL code change.
- For CL teams: medium effort — new fork containers, transition logic, dual-schema historical handling.

## Open questions & controversies
- Hard dependency chain: requires Glamsterdam (ePBS) to have shipped, so this cannot be in any fork before it; Hegota timing depends on Glamsterdam landing cleanly.
- EIP-7688 progressive containers must be live for the removal mechanism to work as specified.
- No known pushback — uncontroversial cleanup once its preconditions hold; the main risk is scheduling (activation condition must be verifiable on mainnet before the fork).
