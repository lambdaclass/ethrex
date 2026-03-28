## ADDED Requirements

### Requirement: SszStatelessInput container

The system SHALL define an SSZ container `SszStatelessInput` with the following fields:
- `new_payload_request`: `SszNewPayloadRequest` (reused from EIP-8025)
- `witness`: `SszExecutionWitness`
- `chain_config`: `SszChainConfig`
- `public_keys`: `SszList<SszList<u8, 48>, MAX_PUBLIC_KEYS>` (empty for PoC)

The container SHALL be serializable and deserializable via `libssz`, and SHALL support `hash_tree_root`.

#### Scenario: Round-trip serialization
- **WHEN** a `SszStatelessInput` is serialized to bytes and deserialized back
- **THEN** all fields match the original values

#### Scenario: Empty public_keys accepted
- **WHEN** `SszStatelessInput` is constructed with an empty `public_keys` list
- **THEN** serialization and deserialization succeed

### Requirement: SszStatelessValidationResult container

The system SHALL define an SSZ container `SszStatelessValidationResult` with the following fields:
- `new_payload_request_root`: `[u8; 32]` (SSZ `hash_tree_root` of the `NewPayloadRequest`)
- `successful_validation`: `bool`
- `chain_config`: `SszChainConfig`

#### Scenario: Round-trip serialization
- **WHEN** a `SszStatelessValidationResult` is serialized and deserialized
- **THEN** all fields match the original values

#### Scenario: Successful validation result
- **WHEN** `successful_validation` is `true`
- **THEN** the serialized output correctly encodes the boolean as SSZ `true`

### Requirement: SszChainConfig container

The system SHALL define an SSZ container `SszChainConfig` with:
- `chain_id`: `u64`

#### Scenario: Chain config serialization
- **WHEN** `SszChainConfig { chain_id: 1 }` is serialized
- **THEN** it produces the correct SSZ encoding (8 bytes, little-endian)

### Requirement: SszExecutionWitness container

The system SHALL define an SSZ container `SszExecutionWitness` matching the execution-specs definition:
- `state`: `SszList<SszList<u8, MAX_WITNESS_NODE_SIZE>, MAX_WITNESS_NODES>` (trie-node preimages)
- `codes`: `SszList<SszList<u8, MAX_CODE_SIZE>, MAX_WITNESS_CODES>` (contract code preimages)
- `headers`: `SszList<SszList<u8, MAX_HEADER_SIZE>, MAX_WITNESS_HEADERS>` (RLP-encoded block headers, max 256)

#### Scenario: Witness with state nodes
- **WHEN** an `SszExecutionWitness` contains trie-node preimages in `state`
- **THEN** they are correctly serialized and deserializable

#### Scenario: Witness with 256 headers
- **WHEN** an `SszExecutionWitness` contains 256 headers (maximum)
- **THEN** serialization succeeds

### Requirement: Conversion from SszExecutionWitness to internal ExecutionWitness

The system SHALL provide a conversion from `SszExecutionWitness` + `SszChainConfig` to the internal `ExecutionWitness` type (which contains embedded trie structures, storage roots, and chain config). This conversion SHALL be used inside `verify_stateless_new_payload` after deserialization.

#### Scenario: SSZ witness converts to internal format
- **WHEN** an `SszExecutionWitness` with valid trie nodes is converted to internal `ExecutionWitness`
- **THEN** the resulting `ExecutionWitness` can build a valid `GuestProgramState`

### Requirement: chain_config as sibling field

The `ChainConfig` SHALL be a field on `StatelessInput`, not embedded inside `ExecutionWitness`. The internal `ExecutionWitness` type MAY still carry `chain_config` for backward compatibility, but the canonical source for `verify_stateless_new_payload` SHALL be `StatelessInput.chain_config`.

#### Scenario: ChainConfig read from StatelessInput
- **WHEN** `verify_stateless_new_payload` executes
- **THEN** it reads `chain_config` from `StatelessInput.chain_config`, not from the witness
