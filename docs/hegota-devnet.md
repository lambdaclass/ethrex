# Hegotá devnet branch — caveats

`hegota-devnet` is the integration branch that combines the EIP-8141 frame-transaction work with its extensions for multi-client interop testing. It is **not** an upstream-clean branch: it carries deliberate divergences from the (still-draft) EIPs, listed here. Each standalone EIP PR (`eip-8250`, `eip-8272`, `eip-7906`) targets `eip-8141-1` and is upstream-faithful; the divergences below exist only to make the combined devnet build and run.

## Composition

```
hegota-devnet = eip-8141-devnet (EIP-8141 + devnet fixes)
              + eip-8250  (Keyed Nonces)
              + eip-8272  (Recent Roots, core)
              + eip-7906  (Tx Assertions, opcodes renumbered)
```

**Not yet included:**
- **FOCIL (EIP-7805)** — **deferred.** The EIP-8141 + FOCIL integration is non-trivial and depends on documentation that is not yet public; it will be merged in a later dedicated step (the `focil` branch's main overlap with frame-tx is `payload.rs`).
- **EIP-8288** (PQ sig + STARK aggregation) — deferred (upstream-blocked: no Lean leanSTARK/leanSPHINCS tooling; `AGGREGATED_VK`/hash TBD).

All included EIPs activate together under the existing single `Fork::Hegota` / `hegota_time`.

## Opcode allocation (0xB region)

| Byte | Opcode | EIP | Note |
|------|--------|-----|------|
| `0xAA` | `APPROVE` | 8141 | |
| `0xB0`–`0xB4` | `TXPARAM`/`FRAMEDATALOAD`/`FRAMEDATACOPY`/`FRAMEPARAM`/`SIGPARAM` | 8141 | |
| `0xB5` | `RECENTROOTREFLOAD` | 8272 | spec says `0xB4` (collides with `SIGPARAM`) → ethrex uses `0xB5` |
| `0xB6` | `TXTRACE` | 7906 | **renumbered** from `0xB5` here |
| `0xB7` | `EVENTDATACOPY` | 7906 | **renumbered** from `0xB6` here |
| `0xB8` | `TXDIFF` | 7906 | **renumbered** from `0xB7` here |
| `0xB9` | `NONCEKEYLOAD` | 8250 | **ethrex-only extension** — indexed `nonce_keys[i]`; spec defines no per-index accessor (see `docs/eip-8250.md`) |

The EIP-7906 renumber lives **only on this branch** — the standalone `eip-7906` PR keeps `0xB5`/`0xB6` (it has no knowledge of EIP-8272). The dedup is intentional and documented; `test/tests/levm/eip7906_tests.rs` updates its `TXTRACE`/`EVENTDATACOPY` consts accordingly.

## Per-EIP divergences

### EIP-8250 (Keyed Nonces) — see `docs/eip-8250.md`
- TXPARAM `nonce_keys[0]` at **`0x10`**, not the spec's `0x0B` (which ethrex keeps for `len(signatures)`); pending an upstream TXPARAM registry.
- `NONCE_MANAGER` predeploy at **`0x…8250`** (spec `TBD`).
- ⚠️ **Strict atomic-batch consumption durability not yet implemented** — flagged for devnet/interop validation.

### EIP-8272 (Recent Roots) — see `docs/eip-8272.md`
- `RECENTROOTREFLOAD` at **`0xB5`** (spec `0xB4`); TXPARAM **`0x0F`** (spec summary-table bug says `0x0D`); `RECENT_ROOT_ADDRESS` at **`0x…8272`** (spec `TBD`); `RECENT_ROOT_CODE` handled **natively** (spec `TBD`).
- ⚠️ **Block-execution reference-validity check and the native `RECENT_ROOT_ADDRESS` write are not yet wired** — flagged for devnet/interop validation.

### EIP-7906 (Tx Assertions)
- Opcodes renumbered as above. Behaviour otherwise unchanged.

## Spec pins

EIP-8250 / EIP-8272 → `eips.ethereum.org` master at implementation time (pin the exact commit when frozen). EIP-7906 → its branch. EIP-8141 → `docs/eip-8141.md`'s pin.

## Upstream items

- EIP-8272 TXPARAM `0x0D → 0x0F` fix PR (drafted; from `lambdaclass/EIPs`).
- EIP-8250/8141 TXPARAM `0x0B` conflict (raise for an authoritative registry).
