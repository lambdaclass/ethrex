# EIP-7686: Linear EVM memory limits
- **Layer:** EL
- **EIP status:** Stagnant
- **Hegota status:** none recorded (no Hegota entry in forkcast metadata)
- **Authors:** Vitalik Buterin
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7686) · [discussion](https://ethereum-magicians.org/t/eip-7686-linear-evm-memory-limits/19448)
- **Prior fork history:** Declined from Glamsterdam (acde/226, 2025-12-18)

## TL;DR
Removes the quadratic term from EVM memory expansion cost and replaces it with a hard rule: memory can never exceed the current call's initial gas. Combined with a modified call-gas cap (a child call gets at most `gas - max(gas/64, memory_used)`), an N-gas transaction can use at most N bytes of memory — a simple, tight, linear bound. Note: the EIP is Stagnant and currently has no recorded Hegota proposal, though it competes with EIP-7923, which does.

## What it changes
- **Memory cost:** `memory_cost = 3 * words` (linear only); the `words² / 512` quadratic term is deleted.
- **Hard per-context memory cap:** any memory expansion that would make `memory_byte_size` exceed the current call's *initial* gas limit reverts.
- **Call-gas rule:** `max_call_gas(gas, memory_byte_size) = gas - max(gas // 64, memory_byte_size)` — replaces the pure EIP-150 63/64 rule, so memory already used in the parent reduces gas forwardable to the child.
- Net invariant: transaction with N gas uses ≤ N bytes of memory total, and the bound is tight (an N-gas call can use `N - O(1)` bytes).
- The 63/64 component is retained to preserve today's ~537 de-facto call depth limit; 3 gas/word is retained because it equals `MCOPY` pricing, so clearing child-call memory costs the same as an `MCOPY`.

## Motivation
Today's memory pricing (quadratic expansion + 63/64 rule) makes it extremely hard to compute the maximum memory an EVM execution can consume — it requires an optimization over the call tree. A clean linear invariant simplifies client implementations, especially constrained ones like ZK-SNARK provers, and makes worst-case resource usage trivially predictable for operators.

## Dependencies & related EIPs
- Modifies the **EIP-150** 63/64 rule (adds a memory term to the call-gas cap).
- **Direct alternative to EIP-7923** (page-based linear memory costing): both linearize memory pricing and bound per-transaction memory; 7686 ties the limit to gas (no fixed cap), 7923 introduces 4 KB pages and a hard 64 MiB transaction-global cap. Both were declined from Glamsterdam in the same call (acde/226); only 7923 has been re-proposed for Hegota (acde/243, 2026-08-13).
- Interacts with any gas-limit increase: 7686's bound scales automatically with the gas limit, whereas 7923's 64 MiB cap is fixed.

## Impact on ethrex / client teams
- **EVM interpreter (LEVM):** replace the quadratic expansion formula in `crates/vm/levm/src/memory.rs:491` (`words * words / MEMORY_EXPANSION_QUOTIENT + 3 * words`, quotient 512 in `crates/vm/levm/src/constants.rs:18`); add the memory-vs-gas check on every expansion site in `crates/vm/levm/src/gas_cost.rs` (roughly ten call sites: `MLOAD`/`MSTORE` family, `CALL*` family, `CREATE*`, `RETURN`/`REVERT`, copies); thread `memory_byte_size` into the call-gas computation for `CALL`/`DELEGATECALL`/etc. in `gas_cost.rs`/`opcode_handlers/system.rs`.
- **RPC/tooling:** `eth_estimateGas` inherits behavior change automatically; memory-heavy `eth_call` semantics change near limits.
- No tx-pool, networking, state, or Engine API changes.
- Effort: **small-to-medium** — mechanically simple, but the backward-compat tail (contracts that access high memory under low gas now fail) needs testing.

## Open questions & controversies
- EIP status is **Stagnant** and it was declined from Glamsterdam — momentum sits with EIP-7923 instead; shipping both would be redundant.
- Backwards compatibility: code accessing high memory offsets under a low gas limit can break, though analysis suggests almost all real applications fit.
- The memory cap is gas-relative, not absolute — some client teams prefer a hard byte cap (as in 7923) for DoS planning, especially for RPC nodes running `eth_call` with inflated gas limits.
