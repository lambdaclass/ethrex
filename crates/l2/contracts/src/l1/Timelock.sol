// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts-upgradeable/governance/TimelockControllerUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {IOnChainProposer} from "./interfaces/IOnChainProposer.sol";
import {ICommonBridge} from "./interfaces/ICommonBridge.sol";
import {ITimelock} from "./interfaces/ITimelock.sol";

/// @title Timelock contract.
/// @author LambdaClass
/// @notice The Timelock contract is the owner of the OnChainProposer contract, it gates access to it by managing roles
/// and adding delay to specific operations for some roles (e.g. updating the contract, in order to provide an exit window).
contract Timelock is TimelockControllerUpgradeable, UUPSUpgradeable, ITimelock {
    /// @notice Role identifier for sequencers.
    /// @dev Accounts with this role can commit and verify batches.
    bytes32 public constant SEQUENCER = keccak256("SEQUENCER");

    /// @notice Role identifier for the Security Council.
    /// @dev Accounts with this role can manage roles and bypass the timelock delay.
    bytes32 public constant SECURITY_COUNCIL = keccak256("SECURITY_COUNCIL");

    /// @notice The OnChainProposer contract controlled by this timelock.
    IOnChainProposer public onChainProposer;

    /// @dev Restricts calls to the timelock itself.
    modifier onlySelf() {
        if (msg.sender != address(this)) {
            revert TimelockCallerNotSelf();
        }
        _;
    }

    /// @notice Disables the parent initialize function to prevent accidental misuse.
    /// @custom:oz-upgrades-unsafe-allow missing-initializer-call
    function initialize(
        uint256,
        address[] memory,
        address[] memory,
        address
    ) public pure override(TimelockControllerUpgradeable) {
        revert TimelockUseCustomInitialize();
    }

    /// @notice Initializes the timelock contract.
    /// @dev Called once after proxy deployment.
    /// @param minDelay The minimum delay (in seconds) for scheduled operations.
    /// @param sequencers Accounts that can commit and verify batches.
    /// @param governance The account that can propose and execute operations, respecting the delay.
    /// @param securityCouncil The Security Council account that can manage roles and bypass the delay.
    /// @param _onChainProposer The deployed `OnChainProposer` contract address.
    function initialize(
        uint256 minDelay,
        address[] memory sequencers,
        address governance,
        address securityCouncil,
        address _onChainProposer
    ) public override(ITimelock) initializer {
        for (uint256 i = 0; i < sequencers.length; ++i) {
            _grantRole(SEQUENCER, sequencers[i]);
        }

        _grantRole(SECURITY_COUNCIL, securityCouncil);

        address[] memory governanceAccounts = new address[](1);
        governanceAccounts[0] = governance;

        TimelockControllerUpgradeable.__TimelockController_init(
            minDelay,
            governanceAccounts, // proposers
            governanceAccounts, // executors
            securityCouncil // admin
        );

        onChainProposer = IOnChainProposer(_onChainProposer);
    }

    /// @notice Returns whether an address has the sequencer role.
    /// @dev This matches the legacy OnChainProposer mapping used by TDXVerifier.
    function isSequencer(address addr) external view returns (bool) {
        return hasRole(SEQUENCER, addr);
    }

    // NOTE: In the future commit and verify will have timelock logic incorporated in case there are any zkVM bugs and we want to avoid applying the changes in the L1. Probably the Security Council would act upon those changes.
    /// @custom:access Restricted to accounts with the `SEQUENCER` role.
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
    ) external onlyRole(SEQUENCER) {
        onChainProposer.commitBatch(
            batchNumber,
            newStateRoot,
            withdrawalsLogsMerkleRoot,
            processedPrivilegedTransactionsRollingHash,
            lastBlockHash,
            nonPrivilegedTransactions,
            commitHash,
            balanceDiffs,
            l2MessageRollingHashes
        );
    }

    /// @custom:access Restricted to accounts with the `SEQUENCER` role.
    function verifyBatch(
        uint256 batchNumber,
        bytes memory risc0BlockProof,
        bytes memory sp1ProofBytes,
        bytes memory tdxSignature
    ) external onlyRole(SEQUENCER) {
        onChainProposer.verifyBatch(
            batchNumber,
            risc0BlockProof,
            sp1ProofBytes,
            tdxSignature
        );
    }

    /// @custom:access Restricted to accounts with the `SEQUENCER` role.
    function verifyBatchesAligned(
        uint256 firstBatchNumber,
        uint256 lastBatchNumber,
        bytes32[][] calldata sp1MerkleProofsList,
        bytes32[][] calldata risc0MerkleProofsList
    ) external onlyRole(SEQUENCER) {
        onChainProposer.verifyBatchesAligned(
            firstBatchNumber,
            lastBatchNumber,
            sp1MerkleProofsList,
            risc0MerkleProofsList
        );
    }

    /// @notice Executes an operation immediately, bypassing the timelock delay.
    /// @dev Intended for emergency use by the Security Council.
    /// @param target The address to call.
    /// @param value The ETH value to send with the call.
    /// @param data The calldata to forward to `target`.
    function emergencyExecute(
        address target,
        uint256 value,
        bytes calldata data
    ) external payable onlyRole(SECURITY_COUNCIL) {
        _execute(target, value, data);
        emit EmergencyExecution(target, value, data);
    }

    /// @notice Allow timelock itself to upgrade the contract in order to respect min time.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlySelf {}
}
