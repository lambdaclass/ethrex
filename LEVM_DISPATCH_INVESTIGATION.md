# levm dispatch overhead analysis (ISZERO microbenchmark gap vs revm)

> Investigation context: on the BAL compute benchmark, `test_comparison.py::test_iszero` (a 3-gas, stack-neutral op, so the loop is essentially pure interpreter dispatch) runs at ethrex 1528 Mgas/s (~1.96 ns/op) vs reth/revm 5841 Mgas/s (~0.51 ns/op) — ethrex rank 5/6, ~0.26x of reth. ethrex's non-trivial comparison ops (LT/GT/EQ) run 3000–4458 Mgas/s (~0.7–1.0x of reth, competitive). The gap is the fixed per-opcode dispatch tax a trivial op cannot amortize.

## 1. The exact dispatch hot path

One iteration of the interpreter loop, in execution order:

| Step | Location | Cost class |
|---|---|---|
| Loop body | `crates/vm/levm/src/vm.rs:1014-1129` (`run_execution`, vm.rs:953) | — |
| Capture `pc_of_current_op` | vm.rs:1016 | load (tracer-only use) |
| Opcode fetch `next_opcode()` | `crates/vm/levm/src/call_frame.rs:402-411` | load `pc`, load `Bytes` len+ptr, **bounds branch** (`pc < len`, else synthesize STOP), load byte |
| PC advance `advance_pc(1)?` | `crates/vm/levm/src/call_frame.rs:537-544`, called at vm.rs:1018 | `checked_add` **branch** + `Result<(), VMError>` + `?` **branch**, store `pc` |
| Tracer gate (pre) | vm.rs:1021-1070 | load `opcode_tracer.active` + **branch** |
| Table lookup + indirect call | vm.rs:1077, table at `crates/vm/levm/src/opcodes.rs:376, 411-431` (`&'static [OpCodeFn; 256]`, const-built per fork) | load fn ptr, indirect call |
| Handler wrapper | `opcodes.rs:392-400` (`wrap::<T>` calls inlined `T::eval`, converts `Result<OpcodeResult, VMError>` -> `OpcodeResult` + `OnceCell<VMError>`) | **branch** on Result |
| Gas accounting (inside handler) | `call_frame.rs:421-429` (`increase_consumed_gas`: `gas_remaining: i64 -= gas; if < 0`) | sub + **branch** |
| Stack ops (ISZERO) | `call_frame.rs:67-77` (`pop1`: `get(offset)` bounds **branch**), `call_frame.rs:81-106` (`push`: `checked_sub` **branch**) | 2 branches, offset bumped up then back down |
| Tracer gate (post) | vm.rs:1087-1108 | reload `active` + **branch** |
| Result match | vm.rs:1110-1116 | **branch** (`Continue => continue`) |

