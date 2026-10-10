# engine_bench

End-to-end benchmark harness for the ethrex engine API. Compares JSON-RPC vs
REST/SSZ transports across four workloads (`newPayload`, `getPayload`,
`blobs` — all three endpoint versions — and `bodies`) on **every fork era**
(Paris → Amsterdam).

Pure serde costs (encode/decode without HTTP) are measured separately by the
criterion bench: `cargo bench -p ethrex-rpc --bench engine_transport`.

## Modes

**Default (self-hosted sweep).** With no `--url`, the harness spins up one
throwaway ethrex devnet per fork (plain node, `--p2p.disabled --syncmode
full`, patched genesis, scratch datadir under the system temp dir), drives
block production itself through the engine API exactly like a CL
(fcU V1–V4 → getPayload V1–V6 → newPayload V1–V5 → fcU), benches every
(workload, transport) cell, then tears the node down. One combined table with
a fork column comes out the other end:

```
cargo run --release -p engine_bench -- --csv-out results.csv
```

Run from the repo root, or pass `--ethrex-bin <path>`. Devnets listen on
`--devnet-port` (default 18551; the harness refuses a busy port). Scratch data
is deleted after a successful run unless `--keep-devnets`.

**External node.** With `--url` + `--jwt-path`, the harness benches a running
node instead. Its fork era is auto-detected from the latest header fields
(Prague vs Osaka — identical headers — is disambiguated by probing
`engine_getPayloadV5` with a freshly built payload), and only that fork's rows
are produced.

## What is measured

Per iteration, the timed window is identical on both transports:

> typed request struct → wire bytes → HTTP round-trip → raw response bytes

Response decoding is excluded on both sides; it only happens after the timer
stops, to fill the `hits` column. The first `--warmup` iterations (default 3)
per cell are discarded — they absorb connection setup (TCP + h2c handshake).

## Workload notes — read before quoting numbers

- **newPayload** sends a synthetic payload whose block hash is intentionally
  invalid: the server fully decodes it, then rejects it. This isolates
  transport + server decode from block execution. JSON reports the rejection
  as `200` + `INVALID`, REST as `422` — the status column shows this by
  design.
- **getPayload** uses a real payload id acquired via forkchoiceUpdated
  (`--payload-id` overrides). Freshly built devnet payloads are near-empty,
  so these rows mostly measure per-request overhead; the criterion bench
  carries the heavyweight (blob-bundle / BAL) comparison.
- **blobs** runs v1, v2, and v3 per fork. Entries only carry data for hashes
  in the node's blob pool — pass `--blob-hashes-file` (newline-separated
  0x-hex versioned hashes, `#` comments allowed) for the hit path; without it
  every entry misses (`hits` reads 0). Note the miss-path shapes: v1/v3 SSZ
  zero-pad misses to full blob size (~8.4 MB for 64 misses), v2 short-circuits
  (JSON `null` / REST `204`). A `hits` value of `-` with a small JSON response
  means the JSON method rejected the call (fork-gated) while REST still
  answered.
- **bodies** fetches `[--bodies-from, +--bodies-count)`; the sweep produces
  enough blocks for the full range (`hits` shows how many came back
  non-null). The REST/SSZ endpoint caps one request at
  MAX_BODIES_PER_REQUEST (32).

The summary's `status` column lists every distinct HTTP status seen for the
row. JSON-RPC errors surface as `200` (error in body), REST errors as
4xx/5xx; an unexpected status means the row measured error handling, not the
workload.

## Output

A markdown summary table on stdout — fork, workload (blobs annotated with
version), transport, request/response bytes (medians), `ms_min` /
`ms_median` / `ms_p99`, hits, statuses. Pass `--csv-out results.csv` for
per-iteration data.
