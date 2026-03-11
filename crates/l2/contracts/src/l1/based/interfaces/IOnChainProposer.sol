// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @title Interface for the OnChainProposer contract.
/// @author LambdaClass
/// @notice A OnChainProposer contract ensures the advancement of the L2. It is used
/// by the proposer to commit batches of l2 blocks and verify proofs.
interface IOnChainProposer {
    // Initialization errors
    error MissingRisc0Verifier(); // 0x1871c084
    error MissingSp1Verifier(); // 0xc0b62621
    error MissingTdxVerifier(); // 0xee7f6b60
    error AlignedModeRequiresSp1(); // 0xb41e395e
    error AlignedModeDoesNotSupportRisc0(); // 0xadae5cff
    error CommitHashIsZero(); // 0x1f14b5aa
    error MissingSp1VerificationKey(); // 0x544c07b4
    error MissingRisc0VerificationKey(); // 0x60c177e0
    error BridgeIsZeroAddress(); // 0x850138f2
    error BridgeIsContractAddress(); // 0xdf655de9
    error AlreadyInitialized(); // 0x0dc149f0
    error SequencerRegistryIsZeroAddress(); // 0xd385dd80
    error SequencerRegistryIsContractAddress(); // 0x7064abcc

    // Commit errors
    error BatchNumberNotSuccessor(); // 0xcd1793fe
    error BatchAlreadyCommitted(); // 0x884923cf
    error LastBlockHashIsZero(); // 0xf1cbbe55
    error InvalidPrivilegedTransactionLogs(); // 0x9e6e5638
    error InvalidL2MessageRollingHash(); // 0x52ba57cb
    error ValidiumBlobPublished(); // 0x0c9c8061
    error RollupBlobNotPublished(); // 0xe1aa9667
    error MissingVerificationKeyForCommit(); // 0xf6b9798e

    // Verify errors
    error UseAlignedVerification(); // 0x2da8b4d4
    error UseSmartContractVerification(); // 0xc8d8ecb9
    error BatchNotSequential(); // 0xceb05a46
    error BatchNotCommitted(); // 0xc26a3294
    error EmptyBatchArray(); // 0x7dc57e7b
    error BatchArrayLengthMismatch(); // 0xfc5221bd
    error ExpiredPrivilegedTransactionDeadline(); // 0x1420ad0b
    error InvalidRisc0Proof(); // 0x14add973
    error InvalidSp1Proof(); // 0x7ff849b5
    error InvalidTdxProof(); // 0x62013a95

    // Aligned verify errors
    error IncorrectFirstBatchNumber(); // 0x5f83abb8
    error LastBatchExceedsCommitted(); // 0xfbd95da9
    error Sp1ProofArrayLengthMismatch(); // 0xc1af923d
    error Risc0ProofArrayLengthMismatch(); // 0x60fecd3e
    error AlignedAggregatorCallFailed(); // 0x63934992
    error AlignedProofVerificationFailed(); // 0x44602025

    // Access control errors
    error CallerHasNoSequencingRights(); // 0xac0192af

    /// @notice The latest committed batch number.
    /// @return The latest committed batch number as a uint256.
    function lastCommittedBatch() external view returns (uint256);

    /// @notice The latest verified batch number.
    /// @return The latest verified batch number as a uint256.
    function lastVerifiedBatch() external view returns (uint256);

    /// @notice A batch has been committed.
    /// @dev Event emitted when a batch is committed.
    /// @param newStateRoot The new state root of the batch that was committed.
    event BatchCommitted(uint256 indexed number, bytes32 indexed newStateRoot);

    /// @notice A batch has been verified.
    /// @dev Event emitted when a batch is verified.
    event BatchVerified(uint256 indexed lastVerifiedBatch);

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
    /// @param commitHash git commit hash that produced the verifier keys for this batch.
    /// @param new_vk new verification key for SP1 verifier
    function upgradeSP1VerificationKey(
        bytes32 commitHash,
        bytes32 new_vk
    ) external;

    /// @notice Upgrades the RISC0 verification key that represents the sequencer's code.
    /// @param commitHash git commit hash that produced the verifier keys for this batch.
    /// @param new_vk new verification key for RISC0 verifier
    function upgradeRISC0VerificationKey(
        bytes32 commitHash,
        bytes32 new_vk
    ) external;

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
    /// @param nonPrivilegedTransactions the number of non-privileged transactions in the batch.
    /// @param commitHash git commit hash that produced the verifier keys for this batch.
    /// @param _rlpEncodedBlocks the list of RLP-encoded blocks in the batch.
    function commitBatch(
        uint256 batchNumber,
        bytes32 newStateRoot,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 processedDepositLogsRollingHash,
        bytes32 lastBlockHash,
        uint256 nonPrivilegedTransactions,
        bytes32 commitHash,
        bytes[] calldata _rlpEncodedBlocks
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
}
