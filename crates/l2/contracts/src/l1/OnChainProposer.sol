// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import "../../lib/openzeppelin-contracts/contracts/access/Ownable.sol";
import "../../lib/openzeppelin-contracts/contracts/utils/ReentrancyGuard.sol";
import "./interfaces/IOnChainProposer.sol";
import {CommonBridge} from "./CommonBridge.sol";
import {ICommonBridge} from "./interfaces/ICommonBridge.sol";

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

    address public BRIDGE;

    /// @inheritdoc IOnChainProposer
    function initialize(address bridge) public nonReentrant {
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
    }

    /// @inheritdoc IOnChainProposer
    function commit(
        uint256 blockNumber,
        bytes32 commitment,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 depositLogs
    ) external override {
        require(
            blockNumber == lastVerifiedBlock + 1,
            "OnChainProposer: block already verified"
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
        emit BlockCommitted(commitment);
    }

    /// @inheritdoc IOnChainProposer
    function verify(
        uint256 blockNumber,
        bytes calldata // blockProof
    ) external override {
        require(
            blockCommitments[blockNumber].commitmentHash != bytes32(0),
            "OnChainProposer: block not committed"
        );
        require(
            blockNumber == lastVerifiedBlock + 1,
            "OnChainProposer: block already verified"
        );

        lastVerifiedBlock = blockNumber;
        ICommonBridge(BRIDGE).removeDepositLogs(
            // The first 2 bytes are the number of deposits.
            uint16(bytes2(blockCommitments[blockNumber].depositLogs))
        );

        // Remove previous block commitment as it is no longer needed.
        delete blockCommitments[blockNumber - 1];

        emit BlockVerified(blockNumber);
    }
}
