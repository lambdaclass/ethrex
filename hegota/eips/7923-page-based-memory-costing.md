# EIP-7923: Linear, Page-Based Memory Costing
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acde/243, 2026-08-13; champion Jochem Brouwer)
- **Authors:** Charles Cooper, Qi Zhou
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7923) · [discussion](https://ethereum-magicians.org/t/eip-linearize-memory-costing/23290)
- **Prior fork history:** Declined from Glamsterdam (acde/226, 2025-12-18)

## TL;DR
Replaces the EVM's quadratic memory-expansion formula with a linear, page-based model: memory is addressed virtually in 4 KB pages, first touch of a page costs 100 gas, and a transaction can allocate at most 64 MiB total across all call frames. Memory limits become invariant to the message-call stack shape, gas estimation gets predictable, and high-level languages can finally use real virtual-memory layouts (heap grows up, stack grows down).

## What it changes
- **Constants:** `ALLOCATE_PAGE_COST = 100`, `PAGE_SIZE = 4096`, `MAXIMUM_MEMORY_SIZE = 64 MiB` (16,384 pages per transaction).
- **Costing:** for each page touched by an instruction, charge 100 gas if not previously touched *in this message call*; page 0 is free (CALL base cost already covers it). Both the linear (3 gas/word) expansion term and the quadratic term are removed. Base cost of memory instructions (`MLOAD` etc., 3 gas) is unchanged.
- **Virtual addressing:** pages are allocated on demand; implementations can back memory with `mmap`/`VirtualAlloc` over a 2^32-byte region, or maintain a `page_id -> [u8; 4096]` map manually. Address space is capped at 32 bits — an address > `2^32 - 1` is an exceptional halt.
- **Transaction-global memory limit:** if total pages allocated in a transaction exceed 16,384 (64 MiB), exceptional halt. Child-call pages are "deallocated" (for the global count) when the child returns.
- **`MSIZE` semantics preserved:** still returns the max byte touched, rounded up to 32 — existing contracts that rely on linear growth keep working; new contracts shouldn't use MSIZE for allocation.
- Reference implementation: ~60-line patch against py-evm included in the spec.

## Motivation
The quadratic model is anachronistic: at 30 M gas you can afford only ~3 MB of memory (and using even 64 KB costs 14,336 gas). Worst-case memory per transaction requires solving an optimization over the call stack and the 63/64 rule — bad for DoS reasoning. And because memory must grow contiguously from zero, Solidity/Vyper can't use classic heap/stack virtual-memory layouts, hurting code efficiency. A paged, linear model matches modern hardware (cache/TLB hierarchy) and enables it.

## Dependencies & related EIPs
- **Direct alternative to EIP-7686** (Vitalik's linear memory limits): both were declined from Glamsterdam in the same call (acde/226); 7923 is the one re-proposed for Hegota. 7686 ties the memory cap to gas; 7923 uses a fixed 64 MiB transaction-global cap decoupled from the gas limit (deliberate, to avoid RPC `eth_call` misconfiguration vectors).
- Removes the quadratic formula introduced in the yellow paper era; touches the **EIP-150**-era reasoning about call-stack memory bounds only indirectly (63/64 rule itself untouched).
- Synergy with any **gas-limit increase** work: decoupled memory cap means raising the gas limit doesn't raise worst-case memory.
- Independent of the cluster's opcode EIPs (7979, 8163, 8219) and of EIP-8131's tx floor.

## Impact on ethrex / client teams
- **EVM interpreter (LEVM):** the biggest change in this cluster. `crates/vm/levm/src/memory.rs` moves from a growable contiguous buffer + quadratic `expansion_cost` (`memory.rs:491`) to a page-table or `mmap`-backed model with per-frame "touched pages" tracking and an `msize` high-water mark. Gas accounting in `crates/vm/levm/src/gas_cost.rs` (ten+ expansion call sites: `MLOAD`/`MSTORE*`, `CALLDATACOPY`/`CODECOPY`/`RETURNDATACOPY`/`MCOPY`, `CALL*` args/returns, `CREATE*`, `RETURN`/`REVERT`, `KECCAK256`, logs) switches to per-page charging. A transaction-global page counter must live in tx context and be checkpointed/restored per child call. New exceptional halt: OOM at 64 MiB / address > 2^32.
- **Tooling/RPC:** tracers, gas estimators, and `eth_call` policies need the new model; LRU(512) page tracking suggested for accurate simulation.
- No tx-pool, networking, Engine API, or state changes. Some contracts that previously ran out of gas may now succeed — that's a consensus-visible behavior change to test carefully.
- Effort: **medium-to-large** (memory subsystem rework + full gas-regression testing).

## Open questions & controversies
- Declined from Glamsterdam once already; competes head-on with 7686 and with simpler "just raise the linear term / cap memory" ideas — client teams must pick one memory-repricing direction.
- `mmap`-backed implementation is elegant on Linux but less so elsewhere (Windows, constrained/zk environments); zkVM/prover implications of a 4 GiB virtual address space need thought for ethrex's proving path (crates/prover, guest program).
- 100 gas/page and 64 MiB cap are benchmark-derived on 2019-era hardware; whether they survive client benchmarking is open.
- Some previously-OOG transactions completing changes gas outcomes — needs careful hive/EF-test coverage.
- Interaction with future gas-limit increases was left out of scope by design (fixed cap), which some see as a missing scaling story.
