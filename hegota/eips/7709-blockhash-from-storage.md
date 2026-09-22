# EIP-7709: Read BLOCKHASH from Storage and Update Cost
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/236, 2026-05-07
- **Authors:** Vitalik Buterin, Tomasz Stanczak, Guillaume Ballet, Gajinder Singh, Tanishq Jasoria, Ignacio Hagopian, Jochem Brouwer, Gabriel Rocheleau
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7709) · [discussion](https://ethereum-magicians.org/t/eip-7709-read-blockhash-opcode-from-storage-and-adjust-gas-cost/20052)
- **Prior fork history:** none recorded in forkcast

## TL;DR
Makes the BLOCKHASH (0x40) opcode resolve in-window hashes by reading the EIP-2935 history system contract's storage instead of relying on a client-side special case, charging cold/warm SLOAD gas accordingly. Return-value semantics are unchanged (still 256-block window, 0 outside it). This aligns block-hash access with ordinary state-access machinery, simplifying witness construction for stateless execution and proving.

## What it changes
- In-window BLOCKHASH (`arg` within last 256 ancestors) becomes equivalent to an SLOAD of slot `arg % 8191` (`HISTORY_SERVE_WINDOW`) at `HISTORY_STORAGE_ADDRESS` (`0x0000F90827F1C53a10cb7A02335B175320002935`): cold/warm SLOAD gas is charged, the slot is warmed, and any fork-required state-access recording (e.g. BAL entries under EIP-7928) applies.
- The base BLOCKHASH opcode cost is still charged on top of the SLOAD cost.
- Out-of-window args still return 0 with no storage effects, even though the EIP-2935 ring buffer holds 8191 slots — the 256-block `BLOCKHASH_SERVE_WINDOW` is preserved for compatibility.
- Clients MAY implement the read as a direct state SLOAD, a system call to the EIP-2935 contract, or from in-memory history — but MUST apply full SLOAD semantics (gas, warming, access recording) regardless.
- Activation requires EIP-2935 to have been live for ≥256 blocks, or activation at genesis on testnets/devnets.

## Motivation
BLOCKHASH is a protocol special case: it depends on historical chain data but is not modeled as a state read. With EIP-2935 (shipped in Pectra) storing recent hashes in system contract storage, the opcode can be routed through the same state-access path as everything else. This matters most for stateless clients and zk proving: witnesses can serve block hashes via ordinary storage proofs instead of a bespoke header-chain mechanism, and gas cost matches the resource actually accessed.

## Dependencies & related EIPs
- **Depends on:** EIP-2935 (blockhash history system contract, shipped in Pectra) — hard requirement, including the 256-block activation lead time.
- **Synergizes with:** EIP-7928 (BALs, Glamsterdam) — BLOCKHASH reads become recorded state accesses; and with statelessness/zk proving work generally (Verkle, LEVM proving in ethrex's prover crates).
- **Cluster context:** gas repricing via alignment — BLOCKHASH goes from a flat ~20–800 gas (fork-dependent) to cold/warm SLOAD pricing (~2100/100), a major increase for in-window reads.
- No conflicts with other Hegota candidates.

## Impact on ethrex / client teams
- Small. The BLOCKHASH opcode handler is in LEVM (`crates/vm/levm/src/opcode_handlers/system.rs`); EIP-2935 system contract support already exists (`crates/vm/system_contracts.rs`, plus 2935 references in `crates/vm/levm/src/utils.rs` and `crates/vm/backends/levm/mod.rs`).
- Work: route in-window BLOCKHASH through the EIP-2935 storage slot read with cold/warm gas accounting and warming; keep the 256-block window check and out-of-window zero. Needs care that warming/access-recording semantics match the active fork (BAL recording under Glamsterdam rules).
- Gas estimation (`eth_estimateGas`) and tracing/debug tooling need the new cost model. No tx pool, networking, Engine API, or state-format changes.

## Open questions & controversies
- Significant gas increase for contracts using BLOCKHASH (e.g. randomness oracles, some VDF/commit-reveal schemes); contracts tuned to the old price may break or become uneconomical.
- The "clients MAY resolve from memory" allowance means the observable difference (warming effects, BAL entries) must be emulated exactly by clients that don't actually SLOAD — a subtle consensus-correctness trap.
- Long-standing EIP (May 2024) that has repeatedly missed forks; championed for Hegota by Ignacio Hagopian (ethrex-adjacent, jsign). No known fundamental opposition, just prioritization.
