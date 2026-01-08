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

        RISC0_VERIFIER_ADDRESS = r0verifier;
        SP1_VERIFIER_ADDRESS = sp1verifier;
        TDX_VERIFIER_ADDRESS = tdxverifier;

        ALIGNED_MODE = aligned;
        ALIGNEDPROOFAGGREGATOR = alignedProofAggregator;

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

    /// @inheritdoc IOnChainProposer
    /// @notice The first `require` checks that the batch number is the subsequent block.
    /// @notice The second `require` checks if the batch has been committed.
    /// @notice The order of these `require` statements is important.
    /// Ordering Reason: After the verification process, we delete the `batchCommitments` for `batchNumber - 1`. This means that when checking the batch,
    /// we might get an error indicating that the batch hasn’t been committed, even though it was committed but deleted. Therefore, it has already been verified.
    function verifyBatch(
        uint256 batchNumber,
        //risc0
        bytes memory risc0BlockProof,
        bytes calldata risc0Journal,
        //sp1
        bytes calldata sp1PublicValues,
        bytes memory sp1ProofBytes,
        //tdx
        bytes calldata tdxPublicValues,
        bytes memory tdxSignature
    ) external override onlyOwner whenNotPaused {
        require(
            !ALIGNED_MODE,
            "008" // Batch verification should be done via Aligned Layer. Call verifyBatchesAligned() instead.
        );

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

        if (REQUIRE_RISC0_PROOF) {
            // If the verification fails, it will revert.
            string memory reason = _verifyPublicData(batchNumber, risc0Journal);
            if (bytes(reason).length != 0) {
                revert(
                    string.concat(
                        "00b", // OnChainProposer: Invalid RISC0 proof:
                        reason
                    )
                );
            }
            bytes32 batchCommitHash = batchCommitments[batchNumber].commitHash;
            bytes32 risc0Vk = verificationKeys[batchCommitHash][
                RISC0_VERIFIER_ID
            ];
            try
                IRiscZeroVerifier(RISC0_VERIFIER_ADDRESS).verify(
                    risc0BlockProof,
                    // we use the same vk as the one set for the commit of the batch
                    risc0Vk,
                    sha256(risc0Journal)
                )
            {} catch {
                revert(
                    "00c" // OnChainProposer: Invalid RISC0 proof failed proof verification
                );
            }
        }

        if (REQUIRE_SP1_PROOF) {
            // If the verification fails, it will revert.
            string memory reason = _verifyPublicData(
                batchNumber,
                sp1PublicValues
            );
            if (bytes(reason).length != 0) {
                revert(
                    string.concat(
                        "00d", // OnChainProposer: Invalid SP1 proof:
                        reason
                    )
                );
            }
            bytes32 batchCommitHash = batchCommitments[batchNumber].commitHash;
            bytes32 sp1Vk = verificationKeys[batchCommitHash][SP1_VERIFIER_ID];
            try
                ISP1Verifier(SP1_VERIFIER_ADDRESS).verifyProof(
                    sp1Vk,
                    sp1PublicValues,
                    sp1ProofBytes
                )
            {} catch {
                revert(
                    "00e" // OnChainProposer: Invalid SP1 proof failed proof verification
                );
            }
        }

        if (REQUIRE_TDX_PROOF) {
            // If the verification fails, it will revert.
            string memory reason = _verifyPublicData(
                batchNumber,
                tdxPublicValues
            );
            if (bytes(reason).length != 0) {
                revert(
                    string.concat(
                        "00f", // OnChainProposer: Invalid TDX proof:
                        reason
                    )
                );
            }
            try
                ITDXVerifier(TDX_VERIFIER_ADDRESS).verify(
                    tdxPublicValues,
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

        lastVerifiedBatch = batchNumber;

        // Remove previous batch commitment as it is no longer needed.
        delete batchCommitments[batchNumber - 1];

        emit BatchVerified(lastVerifiedBatch);
    }

    /// @inheritdoc IOnChainProposer
    function verifyBatchesAligned(
        uint256 firstBatchNumber,
        bytes[] calldata publicInputsList,
        bytes32[][] calldata sp1MerkleProofsList,
        bytes32[][] calldata risc0MerkleProofsList
    ) external override onlyOwner whenNotPaused {
        require(
            ALIGNED_MODE,
            "00h" // Batch verification should be done via smart contract verifiers. Call verifyBatch() instead.
        );
        require(
            firstBatchNumber == lastVerifiedBatch + 1,
            "00i" // OnChainProposer: incorrect first batch number
        );

        if (REQUIRE_SP1_PROOF) {
            require(
                publicInputsList.length == sp1MerkleProofsList.length,
                "00j" // OnChainProposer: SP1 input/proof array length mismatch
            );
        }
        if (REQUIRE_RISC0_PROOF) {
            require(
                publicInputsList.length == risc0MerkleProofsList.length,
                "00k" // OnChainProposer: Risc0 input/proof array length mismatch
            );
        }

        uint256 batchNumber = firstBatchNumber;

        for (uint256 i = 0; i < publicInputsList.length; i++) {
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

            // Verify public data for the batch
            string memory reason = _verifyPublicData(
                batchNumber,
                publicInputsList[i]
            );
            if (bytes(reason).length != 0) {
                revert(
                    string.concat(
                        "00m", // OnChainProposer: Invalid ALIGNED proof:
                        reason
                    )
                );
            }

            if (REQUIRE_SP1_PROOF) {
                _verifyProofInclusionAligned(
                    sp1MerkleProofsList[i],
                    verificationKeys[batchCommitments[batchNumber].commitHash][
                        SP1_VERIFIER_ID
                    ],
                    publicInputsList[i]
                );
            }

            if (REQUIRE_RISC0_PROOF) {
                _verifyProofInclusionAligned(
                    risc0MerkleProofsList[i],
                    verificationKeys[batchCommitments[batchNumber].commitHash][
                        RISC0_VERIFIER_ID
                    ],
                    publicInputsList[i]
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

    function _verifyPublicData(
        uint256 batchNumber,
        bytes calldata publicData
    ) internal view returns (string memory) {
        ICommonBridge.BalanceDiff[] memory balanceDiffs = batchCommitments[
            batchNumber
        ].balanceDiffs;
        uint256 targetedChainsCount = balanceDiffs.length;
        uint256 expected_length = 256;
        for (uint256 i = 0; i < targetedChainsCount; i++) {
            expected_length += 32;
            expected_length += 32;
            expected_length += balanceDiffs[i].assetDiffs.length * 92;
            expected_length += balanceDiffs[i].message_hashes.length * 32;
        }
        expected_length +=
            batchCommitments[batchNumber].l2InMessageRollingHashes.length *
            64;
        if (publicData.length != expected_length) {
            return "00n"; // invalid public data length
        }
        bytes32 initialStateRoot = bytes32(publicData[0:32]);
        if (
            batchCommitments[lastVerifiedBatch].newStateRoot != initialStateRoot
        ) {
            return "00o"; // initial state root public inputs don't match with initial state root
        }
        bytes32 finalStateRoot = bytes32(publicData[32:64]);
        if (batchCommitments[batchNumber].newStateRoot != finalStateRoot) {
            return "00p"; // final state root public inputs don't match with final state root
        }
        bytes32 withdrawalsMerkleRoot = bytes32(publicData[64:96]);
        if (
            batchCommitments[batchNumber].withdrawalsLogsMerkleRoot !=
            withdrawalsMerkleRoot
        ) {
            return "00q"; // withdrawals public inputs don't match with committed withdrawals
        }
        bytes32 privilegedTransactionsHash = bytes32(publicData[96:128]);
        if (
            batchCommitments[batchNumber]
                .processedPrivilegedTransactionsRollingHash !=
            privilegedTransactionsHash
        ) {
            return "00r"; // privileged transactions hash public input does not match with committed transactions
        }
        bytes32 blobVersionedHash = bytes32(publicData[128:160]);
        if (
            batchCommitments[batchNumber].blobKZGVersionedHash !=
            blobVersionedHash
        ) {
            return "00s"; // blob versioned hash public input does not match with committed hash
        }
        bytes32 lastBlockHash = bytes32(publicData[160:192]);
        if (batchCommitments[batchNumber].lastBlockHash != lastBlockHash) {
            return "00t"; // last block hash public inputs don't match with last block hash
        }
        uint256 chainId = uint256(bytes32(publicData[192:224]));
        if (chainId != CHAIN_ID) {
            return ("00u"); // given chain id does not correspond to this network
        }
        uint256 nonPrivilegedTransactions = uint256(
            bytes32(publicData[224:256])
        );
        if (
            batchCommitments[batchNumber].nonPrivilegedTransactions !=
            nonPrivilegedTransactions
        ) {
            return "00w"; // non-privileged transactions public input does not match with committed value
        }

        uint256 offset = 256;
        for (uint256 i = 0; i < targetedChainsCount; i++) {
            uint256 verifiedChainId = uint256(
                bytes32(publicData[offset:offset + 32])
            );
            offset += 32;

            if (balanceDiffs[i].chainId != verifiedChainId) {
                return "00x"; // balance diffs public inputs don't match with committed balance diffs
            }

            uint256 verifiedValue = uint256(
                bytes32(publicData[offset:offset + 32])
            );
            offset += 32;

            if (balanceDiffs[i].value != verifiedValue) {
                return "015"; // balance diffs public inputs don't match with committed balance diffs
            }

            for (uint256 j = 0; j < balanceDiffs[i].assetDiffs.length; j++) {
                (
                    address tokenL1,
                    address tokenL2,
                    address destTokenL2,
                    uint256 tokenValue
                ) = (
                        address(bytes20(publicData[offset:offset + 20])),
                        address(bytes20(publicData[offset + 20:offset + 40])),
                        address(bytes20(publicData[offset + 40:offset + 60])),
                        uint256(bytes32(publicData[offset + 60:offset + 92]))
                    );

                offset += 92;

                if (
                    tokenL1 != balanceDiffs[i].assetDiffs[j].tokenL1 ||
                    tokenL2 != balanceDiffs[i].assetDiffs[j].tokenL2 ||
                    destTokenL2 != balanceDiffs[i].assetDiffs[j].destTokenL2 ||
                    tokenValue != balanceDiffs[i].assetDiffs[j].value
                ) {
                    return "014"; // balance diffs public inputs don't match with committed balance diffs
                }
            }

            bytes32[] memory messageHashes = balanceDiffs[i].message_hashes;
            for (uint256 j = 0; j < messageHashes.length; j++) {
                bytes32 verifiedMessageHash = bytes32(
                    publicData[offset:offset + 32]
                );
                offset += 32;
                if (messageHashes[j] != verifiedMessageHash) {
                    return "00y"; // message hash public inputs don't match with committed message hashes
                }
            }
        }
        uint256 batchL2RollingHashesCount = batchCommitments[batchNumber]
            .l2InMessageRollingHashes
            .length;
        for (uint256 k = 0; k < batchL2RollingHashesCount; k++) {
            uint256 verifiedChainId = uint256(
                bytes32(publicData[offset:offset + 32])
            );
            bytes32 verifiedRollingHash = bytes32(
                publicData[offset + 32:offset + 64]
            );
            ICommonBridge.L2MessageRollingHash
                memory committedRollingHash = batchCommitments[batchNumber]
                    .l2InMessageRollingHashes[k];
            if (
                committedRollingHash.chainId != verifiedChainId ||
                committedRollingHash.rollingHash != verifiedRollingHash
            ) {
                return "00z"; // L2 in message rolling hash public inputs don't match with committed L2 in message rolling hashes
            }
            offset += 64;
        }

        return "";
    }

    function _verifyProofInclusionAligned(
        bytes32[] calldata merkleProofsList,
        bytes32 verificationKey,
        bytes calldata publicInputsList
    ) internal view {
        bytes memory callData = abi.encodeWithSignature(
            "verifyProofInclusion(bytes32[],bytes32,bytes)",
            merkleProofsList,
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
