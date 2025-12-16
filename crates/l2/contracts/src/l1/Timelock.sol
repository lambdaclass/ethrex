// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/governance/TimelockControllerUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import {IOnChainProposer} from "./interfaces/IOnChainProposer.sol";
import {ICommonBridge} from "./interfaces/ICommonBridge.sol";

contract Timelock is TimelockControllerUpgradeable, UUPSUpgradeable {
    error TimelockUnauthorizedCaller(address caller);

    bytes32 public constant SEQUENCER = keccak256("SEQUENCER");
    bytes32 public constant SECURITY_COUNCIL = keccak256("SECURITY_COUNCIL");

    IOnChainProposer public onChainProposer;

    function initialize(
        uint256 minDelay, // This should be the minimum delay for contract upgrades in seconds (e.g. 7 days = 604800 sec).
        address[] memory sequencers, // Will be able to commit and verify batches.
        address owner, // Will be able to propose and execute functions, respecting the delay.
        address securityCouncil, // TODO: Admin role -> Can manage roles. But it can't schedule/execute by itself, maybe we should add that
        address _onChainProposer // deployed OnChainProposer contract.
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

        onChainProposer = IOnChainProposer(_onChainProposer);
    }

    // TODO: In commit and verify we should probably modify logic so that we have a time window between commit and verify,
    // or if we want to do it better we can have commit -> verify -> execute and the time window has to be between commit and execute.
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

    // Logic for updating Timelock contract. Should be triggered by the timelock itself so that it respects min time.
    function _authorizeUpgrade(address newImplementation) internal override {
        address sender = _msgSender();
        if (sender != address(this)) {
            revert TimelockUnauthorizedCaller(sender);
        }
    }
}
