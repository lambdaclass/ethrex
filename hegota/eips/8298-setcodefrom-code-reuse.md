# EIP-8298: SETCODEFROM Code Reuse Instruction
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/240, 2026-07-02
- **Authors:** Liyi Guo, Ben Adams, Carlos Perez, Nicolas Consigny
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8298) · [discussion](https://ethereum-magicians.org/t/eip-8298-setcodefrom-code-reuse-instruction/28779)
- **Prior fork history:** none (created 2026-06-11, very new)

## TL;DR
Adds `SETCODEFROM`, a runtime-only instruction that sets the *current* account's code hash to that of an already-deployed contract, without paying code-deposit gas. Two use cases: cheap factory clones that initialize storage then adopt shared code, and EOA migration that installs regular wallet code — which permanently disables ECDSA transaction authority as a side effect (via EIP-3607).

## What it changes
- New instruction `SETCODEFROM(source)` (opcode byte TBD): pops one stack item (low 160 bits = `source` address), pushes `success` (1/0).
- Sets the current execution-context account's `codeHash` to `source.codeHash`. "Current account" = the `ADDRESS` account whose storage SSTORE writes — so delegated (7702-style) execution updates the calling account, not the code source.
- Valid source: not a precompile, `codeHash != EMPTYCODEHASH`, and code is valid regular deployed code under the active fork (no `0xEF` prefix — excludes 7702 indicators and EIP-3541-reserved code).
- Exceptional halt if executed in initcode or in a static context; invalid source pushes 0 with no state change. Normal revert semantics; frame-local effect (current frame runs its loaded code; later calls in the same tx see the new code).
- Gas: `SETCODEFROM_GAS = 3000 base + warm/cold account access` → 5600 cold / 3100 warm under current EIP-2929 constants. Base 3000 is justified as 100 (validity checks, warm-read-like) + 2900 (warm nonzero-to-nonzero SSTORE proxy for the code-hash update); no code-deposit cost since no new bytecode is stored.
- Address-based (not raw code hash) deliberately: the adopted code hash is confirmed by live consensus state, not a client-local code DB that may differ across nodes.

## Motivation
- Deploying many contracts with identical runtime code wastes state and gas, especially under deployment repricing (EIP-8037). Factory pattern: deploy minimal initializer runtime → initialize per-instance storage → `SETCODEFROM` to the shared implementation.
- Account migration: migration code writes wallet-specific state (e.g. post-quantum pubkey slots), then adopts regular wallet code. Because the account then holds *regular* code (not a delegation indicator), EIP-3607 permanently blocks ECDSA-originated transactions and EIP-7702 redelegation no longer applies — a protocol-level "burn the EOA key" path.

## Dependencies & related EIPs
- Requires (forkcast): EIP-3529, EIP-3607, EIP-6780, EIP-7702, EIP-8037, EIP-8038. Spec header lists EIP-2200, EIP-2929, EIP-3607, EIP-7702. The 3607/7702 interactions are the load-bearing ones.
- Companion: EIP-8151 — `ecrecover` change so permit-style contracts reject accounts with regular deployed code (migrated accounts still recover via `ecrecover` without it).
- Overlaps / competes with: EIP-7851 (alternative key-retirement mechanism via a second delegation prefix) and EIP-7819 (SETDELEGATE; protocol-level delegation instead of code adoption). Differs from older EIP-6913 (SETCODE) by allowing delegated execution to rewrite the caller's code — needed for proxy-upgrade patterns but riskier.
- Synergizes with EIP-8141 (native AA frame txs) as an account-migration building block.

## Impact on ethrex / client teams
- LEVM: new opcode handler that mutates the current account's code hash in the state cache, with strict separation between the frame's loaded code and account code for later calls; interplay with the existing 7702 delegation resolution (`crates/vm/levm/src/utils.rs`, `account.rs`).
- Validation: none new (EIP-3607 rejection of senders-with-code already exists); but re-entrancy and same-tx code-swap semantics need tests.
- Gas accounting: one new constant set.
- Effort: medium — small surface but subtle semantics (frame-local code, delegated-execution authority, revert handling).

## Open questions & controversies
- Very young EIP (June 2026): opcode byte TBD, no test cases section filled in, spec likely to move.
- Security: any code exposing `SETCODEFROM` can permanently change account behavior; delegated modules become upgrade-authorized code — wallets must restrict delegatecall targets accordingly.
- Introduces a "codehash-presentation" risk: an account's observed code identity can change mid-transaction, departing from 7702's conservative introspection design.
- Hegota likely must choose among 7819 / 7851 / 8298 rather than take all three.
