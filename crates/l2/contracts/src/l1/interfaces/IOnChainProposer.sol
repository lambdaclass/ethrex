// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title Interface for the OnChainProposer contract.
/// @author LambdaClass
/// @notice A OnChainProposer contract ensures the advancement of the L2. It is used
/// by the proposer to commit blocks in batches and verify proofs.
interface IOnChainProposer {
    /// @notice The latest committed block number.
    /// @return The latest committed block number as a uint256.
    function lastCommittedBlock() external view returns (uint256);

    /// @notice The latest verified block number.
    /// @return The latest verified block number as a uint256.
    function lastVerifiedBlock() external view returns (uint256);

    /// @notice A batch has been committed.
    /// @dev Event emitted when a batch is committed.
    event BatchCommited(bytes32 indexed currentBatchCommitment);

    /// @notice A batch has been verified.
    /// @dev Event emitted when a batch is verified.
    event BatchVerified(uint256 indexed lastVerifiedBlock);

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
        address[] calldata sequencerAddress
    ) external;

    /// @notice Commits to a batch of L2 blocks.
    /// @dev Committing to an L2 batch means to store the batch's commitment
    /// and to publish withdrawals if any.
    /// @param firstBlockNumber the number of the first block to be committed.
    /// @param lastBlockNumber the number of the last block to be committed.
    /// @param commitment of the batch to be committed.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs
    /// of the batch to be committed.
    /// @param depositLogs the deposit logs of the batch to be committed.
    function commitBatch(
        uint256 firstBlockNumber,
        uint256 lastBlockNumber,
        bytes32 commitment,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 depositLogs
    ) external;

    /// @notice Method used to verify a batch of L2 blocks.
    /// @dev This method is used by the operator when a batch is ready to be
    /// verified (this is after proved).
    /// @param FirstblockNumber is the number of the first block to be verified.
    /// @param LastblockNumber is the number of the last block to be verified.
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
    function verify(
        uint256 FirstblockNumber,
        uint256 LastblockNumber,
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
