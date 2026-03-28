## Context

The ethrex project has two overlapping implementations of stateless block validation:

1. **EIP-8025 (PR #6361)**: Adds SSZ types (`ExecutionPayload`, `NewPayloadRequest`), a dual-mode guest program, and Engine API RPC endpoints for requesting/verifying execution proofs. The function `execution_program()` takes an SSZ `NewPayloadRequest` + `ExecutionWitness`, executes the block, and returns a 33-byte commitment (SSZ root + validity bool).

2. **Native rollups EXECUTE precompile (this branch)**: Implements EIP-8079 PoC with a custom `execute_block()` duplicated inside LEVM, ABI-encoded input (15 slots of individual fields), and ABI-encoded output (160 bytes). Uses a separate `native-rollup` feature flag.

The l2beat native rollups spec (rewritten March 2026) now defines the EXECUTE precompile as a thin wrapper around `verify_stateless_new_payload` — the same function from the L1 ZK-EVM effort (`ethereum/execution-specs`, `projects/zkevm` branch). The spec uses SSZ serialization and standard types (`StatelessInput`, `StatelessValidationResult`).

Both implementations share the same core: execute a block statelessly and return a commitment. The difference is the entry point and encoding.

## Goals / Non-Goals

**Goals:**
- Single `verify_stateless_new_payload` function reusable from all entry points (EXECUTE precompile, EIP-8025 RPC, zkVM guest)
- SSZ-based input/output matching the execution-specs types
- Clean cycle-free dependency graph via trait injection
- EXECUTE precompile aligned with the l2beat spec (SSZ, preprocessing layer, gas charging)
- Both demos working end-to-end (EIP-8025 proof flow + native rollups advance flow)
- Clean, readable code for EF review

**Non-Goals:**
- Implementing the ZK variant (proof-carrying transactions, PROOFROOT opcode) — only the re-execution variant
- Production-grade gas metering for the EXECUTE precompile
- Implementing `public_keys` recovery (empty tuple for now)
- Forced transactions or FOCIL integration
- Custom gas token support beyond the preminted L2Bridge pattern

## Decisions

### 1. Cycle-breaking: StatelessValidator trait in LEVM

`verify_stateless_new_payload` needs LEVM (to execute blocks), and LEVM needs `verify_stateless_new_payload` (for the EXECUTE precompile). This is a Rust crate dependency cycle.

**Decision:** Define a `StatelessValidator` trait in LEVM with a single `verify` method. Implement it in `crates/blockchain/`. Inject it into the EVM context at construction time. The EXECUTE precompile calls through the trait; the guest program calls `verify_stateless_new_payload` directly.

**Alternatives considered:**
- *Put everything in LEVM*: Avoids the cycle but makes LEVM responsible for orchestration logic (header validation, witness building, state root checking). Architecturally unclear for reviewers.
- *Callback/function pointer*: Less idiomatic Rust, harder to test.

**Rationale:** Trait injection is standard Rust for breaking cycles, keeps LEVM focused on EVM execution, and mirrors the spec's separation (precompile is thin shell, real logic is the standard function).

### 2. Function placement: `crates/blockchain/`

**Decision:** `verify_stateless_new_payload` lives in `crates/blockchain/` (or a submodule like `crates/blockchain/stateless.rs`). This crate already depends on LEVM and owns block execution orchestration.

**Alternatives considered:**
- *New `crates/stateless/` crate*: Cleaner isolation but adds a new crate for one function. Overhead not justified yet.
- *`crates/guest-program/`*: Too specific — this function is shared, not guest-specific.

### 3. SSZ types: Extend existing `eip8025_ssz.rs`

**Decision:** Add `SszStatelessInput`, `SszStatelessValidationResult`, `SszChainConfig`, and `SszExecutionWitness` to the same module that already defines `ExecutionPayload` and `NewPayloadRequest`. Reuse the existing `libssz` dependency.

**Alternatives considered:**
- *Separate `stateless_ssz.rs` file*: Would duplicate some imports and split closely-related types across files.

**Rationale:** These types are extensions of the EIP-8025 SSZ types. Same serialization library, same conceptual domain.

### 4. ExecutionWitness: SSZ at the boundary, rkyv internally

**Decision:** Define an `SszExecutionWitness` matching the spec (`state: List[Bytes]`, `codes: List[Bytes]`, `headers: List[Bytes]`). The EXECUTE precompile deserializes SSZ input into `SszStatelessInput`, then converts `SszExecutionWitness` → internal `ExecutionWitness` (with trie nodes, storage roots, etc.) inside `verify_stateless_new_payload`.

**Rationale:** The spec's SSZ format is the public API. Our internal `ExecutionWitness` with embedded trie structures is an implementation detail optimized for our execution engine.

### 5. Feature flag: `stateless-validation`

**Decision:** Unify `native-rollup` and `eip-8025` into a single `stateless-validation` feature flag.

**Rationale:** Both features depend on the same SSZ types, the same `verify_stateless_new_payload` function, and the same stateless execution infrastructure. Separate flags would create combinatorial complexity with no practical benefit.

### 6. L1 anchoring: Merkle root via `parent_beacon_block_root`

**Decision:** Use the `parent_beacon_block_root` field in `NewPayloadRequest` as a generic `bytes32` transport for the L1 messages Merkle root (the commutative Merkle tree root over consumed L1 messages). On L2, this value is accessible via the EIP-4788 `BEACON_ROOTS_ADDRESS` predeploy. Remove the dedicated `L1Anchor` predeploy — it's replaced by the standard EIP-4788 contract.

The anchored value is the **Merkle root of messages** (not an L1 block hash), because:
- The Merkle root is deterministic: both the L2 block producer and the L1 `advance()` call compute the same root from the same messages.
- An L1 block hash cannot be used because `blockhash(block.number - 1)` at `advance()` time is unknowable when the L2 block is produced off-chain. The L2 block producer would not know which L1 block the advancer will call from.
- L2Bridge verification stays lightweight (single Merkle inclusion proof instead of MPT account + storage proofs against L1 state).

**Alternatives considered:**
- *L1 block hash anchor*: More general (can prove any L1 state), but creates a timing problem — L2 block producer cannot predict the L1 block hash at advance time. Would also require heavier MPT proofs on L2.
- *Keep dedicated L1Anchor predeploy*: Works but adds an extra predeploy and system write that the standard EIP-4788 mechanism already provides.

**Trade-off:** Changes the semantics of `parent_beacon_block_root` on L2 — it carries a messages Merkle root, not a real beacon root. The spec's L1 anchoring page explicitly supports passing an arbitrary `bytes32` through this field.

### 7. NativeRollup.sol: Align with spec contract

**Decision:** Rewrite the contract to match the spec's storage layout and `advance()` interface:
- Store: `blockHash`, `stateRoot`, `blockNumber`, `gasLimit`, `chainId`, `stateRootHistory`
- Remove: `lastBaseFeePerGas`, `lastGasUsed`, `relayer`, `advancer`
- `advance()` passes SSZ-encoded `StatelessInput` to EXECUTE and decodes `StatelessValidationResult`
- L1 anchor via `blockhash(block.number - 1)`

**Note:** Messaging (`sendL1Message`, `claimWithdrawal`) stays — it's needed for the demo and aligns with the spec's separate messaging pages.

### 8. Gas charging: Use `execution_payload.gas_used`

**Decision:** The EXECUTE precompile charges gas equal to the L2 block's `gas_used` field from the `ExecutionPayload`, as the spec describes.

**Trade-off:** This means L1 callers pay proportional to L2 activity. For the PoC this is acceptable. Production gas metering is a spec open question.

## Risks / Trade-offs

- **SSZ complexity on-chain**: The NativeRollup.sol `advance()` must construct SSZ-encoded `StatelessInput` from its storage + calldata. SSZ encoding in Solidity is more complex than ABI encoding. → Mitigation: Keep the Solidity SSZ encoding minimal — only encode what's needed, use helper functions.

- **Witness conversion overhead**: Converting `SszExecutionWitness` → internal `ExecutionWitness` inside the EXECUTE precompile adds overhead during L1 execution. → Mitigation: This is a PoC. The spec's re-execution variant is explicitly not for production.

- **`parent_beacon_block_root` semantics**: Repurposing this field means L2 contracts that expect a real beacon root will get an L1 anchor instead. → Mitigation: L2 contracts are purpose-built for native rollups and know this.

- **Merging PR #6361**: The PR is large (81 files, 6k+ lines) and not yet merged to main. Merge conflicts are likely. → Mitigation: Create the combined branch early, resolve conflicts once, and keep both demos as integration test anchors.

- **Single feature flag**: If someone wants EIP-8025 without native rollups (or vice versa), they can't. → Mitigation: Acceptable for now. Can split later if needed. Both features share the same core infrastructure.
