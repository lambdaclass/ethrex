// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

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
    uint256 constant DEFAULT_GAS_LIMIT = 100_000;

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

    /// @dev Verify withdrawal inclusion: account proof → storageRoot, storage proof → sentMessages[hash] == 1.
    function _verifyWithdrawalProof(
        bytes32 root,
        bytes32 withdrawalHash,
        bytes[] calldata _accountProof,
        bytes[] calldata _storageProof
    ) internal pure {
        // 1. Account proof → extract L2Bridge storageRoot
        bytes memory accountPath = _toNibbles(abi.encodePacked(keccak256(abi.encodePacked(L2_BRIDGE_ADDRESS))));
        bytes memory accountRlp = _verifyMptProof(root, accountPath, _accountProof);
        bytes32 storageRoot = _decodeAccountStorageRoot(accountRlp);

        // 2. Storage proof → sentMessages[withdrawalHash] == true
        bytes32 slot = keccak256(abi.encode(withdrawalHash, uint256(L2_BRIDGE_SENT_MESSAGES_SLOT)));
        bytes memory storagePath = _toNibbles(abi.encodePacked(keccak256(abi.encodePacked(slot))));
        bytes memory storageRlp = _verifyMptProof(storageRoot, storagePath, _storageProof);
        uint256 value = _decodeRlpUint(storageRlp);
        require(value == 1, "Withdrawal not in L2 state");
    }

    // ===== MPT Proof Verification (inlined) =====

    /// @dev Core MPT proof verification. Walks the trie from root to leaf.
    function _verifyMptProof(
        bytes32 root,
        bytes memory path,
        bytes[] calldata proof
    ) internal pure returns (bytes memory) {
        bytes32 expectedHash = root;
        uint256 pathOffset = 0;

        for (uint256 i = 0; i < proof.length; i++) {
            bytes calldata node = proof[i];
            require(keccak256(node) == expectedHash, "MPT: invalid node hash");

            (uint256 listLen, uint256 listOffset) = _rlpListHeader(node);
            uint256 listEnd = listOffset + listLen;
            uint256 itemCount = _rlpListItemCount(node, listOffset, listEnd);

            if (itemCount == 17) {
                // Branch node
                if (pathOffset == path.length) {
                    (bytes memory val, ) = _rlpListItem(node, listOffset, listEnd, 16);
                    return val;
                }
                uint8 nibble = uint8(path[pathOffset]);
                pathOffset++;
                (bytes memory child, ) = _rlpListItem(node, listOffset, listEnd, nibble);
                require(child.length == 32, "MPT: branch child not hash");
                expectedHash = bytes32(child);
            } else if (itemCount == 2) {
                // Extension or leaf node
                (bytes memory encodedPath, ) = _rlpListItem(node, listOffset, listEnd, 0);
                uint256 prefix = uint8(encodedPath[0]) >> 4;
                bool isLeaf = (prefix == 2 || prefix == 3);
                bool isOdd = (prefix == 1 || prefix == 3);
                uint256 nibbleStart = isOdd ? 1 : 2;
                uint256 nibbleCount = encodedPath.length * 2 - nibbleStart;

                for (uint256 j = 0; j < nibbleCount; j++) {
                    require(pathOffset < path.length, "MPT: path too short");
                    uint256 byteIdx = (nibbleStart + j) / 2;
                    uint8 expected;
                    if ((nibbleStart + j) % 2 == 0) {
                        expected = uint8(encodedPath[byteIdx]) >> 4;
                    } else {
                        expected = uint8(encodedPath[byteIdx]) & 0x0f;
                    }
                    require(uint8(path[pathOffset]) == expected, "MPT: path mismatch");
                    pathOffset++;
                }

                (bytes memory next, ) = _rlpListItem(node, listOffset, listEnd, 1);
                if (isLeaf) {
                    require(pathOffset == path.length, "MPT: leaf path incomplete");
                    return next;
                }
                require(next.length == 32, "MPT: ext next not hash");
                expectedHash = bytes32(next);
            } else {
                revert("MPT: invalid node");
            }
        }
        revert("MPT: proof incomplete");
    }

    function _toNibbles(bytes memory data) internal pure returns (bytes memory nibbles) {
        nibbles = new bytes(data.length * 2);
        for (uint256 i = 0; i < data.length; i++) {
            nibbles[i * 2] = bytes1(uint8(data[i]) >> 4);
            nibbles[i * 2 + 1] = bytes1(uint8(data[i]) & 0x0f);
        }
    }

    function _rlpListHeader(bytes calldata data) internal pure returns (uint256 length, uint256 offset) {
        uint8 p = uint8(data[0]);
        if (p >= 0xc0 && p <= 0xf7) {
            return (p - 0xc0, 1);
        }
        uint256 lenBytes = p - 0xf7;
        length = 0;
        for (uint256 i = 0; i < lenBytes; i++) {
            length = (length << 8) | uint8(data[1 + i]);
        }
        offset = 1 + lenBytes;
    }

    function _rlpListItemCount(bytes calldata data, uint256 start, uint256 end) internal pure returns (uint256 count) {
        uint256 pos = start;
        while (pos < end) {
            (, uint256 total) = _rlpItemLen(data, pos);
            pos += total;
            count++;
        }
    }

    function _rlpListItem(bytes calldata data, uint256 start, uint256 end, uint256 idx) internal pure returns (bytes memory item, uint256 itemStart) {
        uint256 pos = start;
        uint256 count = 0;
        while (pos < end) {
            (uint256 cOff, uint256 total) = _rlpItemLen(data, pos);
            if (count == idx) {
                uint256 cLen = total - (cOff - pos);
                item = data[cOff : cOff + cLen];
                return (item, pos);
            }
            pos += total;
            count++;
        }
        return (new bytes(0), end);
    }

    function _rlpItemLen(bytes calldata data, uint256 pos) internal pure returns (uint256 contentOffset, uint256 totalLength) {
        uint8 p = uint8(data[pos]);
        if (p < 0x80) {
            return (pos, 1);
        } else if (p <= 0xb7) {
            return (pos + 1, 1 + (p - 0x80));
        } else if (p <= 0xbf) {
            uint256 lenBytes = p - 0xb7;
            uint256 len = 0;
            for (uint256 i = 0; i < lenBytes; i++) {
                len = (len << 8) | uint8(data[pos + 1 + i]);
            }
            return (pos + 1 + lenBytes, 1 + lenBytes + len);
        } else if (p <= 0xf7) {
            return (pos + 1, 1 + (p - 0xc0));
        } else {
            uint256 lenBytes = p - 0xf7;
            uint256 len = 0;
            for (uint256 i = 0; i < lenBytes; i++) {
                len = (len << 8) | uint8(data[pos + 1 + i]);
            }
            return (pos + 1 + lenBytes, 1 + lenBytes + len);
        }
    }

    /// @dev Decode storageRoot (3rd field) from RLP-encoded account [nonce, balance, storageRoot, codeHash].
    function _decodeAccountStorageRoot(bytes memory account) internal pure returns (bytes32 storageRoot) {
        uint256 pos = 0;
        // Skip list header
        uint8 p = uint8(account[pos]);
        if (p >= 0xf8) { pos += 1 + (uint256(p) - 0xf7); }
        else if (p >= 0xc0) { pos += 1; }
        else { revert("MPT: account not list"); }

        // Skip nonce (item 0)
        pos = _skipRlpItem(account, pos);
        // Skip balance (item 1)
        pos = _skipRlpItem(account, pos);
        // Read storageRoot (item 2) — must be 32 bytes
        (uint256 cStart, uint256 cLen) = _decodeRlpItemMem(account, pos);
        require(cLen == 32, "MPT: storageRoot not 32 bytes");
        assembly ("memory-safe") { storageRoot := mload(add(add(account, 32), cStart)) }
    }

    function _skipRlpItem(bytes memory data, uint256 pos) internal pure returns (uint256) {
        uint8 p = uint8(data[pos]);
        if (p < 0x80) return pos + 1;
        if (p <= 0xb7) return pos + 1 + (uint256(p) - 0x80);
        if (p <= 0xbf) {
            uint256 lb = uint256(p) - 0xb7;
            uint256 l = 0;
            for (uint256 i = 0; i < lb; i++) l = (l << 8) | uint8(data[pos+1+i]);
            return pos + 1 + lb + l;
        }
        if (p <= 0xf7) return pos + 1 + (uint256(p) - 0xc0);
        uint256 lb2 = uint256(p) - 0xf7;
        uint256 l2 = 0;
        for (uint256 i = 0; i < lb2; i++) l2 = (l2 << 8) | uint8(data[pos+1+i]);
        return pos + 1 + lb2 + l2;
    }

    function _decodeRlpItemMem(bytes memory data, uint256 pos) internal pure returns (uint256 cStart, uint256 cLen) {
        uint8 p = uint8(data[pos]);
        if (p < 0x80) return (pos, 1);
        if (p <= 0xb7) return (pos + 1, uint256(p) - 0x80);
        uint256 lb = uint256(p) - 0xb7;
        cLen = 0;
        for (uint256 i = 0; i < lb; i++) cLen = (cLen << 8) | uint8(data[pos+1+i]);
        cStart = pos + 1 + lb;
    }

    function _decodeRlpUint(bytes memory data) internal pure returns (uint256 value) {
        for (uint256 i = 0; i < data.length; i++) {
            value = (value << 8) | uint8(data[i]);
        }
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
