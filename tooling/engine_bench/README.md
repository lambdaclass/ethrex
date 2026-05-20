# engine_bench

Benchmark harness for the ethrex engine API. Compares JSON-RPC vs REST/SSZ
transports across the four hot-path workloads (`newPayload`, `getPayload`,
`blobs`, `bodies`).

## Prerequisites

- A running ethrex on a branch that implements the REST/SSZ engine API
  (sub-projects 1+2+3 of `feat/engine-rest-ssz-foundation`).
- The JWT secret file used by that ethrex (typically `<datadir>/jwt.hex`).

## Usage

```
cargo run --release -p engine_bench -- \
    --url http://localhost:8551 \
    --jwt-path /path/to/jwt.hex \
    --iterations 100 \
    --transports json,ssz \
    --workloads newPayload,getPayload,blobs,bodies
```

Output: a markdown summary table on stdout. Pass `--csv-out results.csv`
for per-iteration data.
