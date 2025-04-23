// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

/// @title Interface for the OnChainProposer contract.
/// @author LambdaClass
/// @notice A OnChainProposer contract ensures the advancement of the L2. It is used
/// by the proposer to commit blocks and verify block proofs.
interface IOnChainProposer {
    /// @notice The latest committed block number.
    /// @return The latest committed block number as a uint256.
    function lastCommittedBlock() external view returns (uint256);

    /// @notice The latest verified block number.
    /// @return The latest verified block number as a uint256.
    function lastVerifiedBlock() external view returns (uint256);

    // TODO: We are using the state diff KZG versioned hash to figure out
    // whether a block was committed or not. Ideally, we should use the new
    // state root hash instead.
    /// @notice A block has been committed.
    /// @dev Event emitted when a block is committed.
    /// @param stateDiffKZGVersionedHash the KZG versioned hash of the committed
    /// block's state diff.
    event BlockCommitted(bytes32 indexed stateDiffKZGVersionedHash);

    /// @notice A block has been verified.
    /// @dev Event emitted when a block is verified.
    event BlockVerified(uint256 indexed blockNumber);

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

    /// @notice Commits to an L2 block.
    /// @dev Committing to an L2 block means to store the block's commitment
    /// and to publish withdrawals if any.
    /// @param blockNumber the number of the block to be committed.
    /// @param stateDiffKZGVersionedHash of the block to be committed.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs
    /// of the block to be committed.
    /// @param processedDepositLogs the processed deposits logs of the block to
    /// be committed.
    function commit(
        uint256 blockNumber,
        bytes32 stateDiffKZGVersionedHash,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 processedDepositLogs
    ) external;

    /// @notice Method used to verify an L2 block proof.
    /// @dev This method is used by the operator when a block is ready to be
    /// verified (this is after proved).
    /// @param blockNumber is the number of the block to be verified.
    /// ----------------------------------------------------------------------
    /// @param risc0BlockProof is the proof of the block to be verified.
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
        uint256 blockNumber,
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
