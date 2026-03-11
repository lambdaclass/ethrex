## ADDED Requirements

### Requirement: Static block warming

The system SHALL pre-warm state before block execution by statically analyzing transaction data and contract bytecode, without speculative EVM execution.

#### Scenario: Extract call targets from transactions
- **WHEN** warming a block with transactions
- **THEN** the system SHALL extract all target addresses from `tx.to()` fields and batch prefetch those accounts

#### Scenario: Predict CREATE2 addresses
- **WHEN** warming a block with CREATE2 transactions
- **THEN** the system SHALL compute CREATE2 addresses from sender + salt + initcode hash and prefetch those accounts

#### Scenario: Extract static storage keys from bytecode
- **WHEN** warming a block with contract calls
- **THEN** the system SHALL scan called contracts' bytecode for PUSH1/PUSH2 opcodes followed by SLOAD, and prefetch those storage slots

#### Scenario: Handle missing code gracefully
- **WHEN** warming encounters a contract with code not in database
- **THEN** the system SHALL skip bytecode analysis for that contract but still prefetch the account

#### Scenario: Produce identical execution results
- **WHEN** blocks are executed with static warming vs speculative execution
- **THEN** the system SHALL produce identical block results (same receipts, state root, gas used)
