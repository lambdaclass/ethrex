# EIP-8163: Reserve `EXTENSION (0xae)` opcode
- **Layer:** EL
- **EIP status:** Review
- **Hegota status:** Proposed (CFI, acde/235, 2026-04-23)
- **Authors:** Bruce Collie, Piotr Dobaczewski
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8163) · [discussion](https://ethereum-magicians.org/t/eip-8163-reserve-0xae-extension-opcode/27756)
- **Prior fork history:** none

## TL;DR
Permanently reserves opcode byte `0xae` as `EXTENSION`, guaranteeing it will never become a real instruction on Ethereum L1, so other EVM chains (L2s, alt-L1s) can safely use it as a prefix for their own extension instructions without risking a future collision with a mainnet opcode. On L1 nothing changes: `0xae` already behaves as `INVALID`, and the EIP mandates it keeps behaving exactly like `INVALID (0xfe)`.

## What it changes
- `EXTENSION (0xae)` MUST behave exactly like `INVALID` on all Ethereum L1 chains: exceptional halt, consumes all gas.
- The `0xae` byte MUST NOT affect `JUMPDEST` analysis in any way; any bytecode validation/static analysis must treat it exactly as `INVALID`.
- **No L1 client code changes are required** — the EIP is a social/spec-level commitment ("this byte is taken, forever"), not a behavior change. Reference implementation: "None".
- Off-L1 semantics (informative): chains adopting it must determine extension behavior from the bytes *following* `0xae`; JUMPDEST analysis must still match L1 exactly; if immediate args would swallow a real `JUMPDEST`, execution must halt. A table defines which `0xae…` byte sequences are valid extensions (e.g. `0xae60` truncated is invalid; arguments containing `0x5b` must be encoded via `PUSHx` data).
- Encoding scheme for extensions is deliberately **not** defined; coordination is via the discussion thread.

## Motivation
Non-L1 EVM chains currently can't add their own instructions without risking incompatibility if mainnet later claims the same opcode byte. Reserving one byte creates a safe experimentation space: extensions can be proven on other chains and back-ported to L1 later with a distinct opcode. Strengthens EVM network effects by keeping innovation inside the EVM ecosystem.

## Dependencies & related EIPs
- **Requires: EIP-141** (`INVALID` opcode) — defines the semantics `EXTENSION` must replicate.
- Synergizes with L2-oriented opcode proposals like **EIP-7843** (`SLOTNUM`, already in ethrex's Amsterdam table) — the general theme of letting rollups specialize their EVM.
- No dependency on, or conflict with, other Hegota candidates. Orthogonal to opcode-adding proposals (7979, 8219): those consume free bytes, this one consumes one byte to *prevent* future consumption.
- Note: the `0xA*` range neighbors `LOG0`–`LOG4` (`0xA0`–`0xA4`) and ethrex's EIP-8141 `APPROVE` (`0xAA`); `0xae` itself is currently unassigned everywhere.

## Impact on ethrex / client teams
- **Effectively zero.** In LEVM, unassigned bytes already dispatch to `OpInvalidHandler` via the `Opcode::INVALID` default in the `const` opcode table (`crates/vm/levm/src/opcodes.rs`), so `0xae` already behaves exactly as required. JUMPDEST analysis skips non-JUMPDEST bytes by construction.
- Optional: name `0xae` in the opcode enum / tracers for nicer output. No gas, consensus, tx-pool, networking, Engine API, or RPC impact.
- Effort: **trivial** (arguably a no-op; the work is governance, not code).

## Open questions & controversies
- Permanently spends one of the EVM's scarce unassigned bytes on something L1 never executes — some may prefer keeping the byte available for a real future L1 opcode.
- Deliberately leaves encoding undefined; success depends on off-L1 chains actually coordinating rather than fragmenting into incompatible `EXTENSION` dialects.
- The "promise" is only as strong as the social consensus behind it; a future fork could in principle override it.
- No known active pushback; low-controversy, low-value-for-L1 tradeoff.
