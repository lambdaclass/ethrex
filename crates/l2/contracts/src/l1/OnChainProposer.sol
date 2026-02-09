// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import "./interfaces/IOnChainProposer.sol";
import {CommonBridge} from "./CommonBridge.sol";
import {ICommonBridge} from "./interfaces/ICommonBridge.sol";
import {IRiscZeroVerifier} from "./interfaces/IRiscZeroVerifier.sol";
import {ISP1Verifier} from "./interfaces/ISP1Verifier.sol";
import {ITDXVerifier} from "./interfaces/ITDXVerifier.sol";
import "../l2/interfaces/ICommonBridgeL2.sol";

/// @title OnChainProposer contract.
/// @author LambdaClass
contract OnChainProposer is
    IOnChainProposer,
    Initializable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable,
    PausableUpgradeable
{
    /// @notice Committed batches data.
    /// @dev This struct holds the information about the committed batches.
    /// @dev processedPrivilegedTransactionsRollingHash is the Merkle root of the hashes of the
    /// privileged transactions that were processed in the batch being committed. The amount of
    /// hashes that are encoded in this root are to be removed from the
    /// pendingTxHashes queue of the CommonBridge contract.
    /// @dev withdrawalsLogsMerkleRoot is the Merkle root of the Merkle tree containing
    /// all the withdrawals that were processed in the batch being committed
    /// @dev commitHash: keccak of the git commit hash that produced the proof/verification key used for this batch
    struct BatchCommitmentInfo {
        bytes32 newStateRoot;
        bytes32 blobKZGVersionedHash;
        bytes32 processedPrivilegedTransactionsRollingHash;
        bytes32 withdrawalsLogsMerkleRoot;
        bytes32 lastBlockHash;
        uint256 nonPrivilegedTransactions;
        ICommonBridge.BalanceDiff[] balanceDiffs;
        bytes32 commitHash;
        ICommonBridge.L2MessageRollingHash[] l2InMessageRollingHashes;
    }

    uint8 internal constant SP1_VERIFIER_ID = 1;
    uint8 internal constant RISC0_VERIFIER_ID = 2;

    /// @notice Aligned Layer proving system ID for SP1 in isProofVerified calls.
    /// @dev Currently only SP1 is supported by Aligned in aggregation mode.
    uint16 internal constant ALIGNED_SP1_PROVING_SYSTEM_ID = 1;

    /// @notice The commitments of the committed batches.
    /// @dev If a batch is committed, the commitment is stored here.
    /// @dev If a batch was not committed yet, it won't be here.
    /// @dev It is used by other contracts to verify if a batch was committed.
    /// @dev The key is the batch number.
    mapping(uint256 => BatchCommitmentInfo) public batchCommitments;

    /// @notice The latest verified batch number.
    /// @dev This variable holds the batch number of the most recently verified batch.
    /// @dev All batches with a batch number less than or equal to `lastVerifiedBatch` are considered verified.
    /// @dev Batches with a batch number greater than `lastVerifiedBatch` have not been verified yet.
    /// @dev This is crucial for ensuring that only valid and confirmed batches are processed in the contract.
    uint256 public lastVerifiedBatch;

    /// @notice The latest committed batch number.
    /// @dev This variable holds the batch number of the most recently committed batch.
    /// @dev All batches with a batch number less than or equal to `lastCommittedBatch` are considered committed.
    /// @dev Batches with a block number greater than `lastCommittedBatch` have not been committed yet.
    /// @dev This is crucial for ensuring that only subsequents batches are committed in the contract.
    uint256 public lastCommittedBatch;

    /// @dev Deprecated variable. This is managed inside the Timelock.
    mapping(address _authorizedAddress => bool)
        public authorizedSequencerAddresses;

    address public BRIDGE;
    /// @dev Deprecated variable.
    address public PICO_VERIFIER_ADDRESS;
    address public RISC0_VERIFIER_ADDRESS;
    address public SP1_VERIFIER_ADDRESS;

    /// @dev Deprecated variable.
    bytes32 public SP1_VERIFICATION_KEY;

    /// @notice Indicates whether the contract operates in validium mode.
    /// @dev This value is immutable and can only be set during contract deployment.
    bool public VALIDIUM;

    address public TDX_VERIFIER_ADDRESS;

    /// @notice The address of the AlignedProofAggregatorService contract.
    /// @dev This address is set during contract initialization and is used to verify aligned proofs.
    address public ALIGNEDPROOFAGGREGATOR;

    /// @dev Deprecated variable.
    bytes32 public RISC0_VERIFICATION_KEY;

    /// @notice Chain ID of the network
    uint256 public CHAIN_ID;

    /// @notice True if a Risc0 proof is required for batch verification.
    bool public REQUIRE_RISC0_PROOF;
    /// @notice True if a SP1 proof is required for batch verification.
    bool public REQUIRE_SP1_PROOF;
    /// @notice True if a TDX proof is required for batch verification.
    bool public REQUIRE_TDX_PROOF;

    /// @notice True if verification is done through Aligned Layer instead of smart contract verifiers.
    bool public ALIGNED_MODE;

    /// @notice Verification keys keyed by git commit hash (keccak of the commit SHA string) and verifier type.
    mapping(bytes32 commitHash => mapping(uint8 verifierId => bytes32 vk))
        public verificationKeys;

    /// @notice Initializes the contract.
    /// @dev This method is called only once after the contract is deployed.
    /// @dev The owner is expected to be the Timelock contract.
    /// @dev It sets the bridge address.
    /// @param timelock_owner the Timelock address that can perform upgrades.
    /// @param alignedProofAggregator the address of the alignedProofAggregatorService contract.
    /// @param r0verifier the address of the risc0 groth16 verifier.
    /// @param sp1verifier the address of the sp1 groth16 verifier.
    function initialize(
        bool _validium,
        address timelock_owner,
        bool requireRisc0Proof,
        bool requireSp1Proof,
        bool requireTdxProof,
        bool aligned,
        address r0verifier,
        address sp1verifier,
        address tdxverifier,
        address alignedProofAggregator,
        bytes32 sp1Vk,
        bytes32 risc0Vk,
        bytes32 commitHash,
        bytes32 genesisStateRoot,
        uint256 chainId,
        address bridge
    ) public initializer {
        VALIDIUM = _validium;

        REQUIRE_RISC0_PROOF = requireRisc0Proof;
        REQUIRE_SP1_PROOF = requireSp1Proof;
        REQUIRE_TDX_PROOF = requireTdxProof;

        require(
            !REQUIRE_RISC0_PROOF || r0verifier != address(0),
            "OnChainProposer: missing RISC0 verifier address"
        );
        RISC0_VERIFIER_ADDRESS = r0verifier;
        // In Aligned mode, SP1 proofs are verified through the AlignedProofAggregator,
        // not through a direct SP1 verifier contract, so we don't require sp1verifier.
        require(
            !REQUIRE_SP1_PROOF || aligned || sp1verifier != address(0),
            "OnChainProposer: missing SP1 verifier address"
        );
        SP1_VERIFIER_ADDRESS = sp1verifier;
        require(
            !REQUIRE_TDX_PROOF || tdxverifier != address(0),
            "OnChainProposer: missing TDX verifier address"
        );
        TDX_VERIFIER_ADDRESS = tdxverifier;

        ALIGNED_MODE = aligned;
        ALIGNEDPROOFAGGREGATOR = alignedProofAggregator;

        // Aligned mode requires SP1 proofs to be enabled
        require(
            !aligned || requireSp1Proof,
            "OnChainProposer: Aligned mode requires SP1 proof"
        );
        // Aligned mode does not support RISC0 proofs (not yet available in aggregation mode)
        require(
            !aligned || !requireRisc0Proof,
            "OnChainProposer: Aligned mode does not support RISC0 proof"
        );

        require(
            commitHash != bytes32(0),
            "OnChainProposer: commit hash is zero"
        );
        require(
            !REQUIRE_SP1_PROOF || sp1Vk != bytes32(0),
            "OnChainProposer: missing SP1 verification key"
        );
        require(
            !REQUIRE_RISC0_PROOF || risc0Vk != bytes32(0),
            "OnChainProposer: missing RISC0 verification key"
        );
        verificationKeys[commitHash][SP1_VERIFIER_ID] = sp1Vk;
        verificationKeys[commitHash][RISC0_VERIFIER_ID] = risc0Vk;

        BatchCommitmentInfo storage commitment = batchCommitments[0];
        commitment.newStateRoot = genesisStateRoot;
        commitment.blobKZGVersionedHash = bytes32(0);
        commitment.processedPrivilegedTransactionsRollingHash = bytes32(0);
        commitment.withdrawalsLogsMerkleRoot = bytes32(0);
        commitment.lastBlockHash = bytes32(0);
        commitment.nonPrivilegedTransactions = 0;
        commitment.balanceDiffs = new ICommonBridge.BalanceDiff[](0);
        commitment.commitHash = commitHash;
        commitment
            .l2InMessageRollingHashes = new ICommonBridge.L2MessageRollingHash[](
            0
        );

        CHAIN_ID = chainId;

        require(
            bridge != address(0),
            "001" // OnChainProposer: bridge is the zero address
        );
        require(
            bridge != address(this),
            "000" // OnChainProposer: bridge is the contract address
        );
        BRIDGE = bridge;

        OwnableUpgradeable.__Ownable_init(timelock_owner);
    }

    /// @inheritdoc IOnChainProposer
    function upgradeSP1VerificationKey(
        bytes32 commit_hash,
        bytes32 new_vk
    ) public onlyOwner {
        require(
            commit_hash != bytes32(0),
            "OnChainProposer: commit hash is zero"
        );
        // we don't want to restrict setting the vk to zero
        // as we may want to disable the version
        verificationKeys[commit_hash][SP1_VERIFIER_ID] = new_vk;
        emit VerificationKeyUpgraded("SP1", commit_hash, new_vk);
    }

    /// @inheritdoc IOnChainProposer
    function upgradeRISC0VerificationKey(
        bytes32 commit_hash,
        bytes32 new_vk
    ) public onlyOwner {
        require(
            commit_hash != bytes32(0),
            "OnChainProposer: commit hash is zero"
        );
        // we don't want to restrict setting the vk to zero
        // as we may want to disable the version
        verificationKeys[commit_hash][RISC0_VERIFIER_ID] = new_vk;
        emit VerificationKeyUpgraded("RISC0", commit_hash, new_vk);
    }

    /// @inheritdoc IOnChainProposer
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
    ) external override onlyOwner whenNotPaused {
        // TODO: Refactor validation
        require(
            batchNumber == lastCommittedBatch + 1,
            "002" // OnChainProposer: batchNumber is not the immediate successor of lastCommittedBatch
        );
        require(
            batchCommitments[batchNumber].newStateRoot == bytes32(0),
            "003" // OnChainProposer: tried to commit an already committed batch
        );
        require(
            lastBlockHash != bytes32(0),
            "004" // OnChainProposer: lastBlockHash cannot be zero
        );

        if (processedPrivilegedTransactionsRollingHash != bytes32(0)) {
            bytes32 claimedProcessedTransactions = ICommonBridge(BRIDGE)
                .getPendingTransactionsVersionedHash(
                    uint16(bytes2(processedPrivilegedTransactionsRollingHash))
                );
            require(
                claimedProcessedTransactions ==
                    processedPrivilegedTransactionsRollingHash,
                "005" // OnChainProposer: invalid privileged transaction logs
            );
        }

        for (uint256 i = 0; i < l2MessageRollingHashes.length; i++) {
            bytes32 receivedRollingHash = l2MessageRollingHashes[i].rollingHash;
            bytes32 expectedRollingHash = ICommonBridge(BRIDGE)
                .getPendingL2MessagesVersionedHash(
                    l2MessageRollingHashes[i].chainId,
                    uint16(bytes2(receivedRollingHash))
                );
            require(
                expectedRollingHash == receivedRollingHash,
                "012" // OnChainProposer: invalid L2 message rolling hash
            );
        }

        if (withdrawalsLogsMerkleRoot != bytes32(0)) {
            ICommonBridge(BRIDGE).publishWithdrawals(
                batchNumber,
                withdrawalsLogsMerkleRoot
            );
        }

        // Blob is published in the (EIP-4844) transaction that calls this function.
        bytes32 blobVersionedHash = blobhash(0);
        if (VALIDIUM) {
            require(
                blobVersionedHash == 0,
                "006" // L2 running as validium but blob was published
            );
        } else {
            require(
                blobVersionedHash != 0,
                "007" // L2 running as rollup but blob was not published
            );
        }

        // Validate commit hash and corresponding verification keys are valid
        require(commitHash != bytes32(0), "012");
        if (
            REQUIRE_SP1_PROOF &&
            verificationKeys[commitHash][SP1_VERIFIER_ID] == bytes32(0)
        ) {
            revert("013"); // missing verification key for commit hash
        } else if (
            REQUIRE_RISC0_PROOF &&
            verificationKeys[commitHash][RISC0_VERIFIER_ID] == bytes32(0)
        ) {
            revert("013"); // missing verification key for commit hash
        }

        batchCommitments[batchNumber] = BatchCommitmentInfo(
            newStateRoot,
            blobVersionedHash,
            processedPrivilegedTransactionsRollingHash,
            withdrawalsLogsMerkleRoot,
            lastBlockHash,
            nonPrivilegedTransactions,
            balanceDiffs,
            commitHash,
            l2MessageRollingHashes
        );
        emit BatchCommitted(newStateRoot);

        lastCommittedBatch = batchNumber;
    }

    /// @notice Internal batch verification logic used by verifyBatches.
    function _verifyBatchInternal(
        uint256 batchNumber,
        bytes memory risc0BlockProof,
        bytes memory sp1ProofBytes,
        bytes memory tdxSignature
    ) internal {
        require(
            batchNumber == lastVerifiedBatch + 1,
            "009" // OnChainProposer: batch already verified
        );
        require(
            batchCommitments[batchNumber].newStateRoot != bytes32(0),
            "00a" // OnChainProposer: cannot verify an uncommitted batch
        );

        // The first 2 bytes are the number of privileged transactions.
        uint16 privileged_transaction_count = uint16(
            bytes2(
                batchCommitments[batchNumber]
                    .processedPrivilegedTransactionsRollingHash
            )
        );
        if (privileged_transaction_count > 0) {
            ICommonBridge(BRIDGE).removePendingTransactionHashes(
                privileged_transaction_count
            );
        }

        ICommonBridge.L2MessageRollingHash[]
            memory batchL2InRollingHashes = batchCommitments[batchNumber]
                .l2InMessageRollingHashes;
        for (uint256 i = 0; i < batchL2InRollingHashes.length; i++) {
            uint16 l2_messages_count = uint16(
                bytes2(batchL2InRollingHashes[i].rollingHash)
            );
            ICommonBridge(BRIDGE).removePendingL2Messages(
                batchL2InRollingHashes[i].chainId,
                l2_messages_count
            );
        }

        if (
            ICommonBridge(BRIDGE).hasExpiredPrivilegedTransactions() &&
            batchCommitments[batchNumber].nonPrivilegedTransactions != 0
        ) {
            revert("00v"); // exceeded privileged transaction inclusion deadline, can't include non-privileged transactions
        }

        // Reconstruct public inputs from commitments
        // MUST be BEFORE updating lastVerifiedBatch
        bytes memory publicInputs = _getPublicInputsFromCommitment(batchNumber);

        if (REQUIRE_RISC0_PROOF) {
            bytes32 batchCommitHash = batchCommitments[batchNumber].commitHash;
            bytes32 risc0Vk = verificationKeys[batchCommitHash][
                RISC0_VERIFIER_ID
            ];
            try
                IRiscZeroVerifier(RISC0_VERIFIER_ADDRESS).verify(
                    risc0BlockProof,
                    // we use the same vk as the one set for the commit of the batch
                    risc0Vk,
                    sha256(publicInputs)
                )
            {} catch {
                revert(
                    "00c" // OnChainProposer: Invalid RISC0 proof failed proof verification
                );
            }
        }

        if (REQUIRE_SP1_PROOF) {
            bytes32 batchCommitHash = batchCommitments[batchNumber].commitHash;
            bytes32 sp1Vk = verificationKeys[batchCommitHash][SP1_VERIFIER_ID];
            try
                ISP1Verifier(SP1_VERIFIER_ADDRESS).verifyProof(
                    sp1Vk,
                    publicInputs,
                    sp1ProofBytes
                )
            {} catch {
                revert(
                    "00e" // OnChainProposer: Invalid SP1 proof failed proof verification
                );
            }
        }

        if (REQUIRE_TDX_PROOF) {
            try
                ITDXVerifier(TDX_VERIFIER_ADDRESS).verify(
                    publicInputs,
                    tdxSignature
                )
            {} catch {
                revert(
                    "00g" // OnChainProposer: Invalid TDX proof failed proof verification
                );
            }
        }

        ICommonBridge(BRIDGE).publishL2Messages(
            batchCommitments[batchNumber].balanceDiffs
        );

        // MUST be AFTER _getPublicInputsFromCommitment
        lastVerifiedBatch = batchNumber;

        // Remove previous batch commitment as it is no longer needed.
        delete batchCommitments[batchNumber - 1];
    }

    /// @inheritdoc IOnChainProposer
    function verifyBatches(
        uint256 firstBatchNumber,
        bytes[] memory risc0BlockProofs,
        bytes[] memory sp1ProofsBytes,
        bytes[] memory tdxSignatures
    ) external onlyOwner whenNotPaused {
        require(
            !ALIGNED_MODE,
            "008" // Batch verification should be done via Aligned Layer. Call verifyBatchesAligned() instead.
        );
        uint256 batchCount = risc0BlockProofs.length;
        require(batchCount > 0, "OnChainProposer: empty batch array");
        require(
            sp1ProofsBytes.length == batchCount && tdxSignatures.length == batchCount,
            "OnChainProposer: array length mismatch"
        );
        require(firstBatchNumber == lastVerifiedBatch + 1, "009");
        for (uint256 i = 0; i < batchCount; i++) {
            _verifyBatchInternal(
                firstBatchNumber + i,
                risc0BlockProofs[i],
                sp1ProofsBytes[i],
                tdxSignatures[i]
            );
        }
        emit BatchVerified(lastVerifiedBatch);
    }

    /// @inheritdoc IOnChainProposer
    function verifyBatchesAligned(
        uint256 firstBatchNumber,
        uint256 lastBatchNumber,
        bytes32[][] calldata sp1MerkleProofsList,
        bytes32[][] calldata risc0MerkleProofsList
    ) external override onlyOwner whenNotPaused {
        require(
            ALIGNED_MODE,
            "00h" // Batch verification should be done via smart contract verifiers. Call verifyBatches() instead.
        );
        require(
            firstBatchNumber == lastVerifiedBatch + 1,
            "00i" // OnChainProposer: incorrect first batch number
        );
        require(
            lastBatchNumber <= lastCommittedBatch,
            "014" // OnChainProposer: last batch number exceeds last committed batch"
        );

        uint256 batchesToVerify = (lastBatchNumber - firstBatchNumber) + 1;

        if (REQUIRE_SP1_PROOF) {
            require(
                batchesToVerify == sp1MerkleProofsList.length,
                "00j" // OnChainProposer: SP1 input/proof array length mismatch
            );
        }
        if (REQUIRE_RISC0_PROOF) {
            require(
                batchesToVerify == risc0MerkleProofsList.length,
                "00k" // OnChainProposer: Risc0 input/proof array length mismatch
            );
        }

        uint256 batchNumber = firstBatchNumber;

        for (uint256 i = 0; i < batchesToVerify; i++) {
            require(
                batchCommitments[batchNumber].newStateRoot != bytes32(0),
                "00l" // OnChainProposer: cannot verify an uncommitted batch
            );

            // The first 2 bytes are the number of transactions.
            uint16 privileged_transaction_count = uint16(
                bytes2(
                    batchCommitments[batchNumber]
                        .processedPrivilegedTransactionsRollingHash
                )
            );
            if (privileged_transaction_count > 0) {
                ICommonBridge(BRIDGE).removePendingTransactionHashes(
                    privileged_transaction_count
                );
            }

            ICommonBridge.L2MessageRollingHash[]
                memory batchL2InRollingHashes = batchCommitments[batchNumber]
                    .l2InMessageRollingHashes;
            for (uint256 j = 0; j < batchL2InRollingHashes.length; j++) {
                uint16 l2_messages_count = uint16(
                    bytes2(batchL2InRollingHashes[j].rollingHash)
                );
                ICommonBridge(BRIDGE).removePendingL2Messages(
                    batchL2InRollingHashes[j].chainId,
                    l2_messages_count
                );
            }

            // Reconstruct public inputs from commitments
            bytes memory publicInputs = _getPublicInputsFromCommitment(
                batchNumber
            );

            if (REQUIRE_SP1_PROOF) {
                _verifyProofInclusionAligned(
                    sp1MerkleProofsList[i],
                    ALIGNED_SP1_PROVING_SYSTEM_ID,
                    verificationKeys[batchCommitments[batchNumber].commitHash][
                        SP1_VERIFIER_ID
                    ],
                    publicInputs
                );
            }

            // NOTE: This block is currently unreachable because initialize() prevents
            // aligned mode with RISC0 enabled. It is kept for future compatibility when
            // Aligned re-enables RISC0 support - at that point, update the proving system ID.
            if (REQUIRE_RISC0_PROOF) {
                _verifyProofInclusionAligned(
                    risc0MerkleProofsList[i],
                    0, // Placeholder - RISC0 proving system ID TBD
                    verificationKeys[batchCommitments[batchNumber].commitHash][
                        RISC0_VERIFIER_ID
                    ],
                    publicInputs
                );
            }

            ICommonBridge(BRIDGE).publishL2Messages(
                batchCommitments[batchNumber].balanceDiffs
            );

            // Remove previous batch commitment
            delete batchCommitments[batchNumber - 1];

            lastVerifiedBatch = batchNumber;
            batchNumber++;
        }

        emit BatchVerified(lastVerifiedBatch);
    }

    function _verifyProofInclusionAligned(
        bytes32[] calldata merkleProofsList,
        uint16 provingSystemId,
        bytes32 verificationKey,
        bytes memory publicInputsList
    ) internal view {
        bytes memory callData = abi.encodeWithSignature(
            "isProofVerified(bytes32[],uint16,bytes32,bytes)",
            merkleProofsList,
            provingSystemId,
            verificationKey,
            publicInputsList
        );
        (bool callResult, bytes memory response) = ALIGNEDPROOFAGGREGATOR
            .staticcall(callData);
        require(
            callResult,
            "00y" // OnChainProposer: call to ALIGNEDPROOFAGGREGATOR failed
        );
        bool proofVerified = abi.decode(response, (bool));
        require(
            proofVerified,
            "00z" // OnChainProposer: Aligned proof verification failed
        );
    }

    /// @notice Constructs public inputs from committed batch data for proof verification.
    /// @dev Public inputs structure:
    /// Fixed-size fields (256 bytes):
    /// - bytes 0-32: Initial state root (from the last verified batch)
    /// - bytes 32-64: Final state root (from the current batch)
    /// - bytes 64-96: Withdrawals merkle root (from the current batch)
    /// - bytes 96-128: Processed L1 messages rolling hash (from the current batch)
    /// - bytes 128-160: Blob versioned hash (from the current batch)
    /// - bytes 160-192: Last block hash (from the current batch)
    /// - bytes 192-224: Chain ID
    /// - bytes 224-256: Non-privileged transactions count (from the current batch)
    /// Variable-size fields:
    /// - For each targeted chain in balance diffs:
    ///   - bytes: Chain ID (32 bytes)
    ///   - bytes: Value (32 bytes)
    ///   - For each asset diff in the targeted chain:
    ///     - bytes: Token L1 address (20 bytes)
    ///     - bytes: Token L2 address (20 bytes)
    ///     - bytes: Destination Token L2 address (20 bytes)
    ///     - bytes: Value (32 bytes)
    ///   - For each message hash in the targeted chain:
    ///     - bytes: Message hash (32 bytes)
    /// - For each L2 in message rolling hash:
    ///   - bytes: Chain ID (32 bytes)
    ///   - bytes: Rolling hash (32 bytes)
    /// @param batchNumber The batch number for which to construct public inputs.
    /// @return publicInputs The constructed public inputs as a byte array.
    function _getPublicInputsFromCommitment(
        uint256 batchNumber
    ) internal view returns (bytes memory) {
        BatchCommitmentInfo memory currentBatch = batchCommitments[batchNumber];

        // Fixed-size fields (256 bytes)
        bytes memory publicInputs = abi.encodePacked(
            batchCommitments[lastVerifiedBatch].newStateRoot,
            currentBatch.newStateRoot,
            currentBatch.withdrawalsLogsMerkleRoot,
            currentBatch.processedPrivilegedTransactionsRollingHash,
            currentBatch.blobKZGVersionedHash,
            currentBatch.lastBlockHash,
            bytes32(CHAIN_ID),
            bytes32(currentBatch.nonPrivilegedTransactions)
        );

        // Variable-size fields: balance diffs
        for (uint256 i = 0; i < currentBatch.balanceDiffs.length; i++) {
            ICommonBridge.BalanceDiff memory bd = currentBatch.balanceDiffs[i];

            publicInputs = abi.encodePacked(
                publicInputs,
                bytes32(bd.chainId),
                bytes32(bd.value)
            );

            for (uint256 j = 0; j < bd.assetDiffs.length; j++) {
                ICommonBridge.AssetDiff memory ad = bd.assetDiffs[j];
                publicInputs = abi.encodePacked(
                    publicInputs,
                    ad.tokenL1,
                    ad.tokenL2,
                    ad.destTokenL2,
                    bytes32(ad.value)
                );
            }

            for (uint256 j = 0; j < bd.message_hashes.length; j++) {
                publicInputs = abi.encodePacked(
                    publicInputs,
                    bd.message_hashes[j]
                );
            }
        }

        // Variable-size fields: L2 in message rolling hashes
        for (
            uint256 k = 0;
            k < currentBatch.l2InMessageRollingHashes.length;
            k++
        ) {
            ICommonBridge.L2MessageRollingHash memory rh = currentBatch
                .l2InMessageRollingHashes[k];
            publicInputs = abi.encodePacked(
                publicInputs,
                bytes32(rh.chainId),
                rh.rollingHash
            );
        }

        return publicInputs;
    }

    /// @inheritdoc IOnChainProposer
    function revertBatch(
        uint256 batchNumber
    ) external override onlyOwner whenPaused {
        require(
            batchNumber > lastVerifiedBatch,
            "010" // OnChainProposer: can't revert verified batch
        );
        require(
            batchNumber <= lastCommittedBatch,
            "011" // OnChainProposer: no batches are being reverted
        );

        // Remove batch commitments from batchNumber to lastCommittedBatch
        for (uint256 i = batchNumber; i <= lastCommittedBatch; i++) {
            delete batchCommitments[i];
        }

        lastCommittedBatch = batchNumber - 1;

        emit BatchReverted(batchCommitments[lastCommittedBatch].newStateRoot);
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}

    /// @inheritdoc IOnChainProposer
    function pause() external override onlyOwner {
        _pause();
    }

    /// @inheritdoc IOnChainProposer
    function unpause() external override onlyOwner {
        _unpause();
    }
}
