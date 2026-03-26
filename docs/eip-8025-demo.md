# EIP-8025: Demo Guide

This guide walks through the full EIP-8025 proof generation and verification flow using a local ethrex node, the `ethrex-repl`, and the L1 prover binary. It exercises all three Engine API endpoints and the coordinator-prover TCP protocol.

For architecture and implementation details, see [docs/eip-8025.md](eip-8025.md).

---

## Prerequisites

Build ethrex with the `eip-8025` feature and the L1 prover binary:

```bash
cargo build --release --features eip-8025,dev --bin ethrex
cargo build --release --features "eip-8025,l1-prover-bin" -p ethrex-prover --bin l1_prover
```

Ensure `jwt.hex` exists in the repo root (generated automatically by the node on first run if missing).

---

## Setup

The demo uses three terminals:

| Terminal | Component | Role |
|----------|-----------|------|
| 1 | ethrex node | L1 execution client with proof engine |
| 2 | L1 prover | Connects to coordinator, pulls work, executes |
| 3 | ethrex-repl | Interactive demo driver (acts as mock beacon node) |

### Terminal 1: Start the Node

```bash
cargo run --release --features eip-8025 --bin ethrex -- \
  --network fixtures/genesis/l1.json \
  --http.port 8545 \
  --authrpc.port 8551 \
  --authrpc.jwtsecret jwt.hex \
  --syncmode full \
  --p2p.disabled \
  --proof.callback-url http://localhost:9200
```

Verify the proof engine started:
```
L1 ProofCoordinator bound to 127.0.0.1:9100
EIP-8025 proof coordinator started
```

> **Note:** `--syncmode full` is required (not the default `snap`), otherwise the node remains in `SYNCING` state and rejects `engine_forkchoiceUpdatedV3` calls.

### Terminal 2: Start the Prover

```bash
RUST_LOG=info ./target/release/l1_prover \
  --coordinator http://localhost:9100 \
  --poll-interval-ms 2000
```

The prover polls the coordinator every 2 seconds. When there is no pending work it logs nothing visible at `info` level (the idle message is `debug!`-level and suppressed). The first visible output appears when a proof is requested.

### Terminal 3: Start the REPL

```bash
cargo run --release -p ethrex-repl -- \
  -e http://localhost:8545 \
  --authrpc.jwtsecret jwt.hex
```

---

## Demo Flow

### Step 1 — Set Forkchoice Head

The node starts in `SYNCING` state. A forkchoice call pointing to the genesis block transitions it to synced:

```
head = eth.getBlockByNumber 0 false
engine.forkchoiceUpdatedV3 {"headBlockHash":"$head.hash","safeBlockHash":"$head.hash","finalizedBlockHash":"$head.hash"}
```

Expected: `payloadStatus.status = VALID`

### Step 2 — Produce a Block

Trigger payload building with a second forkchoice call that includes payload attributes.

The `timestamp` must be greater than the parent block's. Use arithmetic on the parent timestamp:

```
ts = $head.timestamp + 1
fcu = engine.forkchoiceUpdatedV3 {"headBlockHash":"$head.hash","safeBlockHash":"$head.hash","finalizedBlockHash":"$head.hash"} {"timestamp":"$ts","prevRandao":"0x0000000000000000000000000000000000000000000000000000000000000000","suggestedFeeRecipient":"0x0000000000000000000000000000000000000000","parentBeaconBlockRoot":"0x0000000000000000000000000000000000000000000000000000000000000000","withdrawals":[]}
```

Expected: `payloadId` is non-null.

### Step 3 — Get the Execution Payload

```
payload = engine.getPayloadV5 $fcu.payloadId
```

Expected: a full `executionPayload` object with `blockHash`, `stateRoot`, `transactions`, etc.

### Step 4 — Submit the Block

```
engine.newPayloadV4 $payload.executionPayload [] 0x0000000000000000000000000000000000000000000000000000000000000000 []
```

Expected: `status = VALID`

### Step 5 — Verify Proof Availability (Before Proving)

Before requesting any proof, check whether one exists for this block. Use the same `verifyNewPayloadRequestHeaderV1` call that will succeed in Step 10:

