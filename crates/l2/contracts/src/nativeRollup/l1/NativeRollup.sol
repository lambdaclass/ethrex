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
///   Slot 0:  blockHash (bytes32)
///   Slot 1:  stateRoot (bytes32)
///   Slot 2:  blockNumber (uint256)
///   Slot 3:  l2GasLimit (uint256)
///   Slot 4:  chainId (uint256)
///   Slot 5:  pendingL1Messages (bytes32[])
///   Slot 6:  l1MessageIndex (uint256)
///   Slot 7:  stateRootHistory (mapping(uint256 => bytes32))
///   Slot 8:  claimedWithdrawals (mapping(bytes32 => bool))
///   Slot 9:  stateRootTimestamps (mapping(uint256 => uint256))
///   Slot 10: _locked (bool)
///   Slot 11: lastFetchedL1Block (uint256)
///   Slot 12: advancer (address)

contract NativeRollup {
    // ===== L2 chain state (spec-aligned) =====
    bytes32 public blockHash;
    bytes32 public stateRoot;
    uint256 public blockNumber;
    uint256 public l2GasLimit;
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

    // ===== L1 watcher cursor =====
    /// Deploy block. Watcher seeds its cursor from this on first poll.
    uint256 public lastFetchedL1Block;

    // ===== Access control =====
    address public advancer;

    // ===== Immutables =====
    uint256 public immutable CHAIN_ID;
    uint256 public immutable FINALITY_DELAY;

    address constant EXECUTE_PRECOMPILE = address(0x0101);
    address constant L2_BRIDGE_ADDRESS = address(0x000000000000000000000000000000000000FfFD);
    // Storage slot of `sentMessages` in L2Bridge.sol. Must match that layout —
    // reordering L2Bridge storage silently breaks withdrawal proofs here.
    uint256 constant SENT_MESSAGES_SLOT = 3;

    // SSZ `StatelessValidationResult` layout: 32B root + 1B bool + 8B chain_id LE.
    uint256 constant RESULT_LEN = 41;
    uint256 constant RESULT_SUCCESS_OFFSET = 32;
    uint256 constant RESULT_CHAIN_ID_OFFSET = 33;

    // Byte offsets inside the SSZ `ExecutionPayload` fixed prefix.
    uint256 constant EP_PARENT_HASH_OFFSET = 0;
    uint256 constant EP_STATE_ROOT_OFFSET = 52;
    uint256 constant EP_BLOCK_NUMBER_OFFSET = 404;
    uint256 constant EP_GAS_LIMIT_OFFSET = 412;
    uint256 constant EP_BLOCK_HASH_OFFSET = 472;
    uint256 constant EP_FIXED_PREFIX_LEN = 528;

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

    modifier onlyAdvancer() {
        require(msg.sender == advancer, "NativeRollup: not advancer");
        _;
    }

    constructor(
        bytes32 _initialStateRoot,
        bytes32 _initialBlockHash,
        uint256 _blockGasLimit,
        uint256 _chainId,
        address _advancer
    ) {
        require(_advancer != address(0), "NativeRollup: advancer is zero");
        stateRoot = _initialStateRoot;
        blockHash = _initialBlockHash;
        blockNumber = 0;
        l2GasLimit = _blockGasLimit;
        chainId = _chainId;
        CHAIN_ID = _chainId;
        FINALITY_DELAY = 0;
        advancer = _advancer;
        lastFetchedL1Block = block.number;
    }

    // ===== L1 Messaging =====

    function sendL1Message(address _to, uint256 _gasLimit, bytes calldata _data) external payable {
        // Reject messages that can never fit in a single L2 block.
        require(_gasLimit > 0, "gasLimit must be > 0");
        require(_gasLimit <= l2GasLimit, "gasLimit exceeds L2 block gas limit");
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
    /// @param _sszStatelessInput SSZ-encoded `StatelessInput` for the block.
    /// @dev    Block fields are read from `_sszStatelessInput`; once the
    ///         precompile reports `successful_validation == true` those bytes
    ///         describe a correctly executed block, so the caller cannot
    ///         substitute the new block_hash, state_root, etc.
    function advance(
        uint256 _l1MessagesCount,
        bytes calldata _sszStatelessInput
    ) external onlyAdvancer {
        uint256 startIdx = l1MessageIndex;
        require(startIdx + _l1MessagesCount <= pendingL1Messages.length, "Not enough L1 messages");

        l1MessageIndex = startIdx + _l1MessagesCount;

        (bool success, bytes memory result) = EXECUTE_PRECOMPILE.staticcall(_sszStatelessInput);
        require(success, "EXECUTE precompile failed");

        require(result.length >= RESULT_LEN, "Invalid result length");
        require(uint8(result[RESULT_SUCCESS_OFFSET]) == 1, "L2 validation failed");

        uint64 provenChainId = _decodeSszUint64LE(result, RESULT_CHAIN_ID_OFFSET);
        require(provenChainId == chainId, "chain_id mismatch");

        (
            uint64 provenBlockNumber,
            bytes32 provenParentHash,
            bytes32 provenBlockHash,
            bytes32 provenStateRoot,
            uint64 provenGasLimit
        ) = _decodeProvenPayloadFields(_sszStatelessInput);

        // Bind the proof to *this* L2 chain.
        uint256 expectedBlockNumber = blockNumber + 1;
        require(uint256(provenBlockNumber) == expectedBlockNumber, "block_number mismatch");
        require(provenParentHash == blockHash, "parent_hash mismatch");
        require(uint256(provenGasLimit) == l2GasLimit, "gas_limit mismatch");

        blockHash = provenBlockHash;
        stateRoot = provenStateRoot;
        blockNumber = expectedBlockNumber;
        stateRootHistory[expectedBlockNumber] = provenStateRoot;
        stateRootTimestamps[expectedBlockNumber] = block.timestamp;

        emit StateAdvanced(expectedBlockNumber, provenStateRoot);
    }

    /// @dev Walk SSZ offsets on a `StatelessInput` buffer:
    ///        StatelessInput[0..4]      -> NewPayloadRequest start
    ///        NewPayloadRequest[0..4]   -> ExecutionPayload start
    ///      Then read the fixed-position fields off ExecutionPayload.
    function _decodeProvenPayloadFields(bytes calldata sszInput)
        internal
        pure
        returns (
            uint64 blockNumber,
            bytes32 parentHash,
            bytes32 blockHash_,
            bytes32 stateRoot_,
            uint64 gasLimit
        )
    {
        require(sszInput.length >= 20, "SSZ: input too short");
        uint256 nprAbs = _readU32LECalldata(sszInput, 0);
        // 44 = NPR fixed prefix: 3 var-field offsets (12) + parent_beacon_block_root (32).
        require(sszInput.length >= nprAbs + 44, "SSZ: NPR offset out of range");
        uint256 epAbs = nprAbs + _readU32LECalldata(sszInput, nprAbs);
        require(
            sszInput.length >= epAbs + EP_FIXED_PREFIX_LEN,
            "SSZ: EP offset out of range"
        );

        parentHash = _readBytes32Calldata(sszInput, epAbs + EP_PARENT_HASH_OFFSET);
        stateRoot_ = _readBytes32Calldata(sszInput, epAbs + EP_STATE_ROOT_OFFSET);
        blockNumber = _readU64LECalldata(sszInput, epAbs + EP_BLOCK_NUMBER_OFFSET);
        gasLimit = _readU64LECalldata(sszInput, epAbs + EP_GAS_LIMIT_OFFSET);
        blockHash_ = _readBytes32Calldata(sszInput, epAbs + EP_BLOCK_HASH_OFFSET);
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

    /// @dev Decode an SSZ `uint64` (8 little-endian bytes) at `offset` into `data` (memory).
    function _decodeSszUint64LE(bytes memory data, uint256 offset) internal pure returns (uint64) {
        require(data.length >= offset + 8, "SSZ: out of bounds");
        uint64 value = 0;
        for (uint256 i = 0; i < 8; i++) {
            value |= uint64(uint8(data[offset + i])) << uint64(8 * i);
        }
        return value;
    }

    /// @dev Decode an SSZ `uint32` (4 little-endian bytes) at `offset` into `data` (calldata).
    function _readU32LECalldata(bytes calldata data, uint256 offset) internal pure returns (uint256) {
        require(data.length >= offset + 4, "SSZ: u32 out of bounds");
        return uint256(uint8(data[offset]))
            | (uint256(uint8(data[offset + 1])) << 8)
            | (uint256(uint8(data[offset + 2])) << 16)
            | (uint256(uint8(data[offset + 3])) << 24);
    }

    /// @dev Decode an SSZ `uint64` (8 little-endian bytes) at `offset` into `data` (calldata).
    function _readU64LECalldata(bytes calldata data, uint256 offset) internal pure returns (uint64 v) {
        require(data.length >= offset + 8, "SSZ: u64 out of bounds");
        for (uint256 i = 0; i < 8; i++) {
            v |= uint64(uint8(data[offset + i])) << uint64(8 * i);
        }
    }

    /// @dev Read 32 contiguous bytes at `offset` from calldata as a `bytes32`.
    function _readBytes32Calldata(bytes calldata data, uint256 offset) internal pure returns (bytes32 out) {
        require(data.length >= offset + 32, "SSZ: bytes32 out of bounds");
        // solhint-disable-next-line no-inline-assembly
        assembly {
            out := calldataload(add(data.offset, offset))
        }
    }
}
