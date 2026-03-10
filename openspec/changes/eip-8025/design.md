# EIP-8025: Design

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────────┐
│  ethrex node  (--features eip-8025)                                      │
│                                                                          │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │  Engine API (crates/networking/rpc/engine/)                         │  │
│  │                                                                     │  │
│  │  #[cfg(feature = "eip-8025")]                                       │  │
│  │  ├─ engine_requestProofsV1        → ProofEngine::request_proofs()   │  │
│  │  ├─ engine_verifyExecutionProofV1 → ProofEngine::verify_proof()     │  │
│  │  └─ engine_verifyNewPayloadReq    → ProofEngine::verify_header()    │  │
│  │       HeaderV1                                                      │  │
│  └────────┬───────────────────────────────┬────────────────────────────┘  │
│           │                               │                               │
│           ▼                               ▼                               │
│  ┌─────────────────┐      ┌──────────────────────────────────────────┐   │
│  │   Blockchain     │      │  ProofEngine                             │   │
│  │                  │      │  (crates/blockchain/proof_engine/)        │   │
│  │  • generate_     │◀────▶│                                          │   │
│  │    witness_for_  │      │  request_proofs():                       │   │
│  │    blocks()      │      │    1. payload → Block                    │   │
│  │                  │      │    2. generate witness (once)             │   │
│  │  precompute_     │      │    3. build ProgramInput                  │   │
│  │  witnesses=true  │      │    4. store input for coordintor          │   │
│  │  (auto-enabled)  │      │    5. return ProofGenId                   │   │
│  └─────────────────┘      │                                          │   │
│                            │  verify_proof():                         │   │
│                            │    1. backend.verify(proof_data)          │   │
│                            │    2. store in EXECUTION_PROOFS           │   │
│                            │                                          │   │
│                            │  verify_header():                        │   │
│                            │    1. compute SSZ root from header        │   │
│                            │    2. lookup EXECUTION_PROOFS             │   │
│                            │    3. check >= MIN_REQUIRED (=1)          │   │
│                            └─────────────┬────────────────────────────┘   │
│                                          │                                │
│  ┌───────────────────────────────────────▼──────────────────────────────┐ │
│  │  ProofCoordinator (TCP, pull model)                                  │ │
│  │  Reused from shared prover infra                                     │ │
│  │                                                                      │ │
│  │  • Prover connects with BatchRequest { prover_type }                 │ │
│  │  • Coordinator sends ProgramInput (same for all proof types)         │ │
│  │  • Prover proves, sends ProofSubmit back                             │ │
│  │  • Coordinator builds GeneratedProof { proof_gen_id,                 │ │
│  │    execution_proof }, POSTs to callback_url                          │ │
│  │  • Coordinator stores verified proof in EXECUTION_PROOFS             │ │
│  └──────────────────────────────────────────────────────────────────────┘ │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────────┐ │
│  │  Store                                                               │ │
│  │  EXECUTION_PROOFS table — 128-block retention, same as witnesses     │ │
│  └──────────────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────────┘

          Provers connect to coordinator (pull model, same as L2)
              ┌────────────────┼─────────────────┐
              ▼                ▼                  ▼
      ┌──────────────┐ ┌──────────────┐  ┌──────────────┐
      │ SP1 Prover   │ │ RISC0 Prover │  │ ZisK Prover  │
      │ (any machine)│ │ (any machine)│  │ (any machine)│
      │ Pulls input  │ │ Pulls input  │  │ Pulls input  │
      │ Proves       │ │ Proves       │  │ Proves       │
      │ Returns proof│ │ Returns proof│  │ Returns proof│
      └──────────────┘ └──────────────┘  └──────────────┘
