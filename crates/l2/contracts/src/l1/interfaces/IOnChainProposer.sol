// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title Interface for the OnChainProposer contract.
/// @author LambdaClass
/// @notice A OnChainProposer contract ensures the advancement of the L2. It is used
/// by the proposer to commit blocks and verify block proofs.
interface IOnChainProposer {
    /// @notice The latest verified block number.
    function lastVerifiedBlock() external view returns (uint256);

    /// @notice A block has been committed.
    /// @dev Event emitted when a block is committed.
    event BlockCommitted(bytes32 indexed currentBlockCommitment);

    /// @notice A block has been verified.
    /// @dev Event emitted when a block is verified.
    event BlockVerified(uint256 indexed blockNumber);

    /// @notice Initializes the contract.
    /// @dev This method is called only once after the contract is deployed.
    /// @dev It sets the bridge address.
    /// @param bridge the address of the bridge contract.
    function initialize(address bridge) external;

    /// @notice Commits to an L2 block.
    /// @dev Committing to an L2 block means to store the block's commitment
    /// and to publish withdrawals if any.
    /// @param blockNumber the number of the block to be committed.
    /// @param commitment of the block to be committed.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs
    /// of the block to be committed.
    /// @param depositLogs the deposit logs of the block to be committed.
    function commit(
        uint256 blockNumber,
        bytes32 commitment,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 depositLogs
    ) external;

    /// @notice Method used to verify an L2 block proof.
    /// @dev This method is used by the operator when a block is ready to be
    /// verified (this is after proved).
    /// @param blockNumber is the number of the block to be verified.
    /// @param blockProof is the proof of the block to be verified.
    function verify(uint256 blockNumber, bytes calldata blockProof) external;
}
