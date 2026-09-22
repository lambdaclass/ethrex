# EIP-7645: Alias ORIGIN to SENDER
- **Layer:** EL
- **EIP status:** Stagnant
- **Hegota status:** Proposed (CFI) — acde/239, 2026-06-18
- **Authors:** Cyrus Adkisson, Eirik Ulversøy
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7645) · [discussion](https://ethereum-magicians.org/t/eip-7645-alias-origin-to-sender/19047)
- **Prior fork history:** none

## TL;DR
Make the `ORIGIN` opcode (0x32) return the same value as `CALLER`/`SENDER` (0x33) — the immediate caller — instead of the transaction initiator. `tx.origin`-style authentication has been discouraged since 2016 because it enables phishing-style attacks; this removes the distinction entirely and clears a standing obstacle for every account-abstraction design.

## What it changes
- Single behavioral change: in all execution contexts, `ORIGIN` MUST push the current call's sender address, exactly as if `CALLER` had executed.
- No transaction-format, gas, or validation changes. No new opcode; the existing `ORIGIN` opcode is redefined.
- Consensus-critical: every client's EVM must make this change at the same fork.
- After the change, the transaction initiator's address is no longer observable from inside the EVM at all.

## Motivation
- `ORIGIN` has been considered deprecated since mid-2016. Contracts that use it for authorization can be tricked: any contract the initiator calls can act with the initiator's authority.
- Concrete AA hazard: an ERC-4337 bundler's address is the `ORIGIN` of every bundled UserOperation, so any bundled tx could hijack authority granted to `ORIGIN == bundler`.
- Every AA proposal (EIP-3074, EIP-7377, EIP-7702, frame transactions / EIP-8141) has to special-case or work around `ORIGIN`; aliasing removes that recurring design debt and simplifies the EVM execution model.

## Dependencies & related EIPs
- No hard dependencies (touches only opcode semantics).
- Synergizes with: EIP-7702 (shipped in Pectra), EIP-8141 (frame transactions), and any AA design — all benefit from `ORIGIN == SENDER` holding universally.
- Related prior art: EIP-3074, EIP-7377 (older AA approaches cited in the motivation).

## Impact on ethrex / client teams
- Trivial to implement in LEVM: `op_origin` in `crates/vm/levm/src/opcode_handlers/environment.rs:78` currently pushes the tx origin; it would push the call-frame sender instead. Opcode table entry in `crates/vm/levm/src/opcodes.rs` (0x32) is unchanged.
- Effort: trivial code change; the real cost is testing and ecosystem coordination (fork-gated behavior, EF tests, checking that nothing in tooling/tracing assumes origin semantics).
- The risk is not implementation but breaking deployed contracts that use `ORIGIN` for auth (e.g., some older multisig/relay patterns).

## Open questions & controversies
- Not backwards compatible: any contract relying on `ORIGIN != SENDER` silently changes behavior. The EIP itself admits no concrete harmful-breakage example has been identified, so real impact is unmeasured.
- EIP status is Stagnant (inactive since 2024); being re-championed for Hegota suggests the spec may need a refresh before serious inclusion debate.
- Some argue `ORIGIN` should instead be banned (revert on deployment of code using it) rather than aliased — the EIP's security section mentions this as an out-of-scope option.
