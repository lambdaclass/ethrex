# EIP-7819: SETDELEGATE instruction
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/242, 2026-07-30
- **Authors:** Hadrien Croubois
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7819) · [discussion](https://ethereum-magicians.org/t/eip-7819-create-delegate/21763)
- **Prior fork history:** Declined from Fusaka; Declined from Glamsterdam (acde/225, 2025-12-04)

## TL;DR
Adds a `SETDELEGATE` opcode (0xf6) letting a factory contract create (or update/clear) EIP-7702-style delegation-indicator accounts at deterministic addresses. This gives upgradeable, 23-byte "clones" managed at protocol level instead of via proxy bytecode — cheaper calls, smaller state — aimed mainly at smart-account factories.

## What it changes
- New opcode `SETDELEGATE` at `0xf6`, halts in static context; pops `salt` and `target`.
- Computes `location = keccak256(0xef0100 ++ caller_address ++ salt)[12:]` (55-byte pre-image, chosen to provably not collide with CREATE/CREATE2 derivations).
- Writes the 23-byte delegation indicator `0xef0100 ++ target` as `location`'s code (same object EIP-7702 creates; all 7702 semantics — CODESIZE/CODECOPY, no chaining, precompile targets — apply).
- `target == 0x0` clears the code and resets the code hash to the empty hash; nonce is set to 1 on first creation so the account never becomes empty (EIP-7523).
- Halts if `location` has non-empty, non-delegation code; delegation is effective immediately (next opcode onward), and can be changed multiple times within one transaction.
- Gas: `EMPTY_ACCOUNT_COST = 25000`, with `EMPTY_ACCOUNT_COST - BASE_COST (12500)` refunded if `location` already existed (same constants as 7702, minus signature-recovery work).
- Pushes `location` onto the stack.

## Motivation
- 97.1% of ~50M deployed contracts share already-deployed code; clones/proxies are a large share (Geth "Not all state is equal", EthCC[9]).
- ERC-1167 clones are cheap but immutable; ERC-1967-style proxies (~708 bytes) are upgradeable but pay a storage lookup per call. Delegation indicators (23 bytes) give upgradeability with zero storage lookup, and call redirection happens in the client, not in EVM bytecode — saving gas on every call, including ERC-721/1155 transfer callbacks.
- Factories (Safe-style accounts, ERC-4337 accounts, future EIP-8141 frames) get atomic create+initialize without front-running risk.

## Dependencies & related EIPs
- Requires: EIP-7702 (shipped in Pectra) — reuses the delegation-indicator format and constants; EIP-2929 (access lists), EIP-7523 (empty-account ban).
- Synergizes with: EIP-8141 (frame transactions / smart-account deployments), EIP-7851 and EIP-8298 (other delegation/code-update instructions in this cluster — 7851 is EOA-self-service redelegation, 8298 adopts full code rather than a delegation pointer; a fork could plausibly take more than one, but they overlap in the "update account code from code" design space).
- Tension with EOF: the motivation notes legacy clones can't delegate to EOF code; SETDELEGATE keeps redirection at protocol level, sidestepping that.

## Impact on ethrex / client teams
- LEVM: new opcode entry in `crates/vm/levm/src/opcodes.rs` (0xf6) plus a handler that writes account code via the existing EIP-7702 delegation-indicator machinery (already implemented for 7702 — see `crates/vm/levm/src/utils.rs`, `account.rs`). Needs correct interplay with the 7702 code-resolution path so calls immediately observe the new delegation.
- Gas accounting and refund-counter handling per the constants above; warm/cold access of `location`.
- Tracing/debugging and indexers must handle mid-transaction code changes at an address.
- Effort: small-to-medium (single opcode, but the account-code-write path and immediate-effect semantics need careful tests).

## Open questions & controversies
- Declined twice already (Fusaka, Glamsterdam acde/225) — client teams have so far prioritized other work; re-proposed for Hegota with a new champion (Derek Chiang).
- Delegation chaining/loops are not resolved (inherited from 7702) — factories must guard against targets that are themselves delegations.
- Lifecycle guarantees (immutability, upgradeability, deletion) are entirely up to the calling factory contract, not the protocol.
- Overlap with EIP-8298 (SETCODEFROM) and EIP-7851 may force a "which code-update primitive(s)?" decision rather than independent accept/reject votes.
