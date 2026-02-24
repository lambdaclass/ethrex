// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import {ICommonBridge} from "./ICommonBridge.sol";

/// @title Interface for the OnChainProposer contract.
/// @author LambdaClass
/// @notice A OnChainProposer contract ensures the advancement of the L2. It is used
/// by the proposer to commit batches of l2 blocks and verify proofs.
interface IOnChainProposer {
    // Initialization errors
    error MissingRisc0Verifier();
    error MissingSp1Verifier();
    error MissingTdxVerifier();
    error AlignedModeRequiresSp1();
    error AlignedModeDoesNotSupportRisc0();
    error CommitHashIsZero();
    error MissingSp1VerificationKey();
    error MissingRisc0VerificationKey();
    error BridgeIsZeroAddress();
    error BridgeIsContractAddress();

    // Commit errors
    error BatchNumberNotSuccessor();
    error BatchAlreadyCommitted();
    error LastBlockHashIsZero();
    error InvalidPrivilegedTransactionLogs();
    error InvalidL2MessageRollingHash();
    error ValidiumBlobPublished();
    error RollupBlobNotPublished();
    error MissingVerificationKeyForCommit();

    // Verify errors
    error UseAlignedVerification();
    error UseSmartContractVerification();
    error BatchNotSequential();
    error BatchNotCommitted();
    error EmptyBatchArray();
    error BatchArrayLengthMismatch();
    error ExpiredPrivilegedTransactionDeadline();
    error InvalidRisc0Proof();
    error InvalidSp1Proof();
    error InvalidTdxProof();

    // Aligned verify errors
    error IncorrectFirstBatchNumber();
    error LastBatchExceedsCommitted();
    error Sp1ProofArrayLengthMismatch();
    error Risc0ProofArrayLengthMismatch();
    error AlignedAggregatorCallFailed();
    error AlignedProofVerificationFailed();

    // Revert errors
    error CannotRevertVerifiedBatch();
    error NoBatchesToRevert();

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

    /// @notice A verification key has been upgraded.
    /// @dev Event emitted when a verification key is upgraded.
    /// @param verifier The name of the verifier whose key was upgraded.
    /// @param commitHash The git commit hash associated to the verification key.
    /// @param newVerificationKey The new verification key.
    event VerificationKeyUpgraded(
        string verifier,
        bytes32 commitHash,
        bytes32 newVerificationKey
    );

    /// @notice Upgrades the SP1 verification key that represents the sequencer's code.
    /// @param new_vk new verification key for SP1 verifier
    /// @param commit_hash git commit hash that produced the new verification key
    function upgradeSP1VerificationKey(
        bytes32 commit_hash,
        bytes32 new_vk
    ) external;

    /// @notice Upgrades the RISC0 verification key that represents the sequencer's code.
    /// @param new_vk new verification key for RISC0 verifier
    /// @param commit_hash git commit hash that produced the new verification key
    function upgradeRISC0VerificationKey(
        bytes32 commit_hash,
        bytes32 new_vk
    ) external;

    /// @notice Commits to a batch of L2 blocks.
    /// @dev Committing to an L2 batch means to store the batch's commitment
    /// and to publish withdrawals if any.
    /// @param batchNumber the number of the batch to be committed.
    /// @param newStateRoot the new state root of the batch to be committed.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs
    /// of the batch to be committed.
    /// @param processedPrivilegedTransactionsRollingHash the rolling hash of the processed
    /// privileged transactions of the batch to be committed.
    /// @param lastBlockHash the hash of the last block of the batch to be committed.
    /// @param nonPrivilegedTransactions the number of non-privileged transactions in the batch to be committed.
    /// @param commitHash git commit hash that produced the verifier keys for this batch.
    /// @param balanceDiffs the balance diffs of the batch to be committed.
    /// @param l2MessageRollingHashes the L2 message rolling hashes of the batch to be committed.
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

    /// @notice Method used to verify one or more consecutive L2 batches in a single transaction.
    /// @param firstBatchNumber The batch number of the first batch to verify. Must be `lastVerifiedBatch + 1`.
    /// @param risc0BlockProofs An array of RISC0 proofs, one per batch.
    /// @param sp1ProofsBytes An array of SP1 proofs, one per batch.
    /// @param tdxSignatures An array of TDX signatures, one per batch.
    function verifyBatches(
        uint256 firstBatchNumber,
        bytes[] calldata risc0BlockProofs,
        bytes[] calldata sp1ProofsBytes,
        bytes[] calldata tdxSignatures
    ) external;

    // TODO: imageid, programvkey and riscvvkey should be constants
    // TODO: organize each zkvm proof arguments in their own structs

    /// @notice Method used to verify a sequence of L2 batches in Aligned, starting from `firstBatchNumber`.
    /// Each proof corresponds to one batch, and batch numbers must increase by 1 sequentially.
    /// @param firstBatchNumber The batch number of the first proof to verify. Must be `lastVerifiedBatch + 1`.
    /// @param lastBatchNumber The batch number of the last proof to verify. Must be `lastBatchNumber <= lastCommittedBatch`.
    /// @param sp1MerkleProofsList An array of Merkle proofs (sibling hashes), one per SP1 proof.
    /// @param risc0MerkleProofsList An array of Merkle proofs (sibling hashes), one per Risc0 proof.
    function verifyBatchesAligned(
        uint256 firstBatchNumber,
        uint256 lastBatchNumber,
        bytes32[][] calldata sp1MerkleProofsList,
        bytes32[][] calldata risc0MerkleProofsList
    ) external;

    /// @notice Allows unverified batches to be reverted
    /// @param batchNumber the number of the batch to revert. The commitment for that batch
    /// and for all subsequent batches will be removed. The batch can only be reverted if it is not verified.
    function revertBatch(uint256 batchNumber) external;

    /// @notice Allows the owner to pause the contract
    function pause() external;

    /// @notice Allows the owner to unpause the contract
    function unpause() external;
}
