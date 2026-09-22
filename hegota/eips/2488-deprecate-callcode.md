# EIP-2488: Deprecate the CALLCODE opcode
- **Layer:** EL
- **EIP status:** Stagnant
- **Hegota status:** Proposed (CFI) — acde/239, 2026-06-18
- **Authors:** Alex Beregszaszi
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-2488) · [discussion](https://ethereum-magicians.org/t/eip-2488-deprecate-the-callcode-opcode/3957)
- **Prior fork history:** none (proposed in 2019, never included in a fork)

## TL;DR
CALLCODE (0xf2) was part of Frontier but its design was flawed within months; EIP-7 introduced DELEGATECALL in Homestead (2016) as the replacement and CALLCODE has been effectively unused since. This EIP makes CALLCODE always return 0 (failure) after the fork, so clients that don't sync from genesis no longer need to implement its real behavior.

## What it changes
- After `FORK_BLOCK`, the CALLCODE instruction always returns 0 (failure) on the stack instead of executing the call semantics.
- Chosen over making the opcode an exceptional abort so that calling contracts get a chance to handle failure and recover.
- No gas schedule changes, no new opcodes, no state format changes — purely an opcode behavior change gated on fork block.
- Single-line spec; the only subtlety is that historical blocks must still be executed with the original semantics by clients syncing from genesis.

## Motivation
Every EVM implementation must still carry and maintain CALLCODE logic (a full call variant with unusual caller/value semantics) even though essentially nothing has used it since 2016. Disabling it removes that burden for light clients and clients syncing from a recent snapshot. Clients syncing from genesis gain nothing, since they must retain the old behavior for historical execution.

## Dependencies & related EIPs
- Requires EIP-7 (DELEGATECALL) — the reason CALLCODE can be deprecated at all.
- Part of the same cleanup family as EIP-4758 (SELFDESTRUCT deactivation) in this cluster, and thematically adjacent to the opcode-cleanup candidates EIP-5920 (PAY opcode) and EIP-7709.
- No dependencies on other Hegota candidates.

## Impact on ethrex / client teams
- Trivial. One branch in the CALLCODE handler in `crates/vm/levm/src/opcode_handlers/system.rs` (handler exists in `opcodes.rs` / gas table in `gas_cost.rs`): return 0 without executing once past fork activation.
- No changes to tx pool, block validation structure, networking, Engine API, or RPC. State/trie untouched.
- The main cost is test coverage: existing CALLCODE tests must be fork-gated and new failure-path tests added. Historical replay (ethrex_replay) still needs the old code path for pre-fork blocks.

## Open questions & controversies
- The spec itself flags an unvalidated claim: "the author expects no contracts of any value should be affected. TODO: validate this claim." A chain-wide usage analysis has not been done and documented.
- Test cases, security considerations, and reference implementation sections are all "TBA" — the spec is minimal and Stagnant since 2019.
- Perennial debate: is removing a live (even if unused) opcode worth a consensus change at all? Some client devs see pure cleanup EIPs as not worth fork complexity.
