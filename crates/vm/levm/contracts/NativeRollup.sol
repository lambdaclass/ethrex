// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import "./MPTProof.sol";

/// @title NativeRollup — PoC L2 state manager using the EXECUTE precompile (EIP-8079).
///
/// Manages the L2 state on L1: state root, block number, gas parameters,
/// pending L1 messages, and state root history. The `advance` method builds
/// ABI-encoded calldata for the EXECUTE precompile at 0x0101, which
/// re-executes the L2 block and verifies the state transition. On success,
/// the precompile returns the new state root, block number, gas used,
/// burned fees, and base fee — and the contract updates its state accordingly.
///
/// Withdrawals are initiated on L2 via the L2Bridge contract, which writes
/// withdrawal hashes to its `sentMessages` mapping. Users claim on L1 via
/// `claimWithdrawal()` with MPT account + storage proofs against the L2 state
/// root — the state root is the single source of truth for all L2 state.
///
/// Storage layout:
///   Slot 0: stateRoot (bytes32)
///   Slot 1: blockNumber (uint256)
///   Slot 2: blockGasLimit (uint256)
///   Slot 3: lastBaseFeePerGas (uint256)
///   Slot 4: lastGasUsed (uint256)
///   Slot 5: pendingL1Messages (bytes32[])
///   Slot 6: l1MessageIndex (uint256)
///   Slot 7: stateRootHistory (mapping(uint256 => bytes32))
///   Slot 8: claimedWithdrawals (mapping(bytes32 => bool))
///   Slot 9: _locked (bool)

struct BlockParams {
    bytes32 postStateRoot;
    bytes32 postReceiptsRoot;
    address coinbase;
    bytes32 prevRandao;
    uint256 timestamp;
}

