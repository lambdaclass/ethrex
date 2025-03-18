// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import "../../lib/openzeppelin-contracts/contracts/access/Ownable.sol";
import "../../lib/openzeppelin-contracts/contracts/utils/ReentrancyGuard.sol";
import "./interfaces/IOnChainProposer.sol";
import {CommonBridge} from "./CommonBridge.sol";
import {ICommonBridge} from "./interfaces/ICommonBridge.sol";
import {IRiscZeroVerifier} from "./interfaces/IRiscZeroVerifier.sol";
import {ISP1Verifier} from "./interfaces/ISP1Verifier.sol";
import {IPicoVerifier} from "./interfaces/IPicoVerifier.sol";

/// @title OnChainProposer contract.
/// @author LambdaClass
contract OnChainProposer is IOnChainProposer, ReentrancyGuard {
    struct BlockCommitmentInfo {
        bytes32 commitmentHash;
        bytes32 depositLogs;
    }

    /// @notice The commitments of the committed blocks.
    /// @dev If a block is committed, the commitment is stored here.
    /// @dev If a block was not committed yet, it won't be here.
    /// @dev It is used by other contracts to verify if a block was committed.
    mapping(uint256 => BlockCommitmentInfo) public blockCommitments;

    /// @notice The latest verified block number.
    /// @dev This variable holds the block number of the most recently verified block.
    /// @dev All blocks with a block number less than or equal to `lastVerifiedBlock` are considered verified.
    /// @dev Blocks with a block number greater than `lastVerifiedBlock` have not been verified yet.
    /// @dev This is crucial for ensuring that only valid and confirmed blocks are processed in the contract.
    uint256 public lastVerifiedBlock;

    /// @notice The latest committed block number.
    /// @dev This variable holds the block number of the most recently committed block.
    /// @dev All blocks with a block number less than or equal to `lastCommittedBlock` are considered committed.
    /// @dev Blocks with a block number greater than `lastCommittedBlock` have not been committed yet.
    /// @dev This is crucial for ensuring that only subsequents blocks are committed in the contract.
    uint256 public lastCommittedBlock;

    /// @notice The next block to commit.
    /// @dev This variable holds the block number of the next block to commit.
    /// @dev `nextBlockToCommit` should be equal to `lastCommittedBlock` + 1.
    /// @dev Only the block with the block number equal to `nextBlockToCommit` will be committed.
    /// @dev This variable is called by the `l1_committer.rs`.
    uint256 public nextBlockToCommit;

    /// @dev The sequencer addresses that are authorized to commit and verify blocks.
    mapping(address _authorizedAddress => bool)
        public authorizedSequencerAddresses;

    address public BRIDGE;
    address public PICOVERIFIER;
    address public R0VERIFIER;
    address public SP1VERIFIER;

    /// @notice Address used to avoid the verification process.
    /// @dev If the `R0VERIFIER` or the `SP1VERIFIER` contract address is set to this address,
    /// the verification process will not happen.
    /// @dev Used only in dev mode.
    address public constant DEV_MODE = address(0xAA);

    modifier onlySequencer() {
        require(
            authorizedSequencerAddresses[msg.sender],
            "OnChainProposer: caller is not the sequencer"
        );
        _;
    }

    /// @inheritdoc IOnChainProposer
    function initialize(
        address bridge,
        address r0verifier,
        address sp1verifier,
        address picoverifier,
        address[] calldata sequencerAddresses
    ) public nonReentrant {
        // Set the CommonBridge address
        require(
            BRIDGE == address(0),
            "OnChainProposer: contract already initialized"
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

        // Set the PicoGroth16Verifier address
        require(
            PICOVERIFIER == address(0),
            "OnChainProposer: contract already initialized"
        );
        require(
            picoverifier != address(0),
            "OnChainProposer: picoverifier is the zero address"
        );
        require(
            picoverifier != address(this),
            "OnChainProposer: picoverifier is the contract address"
        );
        PICOVERIFIER = picoverifier;

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

        for (uint256 i = 0; i < sequencerAddresses.length; i++) {
            authorizedSequencerAddresses[sequencerAddresses[i]] = true;
        }
    }

    /// @inheritdoc IOnChainProposer
    function commit(
        uint256 blockNumber,
        bytes32 commitment,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 depositLogs
    ) external override onlySequencer {
        require(
            blockNumber == nextBlockToCommit ||
                (blockNumber == 0 &&
                    lastCommittedBlock == 0 &&
                    nextBlockToCommit == 0),
            "OnChainProposer: blockNumber is not the immediate successor of lastCommittedBlock"
        );
        require(
            blockCommitments[blockNumber].commitmentHash == bytes32(0),
            "OnChainProposer: block already committed"
        );
        // Check if commitment is equivalent to blob's KZG commitment.

        if (depositLogs != bytes32(0)) {
            bytes32 savedDepositLogs = ICommonBridge(BRIDGE)
                .getDepositLogsVersionedHash(uint16(bytes2(depositLogs)));
            require(
                savedDepositLogs == depositLogs,
                "OnChainProposer: invalid deposit logs"
            );
        }
        if (withdrawalsLogsMerkleRoot != bytes32(0)) {
            ICommonBridge(BRIDGE).publishWithdrawals(
                blockNumber,
                withdrawalsLogsMerkleRoot
            );
        }
        blockCommitments[blockNumber] = BlockCommitmentInfo(
            commitment,
            depositLogs
        );
        lastCommittedBlock = blockNumber;
        nextBlockToCommit = blockNumber + 1;
        emit BlockCommitted(commitment);
    }

    /// @inheritdoc IOnChainProposer
    /// @notice The first `require` checks that the block number is the subsequent block.
    /// @notice The second `require` checks if the block has been committed.
    /// @notice The order of these `require` statements is important.
    /// Ordering Reason: After the verification process, we delete the `blockCommitments` for `blockNumber - 1`. This means that when checking the block,
    /// we might get an error indicating that the block hasn’t been committed, even though it was committed but deleted. Therefore, it has already been verified.
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
    ) external override onlySequencer {
        // TODO: imageid, programvkey and riscvvkey should be constants
        // TODO: organize each zkvm proof arguments in their own structs
        require(
            blockNumber == lastVerifiedBlock + 1,
            "OnChainProposer: block already verified"
        );

        require(
            blockCommitments[blockNumber].commitmentHash != bytes32(0),
            "OnChainProposer: block not committed"
        );

        if (PICOVERIFIER != DEV_MODE) {
            // If the verification fails, it will revert.
            IPicoVerifier(PICOVERIFIER).verifyPicoProof(
                picoRiscvVkey,
                picoPublicValues,
                picoProof
            );
        }

        if (R0VERIFIER != DEV_MODE) {
            // If the verification fails, it will revert.
            IRiscZeroVerifier(R0VERIFIER).verify(
                risc0BlockProof,
                risc0ImageId,
                risc0JournalDigest
            );
        }

        if (SP1VERIFIER != DEV_MODE) {
            // If the verification fails, it will revert.
            ISP1Verifier(SP1VERIFIER).verifyProof(
                sp1ProgramVKey,
                sp1PublicValues,
                sp1ProofBytes
            );
        }

        lastVerifiedBlock = blockNumber;
        // The first 2 bytes are the number of deposits.
        uint16 deposits_amount = uint16(
            bytes2(blockCommitments[blockNumber].depositLogs)
        );
        if (deposits_amount > 0) {
            ICommonBridge(BRIDGE).removeDepositLogs(deposits_amount);
        }

        // Remove previous block commitment as it is no longer needed.
        delete blockCommitments[blockNumber - 1];

        emit BlockVerified(blockNumber);
    }
}
