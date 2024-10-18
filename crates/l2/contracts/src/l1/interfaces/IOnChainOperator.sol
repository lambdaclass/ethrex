// SPDX-License-Identifier: MIT
pragma solidity 0.8.27;

/// @title Interface for the OnChainOperator contract.
/// @author LambdaClass
/// @notice A OnChainOperator contract ensures the advancement of the L2. It is used
/// by the operator to commit blocks and verify block proofs.
interface IOnChainOperator {
    /// @notice A block has been committed.
    /// @dev Event emitted when a block is committed.
    event BlockCommitted(bytes32 indexed currentBlockCommitment);

    /// @notice A block has been verified.
    /// @dev Event emitted when a block is verified.
    event BlockVerified(bytes32 indexed blockHash);

    /// @notice Method used to commit an L2 block to be proved.
    /// @dev This method is used by the operator when a block is ready to be
    /// proved.
    /// @param currentBlockCommitment is the committment to the block to be proved.
    function commit(bytes32 currentBlockCommitment) external;

    /// @notice Method used to verify an L2 block proof.
    /// @dev This method is used by the operator when a block is ready to be
    /// verified (this is after proved).
    /// @param blockProof is the proof of the block to be verified.
    function verify(bytes calldata blockProof) external;
}