```
engine.verifyNewPayloadRequestHeaderV1 {
  "executionPayloadHeader": {
    "parentHash":    "$payload.executionPayload.parentHash",
    "feeRecipient":  "$payload.executionPayload.feeRecipient",
    "stateRoot":     "$payload.executionPayload.stateRoot",
    "receiptsRoot":  "$payload.executionPayload.receiptsRoot",
    "logsBloom":     "$payload.executionPayload.logsBloom",
    "prevRandao":    "$payload.executionPayload.prevRandao",
    "blockNumber":   "$payload.executionPayload.blockNumber",
    "gasLimit":      "$payload.executionPayload.gasLimit",
    "gasUsed":       "$payload.executionPayload.gasUsed",
    "timestamp":     "$payload.executionPayload.timestamp",
    "extraData":     "$payload.executionPayload.extraData",
    "baseFeePerGas": "$payload.executionPayload.baseFeePerGas",
    "blockHash":     "$payload.executionPayload.blockHash",
    "blobGasUsed":   "$payload.executionPayload.blobGasUsed",
    "excessBlobGas": "$payload.executionPayload.excessBlobGas",

    "transactionsRoot":        "0x7ffe241ea60187fdb0187bfa22de35d1f9bed7ab061d9401fd47e34a54fbede1",
    "withdrawalsRoot":         "0x792930bbd5baac43bcc798ee49aa8185ef76bb3b44ba62b91d86ae569e4bb535",
    "depositRequestsRoot":     "0x4a8c3a07c8d23adc5bac61157555c3c784d53d9bc110c1370809bd23cd93777d",
    "withdrawalRequestsRoot":  "0x792930bbd5baac43bcc798ee49aa8185ef76bb3b44ba62b91d86ae569e4bb535",
    "consolidationRequestsRoot":"0xf5a5fd42d16a20302798ef6ed309979b43003d2320d9f0e8ea9831a92759fb4b"
  },
  "versionedHashes": [],
  "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "executionRequests": []
}
```

**Expected: `status = SYNCING`**

No proofs exist yet, so the endpoint correctly reports that proof verification is still pending. Compare with Step 10 where the same call returns `VALID` after the prover completes.

### Step 6 — Request Proof Generation (`engine_requestProofsV1`)

```
proof = engine.requestProofsV1 $payload.executionPayload [] 0x0000000000000000000000000000000000000000000000000000000000000000 [] {"proofTypes":[0]}
```

Expected: a `ProofGenId` (8-byte hex string, e.g. `"0x00000001abcdef01"`). Stored in `$proof` for later retrieval.

Internally, the proof engine:
1. Re-executes the block to generate an `ExecutionWitness`
2. Builds a `ProgramInput` (block + witness)
3. Computes the SSZ `hash_tree_root` of the `NewPayloadRequest`
4. Queues the input for the coordinator

### Step 7 — Prover Pulls, Executes, and Submits

Watch Terminal 2. Within a few seconds:

```
INFO Received payload #1
WARN "exec" prover backend generates no proof, only executes
INFO Proved payload #1
INFO Proof for payload #1 accepted
```

The prover pulled the `ProgramInput` from the coordinator via the `ProofData<ProgramInput>` TCP protocol, executed the block statelessly with `ExecBackend`, and submitted the result. The coordinator stored the proof in the `EXECUTION_PROOFS` table.

### Step 8 — Receive the Proof via Callback

After Step 6, the REPL automatically starts a one-shot HTTP listener on port 9200. When the prover finishes (Step 7), the coordinator POSTs a `GeneratedProof` to this listener.

Watch the REPL output:

```
Proof callback listener started on 127.0.0.1:9200
Waiting for proof delivery...

Proof received via callback! Stored in $generatedProof
  Verify with: engine.verifyExecutionProofV1 $generatedProof.executionProof
```

The proof is now stored in `$generatedProof` with fields `proofGenId` and `executionProof`.

### Step 9 — Verify the Proof (`engine_verifyExecutionProofV1`)

Submit the received proof back to the EL for verification:

```
engine.verifyExecutionProofV1 $generatedProof.executionProof
```

Expected: `status = VALID`

This validates the proof and stores it in `EXECUTION_PROOFS`.

### Step 10 — Verify Proof Availability (`engine_verifyNewPayloadRequestHeaderV1`)

Build the headerized version of the payload. The header replaces variable-length list fields (transactions, withdrawals, requests) with their SSZ `hash_tree_root` values.

