# Distributed Proving

Distributed proving allows running multiple prover instances in parallel, each working on different batches simultaneously. The proof coordinator assigns work to provers and collects their proofs, then the proof sender batches multiple consecutive proofs into a single L1 verification transaction.

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

Multiple provers connect to the same proof coordinator. The coordinator tracks assignments per `(batch_number, prover_type)`, so:
- Two `sp1` provers get assigned **different** batches.
- An `sp1` prover and an `risc0` prover can work on the **same** batch simultaneously (they produce different proof types).

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

## Configuration reference

### Proof coordinator (L2 side)

| Flag | Env Variable | Default | Description |
|------|-------------|---------|-------------|
| `--proof-coordinator.addr` | `ETHREX_PROOF_COORDINATOR_LISTEN_ADDRESS` | `127.0.0.1` | Listen address |
| `--proof-coordinator.port` | `ETHREX_PROOF_COORDINATOR_LISTEN_PORT` | `3900` | Listen port |
| `--proof-coordinator.send-interval` | `ETHREX_PROOF_COORDINATOR_SEND_INTERVAL` | `5000` | How often (ms) the proof sender batches and sends proofs to L1 |
| `--proof-coordinator.prover-timeout` | `ETHREX_PROOF_COORDINATOR_PROVER_TIMEOUT` | `600000` | Timeout (ms) before reassigning a batch to another prover (default: 10 min) |

### Prover client

| Flag | Env Variable | Default | Description |
|------|-------------|---------|-------------|
| `--proof-coordinators` | `PROVER_CLIENT_PROOF_COORDINATOR_URL` | `tcp://127.0.0.1:3900` | Space-separated coordinator URLs |
| `--backend` | `PROVER_CLIENT_BACKEND` | `exec` | Backend: `exec`, `sp1`, `risc0`, `zisk`, `openvm` |
| `--proving-time` | `PROVER_CLIENT_PROVING_TIME` | `5000` | Wait time (ms) between requesting new work |

## How it works

### Batch assignment

When a prover sends a `BatchRequest`, it includes its `prover_type`. The coordinator:

1. Scans batches starting from the oldest unverified one.
2. Skips batches that already have a proof for this `prover_type`.
3. Skips batches currently assigned to another prover of the same type (unless the assignment timed out).
4. Assigns the first available batch and records `(batch_number, prover_type) → Instant::now()`.

### Prover timeout

If a prover doesn't submit a proof within `prover-timeout` (default 10 minutes), its assignment expires and the batch becomes available for reassignment to another prover.

### Multi-batch verification

The proof sender periodically (every `send-interval` ms):

1. Collects all **consecutive** proven batches starting from `last_verified_batch + 1`.
2. Sends them in a single `verifyBatches()` call to L1.
3. Falls back to per-batch verification if any batch has an invalid proof, to isolate the failure.

For example, if batches 1, 2, 3 are proven but 4 is not, only batches 1-3 are sent. Batch 4 waits for its proof.