```

## Key Design Decisions

### 1. Distributed Proving: Pull Model (same as L2)

Reuse the existing L2 pattern where provers **pull** work from a coordinator:

- Coordinator holds `ProgramInput` (NewPayloadRequest + ExecutionWitness)
- Witness is generated **once** by the coordinator (it has state)
- Multiple provers with different backends connect and request work
- Each prover independently proves the same input with its own zkVM
- Provers submit proof back to coordinator
- Coordinator wraps in `GeneratedProof { proof_gen_id, execution_proof }` and delivers via `POST /eth/v1/prover/execution_proofs`

This avoids:
- GPU contention (each prover has its own GPU)
- Redundant witness generation (done once)
- Coordinator needing to know prover URLs (provers connect to it)

### 2. L1 ProofCoordinator (GenServer) and Prover Client (reused from L2)

Uses `spawned_concurrency::GenServer` (same framework as all L2 sequencer actors).

#### L1 ProofCoordinator GenServer

The L2 `ProofCoordinator` in `crates/l2/sequencer/proof_coordinator.rs` is already a GenServer but deeply coupled to L2 (`StoreRollup`, `EthClient`, aligned mode, TDX, batch numbering). We write a new L1-specific one.

```rust
// --- State ---
pub struct L1ProofCoordinator {
    store: Store,
    config: ProofEngineConfig,
    /// Pending inputs waiting for provers to pull.
    /// Key: block_number, Value: (ProofGenId, ProgramInput, assigned_at)
    pending: HashMap<u64, PendingProof>,
    /// HTTP client for callback delivery.
    http_client: reqwest::Client,
}

// --- Messages ---
#[derive(Clone)]
pub enum CoordCastMsg {
    /// Start accepting prover TCP connections.
    Listen { listener: Arc<TcpListener> },
    /// New proof request from ProofEngine::request_proofs().
    NewInput {
        block_number: u64,
        proof_gen_id: ProofGenId,
        program_input: Box<ProgramInput>,
    },
}

// CallMsg = Unused (no sync queries needed initially)

// --- GenServer impl ---
impl GenServer for L1ProofCoordinator {
    type CallMsg = Unused;
    type CastMsg = CoordCastMsg;
    type OutMsg = CoordOutMsg;
    type Error = ProofCoordinatorError;

