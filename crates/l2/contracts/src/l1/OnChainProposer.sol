// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "./interfaces/IOnChainProposer.sol";
import {CommonBridge} from "./CommonBridge.sol";
import {ICommonBridge} from "./interfaces/ICommonBridge.sol";
import {IRiscZeroVerifier} from "./interfaces/IRiscZeroVerifier.sol";
import {ISP1Verifier} from "./interfaces/ISP1Verifier.sol";
import {ITDXVerifier} from "./interfaces/ITDXVerifier.sol";

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
    /// @dev processedDepositLogsRollingHash is the Merkle root of the logs of the
    /// deposits that were processed in the batch being committed. The amount of
    /// logs that is encoded in this root are to be removed from the
    /// pendingDepositLogs queue of the CommonBridge contract.
    /// @dev withdrawalsLogsMerkleRoot is the Merkle root of the Merkle tree containing
    /// all the withdrawals that were processed in the batch being committed
    struct BatchCommitmentInfo {
        bytes32 newStateRoot;
        bytes32 stateDiffKZGVersionedHash;
        bytes32 processedDepositLogsRollingHash;
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

    /// @dev The sequencer addresses that are authorized to commit and verify batches.
    mapping(address _authorizedAddress => bool)
        public authorizedSequencerAddresses;

    address public BRIDGE;
    /// @dev Deprecated variable.
    address public PICOVERIFIER;
    address public R0VERIFIER;
    address public SP1VERIFIER;

    bytes32 public SP1_VERIFICATION_KEY;

    /// @notice Address used to avoid the verification process.
    /// @dev If the `R0VERIFIER` or the `SP1VERIFIER` contract address is set to this address,
    /// the verification process will not happen.
    /// @dev Used only in dev mode.
    address public constant DEV_MODE = address(0xAA);

    /// @notice Indicates whether the contract operates in validium mode.
    /// @dev This value is immutable and can only be set during contract deployment.
    bool public VALIDIUM;

    address public TDXVERIFIER;

    /// @notice The address of the AlignedProofAggregatorService contract.
    /// @dev This address is set during contract initialization and is used to verify aligned proofs.
    address public ALIGNEDPROOFAGGREGATOR;

    modifier onlySequencer() {
        require(
            authorizedSequencerAddresses[msg.sender],
            "OnChainProposer: caller is not the sequencer"
        );
        _;
    }

    /// @notice Initializes the contract.
    /// @dev This method is called only once after the contract is deployed.
    /// @dev It sets the bridge address.
    /// @param owner the address of the owner who can perform upgrades.
    /// @param alignedProofAggregator the address of the alignedProofAggregatorService contract.
    /// @param r0verifier the address of the risc0 groth16 verifier.
    /// @param sp1verifier the address of the sp1 groth16 verifier.
    function initialize(
        bool _validium,
        address owner,
        address r0verifier,
        address sp1verifier,
        address tdxverifier,
        address alignedProofAggregator,
        bytes32 sp1Vk,
        bytes32 genesisStateRoot,
        address[] calldata sequencerAddresses
    ) public initializer {
        VALIDIUM = _validium;

        // Set the AlignedProofAggregator address
        require(
            ALIGNEDPROOFAGGREGATOR == address(0),
            "OnChainProposer: contract already initialized"
        );
        require(
            alignedProofAggregator != address(0),
            "OnChainProposer: alignedProofAggregator is the zero address"
        );
        require(
            alignedProofAggregator != address(this),
            "OnChainProposer: alignedProofAggregator is the contract address"
        );

        ALIGNEDPROOFAGGREGATOR = alignedProofAggregator;

        // Set the Risc0Groth16Verifier address
        require(
            R0VERIFIER == address(0),
            "OnChainProposer: contract already initialized"
        );
        require(
            r0verifier != address(0),
            "OnChainProposer: r0verifier is the zero address"
        );
        require(
            r0verifier != address(this),
            "OnChainProposer: r0verifier is the contract address"
        );
        R0VERIFIER = r0verifier;

        // Set the SP1Groth16Verifier address
        require(
            SP1VERIFIER == address(0),
            "OnChainProposer: contract already initialized"
        );
        require(
            sp1verifier != address(0),
            "OnChainProposer: sp1verifier is the zero address"
        );
        require(
            sp1verifier != address(this),
            "OnChainProposer: sp1verifier is the contract address"
        );
        SP1VERIFIER = sp1verifier;

        // Set the TDXVerifier address
        require(
            TDXVERIFIER == address(0),
            "OnChainProposer: contract already initialized"
        );
        require(
            tdxverifier != address(0),
            "OnChainProposer: tdxverifier is the zero address"
        );
        require(
            tdxverifier != address(this),
            "OnChainProposer: tdxverifier is the contract address"
        );
        TDXVERIFIER = tdxverifier;

        // Set the SP1 program verification key
        require(
            SP1_VERIFICATION_KEY == bytes32(0),
            "OnChainProposer: contract already initialized"
        );
        SP1_VERIFICATION_KEY = sp1Vk;

        batchCommitments[0] = BatchCommitmentInfo(
            genesisStateRoot,
            bytes32(0),
            bytes32(0),
            bytes32(0),
            bytes32(0)
        );

        for (uint256 i = 0; i < sequencerAddresses.length; i++) {
            authorizedSequencerAddresses[sequencerAddresses[i]] = true;
        }

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
        bytes32 processedDepositLogsRollingHash,
        bytes32 lastBlockHash
    ) external override onlySequencer {
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

        if (processedDepositLogsRollingHash != bytes32(0)) {
            bytes32 claimedProcessedDepositLogs = ICommonBridge(BRIDGE)
                .getPendingDepositLogsVersionedHash(
                    uint16(bytes2(processedDepositLogsRollingHash))
                );
            require(
                claimedProcessedDepositLogs == processedDepositLogsRollingHash,
                "OnChainProposer: invalid deposit logs"
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
            require(blobVersionedHash == 0, "L2 running as validium but blob was published");
        } else {
            require(blobVersionedHash != 0, "L2 running as rollup but blob was not published");
        }

        batchCommitments[batchNumber] = BatchCommitmentInfo(
            newStateRoot,
            blobVersionedHash,
            processedDepositLogsRollingHash,
            withdrawalsLogsMerkleRoot,
            lastBlockHash
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
        bytes32 risc0ImageId,
        //sp1
        bytes memory sp1ProofBytes,
        //tdx
        bytes memory tdxSignature
    ) external override onlySequencer {
        // TODO: Refactor validation
        // TODO: imageid, programvkey and riscvvkey should be constants
        // TODO: organize each zkvm proof arguments in their own structs
        require(
            ALIGNEDPROOFAGGREGATOR == DEV_MODE,
            "OnChainProposer: ALIGNEDPROOFAGGREGATOR is set. Use verifyBatchAligned instead"
        );
        require(
            batchNumber == lastVerifiedBatch + 1,
            "OnChainProposer: batch already verified"
        );
        require(
            batchCommitments[batchNumber].newStateRoot != bytes32(0),
            "OnChainProposer: cannot verify an uncommitted batch"
        );

        // Get public inputs from the commitment
        bytes memory publicInputs = _getPublicInputsFromCommitment(batchNumber);

        if (R0VERIFIER != DEV_MODE) {
            // If the verification fails, it will revert.
            IRiscZeroVerifier(R0VERIFIER).verify(
                risc0BlockProof,
                risc0ImageId,
                sha256(publicInputs)
            );
        }

        if (SP1VERIFIER != DEV_MODE) {
            // If the verification fails, it will revert.
            ISP1Verifier(SP1VERIFIER).verifyProof(
                SP1_VERIFICATION_KEY,
                publicInputs,
                sp1ProofBytes
            );
        }

        if (TDXVERIFIER != DEV_MODE) {
            // If the verification fails, it will revert.
            ITDXVerifier(TDXVERIFIER).verify(publicInputs, tdxSignature);
        }

        lastVerifiedBatch = batchNumber;
        // The first 2 bytes are the number of deposits.
        uint16 deposits_amount = uint16(
            bytes2(
                batchCommitments[batchNumber].processedDepositLogsRollingHash
            )
        );
        if (deposits_amount > 0) {
            ICommonBridge(BRIDGE).removePendingDepositLogs(deposits_amount);
        }

        // Remove previous batch commitment as it is no longer needed.
        delete batchCommitments[batchNumber - 1];

        emit BatchVerified(lastVerifiedBatch);
    }

    /// @inheritdoc IOnChainProposer
    function verifyBatchAligned(
        uint256 batchNumber,
        bytes calldata alignedPublicInputs,
        bytes32 alignedProgramVKey,
        bytes32[] calldata alignedMerkleProof
    ) external override onlySequencer {
        require(
            ALIGNEDPROOFAGGREGATOR != DEV_MODE,
            "OnChainProposer: ALIGNEDPROOFAGGREGATOR is not set"
        );
        require(
            batchNumber == lastVerifiedBatch + 1,
            "OnChainProposer: batch already verified"
        );
        require(
            batchCommitments[batchNumber].newStateRoot != bytes32(0),
            "OnChainProposer: cannot verify an uncommitted batch"
        );

        // If the verification fails, it will revert.
        _verifyPublicData(batchNumber, alignedPublicInputs[8:]);

        bytes memory callData = abi.encodeWithSignature(
            "verifyProofInclusion(bytes32[],bytes32,bytes)",
            alignedMerkleProof,
            alignedProgramVKey,
            alignedPublicInputs
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

        lastVerifiedBatch = batchNumber;

        // The first 2 bytes are the number of deposits.
        uint16 deposits_amount = uint16(
            bytes2(
                batchCommitments[batchNumber].processedDepositLogsRollingHash
            )
        );
        if (deposits_amount > 0) {
            ICommonBridge(BRIDGE).removePendingDepositLogs(deposits_amount);
        }

        // Remove previous batch commitment as it is no longer needed.
        delete batchCommitments[batchNumber - 1];

        emit BatchVerified(lastVerifiedBatch);
    }

    function _verifyPublicData(
        uint256 batchNumber,
        bytes calldata publicData
    ) internal view {
        require(
            publicData.length == 192,
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
        bytes32 depositsLogHash = bytes32(publicData[96:128]);
        require(
            batchCommitments[batchNumber].processedDepositLogsRollingHash ==
                depositsLogHash,
            "OnChainProposer: deposits hash public input does not match with committed deposits"
        );
        bytes32 blobVersionedHash = bytes32(publicData[128:160]);
        require(
            batchCommitments[batchNumber].stateDiffKZGVersionedHash ==
                blobVersionedHash,
            "OnChainProposer: blob versioned hash public input does not match with committed hash"
        );
        bytes32 lastBlockHash = bytes32(publicData[160:192]);
        require(
            batchCommitments[batchNumber].lastBlockHash == lastBlockHash,
            "OnChainProposer: last block hash public inputs don't match with last block hash"
        );
    }

    /// @notice Constructs public inputs from committed batch data for proof verification.
    /// @dev This function retrieves the necessary data from batch commitments and formats it
    /// into a 160-byte array that serves as public inputs for all proving systems.
    /// @dev Public inputs structure (160 bytes total):
    /// - bytes 0-32: Initial state root (from the last verified batch)
    /// - bytes 32-64: Final state root (from the current batch)
    /// - bytes 64-96: Withdrawals merkle root (from the current batch)
    /// - bytes 96-128: Deposits log hash (from the current batch)
    /// - bytes 128-160: Last block hash (from the current batch)
    /// @dev The initial state root uses the last verified batch's newStateRoot because
    /// batch verification is sequential and each batch depends on the previous one.
    /// @param batchNumber The batch number to retrieve public inputs for.
    /// @return publicInputs The 160-byte public inputs array for proof verification.
    function _getPublicInputsFromCommitment(
        uint256 batchNumber
    ) internal view returns (bytes memory) {
        BatchCommitmentInfo memory currentBatch = batchCommitments[batchNumber];
        BatchCommitmentInfo memory previousBatch = batchCommitments[lastVerifiedBatch];
        
        // Public inputs are 160 bytes:
        // - bytes 0-32: initial state root
        // - bytes 32-64: final state root
        // - bytes 64-96: withdrawals merkle root
        // - bytes 96-128: deposits log hash
        // - bytes 128-160: last block hash
        bytes memory publicInputs = new bytes(160);
        
        // Initial state root from the last verified batch
        bytes32 initialStateRoot = previousBatch.newStateRoot;
        for (uint i = 0; i < 32; i++) {
            publicInputs[i] = bytes1(uint8(uint256(initialStateRoot) >> (8 * (31 - i))));
        }
        
        // Final state root from the current batch
        bytes32 finalStateRoot = currentBatch.newStateRoot;
        for (uint i = 0; i < 32; i++) {
            publicInputs[32 + i] = bytes1(uint8(uint256(finalStateRoot) >> (8 * (31 - i))));
        }
        
        // Withdrawals merkle root
        bytes32 withdrawalsRoot = currentBatch.withdrawalsLogsMerkleRoot;
        for (uint i = 0; i < 32; i++) {
            publicInputs[64 + i] = bytes1(uint8(uint256(withdrawalsRoot) >> (8 * (31 - i))));
        }
        
        // Deposits log hash
        bytes32 depositsHash = currentBatch.processedDepositLogsRollingHash;
        for (uint i = 0; i < 32; i++) {
            publicInputs[96 + i] = bytes1(uint8(uint256(depositsHash) >> (8 * (31 - i))));
        }
        
        // Last block hash
        bytes32 lastBlockHash = currentBatch.lastBlockHash;
        for (uint i = 0; i < 32; i++) {
            publicInputs[128 + i] = bytes1(uint8(uint256(lastBlockHash) >> (8 * (31 - i))));
        }
        
        return publicInputs;
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}
}
