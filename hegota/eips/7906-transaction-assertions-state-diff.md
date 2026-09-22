# EIP-7906: Transaction Assertions via State Diff Opcode
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acde/242, 2026-07-30)
- **Authors:** Alex Forshtat, Shahaf Nacson, Dror Tirosh, Yoav Weiss, Fredrik Svantes
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7906) · [discussion](https://ethereum-magicians.org/t/eip-restricted-behavior-transaction-type/23130)
- **Prior fork history:** none

## TL;DR
Adds a new `POST_TX` frame mode to EIP-8141 frame transactions plus three opcodes (`TXTRACE`, `TXDIFF`, `EVENTDATACOPY`) that let a contract inspect the complete state diff (balances, storage, deployments, events) produced by the transaction. Wallets can attach an assertion frame at the end of a tx that reverts the whole execution body if the outcome isn't what the user expected — replacing blind signing with enforceable outcome constraints.

## What it changes
- **New frame mode `POST_TX` (mode 3)** amending EIP-8141: POST_TX frames must form a contiguous trailing suffix of `tx.frames`; run as STATICCALL with `ENTRY_POINT` caller; cannot call `APPROVE`. A POST_TX failure does **not** invalidate the transaction — it reverts the execution body unconditionally (overriding atomic-batch unrolling), commits the validation prefix (gas payment, deploys), and yields a `status = 0` receipt.
- **`TXTRACE` opcode:** `(param, index)` introspection of the full transaction diff — counts and enumeration of changed balances, changed `(address, slot)` pairs (with before/after values, sorted by address then slot), deployed contracts (address + codehash), and events (address, topics, data length); plus `gas_pre_charge` and `gas_payer_address` to isolate real ETH transfers from the gas deduction.
- **`TXDIFF` opcode:** direct keyed lookup of before/after slot values, balance, codehash for a specific address; per-address views over slots and events (count + global index mapping); `account_change_flags` bitmask (nonce/balance/storage/codehash) enabling one-call "this account was untouched" shield assertions. Gas follows EIP-2929 warm/cold when falling back to live state.
- **`EVENTDATACOPY` opcode:** copies an event's non-indexed data into memory, CALLDATACOPY semantics and pricing.
- All three opcodes exceptional-halt outside POST_TX frames, including in any legacy tx type. Under EIP-8141's default code, POST_TX frames behave like SENDER/DEFAULT.

## Motivation
Users sign transactions whose behavior they cannot verify; signing is de-facto blind. An on-chain, enforcible assertion over the transaction's actual outcome lets wallets guarantee "at most X tokens leave my account" style policies, closing a large class of drainer/phishing attacks. Multiple independent assertion providers compose without coordination because POST_TX frames form a suffix and any failure reverts everything.

## Dependencies & related EIPs
- **Depends on (hard):** EIP-8141 — the opcodes only exist inside POST_TX frames. Cannot ship without frame transactions.
- **Builds on:** EIP-2929 (access-list pricing for TXDIFF fallback reads); interacts with EIP-7928 (BALs, Glamsterdam) — TXDIFF accesses are recorded in the block-level access list like other reads.
- **In-cluster:** extends the 8141 frame-mode space alongside 8250/8272 (which touch nonce/payload instead).
- **Related:** similar spirit to EIP-7668/7484-style wallet security ideas; complementary to any assertion/privacy tooling built on 8272 recent roots.

## Impact on ethrex / client teams
Medium. The opcodes themselves are straightforward additions to the LEVM interpreter (`crates/vm/levm/src/opcode_handlers/`), but they need a transaction-scoped state-diff tracker: record prestate values for every balance/slot/code touched and the emitted logs, kept in sync with journal checkpoints and atomic-batch rollbacks so the diff reflects the current post-execution state deterministically (sorted enumeration). ethrex's existing EIP-7928 BAL machinery is a natural foundation but the semantics differ (net-diff-at-tx-start vs. per-block index), so it's not free. Also: POST_TX frame mode in the frame execution loop, receipt handling for the status-0-with-committed-prefix case, and gas costs are still TBD. Effort: **medium** on top of 8141; trivial if 8141 isn't shipped (inapplicable).

## Open questions & controversies
- Gas costs for TXTRACE/EVENTDATACOPY are TBD; worst-case enumeration cost over attacker-padded logs (~42k events/tx) is a real DoS/UX concern, mitigated by TXDIFF per-address views but not eliminated.
- A failed assertion still charges the user full gas for the failed execution — expected, but a UX footgun.
- "False sense of security" concern: incomplete assertions may be worse than none; ecosystem conventions needed.
- Nothing enforces inclusion of a POST_TX frame; smart accounts must require it in their VERIFY logic, and the assertion target must be immutable — correctness lives at the wallet layer, not the protocol.
- Adds a novel EVM capability (cross-contract outcome introspection) that breaks code encapsulation; some pushback expected on principle.
