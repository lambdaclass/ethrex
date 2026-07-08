# ethrex-zkevm-bench

Deterministic zkEVM execution benchmark for ethrex's zisk guest program.

It runs the guest under `ziskemu` (CPU emulation, no proving, no GPU, no
network) over real mainnet blocks and EEST micro workloads, capturing the
AIR-cost breakdown (`BASE`/`MAIN`/`OPCODES`/`PRECOMPILES`/`MEMORY`/`TOTAL`)
plus step count as machine-readable JSON. Because emulation is deterministic
for a fixed guest ELF and input, the numbers are stable across runs and
machines, which makes them usable as a regression signal.

## Workload types

Every entry in `fixtures/manifest.toml` has a `type`:

- **`real-block`** — committed, gzipped mainnet blocks in ethrex-replay
  `Cache` format (`fixtures/blocks/*.json.gz`), curated to span the AIR-cost
  spectrum. No download needed.
- **`micro`** — individual EEST zkevm test vectors
  (`tooling/ef_tests/blockchain/vectors_zkevm/`), gitignored downloaded data;
  fetch with `make -C tooling/ef_tests/blockchain zkevm-vectors` (see Setup
  below).
- **`stress`** — worst-case EEST benchmark blocks at 150M gas (one per
  compute/memory/storage/precompile category), committed gzipped under
  `fixtures/stress/` in the same `Cache` format as `real-block`. Generated
  ahead of time with `generate-stress` (see
  [Generating the exhaustive stress set](#generating-the-exhaustive-stress-set-for-slow)
  below).

`real-block` and `stress` both load through the same `Cache` loader
(`src/cache.rs`); `micro` goes through a separate loader for raw EEST test
vectors (`src/micro.rs`). All three types declare their `source` relative to
`fixtures/manifest.toml`'s own directory — `run` resolves them against
wherever `--workloads` points, not the process cwd, so the binary works from
any invocation directory (see [Running](#running) below).

## Setup

1. **ZisK toolchain v0.16.1** (Linux only). From the repo root:

   ```bash
   make zkevm-bench-setup
   ```

   This installs ZisK's apt build dependencies and runs `ziskup -v 0.16.1`
   with `SETUP_KEY=none`, which skips downloading the (large) proving key —
   emulation via `ziskemu` doesn't need it. Afterwards, add `~/.zisk/bin` to
   `PATH` (it provides `ziskemu`):

   ```bash
   export PATH="$HOME/.zisk/bin:$PATH"
   ```

2. **For `micro` workloads only**, the EEST zkevm fixtures under
   `tooling/ef_tests/blockchain/vectors_zkevm/` are gitignored downloaded
   data. Fetch them first:

   ```bash
   make -C tooling/ef_tests/blockchain zkevm-vectors
   ```

   Real-block and stress workloads need no download — their fixtures are
   committed gzipped under `fixtures/blocks/` and `fixtures/stress/`.

## Build

Build from the repo root:

```bash
cargo build -p ethrex-zkevm-bench --features zisk-elf
```

`tooling/zkevm_bench` is a member of the root workspace, but `tooling/`
itself also has its own nested workspace (`tooling/Cargo.toml`, used by
`ef_tests`, `load_test`, etc.) that does *not* include this crate. Running
cargo from inside `tooling/` picks up that nested workspace instead and
won't find `ethrex-zkevm-bench` — always invoke cargo from the repo root.

The `zisk-elf` feature enables `ethrex-guest-program/{zisk-build-elf,ci}`:
`zisk-build-elf` compiles the guest program to the zisk RISC-V target and
embeds it into the benchmark binary, and the `ci` sub-feature skips
`cargo-zisk rom-setup` — a proving-only step that otherwise needs the
proving key skipped in Setup above.

## Running

Workload `source` paths in the manifest resolve relative to the manifest
file itself, so `run` can be invoked from any cwd — e.g. from the repo root:

```bash
cargo run -p ethrex-zkevm-bench --features zisk-elf -- run \
  --mode quick \
  --workloads tooling/zkevm_bench/fixtures/manifest.toml \
  --out r.json
```

No `cd` into `tooling/zkevm_bench` needed. The examples below assume the
binary has already been built (see [Build](#build)) and invoke it directly
from the repo root as `./target/debug/ethrex-zkevm-bench`.

### Tiered modes (`--mode`)

`run` takes a `--mode quick|medium|slow` tier ceiling (default `medium`).
Each workload in the manifest declares an optional `tier` (`quick` or
`medium`; absent means `medium`); `--mode` selects which tiers run:

- **`quick`** — only `tier = "quick"` workloads: a fast, **committed-only**
  sanity subset (~5–10 min) of real blocks + a few stress categories, so it
  needs no downloads (`micro` is deliberately excluded from `quick` because
  it requires `make zkevm-vectors`).
- **`medium`** (default) — `quick` plus untagged/`medium`-tagged
  workloads, i.e. the full committed manifest (~1–2 h).
- **`slow`** — everything `medium` runs, plus (if given) `--stress-dir
  <dir>`, which adds every generated Cache-format fixture found in `<dir>`
  as additional `stress` workloads. This is the exhaustive sweep.

`quick ⊆ medium ⊆ slow` by construction — each wider mode is a superset of
the narrower ones.

```bash
./target/debug/ethrex-zkevm-bench run \
  --workloads tooling/zkevm_bench/fixtures/manifest.toml \
  --mode quick --out quick.json

./target/debug/ethrex-zkevm-bench run \
  --workloads tooling/zkevm_bench/fixtures/manifest.toml \
  --mode slow --stress-dir /path/to/generated-stress --out slow.json
```

### Compare two reports (regression gate)

```bash
./target/debug/ethrex-zkevm-bench compare baseline.json report.json
```

Matches workloads by name, diffs `air_cost.total`, and exits `1` if any
workload regresses beyond `--threshold-pct` (default `3.0`). Pass `--out
diff.json` to also write the per-workload deltas as JSON.

### Curate real-block fixtures

```bash
./target/debug/ethrex-zkevm-bench curate --cache-dir <dir-of-cache_mainnet_*.json> --out curation.json [--ziskemu]
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
      "zkvm_ram_bytes": 7304122,
      "guest_output_ok": true
    }
  ]
}
```

`air_cost.total` equals the sum of `base + main + opcodes + precompiles +
memory`. The example above is a real, verified `mainnet_25087668_light` run.
`gas` is present (and `category` may be `null`) for micro workloads that
carry a fixture-declared gas limit.

`zkvm_ram_bytes` is the guest's peak zkVM memory footprint in bytes, parsed
from ziskemu's `RAM USAGE` line, out of ZisK's ~508 MiB guest RAM budget.
This is the memory metric the source blog (an allocator comparison) centers
on — distinct from `air_cost.memory`, which is the *proving cost* of memory
opcodes, not a footprint measurement.

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
   new file (relative to `fixtures/`, e.g. `blocks/cache_mainnet_<number>.json.gz`).

This is also the recovery path if the `Cache` schema drifts and an existing
fixture stops deserializing (`Block` / `RpcExecutionWitness` no longer
match): regenerate that block's witness against the current tree rather than
dropping the fixture.

## Generating the exhaustive stress set (for `slow`)

`fixtures/stress/*.json.gz` commits one representative fixture per
worst-case category (150M gas each). The execution-specs project publishes a
much larger `tests-benchmark` set — thousands of variants, some far more
expensive than the committed ones (e.g. a 631M-gas selfdestruct case) — which
is generated on demand rather than committed:

1. Download and extract the `tests-benchmark` release's
   `fixtures_benchmark.tar.gz` from the
   [execution-specs releases page](https://github.com/ethereum/execution-specs/releases).
2. Generate Cache-format fixtures from the extracted `blockchain_tests`
   directory using ethrex's own witness generation (no external eth-act
   tool, no zisk toolchain):

   ```bash
   ./target/debug/ethrex-zkevm-bench generate-stress \
     --input-dir <extracted>/blockchain_tests \
     --out-dir <stress-dir>
   ```

3. Point `run --mode slow --stress-dir <stress-dir>` at the output directory
   to include every generated fixture as an additional `stress` workload.

Generation is slow for the heaviest fixtures — the biggest ones (e.g. the
631M-gas selfdestruct case) can take a while to execute and produce a
witness for — and the full set runs to thousands of variants, which is why
it's generated on demand instead of committed.

**Known gap:** the `bls12_381` precompile stress fixture is currently
omitted from the committed set (see the `NOTE` in `fixtures/manifest.toml`)
because ethrex's host-side witness generation needs the `blst` feature
enabled to exercise the BLS12-381 precompile. Enable `blst` and regenerate
via `generate-stress` to add a `worst-precompile-bls` fixture.

To find good candidate blocks, run `curate` with `--ziskemu` over a
directory of candidate `Cache` files — the AIR-cost breakdown it reports
directly indicates which cost component dominates each block.
