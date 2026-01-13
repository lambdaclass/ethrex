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

The runner automatically executes `baseline_loop` and prints an adjusted
`ns/iter` with loop overhead subtracted.

To write CSV output, set `OPCODE_BENCH_CSV`:

```
OPCODE_BENCH_CSV=bench_results.csv cargo run --manifest-path crates/vm/levm/bench/opcode_bench/Cargo.toml --bin opcode_bench -- add 10 100000
```

There is also a Makefile in `crates/vm/levm/bench/opcode_bench/Makefile` with targets to
run all fixtures and save CSV output.

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

Notes:
- For storage opcodes, the first iteration is cold and the remaining iterations are warm.
- The loop uses `PUSH2` jump destinations and expects the code size to fit in 64KB.
