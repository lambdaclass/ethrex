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

    /// @notice Initializes the contract.
    /// @dev This method is called only once after the contract is deployed.
    /// @dev It sets the bridge address.
    /// @param bridge the address of the bridge contract.
    /// @param r0verifier the address of the risc0 groth16 verifier.
    /// @param sp1verifier the address of the sp1 groth16 verifier.
    function initialize(
        address bridge,
        address r0verifier,
        address sp1verifier,
        address picoverifier,
        bytes32 sp1vkey,
        address[] calldata sequencerAddress
    ) external;

    /// @notice Commits to a batch of L2 blocks.
    /// @dev Committing to an L2 batch means to store the batch's commitment
    /// and to publish withdrawals if any.
    /// @param batchNumber the number of the batch to be committed.
    /// @param newStateRoot the new state root of the batch to be committed.
    /// @param stateDiffKZGVersionedHash of the block to be committed.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs
    /// of the batch to be committed.
    /// @param processedDepositLogsRollingHash the rolling hash of the processed
    /// deposits logs of the batch to be committed.
    function commitBatch(
        uint256 batchNumber,
        bytes32 newStateRoot,
        bytes32 stateDiffKZGVersionedHash,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 processedDepositLogsRollingHash
    ) external;

    /// @notice Method used to verify a batch of L2 blocks.
    /// @dev This method is used by the operator when a batch is ready to be
    /// verified (this is after proved).
    /// @param batchNumber is the number of the batch to be verified.
    /// ----------------------------------------------------------------------
    /// @param risc0BlockProof is the proof of the batch to be verified.
    /// @param risc0ImageId Digest of the zkVM imageid.
    /// @param risc0JournalDigest Digest of the public_inputs aka journal
    /// ----------------------------------------------------------------------
    /// @param sp1ProgramVKey Public verifying key
    /// @param sp1PublicValues Values used to perform the execution
    /// @param sp1ProofBytes Groth16 proof
    /// ----------------------------------------------------------------------
    /// @param picoRiscvVkey Public verifying key
    /// @param picoPublicValues Values used to perform the execution
    /// @param picoProof Groth16 proof
    function verifyBatch(
        uint256 batchNumber,
        //risc0
        bytes calldata risc0BlockProof,
        bytes32 risc0ImageId,
        bytes32 risc0JournalDigest,
        //sp1
        bytes32 sp1ProgramVKey,
        bytes calldata sp1PublicValues,
        bytes calldata sp1ProofBytes,
        //pico
        bytes32 picoRiscvVkey,
        bytes calldata picoPublicValues,
        uint256[8] calldata picoProof
    ) external;
    // TODO: imageid, programvkey and riscvvkey should be constants
    // TODO: organize each zkvm proof arguments in their own structs
}
