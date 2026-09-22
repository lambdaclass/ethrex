# EIP-8200: EVMification
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/242, 2026-07-30 (champion: Kevaundray Wedderburn)
- **Authors:** Kevaundray Wedderburn
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8200) · [discussion](https://ethereum-magicians.org/t/eip-8200-evmification/28036)
- **Prior fork history:** none

## TL;DR
Generalizes EIP-7666's trick to three real precompiles: RIPEMD-160 (`0x03`), MODEXP (`0x05`), and BLAKE2f (`0x09`). At the fork block, functionally equivalent EVM bytecode is deployed at each address and the addresses stop being precompiles. Existing contracts keep calling the same addresses unchanged.

## What it changes
- Same mechanism as EIP-7666: at fork activation, set code at `0x03`, `0x05`, `0x09` to EVM bytecode replicating the precompile; drop the precompile special-casing.
- The exact bytecode is **not yet specified** in the draft — a major open item.
- Gas: precompile-specific formulas are abandoned in favor of natural EVM opcode costs. Expected deltas:
  - RIPEMD-160: was `600 + 120/word`; EVM version will differ (comparison TODO in spec).
  - MODEXP: complex length/iteration formula; EVM costs expected **higher for large inputs**.
  - BLAKE2f: was `rounds` gas per call; EVM version charged per opcode (comparison TODO).
- Author's usage analysis: 99.99% of MODEXP calls are 256-bit modulus SNARK verification (rarely RSA); BLAKE2f is now used predominantly by a single contract; RIPEMD-160 has minimal on-chain use.

## Motivation
Each precompile is native code every client must implement and keep byte-identical — a consensus-bug and maintenance surface, and a heavy burden for ZK-EVMs (separate optimized circuits/provers per precompile). These three are little-used enough that paying EVM gas prices for them is acceptable, and removing them shrinks the protocol's special-case surface.

## Dependencies & related EIPs
- **Depends on:** EIP-7666 (establishes the deployment mechanism; explicitly required), EIP-152 (original BLAKE2f definition), EIP-7823 and EIP-7883 (MODEXP input bounds and repricing, shipped in Fusaka — the EVM replacement assumes the bounded inputs).
- **Builds on:** EIP-7666 as the pilot.
- **Conflicts / tension with:** any EIP that reprices or extends these precompiles; also competes for scope with simply keeping the precompiles and repricing them (the path Fusaka took with MODEXP).
- Note: EIP-7823/7883 are not Hegota candidates (already shipped), so the dependency is satisfiable.

## Impact on ethrex / client teams
- Touches: precompile implementations in `crates/vm/levm/src/precompiles.rs` (`ripemd_160`, `modexp`, `blake2f` would be removed from the precompile table), the fork-transition state write (as in 7666), and gas accounting.
- ethrex must still keep the old native implementations around to execute pre-fork history (unless archive sync only replays post-fork), so the net maintenance win accrues slowly.
- The guest-program / ZK side benefits most: three fewer native circuits to prove — but the EVM bytecode must then be provably equivalent to the native code on all inputs.
- Effort: **medium**. Most of the work is writing/auditing the three replacement contracts and the differential test matrix, not client plumbing.

## Open questions & controversies
- Spec maturity is low: replacement bytecode and gas comparisons are still TODOs.
- MODEXP cost increase for large inputs may break contracts passing fixed gas (author argues hardcoded precompile gas is bad practice; past repricings set precedent).
- Correctness risk: the EVM bytecode must match the native precompile on every valid input and replicate invalid-input behavior exactly; needs heavy differential fuzzing across clients.
- Some pushback expected on whether retiring BLAKE2f/RIPEMD is worth it given near-zero usage and the Fusaka MODEXP repricing already blunting the ZK cost argument for MODEXP.
