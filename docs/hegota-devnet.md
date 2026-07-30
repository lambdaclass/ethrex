# Hegotá devnet branch — caveats

`hegota-devnet` is the integration branch that combines the EIP-8141 frame-transaction work with its extensions for multi-client interop testing. It is **not** an upstream-clean branch: it carries deliberate divergences from the (still-draft) EIPs, listed here. Each standalone EIP PR (`eip-8250`, `eip-8272`, `eip-7906`) targets `eip-8141-1` and is upstream-faithful; the divergences below exist only to make the combined devnet build and run.

## Composition

```
hegota-devnet = main       (EIP-8141 frame transactions)
              + eip-8250   (Keyed Nonces)
              + eip-8272   (Recent Roots)
              + eip-7906   (Tx Assertions, opcodes renumbered)
              + devnet-only config, docs, scripts and the
                ethrex-only extensions listed below
```

EIP-8141 itself lives on `main`; this branch adds only the three extension EIPs
on top of it, plus the devnet infrastructure and the ethrex-only extensions.

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

The EIP-7906 renumber lives **only on this branch** — the standalone `eip-7906` PR keeps the spec's `0xB5`/`0xB6`/`0xB7` (it has no knowledge of EIP-8272). The dedup is intentional and documented; `test/tests/levm/eip7906_tests.rs` and `crates/vm/levm/src/opcode_handlers/tx_trace.rs` carry the shifted bytes accordingly.

## Per-EIP divergences

### EIP-8250 (Keyed Nonces) — see `docs/eip-8250.md`
- TXPARAM `nonce_keys[0]` at **`0x10`**, not the spec's `0x0B` (which ethrex keeps for `len(signatures)`); pending an upstream TXPARAM registry.
- `NONCE_MANAGER` predeploy at **`0x…8250`** (spec `TBD`).
- ⚠️ **Strict atomic-batch consumption durability not yet implemented** — flagged for devnet/interop validation.

### EIP-8272 (Recent Roots) — see `docs/eip-8272.md`
- `RECENTROOTREFLOAD` at **`0xB5`** (spec `0xB4`); TXPARAM **`0x0F`** (spec summary-table bug says `0x0D`); `RECENT_ROOT_ADDRESS` at **`0x…8272`** (spec `TBD`); `RECENT_ROOT_CODE` handled **natively** (spec `TBD`).

### EIP-7906 (Tx Assertions)
- Opcodes renumbered as above. Behaviour otherwise unchanged.

## Spec pins

EIP-8250 / EIP-8272 → `eips.ethereum.org` master at implementation time (pin the exact commit when frozen). EIP-7906 → its branch. EIP-8141 → `docs/eip-8141.md`'s pin.

## Upstream items

- EIP-8272 TXPARAM `0x0D → 0x0F` fix PR (drafted; from `lambdaclass/EIPs`).
- EIP-8250/8141 TXPARAM `0x0B` conflict (raise for an authoritative registry).

## Deploying

`hegota-devnet-genesis.md` covers what a fresh genesis must contain: the fork
schedule constraints, the predeploys the genesis generator does not ship, the
ethrex-only chain-config fields, and the post-deployment verification pass.

## Running a single Hegotá node locally

`--dev` needs no consensus client, which makes it the quickest way to reproduce
something on a Hegotá chain. Two things must be right or the node dies within
seconds of startup, both fatally and with a misleading error:

1. **Amsterdam must be scheduled.** Hegotá inherits Amsterdam's rules through the
   fork ordinal, but leaving `amsterdamTime` unset means the EIP-8038 repricing
   and the concurrent BAL-validation path never activate — so a local run can
   silently exercise different code from the devnet. Set both `amsterdamTime` and
   `hegotaTime`.
2. **The EIP-8282 builder deposit/exit predeploys must be preloaded**, with
   `EXCESS_INHIBITOR` (`2**256-1`) in slot 0. The genesis generator does not
   deploy them, and their empty code on an Amsterdam+ block makes the end-of-block
   system call fail: `System contract: 0x0000…8282 has no code after deployment`,
   which surfaces as `engine_getPayloadV5` failing rather than as anything about
   genesis. Copy them from the `additional_preloaded_contracts` block in
   `fixtures/networks/hegota-devnet.yaml`.

Derive the genesis from `fixtures/genesis/l1.json` (Osaka-era, so only the two
fork times need adding), then:

```bash
cargo run --release --features dev -- --dev --network <genesis.json> --datadir memory
```

A healthy node logs `Produced block` and its headers carry both `slotNumber`
(EIP-7843) and `blockAccessListHash` (EIP-7928). The producer stands in for the
slot clock, advancing one slot per block.

The dev producer is fatal by design: three consecutive failures shut the node
down, so a genesis problem looks like an immediate exit rather than a stalled
chain. Read the three `Failed to produce block` lines above the shutdown — they
name the engine-API call that rejected the payload.
