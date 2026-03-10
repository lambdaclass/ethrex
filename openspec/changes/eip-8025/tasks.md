# EIP-8025: Tasks

## Phase 1: Extract Shared Prover Infrastructure

### T1.1: Create `crates/prover/` shared crate
- Extract `ProverBackend` trait from `crates/l2/prover/src/backend/mod.rs`
- Move SP1, RISC0, ZisK, OpenVM backend impls to `crates/prover/src/backend/`
- Move `BackendError` to shared crate
- Move `BackendType` enum to shared crate
- Keep zkVM feature flags (`sp1`, `risc0`, `zisk`, `openvm`)
- Files: `crates/prover/src/{lib.rs, backend/{mod.rs, sp1.rs, risc0.rs, zisk.rs, openvm.rs, exec.rs, error.rs}}`

### T1.2: Extract Prover client to shared crate
- Extract generic `Prover<B: ProverBackend, I>` from `crates/l2/prover/src/prover.rs` where `I: Into<ProgramInput> + Serialize + DeserializeOwned`
- Delete `InputConverter` trait — replaced by `Into<ProgramInput>` bound on `I`
- Extract `ProofData<I>` generic protocol messages to `crates/prover/src/protocol.rs`
  - L2 uses `ProofData<ProverInputData>` (unchanged wire format)
  - L1 uses `ProofData<ProgramInput>`
- `impl Into<ProgramInput> for ProverInputData` replaces `L2InputConverter` (1:1 field copy)
- `Into<ProgramInput> for ProgramInput` is identity (trivial)
- Prover client is a pull loop (runs on separate machine as standalone binary) — reused by both L1 and L2
- **Do NOT extract L2 `ProofCoordinator`** — it stays in `crates/l2/sequencer/` (L2-specific: StoreRollup, aligned mode, TDX, batch numbering)
- New L1 `ProofCoordinator` GenServer is a separate task (T3.4)
- Files: `crates/prover/src/{prover.rs, protocol.rs}`

### T1.3: Make `crates/l2/prover/` a thin wrapper
- `crates/l2/prover/` re-exports from `crates/prover/` and adds L2-specific types
- `crates/l2/sequencer/proof_coordinator.rs` stays as-is (L2-specific, not shared)
- Ensure all existing L2 prover tests pass unchanged

## Phase 2: EIP-8025 Types and SSZ