    async fn handle_cast(&mut self, msg, handle) -> CastResponse {
        match msg {
            CoordCastMsg::Listen { listener } => {
                // Accept loop — spawn ConnectionHandler per connection
                self.accept_loop(listener).await;
                CastResponse::Stop  // only if listener dies
            }
            CoordCastMsg::NewInput { block_number, proof_gen_id, program_input } => {
                self.pending.insert(block_number, PendingProof {
                    proof_gen_id,
                    input: *program_input,
                    created_at: Instant::now(),
                });
                CastResponse::NoReply
            }
        }
    }
}
```

```
┌──────────────────────────────────────────────────────────────────┐
│  L1 ProofCoordinator (GenServer)                                  │
│                                                                   │
│  State:                                                           │
│    store: Store                                                   │
│    config: ProofEngineConfig (callback_url, addr, port)           │
│    pending: HashMap<block_number, PendingProof>                   │
│    http_client: reqwest::Client                                   │
│                                                                   │
│  Cast messages:                                                   │
│    Listen { listener }      → accept loop, spawn ConnectionHandler│
│    NewInput { block, id, input } → store in pending map           │
│                                                                   │
│  TCP protocol (same ProofData<I> as L2):                          │
│    Prover → BatchRequest { commit_hash, prover_type }             │
│    Coord  → BatchResponse { batch_number, input, format }         │
│    Prover → ProofSubmit { batch_number, batch_proof }             │
│    Coord  → ProofSubmitACK { batch_number }                       │
│                                                                   │
│  On proof received:                                               │
│    1. Store in EXECUTION_PROOFS table                             │
│    2. Build GeneratedProof { proof_gen_id, execution_proof }      │
│    3. POST to callback_url                                        │
│    4. Remove from pending map                                     │
└──────────────────────────────────────────────────────────────────┘
```

**ConnectionHandler** — one per TCP connection, same pattern as L2:
```rust
impl GenServer for L1ConnectionHandler {
    type CastMsg = ConnMsg;  // Connection { stream, addr }
    // handle_cast: read request, dispatch to coordinator, respond, Stop
}
```

**Spawning** — in `init_proof_engine()`:
```rust
let coord = L1ProofCoordinator::new(store, config);
let mut coord_handle = coord.start();
let listener = TcpListener::bind(config.coordinator_socket_addr()).await?;
coord_handle.cast(CoordCastMsg::Listen { listener: Arc::new(listener) }).await?;
// Store coord_handle in ProofEngine for request_proofs() to cast NewInput
```

#### Prover client (reused from L2)

The extracted `Prover<B: ProverBackend, I>` in `crates/prover/src/prover.rs` is generic over the input type `I: Into<ProgramInput> + Serialize + DeserializeOwned`. The `InputConverter` trait is deleted — replaced by a standard `Into<ProgramInput>` bound.

- L2 uses `Prover<B, ProverInputData>` with `impl Into<ProgramInput> for ProverInputData` (1:1 field copy, replaces `L2InputConverter`)
- L1 uses `Prover<B, ProgramInput>` where `Into<ProgramInput> for ProgramInput` is identity (trivial)

The TCP protocol uses generic `ProofData<I>`:
- L2 wire format: `ProofData<ProverInputData>` (unchanged, backward compatible)
- L1 wire format: `ProofData<ProgramInput>`

The pull loop, `request_new_input()`, `submit_proof()` — all reused as-is.

The prover client runs on a separate machine as a standalone binary — a polling loop is fine for that use case. The GenServer pattern matters for the coordinator (which lives inside the node process and needs to interact with `ProofEngine` via message passing).

### 3. Shared Prover Infrastructure

Extract from `crates/l2/prover/` into `crates/prover/` (shared by L1 and L2):

| Component | Current Location | Shared Location |
|-----------|-----------------|-----------------|
| `ProverBackend` trait | `crates/l2/prover/src/backend/mod.rs` | `crates/prover/src/backend/mod.rs` |
| SP1/RISC0/ZisK/OpenVM backends | `crates/l2/prover/src/backend/*.rs` | `crates/prover/src/backend/*.rs` |
| `Prover<B, I>` (pull loop) | `crates/l2/prover/src/prover.rs` | `crates/prover/src/prover.rs` |
| `ProofData<I>` protocol | `crates/l2/common/src/prover.rs` | `crates/prover/src/protocol.rs` (generic, L2 re-exports) |

`ProofCoordinator` is NOT shared — L2 keeps its own in `crates/l2/sequencer/`, L1 has a new one in `crates/blockchain/proof_engine/coordinator.rs`.

`crates/l2/prover/` becomes a thin wrapper that imports from the shared crate and adds L2-specific types.

### 4. ProofEngine as Blockchain Module

```
crates/common/src/types/
├── eip8025_ssz.rs       ← SSZ containers (NewPayloadRequest, Header, etc.) — single canonical location
│                          imported by both guest-program and blockchain

crates/blockchain/
├── proof_engine/
│   ├── mod.rs           ← ProofEngine struct, public API
│   ├── types.rs         ← ExecutionProof, PublicInput, ProofAttributes, etc.
│   └── store.rs         ← ProofStore (wraps Store for EXECUTION_PROOFS table)
```

SSZ types live in `ethrex-common` (not `ethrex-blockchain`) because guest programs can't depend on
`ethrex-blockchain` (too heavy for zkVM — pulls in tokio, rocksdb, etc.).

### 5. Guest Program Modification

Aligned with [ere-guests PR #7](https://github.com/eth-act/ere-guests/pull/7):

**Before (current):**
```
Input:  ProgramInput { blocks: Vec<Block>, execution_witness }
Output: ProgramOutput { initial_state_hash, final_state_hash, last_block_hash, chain_id, tx_count }
```

**After (EIP-8025):**
```
Input:  ProgramInput { new_payload_request: NewPayloadRequest, execution_witness }
Output: (hash_tree_root(NewPayloadRequest), valid: bool)  — 33 bytes: 32-byte root + 1-byte boolean
```

Key changes inside the guest:
- Receive `NewPayloadRequest` instead of raw `Block`
- Convert `NewPayloadRequest` → EL block internally
- Validate block_hash, versioned_blob_hashes (checks previously done by CL)
- Compute `hash_tree_root(NewPayloadRequest)` using SSZ/SHA256 (zkVM precompile)
- Wrap execution in panic handler, commit `(root, false)` on failure

**Note on `NewPayloadRequest` structure:** The ere-guests design models `NewPayloadRequest` as a fork-variant enum (`Bellatrix`, `Capella`, `Deneb`, `ElectraFulu`). For our initial implementation we use a flat struct matching the current fork (Electra/Fulu: `ExecutionPayloadV3` + `versioned_hashes` + `parent_beacon_block_root` + `execution_requests`). Fork-variant support can be added later if needed for historical block proving.

### 6. Absorb `--precompute-witnesses`

The existing `precompute_witnesses` flag (CLI + `BlockchainOptions`) is absorbed into the `eip-8025` feature. When `eip-8025` is enabled, witnesses are always precomputed — no separate flag needed.

### 7. Persistent ProofStore

New `EXECUTION_PROOFS` table in Store, following the exact same pattern as `EXECUTION_WITNESSES`:

- Key: `(block_number: u64, new_payload_request_root: H256, proof_type: u64)` — 48 bytes
- Value: serialized `VerifiedProof { proof_data }`
- Retention: 128 blocks (same as `MAX_WITNESSES`)
- Cleanup: same `cleanup_old_*` pattern
- Up to `MAX_EXECUTION_PROOFS_PER_PAYLOAD` (4) proofs per payload (different proof types)

### 8. SSZ via libssz

New SSZ containers needed for `hash_tree_root` computation:

- `NewPayloadRequest` — the full container whose root is the public input
- `NewPayloadRequestHeader` — headerized version for `engine_verifyNewPayloadRequestHeaderV1`
- `ExecutionPayloadHeader` — header with `transactions_root`/`withdrawals_root` instead of full lists

These use libssz for serialization and tree hashing. ere-guests will adopt libssz (currently uses Lighthouse's `ethereum_ssz` + `tree_hash` stack but will migrate).

### 9. Constants

From the consensus specs (PR #4828):

| Name | Value | Description |
|------|-------|-------------|
| `MIN_REQUIRED_EXECUTION_PROOFS` | `1` | Minimum valid proofs for `engine_verifyNewPayloadRequestHeaderV1` to return VALID |
| `MAX_PROOF_SIZE` | `307200` (300 KiB) | Maximum `proofData` size — must validate in `engine_verifyExecutionProofV1` |
| `MAX_EXECUTION_PROOFS_PER_PAYLOAD` | `4` | Maximum proofs stored per payload (bounds ProofStore) |

## Engine API Endpoints

### engine_requestProofsV1

```
Request:
  1. executionPayload: ExecutionPayloadV3
  2. versionedHashes: Array of DATA (32 bytes each)
  3. parentBeaconBlockRoot: DATA (32 bytes)
  4. executionRequests: Array of DATA
  5. proofAttributes: ProofAttributesV1 { proofTypes: Array of QUANTITY (u64) }

Response: DATA (8 bytes) — ProofGenId

Errors:
  -39003: Invalid payload
  -39004: Proof generation unavailable

Timeout: 1s
```

Flow:
1. Generate `ProofGenId` immediately
2. Convert payload → Block
3. Generate `ExecutionWitness` (once, using `generate_witness_for_blocks`)
4. Build `ProgramInput { new_payload_request, execution_witness }`
5. Store input for ProofCoordinator to distribute
6. Provers pull input, prove, submit back
7. Coordinator builds `GeneratedProof { proof_gen_id, execution_proof }` and POSTs to `callback_url`

### engine_verifyExecutionProofV1

```
Request: ExecutionProofV1 { proofData: DATA (max 300 KiB), proofType: QUANTITY (u64), publicInput: { newPayloadRequestRoot: DATA 32 bytes } }
Response: ProofStatusV1 { status: VALID|INVALID|NOT_SUPPORTED, error? }

Errors:
  -39001: Invalid proof format

Timeout: 1s
```

Validation:
1. `proofData` MUST be non-empty and not exceed 300 KiB (MAX_PROOF_SIZE)
2. `proofType` MUST be a supported proof type
3. `publicInput.newPayloadRequestRoot` MUST be a valid 32-byte hash

Flow:
1. If validation fails → error `-39001`
2. If `proofType` not supported → `NOT_SUPPORTED`
3. Verify proof using appropriate backend
4. If valid, store in `EXECUTION_PROOFS` table
5. Return `VALID` or `INVALID`

### engine_verifyNewPayloadRequestHeaderV1

```
Request: NewPayloadRequestHeaderV1 { executionPayloadHeader, versionedHashes, parentBeaconBlockRoot, executionRequests }
Response: ProofStatusV1 { status: VALID|SYNCING, error? }

Errors:
  -39002: Invalid header format

Timeout: 1s
```

Flow:
1. Validate header structure; if malformed → error `-39002`
2. Compute `new_payload_request_root` from header fields (SSZ hash_tree_root)
3. Lookup `EXECUTION_PROOFS` for that root
4. If >= `MIN_REQUIRED_EXECUTION_PROOFS` (=1) proofs exist → `VALID`
5. Otherwise → `SYNCING`

## Data Flow: End-to-End

```
BN                             ethrex (EL)                    Prover Workers
 │                                │                               │
 │ engine_requestProofsV1         │                               │
 │ { payload, hashes,            │                               │
 │   beacon_root, requests,      │                               │
 │   proof_attributes }          │                               │
 │──────────────────────────────▶│                               │
 │                                │ 1. Generate witness           │
 │ ProofGenId                     │ 2. Build ProgramInput         │
 │◀──────────────────────────────│ 3. Store for coordinator      │
 │                                │                               │
 │                                │         ProofCoordinator      │
 │                                │◀──────────────────────────────│ Worker connects
 │                                │  "I'm SP1, give me work"      │
 │                                │──────────────────────────────▶│ ProgramInput
 │                                │                               │
 │                                │                               │ 4. Prove
 │                                │                               │
 │                                │◀──────────────────────────────│ ProofSubmit
 │                                │                               │
 │ POST /eth/v1/prover/          │ 5. Build GeneratedProof        │
 │      execution_proofs          │    { proof_gen_id,             │
 │◀──────────────────────────────│      execution_proof }         │
 │                                │ 6. POST to callback_url       │
 │◀──────────────────────────────│                               │
 │                                │                               │
 │ (prover signs, gossips)        │                               │
 │                                │                               │
 │ engine_verifyExecutionProofV1  │                               │
 │──────────────────────────────▶│ 7. Verify proof                │
 │                                │ 8. Store in EXECUTION_PROOFS  │
 │ { status: VALID }              │                               │
 │◀──────────────────────────────│                               │
 │                                │                               │
 │ engine_verifyNewPayloadReqHdr  │                               │
 │──────────────────────────────▶│ 9. Compute SSZ root            │
 │                                │ 10. Lookup proofs              │
 │ { status: VALID }              │                               │
 │◀──────────────────────────────│                               │
```

## Configuration

No config file — all configuration via CLI flags (feature-gated behind `eip-8025`):

- `--proof-callback.url` — URL to POST `GeneratedProof` to (Beacon API endpoint)
- `--proof-coordinator.addr` — bind address for ProofCoordinator TCP server
- `--proof-coordinator.port` — port for ProofCoordinator TCP server

ProofEngine always starts when the `eip-8025` feature is compiled in. The `--proof-*` flags configure its behavior (callback URL, coordinator bind address/port). No separate runtime flag needed — the compile-time feature flag is sufficient.

```
# Node with proof generation
ethrex --proof-callback.url http://beacon:5052/eth/v1/prover/execution_proofs \
       --proof-coordinator.addr 0.0.0.0 \
       --proof-coordinator.port 9100
```

Prover workers (separate machines, each with one zkVM + GPU):
```
ethrex-prover --backend sp1 --coordinator http://ethrex-node:9100
ethrex-prover --backend risc0 --coordinator http://ethrex-node:9100
ethrex-prover --backend zisk --coordinator http://ethrex-node:9100
```

Single proof type (simplest deployment):
```
ethrex --proof-callback.url http://beacon:5052/eth/v1/prover/execution_proofs \
       --proof-coordinator.addr 0.0.0.0 \
       --proof-coordinator.port 9100
ethrex-prover --backend sp1 --coordinator http://localhost:9100
```

## Feature Gating

All EIP-8025 code is behind `#[cfg(feature = "eip-8025")]`:

- `crates/common/Cargo.toml`: `eip-8025 = ["dep:libssz"]` (SSZ types live here)
- `crates/blockchain/Cargo.toml`: `eip-8025 = ["ethrex-common/eip-8025"]`
- `crates/networking/rpc/Cargo.toml`: `eip-8025 = ["ethrex-blockchain/eip-8025", "ethrex-common/eip-8025"]`
- `crates/guest-program/Cargo.toml`: `eip-8025 = ["ethrex-common/eip-8025"]`
- `cmd/ethrex/Cargo.toml`: `eip-8025 = ["ethrex-blockchain/eip-8025", "ethrex-rpc/eip-8025"]`

Engine API capabilities list extended when feature is active.
`--precompute-witnesses` CLI flag removed; witness precomputation is automatic under `eip-8025`.
