# EIP-7979: Call and Return Opcodes for the EVM
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acde/238, 2026-06-04)
- **Authors:** Greg Colvin, Martin Holst Swende, Brooklyn Zelenka, John Max Skaller
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7979) · [discussion](https://ethereum-magicians.org/t/eip-7979-call-and-return-opcodes-for-the-evm/24615)
- **Prior fork history:** Declined from Glamsterdam (acde/225, 2025-12-04)

## TL;DR
Adds three control-flow instructions — `CALLSUB`, `CALLDEST`, `RETURNSUB` — so contracts can call and return from subroutines without synthesizing them out of dynamic `JUMP`s. This makes EVM control flow statically analyzable in linear time, unlocking cheaper validation, AOT/JIT compilation, and much cheaper zkVM proving. Backwards compatible: existing bytecode is untouched.

## What it changes
- New machine state: a **return stack** of return addresses (max 1024 entries), pushable only by `CALLSUB`, poppable only by `RETURNSUB`, invisible to EVM code otherwise.
- `CALLSUB` (gas 8 / `mid`): pops destination from the data stack, pushes `PC+1` onto the return stack, jumps. Exceptional halt if the destination is not a `CALLDEST` or the return stack is full.
- `CALLDEST` (gas 1 / `jumpdest`): no-op entry label, like `JUMPDEST`. Also a valid `JUMP`/`JUMPI` destination — jumping to it enters the subroutine without pushing a return address, enabling tail-call elimination.
- `RETURNSUB` (gas 5 / `low`): pops the return stack into `PC`. Exceptional halt if empty.
- JUMPDEST analysis gains a second set (`valid_call_destinations`); every `CALLDEST` is in both sets. One extra linear pass, same shape as today's analysis.
- Opcode values are **TBD** — test cases use placeholders `0xB0`/`0xB1`/`0xB2`.
- Reference implementation against EELS provided; spec includes concrete bytecode traces and gas totals.

## Motivation
Synthesizing calls/returns via dynamic `JUMP` makes control flow data-dependent, so static analysis can take quadratic or exponential time — a DoS hazard for any online analysis (deploy-time validation, AOT/JIT compilation). With static call structure, clients can validate code linearly, translate bytecode to register IR or machine code in one pass, and zkVM provers pay far fewer RISC-V instructions (the spec cites 5x–51x cheaper proving vs. interpreted EVM in Zisk benchmarks). Also modest direct savings: ~29% fewer bytes and ~32% less gas on a simple call example.

## Dependencies & related EIPs
- Motivated by **EIP-8173** ("Foundations of EVM Control Flow"), which lays out the problem statement.
- Independent of, and an alternative lineage to, **EOF** (EIP-3540/4200/4750 family): this EIP deliberately takes the minimal route — no code sections, no immediates — whereas EOF went the structural route and was repeatedly deferred.
- Spec cites composability with proposed **64-bit arithmetic opcodes** (gains compound: up to 51x AOT+RISC-V).
- Practical interaction with **EIP-8141** (frame transactions, also a Hegota candidate): ethrex's Hegota opcode table already assigns `0xB0`–`0xB4` to 8141's frame opcodes, the same bytes 7979's test vectors use as placeholders — final opcode assignment must avoid the clash.

## Impact on ethrex / client teams
- **EVM interpreter (LEVM):** add three `OpcodeHandler`s and register them in the per-fork tables in `crates/vm/levm/src/opcodes.rs`; add a `return_stack` field to VM state (`crates/vm/levm/src/vm.rs`); extend jump-destination analysis to also collect CALLDESTs. New exceptional-halt errors for return-stack overflow/underflow.
- **revm backend** (`crates/vm/backends/`): would need revm support; check revm's roadmap if the backend is used in consensus paths.
- **Gas accounting:** three new fixed-cost opcodes, no formula changes.
- Nothing touches tx pool, block validation outside the EVM, state, networking, Engine API, or RPC (except tracers may want to render the new opcodes).
- Effort: **small-to-medium** for the interpreter itself; the larger payoff work (AOT/JIT, validation) is optional follow-up.

## Open questions & controversies
- Opcode byte values not yet assigned (placeholder `0xB0`–`0xB2` conflict with ethrex's existing Hegota EIP-8141 opcodes).
- Perennial "EOF vs. minimal opcodes" debate: some client teams have preferred the full EOF redesign over piecemeal control-flow additions; that fatigue likely contributed to the Glamsterdam decline.
- Gas costs (8/1/5) are unbenchmarked; the spec itself says benchmarking is needed.
- Benefits are mostly indirect (tooling, zk, future compilers) — little immediate user-visible change, which weakens urgency arguments in fork scoping.
