// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/governance/TimelockControllerUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import {IOnChainProposer} from "./interfaces/IOnChainProposer.sol";
import {ICommonBridge} from "./interfaces/ICommonBridge.sol";

contract Timelock is
    TimelockControllerUpgradeable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable
{
    bytes32 public constant SEQUENCER = keccak256("SEQUENCER");
    bytes32 public constant SECURITY_COUNCIL = keccak256("SECURITY_COUNCIL");

    IOnChainProposer public onChainProposer;

    function initialize(
        uint256 minDelay,
        address[] memory sequencers,
        address owner,
        address securityCouncil,
        address _onChainProposer
    ) public initializer {
        for (uint256 i = 0; i < sequencers.length; ++i) {
            _grantRole(SEQUENCER, sequencers[i]);
        }

        _grantRole(SECURITY_COUNCIL, securityCouncil);

        address[] memory owners = new address[](1);
        owners[0] = owner;

        TimelockControllerUpgradeable.__TimelockController_init(
            minDelay,
            owners, // proposers
            owners, // executors
            securityCouncil // admin
        );
        OwnableUpgradeable.__Ownable_init(owner);
        onChainProposer = IOnChainProposer(_onChainProposer);
    }

    function commitBatch(
        uint256 batchNumber,
        bytes32 newStateRoot,
        bytes32 withdrawalsLogsMerkleRoot,
        bytes32 processedPrivilegedTransactionsRollingHash,
        bytes32 lastBlockHash,
        uint256 nonPrivilegedTransactions,
        ICommonBridge.BalanceDiff[] calldata balanceDiffs,
        ICommonBridge.L2MessageRollingHash[] calldata l2MessageRollingHashes
    ) external onlyRole(SEQUENCER) {
        onChainProposer.commitBatch(
            batchNumber,
            newStateRoot,
            withdrawalsLogsMerkleRoot,
            processedPrivilegedTransactionsRollingHash,
            lastBlockHash,
            nonPrivilegedTransactions,
            balanceDiffs,
            l2MessageRollingHashes
        );
    }

    function verifyBatch(
        uint256 batchNumber,
        bytes memory risc0BlockProof,
        bytes calldata risc0Journal,
        bytes calldata sp1PublicValues,
        bytes memory sp1ProofBytes,
        bytes calldata tdxPublicValues,
        bytes memory tdxSignature
    ) external onlyRole(SEQUENCER) {
        onChainProposer.verifyBatch(
            batchNumber,
            risc0BlockProof,
            risc0Journal,
            sp1PublicValues,
            sp1ProofBytes,
            tdxPublicValues,
            tdxSignature
        );
    }

    function verifyBatchesAligned(
        uint256 firstBatchNumber,
        bytes[] calldata publicInputsList,
        bytes32[][] calldata sp1MerkleProofsList,
        bytes32[][] calldata risc0MerkleProofsList
    ) external onlyRole(SEQUENCER) {
        onChainProposer.verifyBatchesAligned(
            firstBatchNumber,
            publicInputsList,
            sp1MerkleProofsList,
            risc0MerkleProofsList
        );
    }

    function revertBatch(
        uint256 batchNumber
    ) external onlyRole(SECURITY_COUNCIL) {
        onChainProposer.revertBatch(batchNumber);
    }

    function _authorizeUpgrade(
        address newImplementation
    ) internal override onlyOwner {
        address sender = _msgSender();
        if (sender != address(this)) {
            revert TimelockUnauthorizedCaller(sender);
        }
    }
}
