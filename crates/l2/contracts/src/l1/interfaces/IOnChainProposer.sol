// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

/// @title Interface for the OnChainProposer contract.
/// @author LambdaClass
/// @notice A OnChainProposer contract ensures the advancement of the L2. It is used
/// by the proposer to commit batches of l2 blocks and verify proofs.
interface IOnChainProposer {
    /// @notice The latest committed batch number.
    /// @return The latest committed batch number as a uint256.
    function lastCommittedBatch() external view returns (uint256);

    /// @notice The latest verified batch number.
    /// @return The latest verified batch number as a uint256.
    function lastVerifiedBatch() external view returns (uint256);

    /// @notice A batch has been committed.
    /// @dev Event emitted when a batch is committed.
    /// @param newStateRoot The new state root of the batch that was committed.
    event BatchCommitted(bytes32 indexed newStateRoot);

    /// @notice A batch has been verified.
    /// @dev Event emitted when a batch is verified.
    event BatchVerified(uint256 indexed lastVerifiedBatch);

    /// @notice A batch has been reverted.
    /// @dev Event emitted when a batch is reverted.
    event BatchReverted(bytes32 indexed newStateRoot);

    /// @notice Set the bridge address for the first time.
    /// @dev This method is separated from initialize because both the CommonBridge
    /// and the OnChainProposer need to know the address of the other. This solves
    /// the circular dependency while allowing to initialize the proxy with the deploy.
    /// @param bridge the address of the bridge contract.
    function initializeBridgeAddress(address bridge) external;

    /// @notice Commits to a batch of L2 blocks.
    /// @dev Committing to an L2 batch means to store the batch's commitment
    /// and to publish withdrawals if any.
    /// @param batchNumber the number of the batch to be committed.
    /// @param newStateRoot the new state root of the batch to be committed.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs
    /// of the batch to be committed.
    /// @param processedDepositLogsRollingHash the rolling hash of the processed
    /// deposits logs of the batch to be committed.
    /// @param lastBlockHash the hash of the last block of the batch to be committed.
    function commitBatch(
        uint256 batchNumber,
        bytes32 newStateRoot,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 processedDepositLogsRollingHash,
        bytes32 lastBlockHash
    ) external;

    /// @notice Method used to verify a batch of L2 blocks.
    /// @dev This method is used by the operator when a batch is ready to be
    /// verified (this is after proved).
    /// @param batchNumber is the number of the batch to be verified.
    /// ----------------------------------------------------------------------
    /// @param risc0BlockProof is the proof of the batch to be verified.
    /// @param risc0ImageId Digest of the zkVM imageid.
    /// @param risc0Journal public_inputs aka journal
    /// ----------------------------------------------------------------------
    /// @param sp1PublicValues Values used to perform the execution
    /// @param sp1ProofBytes Groth16 proof
    /// ----------------------------------------------------------------------
    /// @param tdxPublicValues Values used to perform the execution
    /// @param tdxSignature TDX signature
    function verifyBatch(
        uint256 batchNumber,
        //risc0
        bytes memory risc0BlockProof,
        bytes32 risc0ImageId,
        bytes calldata risc0Journal,
        //sp1
        bytes calldata sp1PublicValues,
        bytes memory sp1ProofBytes,
        //tdx
        bytes calldata tdxPublicValues,
        bytes memory tdxSignature
    ) external;
    // TODO: imageid, programvkey and riscvvkey should be constants
    // TODO: organize each zkvm proof arguments in their own structs

    /// @notice Method used to verify a batch of L2 blocks in Aligned.
    /// @param alignedPublicInputs The public inputs bytes of the proof.
    /// @param alignedMerkleProof  The Merkle proof (sibling hashes) needed to reconstruct the Merkle root.
    function verifyBatchAligned(
        uint256 batchNumber,
        bytes calldata alignedPublicInputs,
        bytes32[] calldata alignedMerkleProof
    ) external;

    /// @notice Allows unverified batches to be reverted
    function revertBatch(uint256 batchNumber) external;

    /// @notice Allows the owner to pause the contract
    function pause() external;

    /// @notice Allows the owner to unpause the contract
    function unpause() external;
}
