# ethrex-evm

A standalone EVM CLI for the `ethrex` execution client, intended primarily
as a drop-in differential-fuzzing target for
[`holiman/goevmlab`](https://github.com/holiman/goevmlab).

Two subcommands:

| Subcommand | Purpose |
|---|---|
| `statetest` | Execute one or more GeneralStateTest JSON files, streaming an EIP-3155 trace and a `{"stateRoot": "0x..."}` terminator on stderr. This is the entry point every goevmlab adapter invokes. |
| `run` | Execute raw EVM bytecode in a minimal pre-state, byte-compatible with `geth evm run --json`. Useful for local debugging and golden-file differential tests. |

## Build

```bash
cargo build -p ethrex-evm --bin ethrex-evm --release
# binary at target/release/ethrex-evm
```

The binary has no runtime dependencies beyond the workspace; it embeds an
in-memory `ethrex-storage` store and runs LEVM directly.

## `statetest` — the goevmlab entry point

### Invocation

```bash
ethrex-evm statetest --trace --trace.format=json \
    --trace.nomemory=true --trace.noreturndata=true \
    path/to/StateTest.json
```

This matches the exact invocation goevmlab's `evms/geth.go` adapter
uses (with the binary path substituted). For each `(fork, subtest)` in the
file, the binary:

1. Builds the pre-state from `pre`.
2. Runs the tx through LEVM with an EIP-3155 streaming tracer attached.
3. Writes each opcode step to stderr as a JSON line.
4. Writes a summary line `{"output": "<hex>", "gasUsed": "0x...", "error?": "..."}`.
5. Writes a terminator line `{"stateRoot": "0x..."}` (note the literal
   `colon-space` — goevmlab parses this verbatim).

### Stdin batch mode

When no positional path is supplied, paths are read from stdin
one-per-line until EOF or a blank line. This is what
`gethbatcher.go` drives:

```bash
echo "path/to/test1.json
path/to/test2.json" | ethrex-evm statetest --trace --trace.format=json
```

### Flags

| Flag | Default | Notes |
|---|---|---|
| `--trace` | off | Enable EIP-3155 streaming. Bare boolean (geth-style). |
| `--trace.format` | `json` | Only `json` accepted; other values exit 1. |
| `--trace.nomemory` | `true` | Suppress `memory` in steps (geth default). |
| `--trace.memory` | `false` | Opt-in alias for the inverse. |
| `--trace.nostack` | `false` | Suppress `stack`. |
| `--trace.noreturndata` | `true` | Suppress `returnData` (geth default). |
| `--trace.nostorage` | `false` | Suppress storage diffs. |
| `--statetest.fork` | _all forks_ | Limit to one fork (e.g. `Prague`). |
| `--statetest.index` | _all subtests_ | Limit to one subtest by index. |
| `--run` | _match all_ | Regex applied to test names. |

### EIP-3155 line schema

Each opcode step is one `\n`-terminated JSON object:

```json
{"pc":4,"op":1,"gas":"0x2540be3fa","gasCost":"0x3","memSize":0,"stack":["0x1","0x1"],"depth":1,"refund":0,"opName":"ADD"}
```

| Field | Encoding | Notes |
|---|---|---|
| `pc` | number | Program counter, decimal |
| `op` | number | Raw byte value (e.g. `96` for PUSH1) |
| `opName` | string | Mnemonic (e.g. `"PUSH1"`); fallback `"opcode 0xNN not defined"` |
| `gas` | hex string | Gas remaining before opcode |
| `gasCost` | hex string | Charged for this opcode |
| `memSize` | number | Bytes |
| `stack` | array of hex strings | Bottom-first; omitted when `--trace.nostack=true` |
| `memory` | hex string | Single contiguous blob; omitted unless `--trace.memory=true` or `--trace.nomemory=false` |
| `returnData` | hex string | Omitted unless enabled |
| `depth` | number | Call depth (1 = top) |
| `refund` | number | Refund counter |
| `error` | string | Present iff the step errored |

Summary line (after the last opcode):

```json
{"output":"<hex without 0x>","gasUsed":"0x...","error":"..."}
```

State-root terminator (after the summary):

```json
{"stateRoot": "0x<64 hex chars>"}
```

The colon-space in `"stateRoot": "` is required for goevmlab's
`ParseStateRoot` literal byte search.

## `run` — bytecode-only execution

```bash
ethrex-evm run --json 0x6001600101
```

Executes raw bytecode against a synthetic sender (`"sender"` left-padded
to 20 bytes, balance `u128::MAX`) calling a synthetic receiver (`"receiver"`
left-padded) whose code is the bytecode argument. Matches geth's
`evm run --json` byte-for-byte for the same input.

### Sources for the bytecode

In priority order (mirrors geth):

1. `--codefile -` — read from stdin
2. `--codefile <path>` — read from a file
3. Positional argument

Accepts `0x` prefix and surrounding whitespace.

### Flags

| Flag | Default | Notes |
|---|---|---|
| `--json` | off | Stream EIP-3155 on stderr. Without it, prints only the final output bytes on stdout. |
| `--codefile <path>` | _none_ | Read bytecode from file. `-` for stdin. |
| `--input <hex>` | empty | Calldata. |
| `--gas <N>` | `10_000_000_000` | Decimal. Matches geth's `GasFlag` default. |
| `--value <hex-or-dec>` | `0` | Accepts both encodings (matches `math.HexOrDecimal256`). |
| `--sender <addr>` | `"sender"` left-padded | Tx origin. |
| `--receiver <addr>` | `"receiver"` left-padded | Tx recipient. |
| `--nomemory` | `true` | |
| `--nostack` | `false` | |
| `--noreturndata` | `true` | |
| `--statdump` | off | After execution, print `EVM gas used: ...` to stderr. |
| `--ethrex-fork <name>` | `Prague` | ethrex-specific extension. |

### Gas accounting

geth's `evm run` calls the EVM interpreter directly without deducting tx
intrinsic gas. LEVM always runs the full tx-prepare path, which deducts
21000 + calldata cost. The binary compensates by adding intrinsic gas to
the supplied `--gas` before execution and subtracting it from `gasUsed`
in the summary, producing byte-identical traces.

This compensation assumes the fork is Amsterdam-or-earlier (the EIP-8037
reservoir model would interfere). Default `Prague` is safe; using
`--ethrex-fork=Osaka` or newer is unsupported by `run`.

## goevmlab integration

`ethrex-evm` is binary-compatible with goevmlab's invocation contract;
no upstream changes to goevmlab are required to drive it from a test
harness. To register `ethrex` as a fuzzing target in your own goevmlab
fork:

1. Build the binary: `cargo build -p ethrex-evm --bin ethrex-evm --release`.
2. Add a goevmlab `evms/ethrex.go` adapter modeled after `evms/geth.go`,
   pointing at the binary path.
3. Run goevmlab's state-fuzzer: it will diff `ethrex-evm`'s output
   against the other configured clients.

The upstream goevmlab adapter PR is tracked separately; this repo only
ships the binary.

## Test fixtures

`tests/fixtures/` holds:

- `statetest_simple.json` — a minimal Shanghai transfer pinned for unit testing.
- `run_push_add.geth.jsonl` — golden capture from `geth v1.17.3-stable`
  for `evm run --json 0x6001600101`. The `run` subcommand's output is
  byte-compared against this file in CI.
- `GETH_VERSION.txt` — pins the geth version the goldens were captured from.

To regenerate the golden after a future geth bump:

```bash
geth-evm run --json 0x6001600101 2> tests/fixtures/run_push_add.geth.jsonl
echo "geth $(geth-evm --version | head -1) — captured $(date -I)" \
    > tests/fixtures/GETH_VERSION.txt
```

## Known limitations

- **`t8n` subcommand not implemented.** Tracked as a follow-up; goevmlab
  does not need it.
- **Cross-fork gas-compensation in `run`** only valid up to
  Amsterdam-style accounting. Default fork is `Prague`.
- **Per-subtest tokio runtime** is constructed inside `compute_post_state_root`
  on every test. Fine at unit-test scale; should be amortized via a
  process-level runtime when running large fuzz corpora. Tracked as a
  follow-up perf item.

## Future work

- **CI workflow** running ethrex-evm against a goevmlab fuzzing corpus
  nightly, diffing against `geth evm`. Out of scope for the initial
  binary PR.
- **Upstream `evms/ethrex.go`** in goevmlab — once this binary lands.
- **`t8n` subcommand** if a future use case requires it (statetest
  covers goevmlab today).
