// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title NativeRollup — PoC L2 state manager using the EXECUTE precompile (EIP-8079).
///
/// Manages the L2 state on L1: state root, block number, gas parameters,
/// pending L1 messages, and withdrawal roots. The `advance` method builds
/// ABI-encoded calldata for the EXECUTE precompile at 0x0101, which
/// re-executes the L2 block and verifies the state transition. On success,
/// the precompile returns the new state root, block number, withdrawal
/// Merkle root, gas used, burned fees, and base fee — and the contract
/// updates its state accordingly.
///
/// The EXECUTE precompile uses the `apply_body` variant: individual block fields
/// are ABI-encoded as 14 slots (12 static + 2 dynamic byte arrays for the
/// RLP-encoded transaction list and JSON execution witness).
///
/// Parent gas parameters (parentBaseFee, parentGasUsed) and blockGasLimit are
/// tracked on-chain from previous executions instead of being provided by the
/// relayer. The contract stores `blockGasLimit` (constant), `lastBaseFeePerGas`,
/// and `lastGasUsed`, and feeds them to the EXECUTE precompile automatically.
///
/// L1 messages are recorded via `sendL1Message(to, gasLimit, data)` or by
/// sending ETH directly to the contract. Each message is hashed as
/// keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))
/// and stored in the `pendingL1Messages` array. When `advance()` is called, it
/// computes a Merkle root over the consumed message hashes and passes it as
/// l1Anchor to the EXECUTE precompile, which writes it to the L1Anchor predeploy
/// on L2 before executing transactions. L2 contracts verify individual messages
/// via Merkle proofs against the anchored root.
///
/// Withdrawals are initiated on L2 via the L2Bridge contract. The EXECUTE
/// precompile extracts withdrawal events and computes a Merkle root. Users can
/// claim withdrawals on L1 via `claimWithdrawal()` with a Merkle proof.
///
/// Storage layout:
///   Slot 0: stateRoot (bytes32)
///   Slot 1: blockNumber (uint256)
///   Slot 2: blockGasLimit (uint256)
///   Slot 3: lastBaseFeePerGas (uint256)
///   Slot 4: lastGasUsed (uint256)
///   Slot 5: pendingL1Messages (bytes32[])
///   Slot 6: l1MessageIndex (uint256)
///   Slot 7: withdrawalRoots (mapping)
///   Slot 8: claimedWithdrawals (mapping)
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

    /// @notice Default gas limit for L1 messages sent via receive() (similar to CommonBridge's 21000 * 5).
    uint256 constant DEFAULT_GAS_LIMIT = 100_000;

    bytes32[] public pendingL1Messages;
    uint256 public l1MessageIndex;

    // Withdrawal state
    mapping(uint256 => bytes32) public withdrawalRoots;
    mapping(bytes32 => bool) public claimedWithdrawals;

    // Reentrancy guard
    bool private _locked;

    event StateAdvanced(uint256 indexed newBlockNumber, bytes32 newStateRoot, bytes32 withdrawalRoot, uint256 burnedFees);
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

    /// @notice Send an L1 message to be included in a future L2 block.
    /// @param _to The target address on L2 to call.
    /// @param _gasLimit Maximum gas for the L2 subcall.
    /// @param _data Calldata to execute on L2 (can be empty for simple ETH transfers).
    function sendL1Message(address _to, uint256 _gasLimit, bytes calldata _data) external payable {
        _recordL1Message(msg.sender, _to, msg.value, _gasLimit, _data);
    }

    /// @notice Receive ETH and record an L1 message for msg.sender.
    receive() external payable {
        require(msg.value > 0, "Must send ETH");
        _recordL1Message(msg.sender, msg.sender, msg.value, DEFAULT_GAS_LIMIT, "");
    }

    /// @notice Internal helper to record an L1 message hash.
    /// @dev Hash: keccak256(abi.encodePacked(from[20], to[20], value[32], gasLimit[32], keccak256(data)[32], nonce[32]))
    ///      = 168 bytes preimage. Uses keccak256(data) instead of raw data to keep hash fixed-size.
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

    /// @notice Advance the L2 by one block.
    /// @dev The Merkle root over consumed L1 messages is computed here BEFORE the
    ///      block is executed by the EXECUTE precompile. This means the block builder
    ///      MUST include the corresponding processL1Message() transactions in the L2
    ///      block — if they are missing or incorrect, the state root will not match
    ///      because the anchored Merkle root won't correspond to the actual messages
    ///      processed. This effectively enforces L1 message inclusion at the protocol
    ///      level via the state root check.
    /// @param _l1MessagesCount Number of pending L1 messages to consume from the queue.
    /// @param _blockParams Block parameters struct (postStateRoot, postReceiptsRoot, coinbase, prevRandao, timestamp).
    /// @param _transactions RLP-encoded transaction list.
    /// @param _witness JSON-serialized ExecutionWitness.
    function advance(
        uint256 _l1MessagesCount,
        BlockParams calldata _blockParams,
        bytes calldata _transactions,
        bytes calldata _witness
    ) external {
        uint256 startIdx = l1MessageIndex;
        require(startIdx + _l1MessagesCount <= pendingL1Messages.length, "Not enough L1 messages");

        // Compute Merkle root over the consumed L1 message batch.
        // Each pendingL1Messages[i] is keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce)).
        // The Merkle root uses commutative hashing (same as withdrawal proofs).
        bytes32 l1Anchor = _computeMerkleRoot(startIdx, _l1MessagesCount);

        l1MessageIndex = startIdx + _l1MessagesCount;

        // Read parent gas parameters from storage
        uint256 nextBlockNumber = blockNumber + 1;
        uint256 _blockGasLimit = blockGasLimit;
        uint256 _lastBaseFeePerGas = lastBaseFeePerGas;
        uint256 _lastGasUsed = lastGasUsed;

        // Build EXECUTE precompile calldata with 14 ABI slots:
        //   slots 0-11: static fields
        //   slots 12-13: dynamic offset pointers (transactions, witness)
        bytes memory input = abi.encode(
            stateRoot,                        // preStateRoot from contract storage
            _blockParams.postStateRoot,
            _blockParams.postReceiptsRoot,
            nextBlockNumber,                  // blockNumber from storage + 1
            _blockGasLimit,                   // blockGasLimit from storage
            _blockParams.coinbase,
            _blockParams.prevRandao,
            _blockParams.timestamp,
            _lastBaseFeePerGas,               // parentBaseFee from storage
            _blockGasLimit,                   // parentGasLimit = blockGasLimit (constant)
            _lastGasUsed,                     // parentGasUsed from storage
            l1Anchor,
            _transactions,
            _witness
        );

        (bool success, bytes memory result) = EXECUTE_PRECOMPILE.call(input);
        require(success && result.length == 192, "EXECUTE precompile verification failed");

        // Decode new state root, block number, withdrawal root, gas used, burned fees, and base fee from precompile return
        (bytes32 newStateRoot, uint256 newBlockNumber, bytes32 withdrawalRoot, uint256 gasUsed, uint256 burnedFees, uint256 baseFeePerGas) = abi.decode(result, (bytes32, uint256, bytes32, uint256, uint256, uint256));

        stateRoot = newStateRoot;
        blockNumber = newBlockNumber;
        lastGasUsed = gasUsed;
        lastBaseFeePerGas = baseFeePerGas;
        withdrawalRoots[newBlockNumber] = withdrawalRoot;

        // Credit burned fees to the relayer (msg.sender) so they can be reimbursed on L1.
        // A separate L2 process (out of scope for this PoC) will credit the relayer on L2.
        if (burnedFees > 0) {
            (bool sent, ) = msg.sender.call{value: burnedFees}("");
            require(sent, "Burned fees transfer failed");
        }

        emit StateAdvanced(newBlockNumber, newStateRoot, withdrawalRoot, burnedFees);
    }

    /// @notice Claim a withdrawal that was initiated on L2.
    /// @param _from L2 address that initiated the withdrawal.
    /// @param _receiver L1 address to receive the funds.
    /// @param _amount Amount of ETH to withdraw.
    /// @param _messageId Message ID from the L2 WithdrawalInitiated event.
    /// @param _blockNumber L2 block number where the withdrawal was included.
    /// @param _merkleProof Merkle proof for the withdrawal.
    function claimWithdrawal(
        address _from,
        address _receiver,
        uint256 _amount,
        uint256 _messageId,
        uint256 _blockNumber,
        bytes32[] calldata _merkleProof
    ) external nonReentrant {
        require(_blockNumber <= blockNumber, "Block not yet finalized");
        require(_receiver != address(0), "Invalid receiver");
        require(_amount > 0, "Amount must be positive");

        // Compute withdrawal hash — must match Rust: keccak256(abi.encodePacked(from, receiver, amount, messageId))
        bytes32 withdrawalHash = keccak256(
            abi.encodePacked(_from, _receiver, _amount, _messageId)
        );

        require(!claimedWithdrawals[withdrawalHash], "Withdrawal already claimed");

        // Verify Merkle proof
        bytes32 root = withdrawalRoots[_blockNumber];
        require(root != bytes32(0), "No withdrawals for this block");

        bool valid = _verifyMerkleProof(_merkleProof, root, withdrawalHash);
        require(valid, "Invalid Merkle proof");

        // Mark as claimed before transfer (checks-effects-interactions)
        claimedWithdrawals[withdrawalHash] = true;

        // Transfer ETH to receiver
        (bool success, ) = _receiver.call{value: _amount}("");
        require(success, "ETH transfer failed");

        emit WithdrawalClaimed(_receiver, _amount, _blockNumber, _messageId);
    }

    /// @dev Verify a Merkle proof using commutative Keccak256 hashing.
    /// Compatible with OpenZeppelin's MerkleProof.verify().
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

    /// @dev Commutative hash pair: H(a, b) == H(b, a).
    function _hashPair(bytes32 a, bytes32 b) private pure returns (bytes32) {
        if (a < b) {
            return keccak256(abi.encodePacked(a, b));
        } else {
            return keccak256(abi.encodePacked(b, a));
        }
    }

    /// @dev Compute a Merkle root over `count` consecutive entries in pendingL1Messages
    ///      starting at `startIdx`. Uses commutative Keccak256 hashing (same as
    ///      _verifyMerkleProof / _hashPair). Returns bytes32(0) if count is 0.
    function _computeMerkleRoot(uint256 startIdx, uint256 count) internal view returns (bytes32) {
        if (count == 0) {
            return bytes32(0);
        }
        if (count == 1) {
            return pendingL1Messages[startIdx];
        }

        // Copy leaves into memory
        bytes32[] memory level = new bytes32[](count);
        for (uint256 i = 0; i < count; i++) {
            level[i] = pendingL1Messages[startIdx + i];
        }

        // Iteratively build tree levels until one root remains
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
