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

    /// @notice The verified blocks.
    /// @dev If a block is verified, the block hash is stored here.
    /// @dev If a block was not verified yet, it won't be here.
    /// @dev It is used by other contracts to verify if a block was verified.
    mapping(uint256 => bool) public verifiedBlocks;

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
        bytes32 newL2StateRoot,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 depositLogs
    ) external override {
        require(
            !verifiedBlocks[blockNumber],
            "OnChainProposer: block already verified"
        );
        require(
            blockCommitments[blockNumber].commitmentHash == bytes32(0),
            "OnChainProposer: block already committed"
        );
        bytes32 blockCommitment = keccak256(
            abi.encode(
                blockNumber,
                newL2StateRoot,
                withdrawalsLogsMerkleRoot,
                depositLogs
            )
        );
        blockCommitments[blockNumber] = BlockCommitmentInfo(
            blockCommitment,
            depositLogs
        );
        if (withdrawalsLogsMerkleRoot != bytes32(0)) {
            ICommonBridge(BRIDGE).publishWithdrawals(
                blockNumber,
                withdrawalsLogsMerkleRoot
            );
        }
        emit BlockCommitted(blockCommitment);
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
            !verifiedBlocks[blockNumber],
            "OnChainProposer: block already verified"
        );

        verifiedBlocks[blockNumber] = true;
        ICommonBridge(BRIDGE).removeDepositLogs(
            // The first 2 bytes are the number of deposits.
            uint16(uint256(blockCommitments[blockNumber].depositLogs >> 240))
        );

        // Remove previous block commitment as it is no longer needed.
        delete blockCommitments[blockNumber - 1];

        emit BlockVerified(blockNumber);
    }
}
