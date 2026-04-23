// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import "./MPTProof.sol";

/// @title NativeRollup — L2 state manager using the EXECUTE precompile.
///
/// Aligned with the l2beat native rollups spec.
/// The `advance()` method forwards SSZ-encoded `StatelessInput` to the
/// EXECUTE precompile, which calls `verify_stateless_new_payload` and
/// returns SSZ-encoded `StatelessValidationResult`.
///
/// Storage layout:
///   Slot 0: blockHash (bytes32)
///   Slot 1: stateRoot (bytes32)
///   Slot 2: blockNumber (uint256)
///   Slot 3: gasLimit (uint256)
///   Slot 4: chainId (uint256)
///   Slot 5: pendingL1Messages (bytes32[])
///   Slot 6: l1MessageIndex (uint256)
///   Slot 7: stateRootHistory (mapping(uint256 => bytes32))
///   Slot 8: claimedWithdrawals (mapping(bytes32 => bool))
///   Slot 9: stateRootTimestamps (mapping(uint256 => uint256))
///   Slot 10: _locked (bool)

contract NativeRollup {
    // ===== L2 chain state (spec-aligned) =====
    bytes32 public blockHash;
    bytes32 public stateRoot;
    uint256 public blockNumber;
    uint256 public gasLimit;
    uint256 public chainId;

    // ===== L1→L2 messaging =====
    bytes32[] public pendingL1Messages;
    uint256 public l1MessageIndex;

    // ===== L2→L1 messaging (state proof-based) =====
    mapping(uint256 => bytes32) public stateRootHistory;
    mapping(bytes32 => bool) public claimedWithdrawals;
    mapping(uint256 => uint256) public stateRootTimestamps;

    // ===== Reentrancy guard =====
    bool private _locked;

    // ===== Immutables =====
    uint256 public immutable CHAIN_ID;
    uint256 public immutable FINALITY_DELAY;

    address constant EXECUTE_PRECOMPILE = address(0x0101);
    address constant L2_BRIDGE_ADDRESS = address(0x000000000000000000000000000000000000FfFD);
    // Storage slot of `sentMessages` in L2Bridge.sol. Must match that layout —
    // reordering L2Bridge storage silently breaks withdrawal proofs here.
    uint256 constant SENT_MESSAGES_SLOT = 3;

    // ===== Events =====
    event StateAdvanced(uint256 indexed newBlockNumber, bytes32 indexed newStateRoot);
    event L1MessageRecorded(address indexed sender, address indexed to, uint256 value, uint256 gasLimit, bytes data, uint256 indexed nonce);
    event WithdrawalClaimed(address indexed receiver, uint256 amount, uint256 blockNumber, uint256 messageId);

    modifier nonReentrant() {
        require(!_locked, "ReentrancyGuard: reentrant call");
        _locked = true;
        _;
        _locked = false;
    }

    constructor(
        bytes32 _initialStateRoot,
        bytes32 _initialBlockHash,
        uint256 _blockGasLimit,
        uint256 _chainId
    ) {
        stateRoot = _initialStateRoot;
        blockHash = _initialBlockHash;
        blockNumber = 0;
        gasLimit = _blockGasLimit;
        chainId = _chainId;
        CHAIN_ID = _chainId;
        FINALITY_DELAY = 0;
    }

    // ===== L1 Messaging =====

    function sendL1Message(address _to, uint256 _gasLimit, bytes calldata _data) external payable {
        _burnGas(_gasLimit);
        _recordL1Message(msg.sender, _to, msg.value, _gasLimit, _data);
    }

    receive() external payable {}

    function _recordL1Message(
        address _from,
        address _to,
        uint256 _value,
        uint256 _gasLimit,
        bytes memory _data
    ) internal {
        uint256 nonce = pendingL1Messages.length;
        bytes32 dataHash = keccak256(_data);
        bytes32 messageHash = keccak256(abi.encodePacked(_from, _to, _value, _gasLimit, dataHash, nonce));
        pendingL1Messages.push(messageHash);
        emit L1MessageRecorded(_from, _to, _value, _gasLimit, _data, nonce);
    }

    function _burnGas(uint256 amount) private view {
        uint256 startingGas = gasleft();
        while (startingGas - gasleft() < amount) {}
    }

    // ===== Block Advancement =====

    /// @notice Advance the L2 state by one block.
    /// @param _l1MessagesCount Number of L1 messages consumed in this block.
    /// @param _sszStatelessInput SSZ-encoded StatelessInput (constructed off-chain by the advancer).
    ///        The advancer encodes: NewPayloadRequest (with parent_beacon_block_root = L1 messages
    ///        Merkle root), ExecutionWitness, ChainConfig, and public_keys.
    /// @param _newBlockHash Expected block hash after execution.
    /// @param _newStateRoot Expected state root after execution.
    function advance(
        uint256 _l1MessagesCount,
        bytes calldata _sszStatelessInput,
        bytes32 _newBlockHash,
        bytes32 _newStateRoot
    ) external {
        uint256 startIdx = l1MessageIndex;
        require(startIdx + _l1MessagesCount <= pendingL1Messages.length, "Not enough L1 messages");

        l1MessageIndex = startIdx + _l1MessagesCount;

        // Call EXECUTE precompile with the SSZ-encoded StatelessInput.
        // The precompile validates L2 constraints, charges gas_used,
        // and delegates to verify_stateless_new_payload.
        (bool success, bytes memory result) = EXECUTE_PRECOMPILE.staticcall(_sszStatelessInput);
        require(success, "EXECUTE precompile failed");

        // Decode SSZ StatelessValidationResult.
        // Format: new_payload_request_root (32 bytes) + successful_validation (1 byte) + chain_config (8 bytes)
        require(result.length >= 41, "Invalid result length");

        // successful_validation is at byte 32 (1 byte, SSZ bool)
        require(uint8(result[32]) == 1, "L2 validation failed");

        // chain_id is at bytes 33..41 (SSZ uint64, little-endian).
        uint64 provenChainId = _decodeSszUint64LE(result, 33);
        require(provenChainId == chainId, "chain_id mismatch");

        // Update onchain state
        uint256 newBlockNumber = blockNumber + 1;
        blockHash = _newBlockHash;
        stateRoot = _newStateRoot;
        blockNumber = newBlockNumber;
        stateRootHistory[newBlockNumber] = _newStateRoot;
        stateRootTimestamps[newBlockNumber] = block.timestamp;

        emit StateAdvanced(newBlockNumber, _newStateRoot);
    }

    // ===== L1 Messages Merkle Tree =====

    /// @notice Compute the commutative Merkle root over a range of pending L1 messages.
    function computeMerkleRoot(uint256 startIdx, uint256 count) external view returns (bytes32) {
        return _computeMerkleRoot(startIdx, count);
    }

    function _computeMerkleRoot(uint256 startIdx, uint256 count) internal view returns (bytes32) {
        if (count == 0) return bytes32(0);
        if (count == 1) return pendingL1Messages[startIdx];

        bytes32[] memory layer = new bytes32[](count);
        for (uint256 i = 0; i < count; i++) {
            layer[i] = pendingL1Messages[startIdx + i];
        }

        while (layer.length > 1) {
            uint256 newLen = (layer.length + 1) / 2;
            bytes32[] memory newLayer = new bytes32[](newLen);
            for (uint256 i = 0; i < newLen; i++) {
                if (2 * i + 1 < layer.length) {
                    newLayer[i] = _hashPair(layer[2 * i], layer[2 * i + 1]);
                } else {
                    newLayer[i] = layer[2 * i];
                }
            }
            layer = newLayer;
        }
        return layer[0];
    }

    function _hashPair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        if (uint256(a) < uint256(b)) {
            return keccak256(abi.encodePacked(a, b));
        }
        return keccak256(abi.encodePacked(b, a));
    }

    // ===== Withdrawal Claiming (MPT proof-based) =====

    function claimWithdrawal(
        address _from,
        address payable _receiver,
        uint256 _amount,
        uint256 _messageId,
        uint256 _atBlockNumber,
        bytes[] calldata _accountProof,
        bytes[] calldata _storageProof
    ) external nonReentrant {
        bytes32 historicStateRoot = stateRootHistory[_atBlockNumber];
        require(historicStateRoot != bytes32(0), "State root not found");

        uint256 stateRootTime = stateRootTimestamps[_atBlockNumber];
        require(block.timestamp >= stateRootTime + FINALITY_DELAY, "Not yet final");

        bytes32 withdrawalHash = keccak256(abi.encodePacked(_from, _receiver, _amount, _messageId));
        require(!claimedWithdrawals[withdrawalHash], "Already claimed");

        _verifyWithdrawalProof(historicStateRoot, withdrawalHash, _accountProof, _storageProof);

        claimedWithdrawals[withdrawalHash] = true;

        (bool sent, ) = _receiver.call{value: _amount}("");
        require(sent, "ETH transfer failed");

        emit WithdrawalClaimed(_receiver, _amount, _atBlockNumber, _messageId);
    }

    function _verifyWithdrawalProof(
        bytes32 _stateRoot,
        bytes32 _withdrawalHash,
        bytes[] calldata _accountProof,
        bytes[] calldata _storageProof
    ) internal pure {
        bytes32 l2BridgeAddressHash = keccak256(abi.encodePacked(L2_BRIDGE_ADDRESS));
        bytes memory accountRLP = MPTProof.verifyMptProof(
            _stateRoot,
            MPTProof.toNibbles(abi.encodePacked(l2BridgeAddressHash)),
            _accountProof
        );
        bytes32 storageRoot = MPTProof.decodeAccountStorageRoot(accountRLP);

        bytes32 slot = keccak256(abi.encode(_withdrawalHash, SENT_MESSAGES_SLOT));
        bytes32 slotHash = keccak256(abi.encodePacked(slot));
        bytes memory valueRLP = MPTProof.verifyMptProof(
            storageRoot,
            MPTProof.toNibbles(abi.encodePacked(slotHash)),
            _storageProof
        );
        uint256 value = MPTProof.decodeRlpUint(valueRLP);
        require(value == 1, "Withdrawal not found in L2Bridge storage");
    }

    /// @dev Decode an SSZ `uint64` (8 little-endian bytes) at `offset` into `data`.
    function _decodeSszUint64LE(bytes memory data, uint256 offset) internal pure returns (uint64) {
        require(data.length >= offset + 8, "SSZ: out of bounds");
        uint64 value = 0;
        for (uint256 i = 0; i < 8; i++) {
            value |= uint64(uint8(data[offset + i])) << uint64(8 * i);
        }
        return value;
    }
}
