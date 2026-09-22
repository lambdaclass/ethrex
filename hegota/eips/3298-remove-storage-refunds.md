# EIP-3298: Remove storage-clear refund and refund cap
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/243, 2026-08-13
- **Authors:** Vitalik Buterin, Martin Swende, Jochem Brouwer
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-3298) · [discussion](https://ethereum-magicians.org/t/eip-3298-removal-of-refunds/5430)
- **Prior fork history:** none recorded in forkcast; originally drafted in 2021 (pre-London, rejected in favor of EIP-3529's partial reduction) and since rewritten as a delta against the Glamsterdam gas EIPs

## TL;DR
Removes the last cross-transaction gas refund: the SSTORE storage-clearing refund (`STORAGE_CLEAR_REFUND`, 11,616 under EIP-8038). Also removes the EIP-3529 refund cap (max 20% of gas used), which becomes unnecessary once the only remaining refund — the same-transaction write-reversal — is self-bounded. Net effect: clearing storage becomes a pure cost, killing GasToken-style gas banking for good.

## What it changes
- Deletes `STORAGE_CLEAR_REFUND` from the gas schedule and the two EIP-8038 refund rules that use it (grant on clearing a non-zero original slot, reversal when such a cleared slot is restored).
- Keeps exactly one refund rule (unchanged from EIP-8038/EIP-2200): `STORAGE_WRITE` is refunded when a slot is restored to its original (start-of-transaction) value — genuine net metering within a transaction.
- Removes the EIP-3529 cap from end-of-transaction settlement: `tx_gas_used = max(tx_gas_used_before_refund - refund_counter, calldata_floor_gas_cost)` instead of capping the refund at `tx_gas_used_before_refund // 5`.
- Refund counter mechanics otherwise unchanged: journaled per call frame (reverting frames discard their additions), applied once at settlement. The EIP-7623/7976 calldata floor still binds after refunds.
- Block-level accounting (EIP-7778 / EIP-8037) untouched: execution-gas dimension already counts pre-refund gas.
- Spec is explicitly a delta against EIP-8037 + EIP-8038 and assumes EIP-2780 and EIP-7778 are active.

## Motivation
Refunds exist to incentivize state hygiene, but the incentive never worked — it was abused to stockpile cheap gas (GasToken) instead. State growth is now priced at creation time (EIP-8037), making cleanup rebates obsolete. The remaining costs of the clearing refund are settlement complexity: a refund counter, a cap with non-obvious interactions with the calldata floor and EIP-8037's two gas dimensions, and spec/implementation burden for a parameter too small to function as either incentive or gas bank. The cap becomes safe to drop because the surviving write-reversal refund can never exceed the STORAGE_WRITE charges already paid in the same transaction.

## Dependencies & related EIPs
- **Depends on:** EIP-8037 (state gas) and EIP-8038 (gas schedule rebase) — both Glamsterdam; the spec is written as a delta against them. Also assumes EIP-2780 and EIP-7778.
- **Builds on:** EIP-3529 (London) and EIP-2200 net-metering lineage.
- **Synergizes with:** EIP-8358 (net gas metering for accounts) — both lean on the same journaled refund counter; 8358 adds `BALANCE_RESET_REFUND` while 3298 removes the cap, so their interaction (8358's refunds uncapped) should be checked if both land. EIP-8279 also touches refund semantics (BAL byte refunds keyed to no-op writes).
- **Cluster context:** this is the storage-refund-removal entry of the gas repricing cluster.

## Impact on ethrex / client teams
- Small. SSTORE refund logic lives in `crates/vm/levm/src/opcode_handlers/stack_memory_storage_flow.rs` and the refund counter in `crates/vm/levm/src/call_frame.rs` / `crates/vm/levm/src/vm.rs`; settlement is in the tx finalize path (`crates/vm/backends/levm/mod.rs`, receipt `cumulative_gas_used`).
- Work: delete the clear-refund rules from the SSTORE case table, drop the `// 5` cap from settlement, keep the write-reversal rule. Most effort is fork-gating and updating the (already extensive) refund tests under `test/tests/levm/`.
- RPC impact: `eth_estimateGas` must drop clearing-refund and cap logic. No tx pool, networking, Engine API, or state/trie changes.

## Open questions & controversies
- Removes any incentive to clear storage; state growth cleanup is left entirely to future statelessness/state-expiry work. Some see this as giving up on hygiene incentives entirely.
- Interaction with EIP-8358's new refunds if both ship: 8358's spec is written against the EIP-3529 cap as defense-in-depth; under 3298 that cap is gone, so refund-soundness arguments must be re-checked against 3298's self-boundedness requirement (any new refund not backed by a same-transaction charge would need the cap reintroduced).
- Spec maturity is decent (full test table included) but it is a 2021 EIP repurposed for the EIP-8037/8038 world — reviewers should confirm no stale pre-Glamsterdam assumptions remain.
