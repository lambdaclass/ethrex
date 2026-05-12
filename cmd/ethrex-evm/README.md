# ethrex-evm

A standalone EVM CLI for the `ethrex` execution client, intended as a
drop-in differential-fuzzing target for
[`holiman/goevmlab`](https://github.com/holiman/goevmlab).

The binary exposes one subcommand: `statetest`. It accepts the exact
invocation goevmlab's `evms/geth.go` adapter uses, reads
GeneralStateTest JSON, runs each `(fork, subtest)` through LEVM, and
streams an EIP-3155 trace plus a `{"stateRoot": "0x..."}` terminator
on stderr.

## Build

```bash
cargo build -p ethrex-evm --bin ethrex-evm --release
# binary at target/release/ethrex-evm
```

## Usage

```bash
ethrex-evm statetest --trace --trace.format=json \
    --trace.nomemory=true --trace.noreturndata=true \
    path/to/StateTest.json
```

Stdin batch mode (one path per line, EOF or blank line terminates):

```bash
echo "path/to/test1.json
path/to/test2.json" | ethrex-evm statetest --trace --trace.format=json
```

### Flags

| Flag | Default | Notes |
|---|---|---|
| `--trace` | off | Enable EIP-3155 streaming. Bare boolean. |
| `--trace.format` | `json` | Only `json` accepted; other values exit 1. |
| `--trace.nomemory` | `true` | Suppress `memory` in steps. |
| `--trace.memory` | `false` | Opt-in alias for the inverse. |
| `--trace.nostack` | `false` | Suppress `stack`. |
| `--trace.noreturndata` | `true` | Suppress `returnData`. |
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
| `opName` | string | Mnemonic; fallback `"opcode 0xNN not defined"` |
| `gas` | hex string | Gas remaining before opcode |
| `gasCost` | hex string | Charged for this opcode |
| `memSize` | number | Bytes |
| `stack` | array of hex strings | Bottom-first; omitted when `--trace.nostack=true` |
| `memory` | hex string | Single contiguous blob; omitted unless enabled |
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

The literal colon-space in `"stateRoot": "` is required for goevmlab's
`ParseStateRoot` byte search.

## Supported transaction shapes

GeneralStateTest vectors using any of these execute end-to-end:

- Legacy / EIP-1559 / EIP-2930 (envelope unified via `EIP1559Transaction`).
- EIP-4844 blob txs (`blobVersionedHashes`, `maxFeePerBlobGas`, `currentExcessBlobGas`).
- EIP-7702 setcode txs (`authorizationList` with `v` or `yParity`).
- Vectors that ship a pre-derived `sender` field instead of `secretKey`.

## goevmlab integration

`ethrex-evm` is binary-compatible with goevmlab's invocation contract.
To register it as a fuzzing target in a goevmlab fork:

1. Build the binary: `cargo build -p ethrex-evm --bin ethrex-evm --release`.
2. Add a goevmlab `evms/ethrex.go` adapter modeled after `evms/geth.go`,
   pointing at the binary path.
3. Run goevmlab's state-fuzzer — it will diff `ethrex-evm`'s output
   against the other configured clients.

The upstream goevmlab adapter PR is tracked separately; this repo only
ships the binary.

## Future work

- `run` subcommand for raw-bytecode debugging.
- `t8n` subcommand.
- CI workflow running ethrex-evm against a goevmlab fuzz corpus nightly.
- Upstream `evms/ethrex.go` in goevmlab.
