# EIP-8025: Optional Execution Proofs
- **Layer:** CL (with an execution-layer artifact: the stateless guest program)
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acdc/178, 2026-05-14
- **Authors:** Kevaundray Wedderburn, Justin Drake, Ignacio Hagopian, Han, Francesco Risitano, Cody Gunton
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8025) · [discussion](https://ethereum-magicians.org/t/eip-optional-execution-proofs/25500)
- **Prior fork history:** none

## TL;DR
Lets consensus nodes verify execution payloads via zkEVM proofs gossiped over the CL P2P network instead of (well, in addition to) re-executing them locally. Two new opt-in roles: altruistic **provers** generate proofs, **zkAttesters** verify them as a supplementary validity signal. Fully opt-in, changes no consensus validity rules — nodes that don't opt in see zero change. It is the "optional first" phase of making L1 zkEVM-proven, with a mandatory phase deferred to a later fork.

## What it changes
- New CL gossip topic `execution_proof` carrying `SignedExecutionProof { message: ExecutionProof, validator_index, signature }` (BLS, `DOMAIN_EXECUTION_PROOF`).
- `ExecutionProof = { proof_data: ByteList[MAX_PROOF_SIZE=400KiB], proof_type: uint8, public_input }`; public input binds the proof to `hash_tree_root(NewPayloadRequest)`, a `successful_validation` bool, and the chain config. Up to 4 proof types per payload (`MAX_EXECUTION_PROOFS_PER_PAYLOAD`); multi-proof-type support so zkVMs can evolve independently.
- New `ProofEngine` interface on the CL (modelled on the Engine API): `request_proofs`, `verify_execution_proof`, `notify_new_payload`, `notify_forkchoice_updated`. Real proving work is delegated to an external proof node behind SSE event streams.
- `process_block` is extended to also notify the proof engine; a new `process_execution_proof` handler runs **outside** the state transition (checks prover is an active validator, verifies BLS sig, delegates proof verification).
- New req/resp protocols `ExecutionProofsByRange` / `ExecutionProofsByRoot` / `ExecutionProofStatus`; new `eproof` ENR key for discovery. Proof-aware nodes must retain proofs back to the finalized checkpoint.
- EL side (normative in execution-specs): a **stateless guest program** that validates a payload from a `StatelessInput` (NewPayloadRequest + `ExecutionWitness` of trie nodes/codes/headers + chain_id + tx signer public keys), serialized as SSZ with a 2-byte schema prefix (Amsterdam = `0x1501`), and emits `StatelessValidationResult` as public output. Witness construction reuses the BAL read/write tracker.

## Motivation
Payload re-execution cost scales linearly with gas limit and requires full EL state, coupling validator hardware requirements to L1 scaling. Proof verification is constant-time and stateless, decoupling validation cost from both. Shipping opt-in first lets the proving stack (proof size, latency, gossip behavior, prover diversity) mature under mainnet conditions without putting fork choice or attestation on the critical path, and anchors the large spec/tooling effort in the upgrade process.

## Dependencies & related EIPs
- **Depends on:** EIP-4844 (versioned hashes), 6110/7002/7251 (typed execution requests), 7688 (progressive containers for the SSZ schema), 7732 (ePBS — the payload-validation window gives provers time), 7928 (BAL — `block_access_list` field in the SSZ payload + witness tracker), 8282 (builder deposit/exit requests).
- **Synergizes with:** EIP-7928 (BALs feed witness construction) and the statelessness roadmap generally.
- **Related:** a future "mandatory execution proofs" EIP would make proofs load-bearing and let attesters drop the stateful EL; this EIP explicitly does not do that.

## Impact on ethrex / client teams
- EL work is real but not consensus-critical: implement the stateless guest program, execution witness construction, host-side input assembly, and the SSZ stateless input/output schema. ethrex already has `crates/guest-program` (its own zkVM guest for L2 proving) and stateless-validation infrastructure, which is a head start, but the exact EIP-8025 `StatelessInput` schema and conformance tests are new.
- No Engine API, EVM, gas, tx-pool, or devp2p changes; the EVM is unmodified.
- CL work (not ethrex): gossip topic, ProofEngine, req/resp, ENR.
- Rough effort: **medium-large** on the EL side if ethrex participates as host/guest; zero if it doesn't (fully opt-in).

## Open questions & controversies
- The "k valid proofs per payload" threshold for calling a payload proof-verified is deliberately unpinned until real data exists.
- No incentives for provers — relies on altruism; proving is O(n log² n) and needs full EL state, so there is a verifier/prover centralization asymmetry.
- Validator opt-in guidance is unsettled (late proofs cost timeliness rewards; extra bandwidth on the propagation path).
- Overall risk is low by design: a forged or buggy proof cannot fork the chain; the EL remains authoritative.
