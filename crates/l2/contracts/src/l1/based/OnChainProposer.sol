// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "./interfaces/IOnChainProposer.sol";
import {CommonBridge} from "../CommonBridge.sol";
import {ICommonBridge} from "../interfaces/ICommonBridge.sol";
import {IRiscZeroVerifier} from "../interfaces/IRiscZeroVerifier.sol";
import {ISP1Verifier} from "../interfaces/ISP1Verifier.sol";
import {ITDXVerifier} from "../interfaces/ITDXVerifier.sol";
import {ISequencerRegistry} from "../interfaces/ISequencerRegistry.sol";

/// @title OnChainProposer contract.
/// @author LambdaClass
contract OnChainProposer is
    IOnChainProposer,
    Initializable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable
{
    /// @notice Committed batches data.
    /// @dev This struct holds the information about the committed batches.
    /// @dev processedPrivilegedTransactionsRollingHash is the Merkle root of the hashes of the
    /// privileged transactions that were processed in the batch being committed. The amount of
    /// hashes that are encoded in this root are to be removed from the
    /// pendingTxHashes queue of the CommonBridge contract.
    /// @dev withdrawalsLogsMerkleRoot is the Merkle root of the Merkle tree containing
    /// all the withdrawals that were processed in the batch being committed
    struct BatchCommitmentInfo {
        bytes32 newStateRoot;
        bytes32 blobVersionedHash;
        bytes32 processedPrivilegedTransactionsRollingHash;
        bytes32 withdrawalsLogsMerkleRoot;
        bytes32 lastBlockHash;
    }

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

    address public BRIDGE;
    /// @dev Deprecated variable.
    address public PICO_VERIFIER_ADDRESS;
    address public RISC0_VERIFIER_ADDRESS;
    address public SP1_VERIFIER_ADDRESS;
    address public TDX_VERIFIER_ADDRESS;
    address public SEQUENCER_REGISTRY;

    bytes32 public SP1_VERIFICATION_KEY;

    /// @notice Indicates whether the contract operates in validium mode.
    /// @dev This value is immutable and can only be set during contract deployment.
    bool public VALIDIUM;

    /// @notice The address of the AlignedProofAggregatorService contract.
    /// @dev This address is set during contract initialization and is used to verify aligned proofs.
    address public ALIGNEDPROOFAGGREGATOR;

    bytes32 public RISC0_VERIFICATION_KEY;

    /// @notice True if a Risc0 proof is required for batch verification.
    bool public REQUIRE_RISC0_PROOF;
    /// @notice True if a SP1 proof is required for batch verification.
    bool public REQUIRE_SP1_PROOF;
    /// @notice True if a TDX proof is required for batch verification.
    bool public REQUIRE_TDX_PROOF;

    /// @notice True if verification is done through Aligned Layer instead of smart contract verifiers.
    bool public ALIGNED_MODE;

    /// @notice Chain ID of the network
    uint256 public CHAIN_ID;

    modifier onlyLeaderSequencer() {
        require(
            msg.sender ==
                ISequencerRegistry(SEQUENCER_REGISTRY).leaderSequencer(),
            "OnChainProposer: caller has no sequencing rights"
        );
        _;
    }

    /// @notice Initializes the contract.
    /// @dev This method is called only once after the contract is deployed.
    /// @dev It sets the bridge address.
    /// @param _validium initialize the contract in validium mode.
    /// @param owner the address of the owner who can perform upgrades.
    /// @param alignedProofAggregator the address of the alignedProofAggregatorService contract.
    /// @param r0verifier the address of the risc0 groth16 verifier.
    /// @param sp1verifier the address of the sp1 groth16 verifier.
    function initialize(
        bool _validium,
        address owner,
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
        bytes32 genesisStateRoot,
        address sequencer_registry,
        uint256 chainId
    ) public initializer {
        VALIDIUM = _validium;

        // Risc0 constants
        REQUIRE_RISC0_PROOF = requireRisc0Proof;
        RISC0_VERIFIER_ADDRESS = r0verifier;
        RISC0_VERIFICATION_KEY = risc0Vk;

        // SP1 constants
        REQUIRE_SP1_PROOF = requireSp1Proof;
        SP1_VERIFIER_ADDRESS = sp1verifier;
        SP1_VERIFICATION_KEY = sp1Vk;
        RISC0_VERIFICATION_KEY = risc0Vk;

        // TDX constants
        REQUIRE_TDX_PROOF = requireTdxProof;
        TDX_VERIFIER_ADDRESS = tdxverifier;

        // Aligned Layer constants
        ALIGNED_MODE = aligned;
        ALIGNEDPROOFAGGREGATOR = alignedProofAggregator;

        batchCommitments[0] = BatchCommitmentInfo(
            genesisStateRoot,
            bytes32(0),
            bytes32(0),
            bytes32(0),
            bytes32(0)
        );

        // Set the SequencerRegistry address
        require(
            SEQUENCER_REGISTRY == address(0),
            "OnChainProposer: contract already initialized"
        );
        require(
            sequencer_registry != address(0),
            "OnChainProposer: sequencer_registry is the zero address"
        );
        require(
            sequencer_registry != address(this),
            "OnChainProposer: sequencer_registry is the contract address"
        );
        SEQUENCER_REGISTRY = sequencer_registry;

        CHAIN_ID = chainId;

        OwnableUpgradeable.__Ownable_init(owner);
    }

    /// @inheritdoc IOnChainProposer
    function initializeBridgeAddress(address bridge) public onlyOwner {
        require(
            BRIDGE == address(0),
            "OnChainProposer: bridge already initialized"
        );
        require(
            bridge != address(0),
            "OnChainProposer: bridge is the zero address"
        );
        require(
            bridge != address(this),
            "OnChainProposer: bridge is the contract address"
        );
        BRIDGE = bridge;
    }

    /// @inheritdoc IOnChainProposer
    function commitBatch(
        uint256 batchNumber,
        bytes32 newStateRoot,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 processedPrivilegedTransactionsRollingHash,
        bytes32 lastBlockHash,
        bytes[] calldata //rlpEncodedBlocks
    ) external override onlyLeaderSequencer {
        // TODO: Refactor validation
        require(
            batchNumber == lastCommittedBatch + 1,
            "OnChainProposer: batchNumber is not the immediate successor of lastCommittedBatch"
        );
        require(
            batchCommitments[batchNumber].newStateRoot == bytes32(0),
            "OnChainProposer: tried to commit an already committed batch"
        );
        require(
            lastBlockHash != bytes32(0),
            "OnChainProposer: lastBlockHash cannot be zero"
        );

        // Check if commitment is equivalent to blob's KZG commitment.

        if (processedPrivilegedTransactionsRollingHash != bytes32(0)) {
            bytes32 claimedProcessedTransactions = ICommonBridge(BRIDGE)
                .getPendingTransactionsVersionedHash(
                    uint16(bytes2(processedPrivilegedTransactionsRollingHash))
                );
            require(
                claimedProcessedTransactions ==
                    processedPrivilegedTransactionsRollingHash,
                "OnChainProposer: invalid privileged transactions log"
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
                "L2 running as validium but blob was published"
            );
        } else {
            require(
                blobVersionedHash != 0,
                "L2 running as rollup but blob was not published"
            );
        }

        batchCommitments[batchNumber] = BatchCommitmentInfo(
            newStateRoot,
            blobVersionedHash,
            processedPrivilegedTransactionsRollingHash,
            withdrawalsLogsMerkleRoot,
            lastBlockHash
        );
        emit BatchCommitted(batchNumber, newStateRoot);

        lastCommittedBatch = batchNumber;
        ISequencerRegistry(SEQUENCER_REGISTRY).pushSequencer(
            batchNumber,
            msg.sender
        );
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
    ) external {
        require(!ALIGNED_MODE, "Batch verification should be done via Aligned Layer. Call verifyBatchesAligned() instead.");

        require(
            batchCommitments[batchNumber].newStateRoot != bytes32(0),
            "OnChainProposer: cannot verify an uncommitted batch"
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

        if (REQUIRE_RISC0_PROOF) {
            // If the verification fails, it will revert.
            _verifyPublicData(batchNumber, risc0Journal);
            IRiscZeroVerifier(RISC0_VERIFIER_ADDRESS).verify(
                risc0BlockProof,
                RISC0_VERIFICATION_KEY,
                sha256(risc0Journal)
            );
        }

        if (REQUIRE_SP1_PROOF) {
            // If the verification fails, it will revert.
            _verifyPublicData(batchNumber, sp1PublicValues[16:]);
            ISP1Verifier(SP1_VERIFIER_ADDRESS).verifyProof(
                SP1_VERIFICATION_KEY,
                sp1PublicValues,
                sp1ProofBytes
            );
        }

        if (REQUIRE_TDX_PROOF) {
            // If the verification fails, it will revert.
            _verifyPublicData(batchNumber, tdxPublicValues);
            ITDXVerifier(TDX_VERIFIER_ADDRESS).verify(tdxPublicValues, tdxSignature);
        }

        lastVerifiedBatch = batchNumber;

        // Remove previous batch commitment as it is no longer needed.
        delete batchCommitments[batchNumber - 1];

        emit BatchVerified(lastVerifiedBatch);
    }

    /// @inheritdoc IOnChainProposer
    function verifyBatchesAligned(
        uint256 firstBatchNumber,
        bytes[] calldata alignedPublicInputsList,
        bytes32[][] calldata alignedMerkleProofsList
    ) external override {
        require(ALIGNED_MODE, "Batch verification should be done via smart contract verifiers. Call verifyBatch() instead.");

        require(
            alignedPublicInputsList.length == alignedMerkleProofsList.length,
            "OnChainProposer: input/proof array length mismatch"
        );
        require(
            firstBatchNumber == lastVerifiedBatch + 1,
            "OnChainProposer: incorrect first batch number"
        );

        uint256 batchNumber = firstBatchNumber;

        for (uint256 i = 0; i < alignedPublicInputsList.length; i++) {
            require(
                batchCommitments[batchNumber].newStateRoot != bytes32(0),
                "OnChainProposer: cannot verify an uncommitted batch"
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

            // Verify public data for the batch
            _verifyPublicData(batchNumber, alignedPublicInputsList[i][8:]);

            bytes memory callData = abi.encodeWithSignature(
                "verifyProofInclusion(bytes32[],bytes32,bytes)",
                alignedMerkleProofsList[i],
                SP1_VERIFICATION_KEY,
                alignedPublicInputsList[i]
            );
            (bool callResult, bytes memory response) = ALIGNEDPROOFAGGREGATOR
                .staticcall(callData);
            require(
                callResult,
                "OnChainProposer: call to ALIGNEDPROOFAGGREGATOR failed"
            );
            bool proofVerified = abi.decode(response, (bool));
            require(
                proofVerified,
                "OnChainProposer: Aligned proof verification failed"
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
    ) internal view {
        require(
            publicData.length == 224,
            "OnChainProposer: invalid public data length"
        );
        bytes32 initialStateRoot = bytes32(publicData[0:32]);
        require(
            batchCommitments[lastVerifiedBatch].newStateRoot ==
                initialStateRoot,
            "OnChainProposer: initial state root public inputs don't match with initial state root"
        );
        bytes32 finalStateRoot = bytes32(publicData[32:64]);
        require(
            batchCommitments[batchNumber].newStateRoot == finalStateRoot,
            "OnChainProposer: final state root public inputs don't match with final state root"
        );
        bytes32 withdrawalsMerkleRoot = bytes32(publicData[64:96]);
        require(
            batchCommitments[batchNumber].withdrawalsLogsMerkleRoot ==
                withdrawalsMerkleRoot,
            "OnChainProposer: withdrawals public inputs don't match with committed withdrawals"
        );
        bytes32 privilegedTransactionsHash = bytes32(publicData[96:128]);
        require(
            batchCommitments[batchNumber]
                .processedPrivilegedTransactionsRollingHash ==
                privilegedTransactionsHash,
            "OnChainProposer: privileged transaction hash public input does not match with committed transactions"
        );
        bytes32 lastBlockHash = bytes32(publicData[128:160]);
        require(
            batchCommitments[batchNumber].lastBlockHash == lastBlockHash,
            "OnChainProposer: last block hash public inputs don't match with last block hash"
        );
        uint256 chainId = uint256(bytes32(publicData[160:192]));
        require(
            chainId == CHAIN_ID,
            "OnChainProposer: given chain id does not correspond to this network"
        );
        uint256 nonPrivilegedTransactions = uint256(
            bytes32(publicData[192:224])
        );
        require(
            !ICommonBridge(BRIDGE).hasExpiredPrivilegedTransactions() ||
                nonPrivilegedTransactions == 0,
            "OnChainProposer: exceeded privileged transaction inclusion deadline, can't include non-privileged transactions"
        );
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}
}
