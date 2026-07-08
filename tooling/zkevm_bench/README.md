# ethrex-zkevm-bench

Deterministic zkEVM execution benchmark for ethrex's zisk guest program.

It runs the guest under `ziskemu` (CPU emulation, no proving, no GPU, no
network) over real mainnet blocks and EEST micro workloads, capturing the
AIR-cost breakdown (`BASE`/`MAIN`/`OPCODES`/`PRECOMPILES`/`MEMORY`/`TOTAL`)
plus step count as machine-readable JSON. Because emulation is deterministic
for a fixed guest ELF and input, the numbers are stable across runs and
machines, which makes them usable as a regression signal.

## Prerequisites

1. **ZisK toolchain v0.16.1**, installed via `ziskup`:

   ```bash
   curl -fsSL https://raw.githubusercontent.com/0xPolygonHermez/zisk/v0.16.1/ziskup/ziskup -o ziskup
   SETUP_KEY=none ziskup -v 0.16.1
   ```

   `SETUP_KEY=none` skips downloading the (large) proving key — emulation via
   `ziskemu` doesn't need it. Make sure `~/.zisk/bin` is on `PATH` (it provides
   `ziskemu`).

2. **The guest ELF**, built into the benchmark binary via the `zisk-elf`
   cargo feature. This feature enables
   `ethrex-guest-program/{zisk-build-elf,ci}`: `zisk-build-elf` compiles the
   guest program to the zisk RISC-V target, and the `ci` sub-feature skips
   `cargo-zisk rom-setup` — a proving-only step that otherwise needs the
   proving key we just skipped installing.

3. **For `micro` workloads only**, the EEST zkevm fixtures under
   `../ef_tests/blockchain/vectors_zkevm/` are gitignored downloaded data.
   Fetch them first:

   ```bash
   make -C ../ef_tests/blockchain zkevm-vectors
   ```

   Real-block workloads need no download — their fixtures are committed
   gzipped under `fixtures/blocks/`.

## Running

Build from the workspace root, then run from the crate directory —
`fixtures/manifest.toml`'s `source` paths are relative to
`tooling/zkevm_bench`, which is the intended cwd for a run:

```bash
cargo build -p ethrex-zkevm-bench --features zisk-elf
cd tooling/zkevm_bench
../../target/debug/ethrex-zkevm-bench run --workloads fixtures/manifest.toml --out report.json
```

### Compare two reports (regression gate)

```bash
../../target/debug/ethrex-zkevm-bench compare baseline.json report.json
```

Matches workloads by name, diffs `air_cost.total`, and exits `1` if any
workload regresses beyond `--threshold-pct` (default `3.0`). Pass `--out
diff.json` to also write the per-workload deltas as JSON.

### Curate real-block fixtures

```bash
../../target/debug/ethrex-zkevm-bench curate --cache-dir <dir-of-cache_mainnet_*.json> --out curation.json [--ziskemu]
```

Scans a directory of ethrex-replay `cache_mainnet_*.json` files, records
size/gas/tx-count/precompile-tx-count for each, and — with `--ziskemu` —
also runs each block through `ZiskBackend::execute_profiled` and records its
AIR-cost breakdown. Used to select which blocks to commit as fixtures (see
[Real-block fixtures](#real-block-fixtures) below).

## Output JSON schema

```json
{
  "meta": {
    "zisk_version": "v0.16.1",
    "guest_elf_sha256": "<sha256 of the built guest ELF>",
    "generated_by": "ethrex-zkevm-bench",
    "git_commit": "<optional, from $GIT_COMMIT>"
  },
  "workloads": [
    {
      "name": "mainnet_25087668_light",
      "type": "real-block",
      "category": "light",
      "air_cost": {
        "base": 293601280,
        "main": 581978408,
        "opcodes": 129090301,
        "precompiles": 218288244,
        "memory": 80012388,
        "total": 1302970621
      },
      "steps": 8558506,
      "guest_output_ok": true
    }
  ]
}
```

`air_cost.total` equals the sum of `base + main + opcodes + precompiles +
memory`. The example above is a real, verified `mainnet_25087668_light` run.
`gas` is present (and `category` may be `null`) for micro workloads that
carry a fixture-declared gas limit.

If a workload's input fails to build or the guest execution errors, it still
appears in the report with `guest_output_ok: false` and zeroed `air_cost` /
`steps` — one bad fixture doesn't abort the whole run.

## Determinism

A fixed guest ELF plus a fixed input always produces the same AIR-cost
breakdown and step count — `ziskemu` emulation has no wall-clock or
randomness component. Because cost numbers are specific to the exact ZisK
toolchain and guest ELF build, every report's `meta.guest_elf_sha256`
records the sha256 of the ELF that produced it; only compare reports that
share the same ELF hash (and ideally the same `zisk_version`).

## Real-block fixtures

`fixtures/blocks/*.json.gz` are committed, gzipped ethrex-replay `Cache`
JSON files — each holds a block, its `RpcExecutionWitness`, and the source
network. They were curated from mainnet to span both the AIR-cost spectrum
and the dominant cost component: `light`/`small`/`typical` cost tiers, plus
`compute` (high MAIN/OPCODES, low precompiles), `memory` (highest memory
fraction), `tx-heavy` (most transactions), and `large-state` (biggest
witness/state footprint).

To regenerate a fixture or add a new one:

1. Produce a `Cache` file for the target block — e.g. via the
   `ethrex-replay` tool's `cache` subcommand, which fetches
   `debug_executionWitness` from a node for the block(s) you specify.
2. `gzip` it into `fixtures/blocks/` (`cache_mainnet_<number>.json.gz`).
3. Add a `[[workload]]` entry to `fixtures/manifest.toml` with a descriptive
   `name`, `type = "real-block"`, a `category`, and `source` pointing at the
   new file.

This is also the recovery path if the `Cache` schema drifts and an existing
fixture stops deserializing (`Block` / `RpcExecutionWitness` no longer
match): regenerate that block's witness against the current tree rather than
dropping the fixture.

To find good candidate blocks, run `curate` with `--ziskemu` over a
directory of candidate `Cache` files — the AIR-cost breakdown it reports
directly indicates which cost component dominates each block.