Most fields are taken directly from the payload via variable references. The list root fields (`transactionsRoot`, `withdrawalsRoot`, and the three request roots) cannot be templated — they are SSZ `hash_tree_root` values computed over the list contents, not fields present in the payload object. For an empty block use the precomputed values from the [reference table](#reference-ssz-empty-list-roots) below.

```
engine.verifyNewPayloadRequestHeaderV1 {
  "executionPayloadHeader": {
    "parentHash":    "$payload.executionPayload.parentHash",
    "feeRecipient":  "$payload.executionPayload.feeRecipient",
    "stateRoot":     "$payload.executionPayload.stateRoot",
    "receiptsRoot":  "$payload.executionPayload.receiptsRoot",
    "logsBloom":     "$payload.executionPayload.logsBloom",
    "prevRandao":    "$payload.executionPayload.prevRandao",
    "blockNumber":   "$payload.executionPayload.blockNumber",
    "gasLimit":      "$payload.executionPayload.gasLimit",
    "gasUsed":       "$payload.executionPayload.gasUsed",
    "timestamp":     "$payload.executionPayload.timestamp",
    "extraData":     "$payload.executionPayload.extraData",
    "baseFeePerGas": "$payload.executionPayload.baseFeePerGas",
    "blockHash":     "$payload.executionPayload.blockHash",
    "blobGasUsed":   "$payload.executionPayload.blobGasUsed",
    "excessBlobGas": "$payload.executionPayload.excessBlobGas",

    "transactionsRoot":        "0x7ffe241ea60187fdb0187bfa22de35d1f9bed7ab061d9401fd47e34a54fbede1",
    "withdrawalsRoot":         "0x792930bbd5baac43bcc798ee49aa8185ef76bb3b44ba62b91d86ae569e4bb535",
    "depositRequestsRoot":     "0x4a8c3a07c8d23adc5bac61157555c3c784d53d9bc110c1370809bd23cd93777d",
    "withdrawalRequestsRoot":  "0x792930bbd5baac43bcc798ee49aa8185ef76bb3b44ba62b91d86ae569e4bb535",
    "consolidationRequestsRoot":"0xf5a5fd42d16a20302798ef6ed309979b43003d2320d9f0e8ea9831a92759fb4b"
  },
  "versionedHashes": [],
  "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "executionRequests": []
}
```

**Expected: `status = VALID`**

This confirms:
1. The SSZ root computed from the header matches the root from `requestProofsV1`
2. At least 1 proof was found in storage for that root
3. The block's execution validity is confirmed by proof

Compare with Step 6 where the same call returned `SYNCING` — the only difference is that the prover has since generated and submitted a proof.

> **`baseFeePerGas` encoding:** The server accepts both u64 QUANTITY hex (e.g. `"0x342770c0"`) from the payload and full 32-byte big-endian hex (e.g. `"0x00...342770c0"`). You can use `$payload.executionPayload.baseFeePerGas` directly.

> **List roots for non-empty blocks:** The `transactionsRoot`, `withdrawalsRoot`, and request roots above apply to empty lists only. If the block contains transactions or withdrawals, these roots must be computed from the actual list contents using SSZ `hash_tree_root`.

---

## Data Flow

```
Beacon Node (REPL)          ethrex (EL)                    Prover (l1_prover)
─────────────────           ──────────                     ──────────────────
1. forkchoiceUpdatedV3 ───▶ Set head to genesis
                      ◀─── VALID

2. forkchoiceUpdatedV3 ───▶ Build payload
                      ◀─── payloadId

3. getPayloadV5 ──────────▶ Return ExecutionPayload
                      ◀─── { executionPayload, ... }

4. newPayloadV4 ──────────▶ Validate & execute block
                      ◀─── VALID

5. verifyNewPayload  ────▶ Compute SSZ root from header
   RequestHeaderV1         Look up proofs (count == 0)
                      ◀─── SYNCING (no proof yet)

6. requestProofsV1 ───────▶ Generate witness
                            Compute SSZ root
                            Queue ProgramInput
                      ◀─── ProofGenId
                                                           7. Pull ProgramInput (TCP)
                                                              Execute block (ExecBackend)
                                                              Submit proof (TCP)
                            Store in EXECUTION_PROOFS ◀───── ProofSubmitACK

   POST GeneratedProof ────▶  8. Coordinator POSTs to
◀──── (callback to REPL)        callback_url (port 9200)

 9. verifyExecutionProofV1 ▶ Validate & re-store proof
                       ◀─── VALID

10. verifyNewPayload  ────▶ Compute SSZ root from header
    RequestHeaderV1         Look up proofs (count >= 1)
                      ◀─── VALID
```

---

## Reference: SSZ Empty-List Roots

For blocks with no transactions, withdrawals, or execution requests, use these SSZ `hash_tree_root` values in the `ExecutionPayloadHeader`:

| Field | Root |
|-------|------|
| `transactionsRoot` | `0x7ffe241ea60187fdb0187bfa22de35d1f9bed7ab061d9401fd47e34a54fbede1` |
| `withdrawalsRoot` | `0x792930bbd5baac43bcc798ee49aa8185ef76bb3b44ba62b91d86ae569e4bb535` |
| `depositRequestsRoot` | `0x4a8c3a07c8d23adc5bac61157555c3c784d53d9bc110c1370809bd23cd93777d` |
| `withdrawalRequestsRoot` | `0x792930bbd5baac43bcc798ee49aa8185ef76bb3b44ba62b91d86ae569e4bb535` |
| `consolidationRequestsRoot` | `0xf5a5fd42d16a20302798ef6ed309979b43003d2320d9f0e8ea9831a92759fb4b` |

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `forkchoiceUpdatedV3` returns `SYNCING` | Default sync mode is `snap` | Start with `--syncmode full` |
| `requestProofsV1` returns "World State Root does not match" | Payload has incorrect `stateRoot` | Use the payload from `getPayloadV5`, not a manually constructed one |
| `verifyNewPayloadRequestHeaderV1` returns `SYNCING` | No proof stored yet | Wait for the prover to submit, or check prover logs for errors |
| Prover shows no output after starting | Idle message is `debug!`-level and suppressed at `RUST_LOG=info` — this is normal | The first visible log appears only when a proof is requested |
| Port 9100 "Address already in use" | Previous node process still running | Kill stale processes: `pkill -9 -f ethrex` and wait a few seconds |
| Prover polls but finds no work despite `requestProofsV1` succeeding | Stale proofs from a previous run in the data directory | Delete the data directory (default: `~/Library/Application Support/ethrex` on macOS, `~/.local/share/ethrex` on Linux) and restart the node |