contract NativeRollup {
    bytes32 public stateRoot;
    uint256 public blockNumber;
    uint256 public blockGasLimit;
    uint256 public lastBaseFeePerGas;
    uint256 public lastGasUsed;

    address constant EXECUTE_PRECOMPILE = address(0x0101);

    /// @notice L2Bridge predeploy address on L2 (for storage proof verification).
    address constant L2_BRIDGE_ADDRESS = address(0x000000000000000000000000000000000000FfFD);

    /// @notice Storage slot of sentMessages mapping in L2Bridge (slot 3).
    uint256 constant L2_BRIDGE_SENT_MESSAGES_SLOT = 3;

    /// @notice Default gas limit for L1 messages sent via receive().
    /// Matches CommonBridge's deposit gas limit (21000 * 5).
    uint256 constant DEFAULT_GAS_LIMIT = 21_000 * 5;

    bytes32[] public pendingL1Messages;
    uint256 public l1MessageIndex;

    // State root history for withdrawal proving
    mapping(uint256 => bytes32) public stateRootHistory;
    mapping(bytes32 => bool) public claimedWithdrawals;

    // Reentrancy guard
    bool private _locked;

    event StateAdvanced(uint256 indexed newBlockNumber, bytes32 newStateRoot, uint256 burnedFees);
    event L1MessageRecorded(address indexed sender, address indexed to, uint256 value, uint256 gasLimit, bytes32 dataHash, uint256 indexed nonce);
    event WithdrawalClaimed(address indexed receiver, uint256 amount, uint256 indexed blockNumber, uint256 indexed messageId);

    modifier nonReentrant() {
        require(!_locked, "ReentrancyGuard: reentrant call");
        _locked = true;
        _;
        _locked = false;
    }

    constructor(bytes32 _initialStateRoot, uint256 _blockGasLimit, uint256 _initialBaseFee) {
        stateRoot = _initialStateRoot;
        blockGasLimit = _blockGasLimit;
        lastBaseFeePerGas = _initialBaseFee;
        lastGasUsed = _blockGasLimit / 2;
    }

    // ===== L1 Messaging =====

    function sendL1Message(address _to, uint256 _gasLimit, bytes calldata _data) external payable {
        _recordL1Message(msg.sender, _to, msg.value, _gasLimit, _data);
    }

    receive() external payable {
        require(msg.value > 0, "Must send ETH");
        _burnGas(DEFAULT_GAS_LIMIT);
        _recordL1Message(msg.sender, msg.sender, msg.value, DEFAULT_GAS_LIMIT, "");
    }

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
        emit L1MessageRecorded(_from, _to, _value, _gasLimit, dataHash, nonce);
    }

    /// @dev Consume gas in a tight loop so the L1 caller pays for the gas that
    ///      the relayer will spend on L2 when executing the corresponding message.
    function _burnGas(uint256 amount) private view {
        uint256 startingGas = gasleft();
        while (startingGas - gasleft() < amount) {}
    }

    // ===== Block Advancement =====

    function advance(
        uint256 _l1MessagesCount,
        BlockParams calldata _blockParams,
        bytes calldata _transactions,
        bytes calldata _witness
    ) external {
        uint256 startIdx = l1MessageIndex;
        require(startIdx + _l1MessagesCount <= pendingL1Messages.length, "Not enough L1 messages");

        bytes32 l1Anchor = _computeMerkleRoot(startIdx, _l1MessagesCount);

        l1MessageIndex = startIdx + _l1MessagesCount;

        uint256 nextBlockNumber = blockNumber + 1;
        uint256 _blockGasLimit = blockGasLimit;
        uint256 _lastBaseFeePerGas = lastBaseFeePerGas;
        uint256 _lastGasUsed = lastGasUsed;

        bytes memory input = abi.encode(
            stateRoot,
            _blockParams.postStateRoot,
            _blockParams.postReceiptsRoot,
            nextBlockNumber,
            _blockGasLimit,
            _blockParams.coinbase,
            _blockParams.prevRandao,
            _blockParams.timestamp,
            _lastBaseFeePerGas,
            _blockGasLimit,
            _lastGasUsed,
            l1Anchor,
            _transactions,
            _witness
        );

        (bool success, bytes memory result) = EXECUTE_PRECOMPILE.call(input);
        require(success && result.length == 160, "EXECUTE precompile verification failed");

        // Decode: postStateRoot, blockNumber, gasUsed, burnedFees, baseFeePerGas (5 fields, 160 bytes)
        (bytes32 newStateRoot, uint256 newBlockNumber, uint256 gasUsed, uint256 burnedFees, uint256 baseFeePerGas) = abi.decode(result, (bytes32, uint256, uint256, uint256, uint256));

        stateRoot = newStateRoot;
        blockNumber = newBlockNumber;
        lastGasUsed = gasUsed;
        lastBaseFeePerGas = baseFeePerGas;
        stateRootHistory[newBlockNumber] = newStateRoot;

        if (burnedFees > 0) {
            (bool sent, ) = msg.sender.call{value: burnedFees}("");
            require(sent, "Burned fees transfer failed");
        }

        emit StateAdvanced(newBlockNumber, newStateRoot, burnedFees);
    }

    // ===== Withdrawal Claiming (MPT proof-based) =====

    function claimWithdrawal(
        address _from,
        address _receiver,
        uint256 _amount,
        uint256 _messageId,
        uint256 _atBlockNumber,
        bytes[] calldata _accountProof,
        bytes[] calldata _storageProof
    ) external nonReentrant {
        bytes32 root = stateRootHistory[_atBlockNumber];
        require(root != bytes32(0), "Unknown block");

        bytes32 withdrawalHash = keccak256(abi.encodePacked(_from, _receiver, _amount, _messageId));
        require(!claimedWithdrawals[withdrawalHash], "Already claimed");

        _verifyWithdrawalProof(root, withdrawalHash, _accountProof, _storageProof);

        claimedWithdrawals[withdrawalHash] = true;
        (bool success, ) = _receiver.call{value: _amount}("");
        require(success, "ETH transfer failed");
        emit WithdrawalClaimed(_receiver, _amount, _atBlockNumber, _messageId);
    }

    /// @dev Verify withdrawal inclusion: account proof -> storageRoot, storage proof -> sentMessages[hash] == 1.
    function _verifyWithdrawalProof(
        bytes32 root,
        bytes32 withdrawalHash,
        bytes[] calldata _accountProof,
        bytes[] calldata _storageProof
    ) internal pure {
        // 1. Account proof -> extract L2Bridge storageRoot
        bytes memory accountPath = MPTProof.toNibbles(abi.encodePacked(keccak256(abi.encodePacked(L2_BRIDGE_ADDRESS))));
        bytes memory accountRlp = MPTProof.verifyMptProof(root, accountPath, _accountProof);
        bytes32 storageRoot = MPTProof.decodeAccountStorageRoot(accountRlp);

        // 2. Storage proof -> sentMessages[withdrawalHash] == true
        bytes32 slot = keccak256(abi.encode(withdrawalHash, uint256(L2_BRIDGE_SENT_MESSAGES_SLOT)));
        bytes memory storagePath = MPTProof.toNibbles(abi.encodePacked(keccak256(abi.encodePacked(slot))));
        bytes memory storageRlp = MPTProof.verifyMptProof(storageRoot, storagePath, _storageProof);
        uint256 value = MPTProof.decodeRlpUint(storageRlp);
        require(value == 1, "Withdrawal not in L2 state");
    }

    // ===== Commutative Merkle Tree (for L1→L2 messaging) =====

    function _verifyMerkleProof(
        bytes32[] calldata proof,
        bytes32 root,
        bytes32 leaf
    ) internal pure returns (bool) {
        bytes32 computedHash = leaf;
        for (uint256 i = 0; i < proof.length; i++) {
            computedHash = _hashPair(computedHash, proof[i]);
        }
        return computedHash == root;
    }

    function _hashPair(bytes32 a, bytes32 b) private pure returns (bytes32) {
        if (a < b) {
            return keccak256(abi.encodePacked(a, b));
        } else {
            return keccak256(abi.encodePacked(b, a));
        }
    }

    function _computeMerkleRoot(uint256 startIdx, uint256 count) internal view returns (bytes32) {
        if (count == 0) return bytes32(0);
        if (count == 1) return pendingL1Messages[startIdx];

        bytes32[] memory level = new bytes32[](count);
        for (uint256 i = 0; i < count; i++) {
            level[i] = pendingL1Messages[startIdx + i];
        }

        while (level.length > 1) {
            uint256 nextLen = (level.length + 1) / 2;
            bytes32[] memory next = new bytes32[](nextLen);
            for (uint256 i = 0; i < level.length / 2; i++) {
                next[i] = _hashPair(level[2 * i], level[2 * i + 1]);
            }
            if (level.length % 2 == 1) {
                next[nextLen - 1] = level[level.length - 1];
            }
            level = next;
        }
        return level[0];
    }
}
