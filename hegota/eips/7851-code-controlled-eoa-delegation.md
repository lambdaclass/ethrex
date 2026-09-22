# EIP-7851: Code-Controlled EOA Delegation
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/238, 2026-06-04
- **Authors:** Liyi Guo, Nicolas Consigny
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7851) · [discussion](https://ethereum-magicians.org/t/eip-7851-code-controlled-eoa-delegation/22344)
- **Prior fork history:** none

## TL;DR
Extends EIP-7702 with a second delegation prefix `0xef0101` ("ECDSA-disabled") and a self-only opcode `SETSELFDELEGATE`. Wallet code running under a 7702 delegation can call it to re-point its own delegation — and the first call permanently burns the original EOA private key's power to send transactions or change the delegation.

## What it changes
- New delegation prefix: `0xef0101 || delegate_address`. For code execution it behaves identically to `0xef0100` (clients resolve it the same way); for *validation* it marks the authority as ECDSA-disabled.
- New opcode `SETSELFDELEGATE` (opcode byte TBD, gas 9500 = `PER_AUTH_BASE_COST` minus signature-recovery 3000). Self-only: it updates only the current execution-context address (`authority`).
  - Halts in static context; normal revert semantics apply; effect is frame-local (current frame keeps executing loaded code; later/re-entrant calls follow the new indicator).
  - Pushes 0 (no-op) if `delegate_address == 0` or if the account's raw stored code isn't a 23-byte `0xef0100`/`0xef0101` indicator; otherwise writes `0xef0101 || delegate_address` and pushes 1.
- Validation changes:
  - During 7702 authorization processing, an authorization signed by an authority whose code starts with `0xef0101` MUST be skipped as invalid.
  - An ECDSA transaction whose recovered sender has `0xef0101` code is invalid; mempools must reject it.
- Irreversible invariant: once `0xef0101` is set, ECDSA authority can never be restored, but the delegate can still be updated by the wallet code.

## Motivation
- After a 7702 "upgrade", the EOA's original key retains full power: it can take the account back or be stolen/coerced. Users who operate entirely through wallet code (passkeys, multisig, social recovery) want to retire that key without locking the account to one implementation.
- Today this requires either trusting the key stays safe or migrating to a new address. `SETSELFDELEGATE` puts key retirement in the account's own execution path.

## Dependencies & related EIPs
- Requires: EIP-7702 (shipped in Pectra).
- Companion: EIP-8151 (in the Hegota candidate list) — makes `ecrecover` reject ECDSA-disabled authorities; without it, `permit`-style contracts still accept the retired key's signatures (explicitly out of scope here).
- Related / partially overlapping: EIP-7819 (SETDELEGATE, factory-managed delegations) and EIP-8298 (SETCODEFROM, which disables ECDSA authority by installing regular code via EIP-3607 instead of a second delegation prefix — an alternative mechanism for the same "burn the key" goal).
- Synergizes with EIP-8141 (frame transactions / native AA): retired-ECDSA accounts are cleaner AA citizens.

## Impact on ethrex / client teams
- LEVM: new opcode + recognition of `0xef0101` alongside `0xef0100` in the delegation-resolution path (existing 7702 code in `crates/vm/levm/src/utils.rs` / `account.rs`); raw-code vs resolved-code distinction must be respected.
- Transaction validation: extra account-code read for senders to reject ECDSA txs from `0xef0101` accounts — touches `crates/blockchain/mempool.rs` and block validation.
- Gas accounting: one new constant.
- Effort: small-to-medium; the opcode is simple, but validation/mempool interplay and the frame-local semantics need careful tests.

## Open questions & controversies
- Opcode byte still TBD; EIP is early Draft — spec maturity is low.
- Irreversibility is a footgun: a buggy or malicious wallet module could brick ECDSA recovery permanently.
- Does not cover contract-level ECDSA checks (`ecrecover`, token `permit`) without EIP-8151 — so the "key is retired" guarantee is incomplete at the app layer.
- Competes conceptually with EIP-8298's migration path; Hegota may not want both.
