// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title NativeRollup — PoC L2 state manager using the EXECUTE precompile (EIP-8079).
///
/// Maintains the current L2 state root and block number. The `advance` method
/// builds ABI-encoded calldata for the EXECUTE precompile at 0x0101, which
/// re-executes the L2 block and verifies the state transition. On success,
/// the precompile returns the new state root, block number, and withdrawal
/// Merkle root, and the contract updates its state.
///
/// Deposits are recorded via `deposit(address)` as keccak256 hashes. When
/// `advance()` is called, it computes a rolling hash over the consumed deposits
/// and passes it to the EXECUTE precompile, which verifies it against
/// DepositProcessed events from the L2Bridge predeploy.
///
/// Withdrawals are initiated on L2 via the L2Bridge contract. The EXECUTE
/// precompile extracts withdrawal events and computes a Merkle root. Users can
/// claim withdrawals on L1 via `claimWithdrawal()` with a Merkle proof.
///
/// Storage layout:
///   Slot 0: stateRoot (bytes32)
///   Slot 1: blockNumber (uint256)
///   Slot 2: pendingDeposits (bytes32[])
///   Slot 3: depositIndex (uint256)
///   Slot 4: withdrawalRoots (mapping)
///   Slot 5: claimedWithdrawals (mapping)
///   Slot 6: _locked (bool)
contract NativeRollup {
    bytes32 public stateRoot;
    uint256 public blockNumber;

    address constant EXECUTE_PRECOMPILE = address(0x0101);

    bytes32[] public pendingDeposits;
    uint256 public depositIndex;

    // Withdrawal state
    mapping(uint256 => bytes32) public withdrawalRoots;
    mapping(bytes32 => bool) public claimedWithdrawals;

    // Reentrancy guard
    bool private _locked;

    event StateAdvanced(uint256 indexed newBlockNumber, bytes32 newStateRoot, bytes32 withdrawalRoot);
    event DepositRecorded(address indexed recipient, uint256 amount, uint256 indexed nonce);
    event WithdrawalClaimed(address indexed receiver, uint256 amount, uint256 indexed blockNumber, uint256 indexed messageId);

    modifier nonReentrant() {
        require(!_locked, "ReentrancyGuard: reentrant call");
        _locked = true;
        _;
        _locked = false;
    }

    constructor(bytes32 _initialStateRoot) {
        stateRoot = _initialStateRoot;
    }

    /// @notice Record a deposit to be included in a future L2 block.
    /// @param _recipient The L2 address that will receive the deposited ETH.
    function deposit(address _recipient) external payable {
        require(msg.value > 0, "Must send ETH");
        uint256 nonce = pendingDeposits.length;
        bytes32 depositHash = keccak256(abi.encodePacked(_recipient, msg.value, nonce));
        pendingDeposits.push(depositHash);
        emit DepositRecorded(_recipient, msg.value, nonce);
    }

    /// @notice Receive ETH and record a deposit for msg.sender.
    receive() external payable {
        require(msg.value > 0, "Must send ETH");
        uint256 nonce = pendingDeposits.length;
        bytes32 depositHash = keccak256(abi.encodePacked(msg.sender, msg.value, nonce));
        pendingDeposits.push(depositHash);
        emit DepositRecorded(msg.sender, msg.value, nonce);
    }

    /// @notice Advance the L2 by one block.
    /// @param _depositsCount Number of pending deposits to consume from the queue.
    /// @param _block RLP-encoded L2 block.
    /// @param _witness JSON-serialized ExecutionWitness.
    function advance(
        uint256 _depositsCount,
        bytes calldata _block,
        bytes calldata _witness
    ) external {
        uint256 startIdx = depositIndex;
        require(startIdx + _depositsCount <= pendingDeposits.length, "Not enough deposits");

        // Compute rolling hash over the consumed deposit batch.
        // Each pendingDeposits[i] is keccak256(abi.encodePacked(recipient, amount, nonce)).
        // The rolling hash is: rolling_i = keccak256(abi.encodePacked(rolling_{i-1}, deposit_hash_i))
        bytes32 depositsRollingHash = bytes32(0);
        for (uint256 i = 0; i < _depositsCount; i++) {
            depositsRollingHash = keccak256(
                abi.encodePacked(depositsRollingHash, pendingDeposits[startIdx + i])
            );
        }

        depositIndex = startIdx + _depositsCount;

        // ABI layout for the EXECUTE precompile:
        //   slot 0: preStateRoot        (bytes32, static)
        //   slot 1: offset_to_block     (uint256, dynamic pointer → 0x80)
        //   slot 2: offset_to_witness   (uint256, dynamic pointer)
        //   slot 3: depositsRollingHash (bytes32, static — NOT a pointer)
        //   tail:   [block data] [witness data]
        bytes memory input = abi.encode(stateRoot, _block, _witness, depositsRollingHash);

        (bool success, bytes memory result) = EXECUTE_PRECOMPILE.call(input);
        require(success && result.length == 128, "EXECUTE precompile verification failed");

        // Decode new state root, block number, withdrawal root, and gas used from precompile return
        (bytes32 newStateRoot, uint256 newBlockNumber, bytes32 withdrawalRoot) = abi.decode(result, (bytes32, uint256, bytes32));

        stateRoot = newStateRoot;
        blockNumber = newBlockNumber;
        withdrawalRoots[newBlockNumber] = withdrawalRoot;

        emit StateAdvanced(newBlockNumber, newStateRoot, withdrawalRoot);
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
}
