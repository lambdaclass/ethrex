// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import {ICommonBridge} from "./ICommonBridge.sol";

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

    /// @notice A verification key has been upgraded (legacy event with verifier name).
    /// @dev Event emitted when a verification key is upgraded via the SP1/RISC0 specific functions.
    /// @param verifier The name of the verifier whose key was upgraded.
    /// @param commitHash The git commit hash associated to the verification key.
    /// @param newVerificationKey The new verification key.
    event VerificationKeyUpgraded(
        string verifier,
        bytes32 commitHash,
        bytes32 newVerificationKey
    );

    /// @notice A verification key has been upgraded (generic event with program type).
    /// @dev Event emitted when a verification key is upgraded via the generic function.
    /// @param programTypeId The program type for which the key was upgraded.
    /// @param verifierId The verifier ID for which the key was upgraded.
    /// @param commitHash The git commit hash associated to the verification key.
    /// @param newVerificationKey The new verification key.
    event VerificationKeyUpgraded(
        uint8 programTypeId,
        uint8 verifierId,
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

    /// @notice Upgrades a verification key for any program type and verifier combination.
    /// @param commit_hash git commit hash that produced the new verification key
    /// @param programTypeId the guest program type (1=EVM-L2, etc.)
    /// @param verifierId the verifier ID (1=SP1, 2=RISC0)
    /// @param new_vk new verification key
    function upgradeVerificationKey(
        bytes32 commit_hash,
        uint8 programTypeId,
        uint8 verifierId,
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
    /// @param programTypeId the guest program type (1=EVM-L2, etc.). 0 defaults to EVM-L2.
    /// @param publicValuesHash keccak256 hash of proof public values for custom programs (programTypeId > 1).
    ///        Must be bytes32(0) for EVM-L2 (programTypeId == 1).
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
        uint8 programTypeId,
        bytes32 publicValuesHash,
        ICommonBridge.BalanceDiff[] calldata balanceDiffs,
        ICommonBridge.L2MessageRollingHash[] calldata l2MessageRollingHashes
    ) external;

    /// @notice Method used to verify a batch of L2 blocks.
    /// @dev This method is used by the operator when a batch is ready to be
    /// verified (this is after proved).
    /// @param batchNumber is the number of the batch to be verified.
    /// ----------------------------------------------------------------------
    /// @param risc0BlockProof is the proof of the batch to be verified.
    /// ----------------------------------------------------------------------
    /// @param sp1ProofBytes Groth16 proof
    /// ----------------------------------------------------------------------
    /// @param tdxSignature TDX signature
    function verifyBatch(
        uint256 batchNumber,
        //risc0
        bytes memory risc0BlockProof,
        //sp1
        bytes memory sp1ProofBytes,
        //tdx
        bytes memory tdxSignature,
        // Custom program public values (only needed for programTypeId > 1)
        bytes memory customPublicValues
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