### T2.1: Add libssz dependency and SSZ types in ethrex-common
- Add `libssz` to `crates/common/Cargo.toml` behind `eip-8025` feature (SSZ types must live in ethrex-common because guest programs can't depend on ethrex-blockchain — too heavy for zkVM, pulls in tokio, rocksdb, etc.)
- Implement SSZ containers:
  - `NewPayloadRequest { execution_payload, versioned_hashes, parent_beacon_block_root, execution_requests }`
  - `NewPayloadRequestHeader { execution_payload_header, versioned_hashes, parent_beacon_block_root, execution_requests }`
  - `ExecutionPayloadHeader` (with `transactions_root`, `withdrawals_root` instead of full lists)
  - `PublicInput { new_payload_request_root }`
- Implement `hash_tree_root()` for `NewPayloadRequest` and `NewPayloadRequestHeader`
- File: `crates/common/src/types/eip8025_ssz.rs` (single canonical location, imported by both guest-program and blockchain)

### T2.2: Define EIP-8025 Engine API types
- `PublicInputV1 { new_payload_request_root: H256 }`
- `ExecutionProofV1 { proof_data: Bytes (max 300 KiB), proof_type: u64, public_input: PublicInputV1 }`
- `ProofAttributesV1 { proof_types: Vec<u64> }`
- `ProofStatusV1 { status: String, error: Option<String> }` with `VALID`, `INVALID`, `SYNCING`, `NOT_SUPPORTED`
- `ProofGenId`: 8-byte identifier (`[u8; 8]`)
- `GeneratedProof { proof_gen_id: ProofGenId, execution_proof: ExecutionProofV1 }` — callback POST body
- `ExecutionPayloadHeaderV1` — 17 fields mapping to CL `ExecutionPayloadHeader`
- `NewPayloadRequestHeaderV1 { execution_payload_header, versioned_hashes, parent_beacon_block_root, execution_requests }`
- Constants: `MAX_PROOF_SIZE = 307200`, `MAX_EXECUTION_PROOFS_PER_PAYLOAD = 4`, `MIN_REQUIRED_EXECUTION_PROOFS = 1`
- JSON serialization matching the Engine API spec (`proofType` is u64 in JSON-RPC QUANTITY encoding)
- File: `crates/blockchain/proof_engine/types.rs` and `crates/networking/rpc/types/proof.rs`

## Phase 3: ProofEngine Module

### T3.1: ProofStore (persistent proof storage)
- Add `EXECUTION_PROOFS` table to `crates/storage/api/tables.rs`
- Implement `store_execution_proof()`, `get_proofs_by_root()`, `cleanup_old_proofs()`
- Key: `(block_number: u64, new_payload_request_root: H256, proof_type: u64)` — 48 bytes
- Value: serialized proof data (bytes)
- Up to `MAX_EXECUTION_PROOFS_PER_PAYLOAD` (4) proofs per payload (different proof types)
- 128-block retention (same as `MAX_WITNESSES`)
- Follow exact same pattern as `store_witness()` / `cleanup_old_witnesses()`
- File: `crates/storage/store.rs` (extend existing)

### T3.2: ProofEngine core
- `ProofEngine` struct: holds `Arc<Blockchain>`, `Store`, coordinator handle
- `request_proofs()`: convert payload → Block, generate witness, build ProgramInput, store for coordinator, return ProofGenId
- `verify_proof()`: verify via backend, store in EXECUTION_PROOFS, return ProofStatusV1
- `verify_header()`: compute SSZ root, lookup proofs, check MIN_REQUIRED_EXECUTION_PROOFS (=1)
- Callback delivery: HTTP POST `GeneratedProof { proof_gen_id, execution_proof }` to configured callback_url
- File: `crates/blockchain/proof_engine/mod.rs`

### T3.3: ProofEngine configuration
- Config struct: `callback_url`, `coordinator_addr`, `coordinator_port`
- Parsed from CLI flags: `--proof-callback.url`, `--proof-coordinator.addr`, `--proof-coordinator.port`
- ProofEngine always starts when `eip-8025` feature is enabled; flags configure its behavior
- File: `crates/blockchain/proof_engine/config.rs`

### T3.4: L1 ProofCoordinator GenServer
- New `L1ProofCoordinator` GenServer using `spawned_concurrency::GenServer`
- State: `Store`, `ProofEngineConfig`, `pending: HashMap<u64, PendingProof>`, `http_client: reqwest::Client`
- `CallMsg = Unused` (no sync queries needed initially)
- `CastMsg`:
  - `Listen { listener: Arc<TcpListener> }` — start TCP accept loop
  - `NewInput { block_number, proof_gen_id, program_input }` — add to pending map
- TCP protocol uses generic `ProofData<ProgramInput>` from `crates/prover/src/protocol.rs` (L1 sends `ProgramInput` directly)
- `L1ConnectionHandler` GenServer — one per TCP connection, same pattern as L2's `ConnectionHandler`
  - On `BatchRequest`: find pending input, respond with `BatchResponse`
  - On `ProofSubmit`: store in `EXECUTION_PROOFS`, build `GeneratedProof`, POST to `callback_url`, ACK
- Spawned in `init_proof_engine()`:
  1. Create `L1ProofCoordinator::new(store, config)`
  2. `let coord_handle = coordinator.start()`
  3. Bind `TcpListener` on `coordinator_addr:coordinator_port`
  4. `coord_handle.cast(CoordCastMsg::Listen { listener })`
  5. Store `coord_handle` in `ProofEngine` for `request_proofs()` to cast `NewInput`
- File: `crates/blockchain/proof_engine/coordinator.rs`

## Phase 4: Engine API Endpoints

### T4.1: engine_requestProofsV1 RPC handler
- Parse params: `ExecutionPayloadV3`, `versioned_hashes`, `parent_beacon_block_root`, `execution_requests`, `ProofAttributesV1`
- Call `proof_engine.request_proofs()`
- Return `ProofGenId`
- Error `-39003` if payload is invalid
- Error `-39004` if proof engine not configured
- Timeout: 1s
- File: `crates/networking/rpc/engine/proof.rs`

### T4.2: engine_verifyExecutionProofV1 RPC handler
- Parse params: `ExecutionProofV1`
- Validate: `proofData` non-empty and <= 300 KiB, `proofType` supported, `newPayloadRequestRoot` 32 bytes
- Error `-39001` (Invalid proof format) if validation fails
- Call `proof_engine.verify_proof()`
- Return `ProofStatusV1` (`VALID`, `INVALID`, or `NOT_SUPPORTED`)
- Timeout: 1s
- File: `crates/networking/rpc/engine/proof.rs`

### T4.3: engine_verifyNewPayloadRequestHeaderV1 RPC handler
- Parse params: `NewPayloadRequestHeaderV1`
- Validate header structure
- Error `-39002` (Invalid header format) if malformed
- Call `proof_engine.verify_header()`
- Return `ProofStatusV1` (`VALID` or `SYNCING`)
- Timeout: 1s
- File: `crates/networking/rpc/engine/proof.rs`

### T4.4: Wire endpoints into RPC router
- Add `proof_engine: Option<Arc<ProofEngine>>` to `RpcApiContext`
- Add three new routes to `map_engine_requests()`
- Add to `CAPABILITIES` list (conditional on feature)
- Register `engine_exchangeCapabilities` to include new methods
- File: `crates/networking/rpc/rpc.rs`, `crates/networking/rpc/engine/mod.rs`

## Phase 5: Guest Program Modification

### T5.1: New ProgramInput for EIP-8025
- Define `NewPayloadRequest` type in guest program (minimal CL types, aligned with ere-guests)
- New `ProgramInput`: `{ new_payload_request: NewPayloadRequest, execution_witness: ExecutionWitness }`
- Implement `NewPayloadRequest` → EL `Block` conversion (field moves, minimal allocations)
- Feature-gated: `#[cfg(feature = "eip-8025")]`
- File: `crates/guest-program/src/l1/input.rs`

### T5.2: New ProgramOutput for EIP-8025
- Output: `(hash_tree_root(NewPayloadRequest), valid: bool)` encoded as fixed bytes
- SSZ types imported from `ethrex-common` (already added in T2.1), not duplicated
- Feature-gated: `#[cfg(feature = "eip-8025")]`
- File: `crates/guest-program/src/l1/output.rs`

### T5.3: Modified execution program
- Transform `NewPayloadRequest` → Block
- Validate block_hash matches derived block hash
- Validate versioned_blob_hashes against transaction blob hashes
- Execute block statelessly (reuse existing `execute_blocks`)
- Compute `hash_tree_root(NewPayloadRequest)` for public input
- Wrap in panic handler: commit `(root, false)` on failure
- Feature-gated: `#[cfg(feature = "eip-8025")]`
- File: `crates/guest-program/src/l1/program.rs`

### T5.4: Update zkVM guest binaries
- SP1, RISC0, ZisK, OpenVM guest `main.rs` files
- Read `NewPayloadRequest` + witness as input
- Commit `(hash_tree_root, valid)` as public output
- Feature-gated
- Files: `crates/guest-program/bin/*/src/main.rs`

## Phase 6: Feature Flag and CLI Integration

### T6.1: Feature flag wiring
- Feature chain: `cmd/ethrex` → `ethrex-rpc/eip-8025` + `ethrex-blockchain/eip-8025` → `ethrex-common/eip-8025` (SSZ types)
- `crates/common/Cargo.toml`: `eip-8025 = ["dep:libssz"]`
- `crates/blockchain/Cargo.toml`: `eip-8025 = ["ethrex-common/eip-8025"]`
- `crates/networking/rpc/Cargo.toml`: `eip-8025 = ["ethrex-blockchain/eip-8025", "ethrex-common/eip-8025"]`
- `crates/guest-program/Cargo.toml`: `eip-8025 = ["ethrex-common/eip-8025"]`
- `cmd/ethrex/Cargo.toml`: `eip-8025 = ["ethrex-blockchain/eip-8025", "ethrex-rpc/eip-8025"]`
- Gate all new code behind `#[cfg(feature = "eip-8025")]`
- Ensure `cargo build` without the feature compiles cleanly (no EIP-8025 code included)
- Ensure `cargo build --features eip-8025` compiles

### T6.2: Absorb `--precompute-witnesses`
- Remove `--precompute-witnesses` CLI flag
- Remove `precompute_witnesses` from `BlockchainOptions`
- Under `eip-8025` feature, always enable witness precomputation
- Ensure `debug_executionWitness` RPC still works (uses stored witnesses)

### T6.3: CLI flags for EIP-8025
- `--proof-callback.url <url>`: URL to POST `GeneratedProof` (Beacon API endpoint)
- `--proof-coordinator.addr <addr>`: bind address for ProofCoordinator TCP server
- `--proof-coordinator.port <port>`: port for ProofCoordinator TCP server
- `--proof-*` flags only available when compiled with `eip-8025` feature
- ProofEngine always starts when `eip-8025` feature is enabled
- Gated behind `#[cfg(feature = "eip-8025")]`
- File: `cmd/ethrex/cli.rs`

### T6.4: Node initialization
- Create ProofEngine during startup if `eip-8025` feature is enabled
- Start ProofCoordinator TCP server on `--proof-coordinator.addr`:`--proof-coordinator.port`
- Pass `Arc<ProofEngine>` into `RpcApiContext`
- File: `cmd/ethrex/initializers.rs`

## Phase 7: Testing

### T7.1: Unit tests for SSZ types
- Test `hash_tree_root` computation for `NewPayloadRequest`
- Compare against known values (e.g., from ere-guests integration tests using Lighthouse-derived roots)

### T7.2: Integration tests for Engine API endpoints
- Test `engine_requestProofsV1` returns ProofGenId
- Test `engine_requestProofsV1` returns -39004 when no prover configured
- Test `engine_verifyExecutionProofV1` with valid/invalid/unsupported proofs
- Test `engine_verifyNewPayloadRequestHeaderV1` returns SYNCING when no proofs, VALID when proofs exist

### T7.3: Integration test for full prove-verify cycle
- Generate witness for a test block
- Build ProgramInput
- Use Exec backend to "prove" (fast, no real zkVM)
- Verify proof
- Check ProofStore contains verified proof
- Check `engine_verifyNewPayloadRequestHeaderV1` returns VALID

### T7.4: Ensure L2 prover tests still pass
- After extracting shared infrastructure, run all existing L2 prover tests
- Verify no regressions in L2 proving flow

## Phase 8: (Removed — libssz used from the start, SSZ types in ethrex-common per T2.1)
