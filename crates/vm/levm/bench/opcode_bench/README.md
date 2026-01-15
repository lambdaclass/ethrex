# opcode micro bench

Opcode microbench harness for LEVM using looped bytecode.

## Usage

List fixtures:

```
cargo run --manifest-path crates/vm/levm/bench/opcode_bench/Cargo.toml --bin opcode_bench -- list
```

Run a fixture:

```
cargo run --manifest-path crates/vm/levm/bench/opcode_bench/Cargo.toml --bin opcode_bench -- add 10 100000
```

Run all fixtures in a category:

```
cargo run --manifest-path crates/vm/levm/bench/opcode_bench/Cargo.toml --bin opcode_bench -- category:calldata 10 100000
```

The runner automatically executes `baseline_loop` (using the same loop counter
offset as the fixture) and prints an adjusted `ns/iter` with loop overhead
subtracted. The output also includes `ns/op` when fixtures use `repeat`.

To write CSV output, set `OPCODE_BENCH_CSV`:

```
OPCODE_BENCH_CSV=bench_results.csv cargo run --manifest-path crates/vm/levm/bench/opcode_bench/Cargo.toml --bin opcode_bench -- add 10 100000
```

There is also a Makefile in `crates/vm/levm/bench/opcode_bench/Makefile` with targets to
run all fixtures and save CSV output.

Optional environment variables:
- `OPCODE_BENCH_WARMUP`: number of warmup runs to execute before timing (default: 0).
- `OPCODE_BENCH_RESET_DB`: if set to `1`/`true`, reinitialize the DB per run to avoid
  cross-run warm state (default: false).

For per-fixture parameters, edit `crates/vm/levm/bench/opcode_bench/run_config.json`
and use:

```
make -C crates/vm/levm/bench/opcode_bench csv-config CSV=bench_results.csv
```

To compare two CSV runs and report percent change:

```
crates/vm/levm/bench/opcode_bench/scripts/compare_csv.py baseline.csv comparison.csv
```

Arguments:
- fixture name
- category: `category:<name>` or `cat:<name>`
- repetitions (default: 10)
- iterations per run (default: 100000)

## Fixture format

Each fixture lives in `fixtures/*.json` and defines the opcode body that is
inserted into a standard loop. The harness wraps the body with a loop that
stores a counter in memory slot `0x80` and decrements it each iteration to
avoid clobbering opcode fixtures that use low memory.

Fields:
- `name`: fixture name
- `category`: fixture category (e.g. arithmetic, calldata, code, environment, storage, precompile)
- `description`: optional description
- `body_hex`: bytecode hex for the opcode body, including any stack setup and cleanup
- `calldata_hex`: optional calldata hex
- `repeat`: optional number of times to repeat `body_hex` inside the loop (default: 1)
- `counter_offset`: optional memory offset for the loop counter (default: `0x80`)
- `storage`: optional array of `{ key, value }` 32-byte hex strings
- `env`: optional overrides for environment fields:
  - `block_number`, `timestamp`, `difficulty`, `chain_id`, `base_fee_per_gas`, `gas_price`
    (decimal strings or `0x`-prefixed hex)
  - `coinbase` (20-byte hex), `prev_randao` (32-byte hex)
  - `gas_limit`, `block_gas_limit` (u64 JSON numbers)

Notes:
- For storage opcodes, the first iteration is cold and the remaining iterations are warm.
- The loop uses `PUSH2` jump destinations and expects the code size to fit in 64KB.
