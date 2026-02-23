// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import {IOnChainProposer} from "./IOnChainProposer.sol";
import {ICommonBridge} from "./ICommonBridge.sol";

/// @title Interface for the Timelock contract.
/// @author LambdaClass
/// @notice Timelock owner for OnChainProposer, manages roles and forwards sequencer actions.
interface ITimelock {
    /// @notice Emitted when the Security Council executes a call bypassing the delay.
    /// @param target The address that was called.
    /// @param value The ETH value that was sent.
    /// @param data The calldata that was forwarded to `target`.
    event EmergencyExecution(address indexed target, uint256 value, bytes data);

    // @notice Used for functions that can only be called by the Timelock itself.
    error TimelockCallerNotSelf();

    // @notice Used for other initialize() from contracts that the Timelock inherits from
    error TimelockUseCustomInitialize();

    /// @notice The OnChainProposer contract controlled by this timelock.
    function onChainProposer() external view returns (IOnChainProposer);

    /// @notice Initializes the timelock contract.
    /// @dev Called once after proxy deployment.
    /// @param minDelay The minimum delay (in seconds) for scheduled operations.
    /// @param sequencers Accounts that can commit and verify batches.
    /// @param governance The account that can propose and execute operations, respecting the delay.
    /// @param securityCouncil The Security Council account that can manage roles and bypass the delay.
    /// @param _onChainProposer The deployed `OnChainProposer` contract address.
    function initialize(
        uint256 minDelay,
        address[] memory sequencers,
        address governance,
        address securityCouncil,
        address _onChainProposer
    ) external;

    /// @notice Returns whether an address has the sequencer role.
    function isSequencer(address addr) external view returns (bool);

    /// @notice Commits a batch through the timelock.
    function commitBatch(
        uint256 batchNumber,
        bytes32 newStateRoot,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 processedPrivilegedTransactionsRollingHash,
        bytes32 lastBlockHash,
        uint256 nonPrivilegedTransactions,
        bytes32 commitHash,
        ICommonBridge.BalanceDiff[] calldata balanceDiffs,
        ICommonBridge.L2MessageRollingHash[] calldata l2MessageRollingHashes
    ) external;

    /// @notice Verifies one or more consecutive batches through the timelock.
    function verifyBatches(
        uint256 firstBatchNumber,
        bytes[] calldata risc0BlockProofs,
        bytes[] calldata sp1ProofsBytes,
        bytes[] calldata tdxSignatures
    ) external;

    /// @notice Verifies multiple batches through the timelock using aligned proofs.
    function verifyBatchesAligned(
        uint256 firstBatchNumber,
        uint256 lastBatchNumber,
        bytes32[][] calldata sp1MerkleProofsList,
        bytes32[][] calldata risc0MerkleProofsList
    ) external;

    /// @notice Executes an operation immediately, bypassing the timelock delay.
    function emergencyExecute(
        address target,
        uint256 value,
        bytes calldata data
    ) external payable;
}
