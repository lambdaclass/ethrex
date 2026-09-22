# EIP-7666: EVM-ify the identity precompile
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/242, 2026-07-30
- **Authors:** Vitalik Buterin, Kevaundray Wedderburn
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7666) · [discussion](https://ethereum-magicians.org/t/eip-7561-evm-ify-the-identity-precompile/19445)
- **Prior fork history:** Declined from Fusaka (call/date not recorded in forkcast)

## TL;DR
Delete the identity precompile at `0x04` (which just copies calldata to returndata) and, at the fork block, write 10 bytes of EVM code at that address that does the same thing using `MCOPY`-era opcodes. It is the pilot for a broader "EVMification" program to retire little-used precompiles.

## What it changes
- At the start of the fork-activation block, the code of `0x0000...0004` is set to `0x365f5f37365ff3`, i.e. `CALLDATASIZE PUSH0 PUSH0 CALLDATACOPY CALLDATASIZE PUSH0 RETURN`.
- From that block on, `0x04` is no longer special-cased as a precompile; calls to it execute as normal contract calls.
- Functionality is preserved bit-for-bit; gas cost changes slightly (charged as ordinary EVM execution instead of the precompile's `15 + 3/word` formula).
- This is a one-off consensus-level state modification ("state surgery") at the transition block, similar in spirit to past irregular state changes.

## Motivation
Ethereum's precompiles are hand-written native code in every client; each is a consensus-bug and maintenance surface, and a separate optimized implementation burden for ZK-EVMs and formally-verified clients. The identity precompile was only needed because the EVM lacked a memory-copy opcode; since `MCOPY` (EIP-5656, Cancun) exists, it is redundant. Retiring it first establishes the template for harder cases (MODEXP, little-used hash functions — see EIP-8200).

## Dependencies & related EIPs
- **Depends on:** EIP-3855 (`PUSH0`, shipped Shanghai) — required by the replacement bytecode; also conceptually on EIP-5656 (`MCOPY`) existing.
- **Builds into / template for:** EIP-8200 (EVMification of RIPEMD-160, MODEXP, BLAKE2f), which explicitly requires this one.
- **Synergizes with:** the general precompile-reduction theme; no conflicts known.

## Impact on ethrex / client teams
- Touches: precompile table (`crates/vm/levm/src/precompiles.rs` — remove `identity` from the precompile set), plus a fork-transition hook that writes the bytecode into state at the activation block (block processing in `crates/blockchain` / VM backend).
- Gas accounting for calls to `0x04` changes automatically once it stops being treated as a precompile.
- Stateless/ZK path (ethrex's guest program) also drops the special case — a small win.
- Effort: **small**. The tricky part is not code volume but test coverage (transition-block behavior, calls before/after the fork, gas deltas).

## Open questions & controversies
- Gas-cost change for existing callers of `0x04` (generally accepted as benign; hardcoded-gas callers could break).
- Some hesitation about doing consensus state surgery for a cosmetic gain — it was declined from Fusaka, so appetite is not guaranteed.
- Mechanism is deliberately minimal; main risk is sloppy transition handling across clients, not design.
