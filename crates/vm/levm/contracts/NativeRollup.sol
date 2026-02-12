// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title NativeRollup — PoC L2 state manager using the EXECUTE precompile (EIP-8079).
///
/// Maintains the current L2 state root and block number. The `advance` method
/// builds ABI-encoded calldata for the EXECUTE precompile at 0x0101, which
/// re-executes the L2 block and verifies the state transition. On success,
/// the precompile returns the new state root and block number, and the
/// contract updates its state.
///
/// Deposits are recorded via `deposit(address)` and consumed by `advance()`.
/// ETH sent directly to the contract is deposited for `msg.sender`.
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

    /// @notice Receive ETH and record a deposit for msg.sender.
    receive() external payable {
        require(msg.value > 0, "Must send ETH");
        pendingDeposits.push(PendingDeposit(msg.sender, msg.value));
        emit DepositRecorded(msg.sender, msg.value);
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

        // Build packed deposits data: each deposit = 20 bytes address + 32 bytes amount
        bytes memory depositsData;
        for (uint256 i = 0; i < _depositsCount; i++) {
            PendingDeposit storage dep = pendingDeposits[startIdx + i];
            depositsData = bytes.concat(depositsData, abi.encodePacked(dep.recipient, dep.amount));
        }

        depositIndex = startIdx + _depositsCount;

        // Build ABI-encoded precompile input:
        //   abi.encode(bytes32 preStateRoot, bytes blockRlp, bytes witnessJson, bytes depositsData)
        bytes memory input = abi.encode(stateRoot, _block, _witness, depositsData);

        (bool success, bytes memory result) = EXECUTE_PRECOMPILE.call(input);
        require(success && result.length == 64, "EXECUTE precompile verification failed");

        // Decode new state root and block number from precompile return
        (bytes32 newStateRoot, uint256 newBlockNumber) = abi.decode(result, (bytes32, uint256));

        stateRoot = newStateRoot;
        blockNumber = newBlockNumber;

        emit StateAdvanced(newBlockNumber, newStateRoot);
    }
}
