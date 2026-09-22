# EIP-8219: Checked Arithmetic Opcodes
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acde/243, 2026-08-13)
- **Authors:** Hubert Ritzdorf
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8219) · [discussion](https://ethereum-magicians.org/t/eip-8219-checked-arithmetic-opcodes/27913)
- **Prior fork history:** none

## TL;DR
Adds four opcodes — `SAFEADD`, `SAFESUB`, `SAFEMUL`, `SAFEDIV` — that do unsigned 256-bit arithmetic and *revert the current frame* on overflow, underflow, or division by zero, in one instruction. Solidity and Vyper emit multi-instruction overflow checks around every arithmetic op today; a single `SAFEADD` cuts a checked add from ~79 gas / 12 bytes to 5 gas / 1 byte.

## What it changes
- New opcodes in the unassigned arithmetic range `0x0c`–`0x0f` (right after `ADD`–`SIGNEXTEND`, `0x01`–`0x0b`):
  - `SAFEADD (0x0c)` — gas 5; reverts if `a + b > 2^256 - 1`.
  - `SAFESUB (0x0d)` — gas 5; reverts if `b > a` (stack order matches `SUB`).
  - `SAFEMUL (0x0e)` — gas 7; reverts on overflow (`a==0` yields 0, no revert).
  - `SAFEDIV (0x0f)` — gas 7; reverts on division by zero.
- Error behavior: **revert the current call frame with empty returndata** — semantically `REVERT(0, 0)`, *not* an exceptional halt, so remaining gas is kept (important for slippage-guard-style expected reverts).
- Existing `ADD`/`SUB`/`MUL`/`DIV` semantics unchanged; compilers opt in by targeting the post-fork EVM.
- Gas pricing: base op cost + 2 gas premium for the check (5 = `verylow`+2, 7 = `low`+2).
- Benchmarks cited: ~94% (Solidity 0.8.33) and ~88% (Vyper 0.4.3) execution-gas reduction per checked add; deployment gas drops from ~2,352 to ~200 per checked add.

## Motivation
Checked arithmetic is the default in Solidity (≥0.8) and Vyper, but the EVM has no native support, so compilers pay a ~25x gas multiplier per operation. That cost creates an incentive to use `unchecked {}` / `unsafe_*` even when safety isn't proven — a source of real exploits. At 5 gas vs. 3 gas, there is no longer a meaningful reason to opt out of overflow protection.

## Dependencies & related EIPs
- **Alternative to** prior flag-based proposals **EIP-1051** (overflow flags + `OFV`/`SOVF`) and **EIP-6888** (`carry`/`overflow` flags + `JUMPC`/`JUMPO`) — this EIP explicitly rejects implicit flag state in favor of stateless self-contained opcodes.
- Signed variants (`SSAFEADD`, etc.) deferred to a **companion EIP** (unnumbered).
- No hard dependency on other Hegota candidates. Independent of the cluster's repricing EIPs (7686/7923/8131).
- Minor synergy with **EIP-7979** (call/return opcodes): both shrink compiler-emitted boilerplate; 8219's `REVERT(0,0)`-style errors avoid needing return-data plumbing.

## Impact on ethrex / client teams
- **EVM interpreter (LEVM):** four new `OpcodeHandler`s in `crates/vm/levm/src/opcode_handlers/arithmetic.rs`, enum entries and `const` table assignments at `0x0c`–`0x0f` in `crates/vm/levm/src/opcodes.rs` (currently unassigned — free slots, no conflict with the Hegota table's 8141 opcodes at `0xAA`/`0xB0`–`0xB4`), gated into `build_opcode_table_hegota()`. Revert-not-halt semantics need care: the handler must trigger frame revert, not `VMError` exceptional halt.
- **revm backend:** needs upstream revm support if used in consensus paths.
- Nothing else touched: no gas formula changes, no tx-pool/block-validation/RPC impact. Tracers/disassemblers may want names.
- Effort: **small**. The opcodes are simple `U256` checked ops already available in Rust (`checked_add` etc.).

## Open questions & controversies
- Very new (created 2026-04, Draft); spec is fresh and unreviewed by most client teams — costs and edge cases not yet battle-tested.
- Spends **four** of the remaining unassigned opcode bytes at once; opcode-space scarcity arguments apply (contrast with 8163's one-byte reservation ethos).
- Reverts with empty returndata, so Solidity loses its `Panic(0x11)` payload — a UX/debugging regression traded for gas.
- Unsigned-only scope; signed and smaller-width types out of scope, limiting coverage.
- Unclear demand vs. the alternative of compilers emitting better check sequences; requires Solidity/Vyper to actually adopt.
