## ADDED Requirements

### Requirement: SSZ input format

The EXECUTE precompile SHALL accept SSZ-serialized `StatelessInput` as its calldata. The precompile SHALL deserialize the input and reject malformed SSZ with an exceptional halt.

#### Scenario: Valid SSZ input
- **WHEN** the precompile receives valid SSZ-encoded `StatelessInput`
- **THEN** it successfully deserializes and proceeds with validation

#### Scenario: Malformed SSZ input
- **WHEN** the precompile receives bytes that are not valid SSZ for `StatelessInput`
- **THEN** it reverts with an exceptional halt

### Requirement: SSZ output format

The EXECUTE precompile SHALL return SSZ-serialized `StatelessValidationResult` as its output. The output contains `new_payload_request_root`, `successful_validation`, and `chain_config`.

#### Scenario: Successful validation output
- **WHEN** block execution succeeds and all roots match
- **THEN** the precompile returns SSZ-encoded `StatelessValidationResult` with `successful_validation: true`

#### Scenario: Failed validation output
- **WHEN** block execution fails (state root mismatch, invalid transactions, etc.)
- **THEN** the precompile reverts with an exceptional halt (per spec: `raise ExceptionalHalt`)

### Requirement: Gas charging based on gas_used

The EXECUTE precompile SHALL charge gas equal to `StatelessInput.new_payload_request.execution_payload.gas_used`. This replaces the fixed 100,000 gas cost.

#### Scenario: Gas charged proportionally
- **WHEN** the `ExecutionPayload` reports `gas_used: 500_000`
- **THEN** the precompile charges 500,000 gas to the L1 caller

#### Scenario: Insufficient L1 gas
- **WHEN** the L1 caller has less gas remaining than `execution_payload.gas_used`
- **THEN** the precompile reverts with out-of-gas

### Requirement: L2-specific preprocessing layer

Before calling `verify_stateless_new_payload`, the EXECUTE precompile SHALL validate L2 constraints:
- `execution_payload.blob_gas_used` MUST equal 0
- `execution_payload.excess_blob_gas` MUST equal 0
- `execution_payload.withdrawals` MUST be empty
- `new_payload_request.execution_requests` MUST be empty
- Blob transactions (EIP-4844) in the transaction list MUST be rejected

If any constraint is violated, the precompile SHALL revert.

#### Scenario: Valid L2 block with no blobs
- **WHEN** all L2 constraints are satisfied
- **THEN** preprocessing passes and `verify_stateless_new_payload` is called

#### Scenario: Block with blob_gas_used > 0
- **WHEN** `execution_payload.blob_gas_used` is not zero
- **THEN** the precompile reverts

#### Scenario: Block containing a blob transaction
- **WHEN** the transaction list contains an EIP-4844 blob transaction
- **THEN** the precompile reverts

### Requirement: L1 anchor via parent_beacon_block_root

The EXECUTE precompile SHALL use the `parent_beacon_block_root` field from `NewPayloadRequest` as the L1 messages Merkle root anchor. The NativeRollup.sol contract computes this root from consumed L1 messages and passes it in the `StatelessInput`. During L2 block execution, the EVM system contract at `BEACON_ROOTS_ADDRESS` (EIP-4788) stores this value automatically. The dedicated `L1Anchor` predeploy SHALL be deleted.

#### Scenario: L1 messages Merkle root accessible on L2
- **WHEN** the EXECUTE precompile processes a block with `parent_beacon_block_root` set to the L1 messages Merkle root
- **THEN** L2 contracts can read this Merkle root via `BEACON_ROOTS_ADDRESS` using `block.timestamp`

#### Scenario: L2Bridge verifies message inclusion
- **WHEN** `L2Bridge.processL1Message` is called with a Merkle proof
- **THEN** it reads the Merkle root from `BEACON_ROOTS_ADDRESS` and verifies the proof against it (same verification as before, different source)

### Requirement: Delegation to StatelessValidator trait

The EXECUTE precompile SHALL NOT contain block execution logic. It SHALL:
1. Deserialize SSZ input (to extract `gas_used` and validate L2 constraints)
2. Charge gas
3. Run L2-specific preprocessing
4. Delegate the full SSZ bytes to the `StatelessValidator` trait
5. Return the trait's SSZ output

#### Scenario: Precompile delegates to trait
- **WHEN** the EXECUTE precompile is called with valid input
- **THEN** it calls `StatelessValidator::verify` with the SSZ bytes and returns the result

### Requirement: NativeRollup.sol contract alignment

The `NativeRollup.sol` L1 contract SHALL store:
- `blockHash` (bytes32)
- `stateRoot` (bytes32)
- `blockNumber` (uint256)
- `gasLimit` (uint256)
- `chainId` (uint256)
- `stateRootHistory` (mapping blockNumber → stateRoot)

The `advance()` function SHALL:
1. Compute L1 anchor via `blockhash(block.number - 1)`
2. Construct SSZ-encoded `StatelessInput` from storage, calldata, and L1 anchor
3. Call the EXECUTE precompile with the SSZ bytes
4. Decode `StatelessValidationResult` and verify `successful_validation` and `chainId`
5. Update on-chain state

The contract SHALL retain messaging functions (`sendL1Message`, `claimWithdrawal`) for the demo.

#### Scenario: Successful advance
- **WHEN** `advance()` is called with valid block data and witness
- **THEN** the contract calls EXECUTE, verifies the result, and updates `stateRoot`, `blockNumber`, `blockHash`, and `stateRootHistory`

#### Scenario: Failed validation reverts advance
- **WHEN** the EXECUTE precompile returns `successful_validation: false` (or reverts)
- **THEN** the `advance()` transaction reverts

#### Scenario: Chain ID mismatch
- **WHEN** the `StatelessValidationResult.chain_config.chain_id` does not match the stored `chainId`
- **THEN** the `advance()` transaction reverts

### Requirement: Unified feature flag

All stateless validation code (SSZ types, `verify_stateless_new_payload`, EXECUTE precompile, EIP-8025 RPC endpoints) SHALL be gated behind a single `stateless-validation` feature flag.

#### Scenario: Feature disabled
- **WHEN** the `stateless-validation` feature is not enabled
- **THEN** none of the SSZ types, stateless validation code, or EXECUTE precompile are compiled

#### Scenario: Feature enabled
- **WHEN** the `stateless-validation` feature is enabled
- **THEN** both the EIP-8025 RPC endpoints and the EXECUTE precompile are available
