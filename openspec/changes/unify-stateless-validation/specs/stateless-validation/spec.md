## ADDED Requirements

### Requirement: StatelessValidator trait in LEVM

LEVM SHALL define a `StatelessValidator` trait with a single method that accepts raw bytes (SSZ-encoded `StatelessInput`) and returns raw bytes (SSZ-encoded `StatelessValidationResult`) or an error. The trait SHALL be optional on the EVM context — only present when stateless validation is enabled.

#### Scenario: Trait is available when constructing EVM with stateless support
- **WHEN** the EVM is constructed with a `StatelessValidator` implementation
- **THEN** the EXECUTE precompile at address `0x0101` is callable and delegates to the trait

#### Scenario: Trait is absent on standard EVM
- **WHEN** the EVM is constructed without a `StatelessValidator` implementation
- **THEN** calls to the EXECUTE precompile address revert

### Requirement: verify_stateless_new_payload function

The `crates/blockchain/` crate SHALL expose a `verify_stateless_new_payload` function that:
1. Accepts a `StatelessInput` (containing `NewPayloadRequest`, `ExecutionWitness`, `ChainConfig`, `public_keys`)
2. Computes `new_payload_request_root` via SSZ `hash_tree_root` of the `NewPayloadRequest`
3. Validates block headers from the witness
4. Builds a `GuestProgramState` from the witness
5. Converts `NewPayloadRequest` to a `Block` and executes it via LEVM
6. Returns `StatelessValidationResult` with the root, success boolean, and chain config

#### Scenario: Valid block execution
- **WHEN** `verify_stateless_new_payload` is called with a valid `StatelessInput` where the `ExecutionPayload` fields match the result of executing the transactions against the witness state
- **THEN** it returns `StatelessValidationResult { new_payload_request_root, successful_validation: true, chain_config }`

#### Scenario: Invalid state root
- **WHEN** `verify_stateless_new_payload` is called with a `StatelessInput` where the `ExecutionPayload.state_root` does not match the post-execution state
- **THEN** it returns `StatelessValidationResult { new_payload_request_root, successful_validation: false, chain_config }`

#### Scenario: Invalid receipts root
- **WHEN** the `ExecutionPayload.receipts_root` does not match computed receipts
- **THEN** it returns `successful_validation: false`

### Requirement: verify_stateless_new_payload implements StatelessValidator

The `crates/blockchain/` crate SHALL provide a struct implementing the `StatelessValidator` trait. The implementation SHALL deserialize the SSZ input into `StatelessInput`, call `verify_stateless_new_payload`, and serialize the result back to SSZ bytes.

#### Scenario: Trait implementation wires correctly
- **WHEN** the `StatelessValidator` trait's `verify` method is called with SSZ-encoded bytes
- **THEN** it deserializes, calls `verify_stateless_new_payload`, and returns SSZ-encoded `StatelessValidationResult`

### Requirement: Shared function across entry points

The same `verify_stateless_new_payload` function SHALL be callable from:
1. The EXECUTE precompile (via `StatelessValidator` trait)
2. The EIP-8025 RPC proof generation flow
3. The zkVM guest program

#### Scenario: EIP-8025 RPC uses shared function
- **WHEN** `engine_requestProofsV1` triggers proof generation
- **THEN** it calls `verify_stateless_new_payload` (not a separate `execution_program`)

#### Scenario: Guest program uses shared function
- **WHEN** the zkVM guest binary executes
- **THEN** it calls `verify_stateless_new_payload` with a `StatelessInput` built from its input

### Requirement: Delete duplicated execute_block from LEVM

The custom `execute_block` function currently in `crates/vm/levm/src/execute_precompile.rs` SHALL be removed. All block execution SHALL go through `verify_stateless_new_payload`.

#### Scenario: No duplicate execution logic
- **WHEN** the EXECUTE precompile processes a call
- **THEN** it delegates to `verify_stateless_new_payload` via the trait, not to any local execution function
