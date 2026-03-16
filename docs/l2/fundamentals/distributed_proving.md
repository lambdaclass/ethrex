# Distributed Proving

## Overview

Distributed proving enables running multiple prover instances in parallel, each working on different batches simultaneously. It has two key aspects:

1. **Parallel batch assignment**: the proof coordinator assigns different batches to different provers, so multiple provers work simultaneously.
2. **Multi-batch verification**: the proof sender collects consecutive proven batches and submits them in a single `verifyBatches()` L1 transaction, saving gas.

## Architecture

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   Prover 1   │     │   Prover 2   │     │   Prover 3   │
│    (sp1)     │     │    (sp1)     │     │   (risc0)    │
└──────┬───────┘     └──────┬───────┘     └──────┬───────┘
       │                    │                    │
       │    TCP             │    TCP             │    TCP
       │                    │                    │
       └────────────┬───────┘────────────────────┘
                    │
          ┌─────────▼──────────┐
          │  Proof Coordinator │  (part of L2 sequencer)
          │  tcp://0.0.0.0:3900│
          └─────────┬──────────┘
                    │
          ┌─────────▼──────────┐
          │   Proof Sender     │  Batches proofs → single L1 tx
          └─────────┬──────────┘
                    │
              ┌─────▼─────┐
              │    L1      │
              └────────────┘
```

Multiple provers connect to the same proof coordinator over TCP. The coordinator tracks assignments per `(batch_number, prover_type)`, so:

- Two `sp1` provers get assigned **different** batches.
- An `sp1` prover and a `risc0` prover can work on the **same** batch simultaneously (they produce different proof types).

## Batch assignment

When a prover sends a `BatchRequest`, it includes its `prover_type`. The coordinator:

1. Scans batches starting from the oldest unverified one.
2. Skips batches that already have a proof for this `prover_type`.
3. Skips batches currently assigned to another prover of the same type (unless the assignment has timed out).
4. Assigns the first available batch and records `(batch_number, prover_type) → Instant::now()`.

The assignment map is in-memory only — it is lost on restart. On restart, the coordinator simply reassigns batches from scratch, which is safe because storing a duplicate proof is a no-op.

## Prover timeout

If a prover doesn't submit a proof within `prover-timeout` (default 10 minutes), its assignment expires and the batch becomes available for reassignment to another prover. This handles prover crashes, network issues, or slow provers without manual intervention.

## Multi-batch verification

The proof sender runs on a periodic tick (every `send-interval` ms). On each tick it:

1. Queries the on-chain `lastVerifiedBatch` and `lastCommittedBatch`.
2. Collects all **consecutive** proven batches starting from `lastVerifiedBatch + 1`, checking that every required proof type is present for each batch.
3. Sends them in a single `verifyBatches()` call to L1.

For example, if batches 5, 6, 7 are fully proven but batch 8 is missing a proof, only batches 5–7 are sent. Batch 8 waits for its proof.

### Fallback to single-batch sending

On **any** multi-batch error (gas limit exceeded, calldata too large, invalid proof, etc.), the proof sender falls back to sending each batch individually. Since on-chain verification is sequential (`batchNumber == lastVerifiedBatch + 1`), the fallback stops at the first failing batch — remaining batches are retried on the next tick.

During single-batch fallback, if the error indicates an invalid proof (e.g. "Invalid SP1 proof"), that proof is deleted from the store so a prover can re-prove it.

## Configuration reference

### Proof coordinator (sequencer side)

| Flag | Env Variable | Default | Description |
|------|-------------|---------|-------------|
| `--proof-coordinator.addr` | `ETHREX_PROOF_COORDINATOR_LISTEN_ADDRESS` | `127.0.0.1` | Listen address |
| `--proof-coordinator.port` | `ETHREX_PROOF_COORDINATOR_LISTEN_PORT` | `3900` | Listen port |
| `--proof-coordinator.send-interval` | `ETHREX_PROOF_COORDINATOR_SEND_INTERVAL` | `5000` | How often (ms) the proof sender collects and sends proofs to L1 |
| `--proof-coordinator.prover-timeout` | `ETHREX_PROOF_COORDINATOR_PROVER_TIMEOUT` | `600000` | Timeout (ms) before reassigning a batch to another prover (default: 10 min) |

### Prover client

| Flag | Env Variable | Default | Description |
|------|-------------|---------|-------------|
| `--proof-coordinators` | `PROVER_CLIENT_PROOF_COORDINATOR_URL` | `tcp://127.0.0.1:3900` | Space-separated coordinator URLs |
| `--backend` | `PROVER_CLIENT_BACKEND` | `exec` | Backend: `exec`, `sp1`, `risc0`, `zisk`, `openvm` |
| `--proving-time` | `PROVER_CLIENT_PROVING_TIME` | `5000` | Wait time (ms) between requesting new work |

## Testing locally

### 1. Start L1

```bash
cd crates/l2
make init-l1
```

### 2. Deploy contracts

```bash
cd crates/l2
make deploy-l1
```

### 3. Start L2 with a long proof send interval

Set a long send interval so that multiple batch proofs accumulate before the proof sender submits them to L1 in a single transaction. The default is 5 seconds (5000ms).

```bash
cd crates/l2
ETHREX_PROOF_COORDINATOR_SEND_INTERVAL=120000 make init-l2
```

This sets the interval to 120 seconds, giving provers time to complete multiple batches before verification.

### 4. Start multiple provers

Once some batches have been committed, start multiple prover instances in separate terminals. They all connect to the same coordinator at `tcp://127.0.0.1:3900`.

```bash
# Terminal A
cd crates/l2
make init-prover-exec

# Terminal B
cd crates/l2
make init-prover-exec
```

Each prover will be assigned a different batch. When both finish, the proof sender will collect the consecutive proven batches and submit them in a single `verifyBatches` transaction on L1.