What levm already does right (parity with revm): a flat `fn`-pointer table (not trait objects, not a `match`), const-built per fork and shared as `&'static` (opcodes.rs:411-431); signed-i64 gas with a single sub+sign-check (same cost class as revm's `gas!`); a flat preallocated 1024-slot downward-growing stack (call_frame.rs:33-36); `current_call_frame` stored by value in `VM` (vm.rs:467), no pointer chase; fused JUMP->JUMPDEST (stack_memory_storage_flow.rs:424-432). **Dispatch mechanism and per-op gas are not the gap.**

## 2. Ranked root causes for the ~1.45 ns/op gap (~7 cycles @ ~4.8 GHz)

Because every handler takes `&mut VM` through an opaque fn pointer, the compiler must reload `pc`, the bytecode `Bytes` ptr/len, and `tracer.active` from memory every iteration. The per-iteration critical chain is what differs from revm:

- revm fetch chain: load instruction ptr -> load byte -> load fn ptr -> indirect call (revm pads bytecode with STOP bytes and keeps a raw instruction pointer; fetch is an unchecked deref, advance is a pointer add, no branches, no Result).
- levm fetch chain: load `pc` -> load `bytecode.bytecode.len` -> compare/branch (call_frame.rs:403) -> load `Bytes` ptr -> load byte (call_frame.rs:406) -> reload `pc`, `checked_add` branch, store `pc`, `?` branch (call_frame.rs:537-544 + vm.rs:1018) -> index table -> load fn ptr -> call.

**RC1 — PC/fetch machinery (largest, est. ~40-50% of gap).** ~3 extra dependent loads, 2-3 extra branches, and a `Result<(), VMError>` return per op vs revm's two instructions. All branches are predicted, but they consume fetch/retire bandwidth and lengthen the loop's dependency chain, which is exactly what caps a 2.5-cycle/iter loop at 9 cycles. Evidence: call_frame.rs:402-411, call_frame.rs:537-544, vm.rs:1016-1018.

**RC2 — Tracer code resident in the hot loop (est. ~15-25%).** `opcode_tracer.active` is reloaded (can't be cached across the opaque handler call) and branched on twice per op (vm.rs:1021-1024, 1087). The ~50-line pre-step capture block (vm.rs:1024-1070) and post-step block (vm.rs:1087-1108) are inline in the loop body, bloating the hot code footprint. `active` is fixed for the lifetime of the VM, so this is hoistable out of the loop entirely.

**RC3 — Error plumbing: large `VMError` + `Result` + `OnceCell` (est. ~10-15%).** Every handler builds `Result<OpcodeResult, VMError>` where `VMError` (`crates/vm/levm/src/errors.rs:11-20`) carries variants with two `U256`s and `String`s (~80 bytes); `wrap` (opcodes.rs:392-400) branches on it and conditionally writes the `OnceCell`. `eval` is `#[inline(always)]` into `wrap` so LLVM usually reduces the Ok path to a flag, but the multiple `?` edges per handler (gas, pop, push each return distinct error types with `From` conversions) generate cold blocks and an extra branch per fallible call vs revm's "return nothing, set a field on halt" ABI.

**RC4 — Stack micro-inefficiency for stack-neutral unary ops (small).** ISZERO does `pop1` (offset += 1, bounds branch) then `push` (checked_sub branch, offset -= 1) — two offset round-trips and two checks where a read-modify-write of the top slot needs zero (bitwise_comparison.rs:130-141, call_frame.rs:67-106). LLVM may fold some of this since both inline into `eval`, but the checks remain.

**RC5 — Per-op gas: NOT the gap.** `increase_consumed_gas` (call_frame.rs:421-429) is equivalent to revm's per-instruction `gas!`. Listed only because batching it (basic-block gas) is available upside beyond revm parity, not a deficit.

This also explains why LT/GT/EQ are competitive (0.7-1.0x revm): they pay the same fixed ~1.45 ns tax, but their op work (64-byte pop, compare) amortizes it; ISZERO at ~0.5 ns of real work cannot.

## 3. Ranked improvement proposals (describe-only; no code changed)

**P1. Padded bytecode + branch-free fetch + plain PC add.** Pad `Code.bytecode` with 33 zero bytes (STOP) at construction (`crates/common/types/account.rs:49-65`, where `jump_targets` is already precomputed) and keep the logical length separately for CODESIZE/CODECOPY/EXTCODE* /jump validation. Then `next_opcode` becomes an unchecked load, and `advance_pc` a plain `pc += n` with no `Result` (pc is bounded by padded len; PUSH's `checked_add` at push.rs:34-39 also drops). Removes 2-3 branches + 2 loads + Result plumbing per op.
- Impact: the largest single lever; est. 10-25% on dispatch-bound code, lifts all opcodes. Likely closes half the ISZERO gap alone.
- Risk/effort: medium. Every consumer of `bytecode.len()` and code-copy semantics must use the logical length (jump validation at stack_memory_storage_flow.rs:437-450 already checks `jump_targets`, but the `get(target)` byte check would see padding). An intermediate, low-risk variant: keep the fetch branch but make `advance_pc` infallible (`saturating_add` or plain add with a debug assert) — removes the `Result`/`?` and one branch for a tiny diff.

**P2. Hoist the tracer out of the loop.** `opcode_tracer.active` is set at VM construction; branch once before the loop and run two loop bodies (duplicate loop, or monomorphize `run_execution` over `const TRACED: bool`). Removes one load + two branches per op and shrinks the hot loop's I-cache footprint by ~100 lines of capture code (vm.rs:1021-1108).
- Impact: est. 5-15% on dispatch-bound code, all opcodes.
- Risk/effort: low; purely mechanical. Best ROI per line changed.

**P3. Shrink the handler error ABI.** Either (a) box/split the large `VMError` payloads so the in-loop `Result` is <=16 bytes (opcode handlers can only produce `ExceptionalHalt`/`InternalError`/`RevertOpcode`, never `TxValidationError` — a narrower `OpcodeError` enum would be a single byte), or (b) go full revm-style: handlers return plain `OpcodeResult` and record the halt reason in a `VM` field, deleting the `OnceCell` parameter and `wrap` (opcodes.rs:392-400).
- Impact: est. 2-5%; also reduces code size in every handler.
- Risk/effort: (a) low-medium; (b) medium — touches all ~150 handlers, but mechanically.

**P4. Basic-block pre-analysis ("plan" / evmone-advanced style).** Pre-decode each `Code` (cached by code hash, alongside the existing `jump_targets`) into a stream of (handler, immediate) with per-basic-block summed gas and max stack delta; charge gas and check stack bounds once per block, making per-op handlers branch-free for gas/stack.
- Impact: the only proposal that leapfrogs revm rather than matching it; plausibly 1.5-2x interpreter throughput on compute-heavy code, all opcodes.
- Risk/effort: high. New per-code cache, consensus-sensitive gas edge cases (OOG must still occur at the exact op for tracing/refund semantics), tracer needs a per-op fallback path. Do this only after P1-P3, if interpreter throughput is still a priority.

**P5. In-place top-of-stack mutation for unary stack-neutral ops.** Add `Stack::top_mut()`/`replace_top()` and use it in ISZERO/NOT (bitwise_comparison.rs:130-141): one bounds check, no offset round-trip.
- Impact: microbenchmark-visible only (~1 branch saved on 2 opcodes). Trivial effort; do it opportunistically.

## 4. Measurement plan

Before/after on each change, in `crates/vm/levm`:
- Existing harness: `make` benchmark targets in `crates/vm/levm/Makefile` (hyperfine-driven `target/release/benchmark levm <bench> ...` vs `revm`, lines 44-52). Add/use a tight ISZERO-loop contract under `bench/revm_comparison/contracts` to mirror `test_comparison.py::test_iszero`.
- Branch/load accounting to confirm the mechanism (not just the time): `perf stat -e cycles,instructions,branches,branch-misses,L1-dcache-loads -- target/release/benchmark levm <iszero_bench> 1 <n>` — expect instructions/op and branches/op to drop by the counts predicted above (P1: -2-3 branches/op; P2: -2 branches/op).
- Per-opcode attribution already exists: build with `--features perf_opcode_timings` (`crates/vm/levm/Cargo.toml:32`) — note this feature itself adds two `Instant::now()` per op (vm.rs:1072-1083), so use it for relative attribution only.
- Final check on the real target: the BAL compute benchmark `test_comparison.py::test_iszero` plus at least one mixed-opcode bench (e.g. the LT/GT ones) to confirm no regression on non-trivial ops.

## 5. Honest framing

- This is low recoverable time on the benchmark itself: ISZERO at 300M gas is ~100M ops × 1.45 ns ≈ 145 ms of total gap. It's a rank-position lever on the microbenchmark table, not a real-workload cliff.
- It is, however, a general interpreter-throughput tax: the fixed per-op overhead applies to every executed opcode, and real blocks are dominated by cheap ops (PUSH/DUP/SWAP/arith/MLOAD). P1+P2+P3 lift all-opcode dispatch and should give low-single-digit-% on real compute-heavy blocks; state-access-bound blocks (SLOAD/SSTORE-heavy) will barely move, since their cost is in the DB/journal, not dispatch. P4 is the only proposal with the potential to materially shift real-workload Mgas/s, at correspondingly higher risk.

## Key files
- `crates/vm/levm/src/vm.rs` (loop, 1014-1129)
- `crates/vm/levm/src/opcodes.rs` (table, 376-431)
- `crates/vm/levm/src/call_frame.rs` (fetch/pc/gas/stack, 67-106, 402-429, 537-544)
- `crates/vm/levm/src/errors.rs` (VMError, 11-20)
- `crates/common/types/account.rs` (Code, 28-94)
