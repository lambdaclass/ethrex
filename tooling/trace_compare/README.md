# trace_compare

Spot-checks ethrex's `debug_traceTransaction` output against the other EL clients
running side-by-side in a kurtosis enclave. Useful for sanity-checking
[`OpcodeStep`](../../crates/common/tracing.rs) wire-format changes against geth and besu
without leaving the local machine.

## Prereqs

- Docker (OrbStack on Mac, or Docker Desktop)
- `kurtosis` CLI (`brew install kurtosis-tech/tap/kurtosis-cli`)
- `curl`, `jq`

## Usage

```bash
# 1. Start a multi-client enclave (~5 min on first run, builds the ethrex image)
make localnet

# 2. Once the chain is producing blocks, compare a tx across every EL
tooling/trace_compare/compare.sh
# auto-discovers el-* services in the `lambdanet` enclave, auto-picks a tx
# from `latest`, traces it on every client, prints suggested diffs.

# Trace a specific tx
tooling/trace_compare/compare.sh --tx 0xabcd...
```

## What it does

1. Parses `kurtosis enclave inspect lambdanet` to find every `el-*` service and
   its host-mapped RPC port (`kurtosis port print … rpc` / `… http`).
2. Picks the first tx from the latest block (via the first discovered client) if
   `--tx` wasn't supplied.
3. Calls `debug_traceTransaction` against each client's RPC and saves the
   responses to `trace-compare-<timestamp>/<service>.json`.
4. Prints suggested pairwise `diff` commands for `structLogs` only (the
   wrapper fields can introduce noise that's not interesting for tracer work).

## What divergences mean

After [EIP-3155 alignment](https://eips.ethereum.org/EIPS/eip-3155) for
`OpcodeStep` (commit `dc11a20e1` on `feat/eip-3155-tracer`), per-step output
should match geth byte-for-byte on the fields it has in common
(`pc`, `op`, `gas`, `gasCost`, `depth`, `stack`, `memSize`, `returnData`,
`refund`, `opName`). Besu emits the same shape but sometimes with extra fields.

Diffs in the wrapper (`failed` / `gas` / `returnValue` / `structLogs`) are
expected to match — all three clients emit the geth structLogger wrapper.

Step-count differences usually point at fused-opcode handling (cf. the
[JUMPDEST regression test](../../test/tests/levm/opcode_tracer_tests.rs))
or at a real divergence in execution.
