// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title NativeRollup — PoC L2 state manager using the EXECUTE precompile (EIP-8079).
///
/// Maintains the current L2 state root and block number. The `advance` method
/// builds the binary calldata for the EXECUTE precompile at 0x0101, which
/// re-executes the L2 block and verifies the state transition. On success,
/// the contract updates its state.
///
/// Deposits are recorded via `deposit(address)` and consumed by `advance()`.
contract NativeRollup {
    bytes32 public stateRoot;
    uint256 public blockNumber;

    address constant EXECUTE_PRECOMPILE = address(0x0101);

    struct PendingDeposit {
        address recipient;
        uint256 amount;
    }
    PendingDeposit[] public pendingDeposits;
    uint256 public depositIndex;

    event StateAdvanced(uint256 indexed newBlockNumber, bytes32 newStateRoot);
    event DepositRecorded(address indexed recipient, uint256 amount);

    constructor(bytes32 _initialStateRoot) {
        stateRoot = _initialStateRoot;
    }

    /// @notice Record a deposit to be included in the next L2 block.
    /// @param _recipient The L2 address that will receive the deposited ETH.
    function deposit(address _recipient) external payable {
        require(msg.value > 0, "Must send ETH");
        pendingDeposits.push(PendingDeposit(_recipient, msg.value));
        emit DepositRecorded(_recipient, msg.value);
    }

    /// @notice Advance the L2 by one block.
    /// @param _newStateRoot Expected post-state root (verified by the precompile).
    /// @param _newBlockNumber The L2 block number being applied.
    /// @param _depositsCount Number of pending deposits to consume from the queue.
    /// @param _block RLP-encoded L2 block.
    /// @param _witness JSON-serialized ExecutionWitness.
    function advance(
        bytes32 _newStateRoot,
        uint256 _newBlockNumber,
        uint256 _depositsCount,
        bytes calldata _block,
        bytes calldata _witness
    ) external {
        uint256 startIdx = depositIndex;
        require(startIdx + _depositsCount <= pendingDeposits.length, "Not enough deposits");

        // Build binary precompile input:
        //   [32] pre_state_root
        //   [32] post_state_root
        //   [4]  num_deposits (uint32 big-endian)
        //   [52 * num_deposits] deposits (20 address + 32 amount each)
        //   [4]  block_rlp_length (uint32 big-endian)
        //   [block_rlp_length] block RLP
        //   [remaining] witness JSON
        bytes memory input = abi.encodePacked(
            stateRoot,
            _newStateRoot,
            uint32(_depositsCount)
        );

        for (uint256 i = 0; i < _depositsCount; i++) {
            PendingDeposit storage dep = pendingDeposits[startIdx + i];
            input = bytes.concat(input, abi.encodePacked(dep.recipient, dep.amount));
        }

        // _block = RLP-encoded block, _witness = JSON-serialized ExecutionWitness
        input = bytes.concat(input, abi.encodePacked(uint32(_block.length)), _block, _witness);

        depositIndex = startIdx + _depositsCount;

        (bool success, bytes memory result) = EXECUTE_PRECOMPILE.call(input);
        require(
            success && result.length == 1 && uint8(result[0]) == 0x01,
            "EXECUTE precompile verification failed"
        );

        stateRoot = _newStateRoot;
        blockNumber = _newBlockNumber;

        emit StateAdvanced(_newBlockNumber, _newStateRoot);
    }
}
