# EIP-8025: Optional Execution Proofs

## Status: Draft

## Motivation

EIP-8025 enables beacon nodes to verify execution payload validity without re-executing transactions, using zkEVM proofs instead. ethrex is uniquely positioned to implement this — it already has a complete witness generation pipeline, guest programs for stateless re-execution inside zkVMs (SP1, RISC0, ZisK, OpenVM), and a distributed proving infrastructure for L2. This change brings those capabilities to L1 via the Engine API.

## Summary

Implement EIP-8025 in ethrex under the `eip-8025` feature flag. This adds:

1. **Three new Engine API endpoints** for proof generation, verification, and header checking
2. **A ProofEngine module** in `crates/blockchain/` that coordinates witness generation, proof storage, and distributed proof generation
3. **A shared prover infrastructure** extracted from the L2 prover codebase, enabling both L1 (EIP-8025) and L2 to reuse the same `ProverBackend` trait and `Prover` pull loop. `ProofCoordinator` is NOT shared — L2 keeps its own (L2-specific: StoreRollup, aligned mode, TDX, batch numbering), and L1 gets a new dedicated `L1ProofCoordinator`
4. **Modified guest program** aligned with the [ere-guests](https://github.com/eth-act/ere-guests) design — input becomes `NewPayloadRequest` + `ExecutionWitness`, output becomes `(hash_tree_root(NewPayloadRequest), valid: bool)`
5. **SSZ support** via [libssz](https://github.com/lambdaclass/libssz) for `NewPayloadRequest` hash tree root computation (ere-guests will adopt libssz)
6. **Distributed multi-proof generation** using a pull-model coordinator (same pattern as L2), where multiple prover workers with different zkVM backends connect to the coordinator and independently prove the same input
7. **Async proof delivery** via `POST /eth/v1/prover/execution_proofs` using `GeneratedProof` (wraps `ExecutionProof` + `ProofGenId` to link proofs back to the original request)

## Scope

### In scope

- Engine API: `engine_requestProofsV1`, `engine_verifyExecutionProofV1`, `engine_verifyNewPayloadRequestHeaderV1`
- `ProofEngine` as a `crates/blockchain/` module
- Persistent `EXECUTION_PROOFS` table in Store (128-block retention)
- ProofCoordinator for distributed proving (pull model, reusing L2 pattern)
- Callback delivery: HTTP POST to Beacon API `POST /eth/v1/prover/execution_proofs`
- Guest program modification for EIP-8025 public input format
- Absorb `--precompute-witnesses` into the `eip-8025` feature
- Extract shared prover infrastructure from `crates/l2/prover/` to a shared location

### Out of scope

- Consensus layer (beacon chain) changes — ethrex is an EL client
- P2P gossip topics, req/resp protocols, MetaData/ENR changes (CL concerns)
- BLS signature verification of `SignedExecutionProof` (CL concern)
- Prover whitelist management (CL concern, uses validator set)

## References

- [EIP-8025](https://eips.ethereum.org/EIPS/eip-8025)
- [Consensus specs PR #4828](https://github.com/ethereum/consensus-specs/pull/4828)
- [Engine API PR #735](https://github.com/ethereum/execution-apis/pull/735)
- [Beacon API PR #569](https://github.com/ethereum/beacon-APIs/pull/569)
- [ere-guests PR #7](https://github.com/eth-act/ere-guests/pull/7) — guest program design for EIP-8025
- [libssz](https://github.com/lambdaclass/libssz)

## Spec Notes

The Engine API spec (PR #735) has internal inconsistencies we follow the markdown version for:

1. **`ProofStatusV1`**: The markdown defines `status` as a string enum (`VALID`/`INVALID`/`SYNCING`/`NOT_SUPPORTED`), but the OpenRPC schema defines it as `{valid: boolean}`. We follow the markdown — a boolean can't express SYNCING or NOT_SUPPORTED.
2. **`engine_requestProofsV1` params**: The markdown defines 5 params (including `executionRequests`), but the OpenRPC methods file only lists 4 (omitting `executionRequests`). We follow the markdown — `executionRequests` is essential to build `NewPayloadRequest`.
3. **`proofType` width**: The Engine API uses `QUANTITY, 64 Bits` (uint64) while the Beacon API SSZ container uses `Uint8`. We use u64 in the Engine API RPC layer to match the spec exactly.
