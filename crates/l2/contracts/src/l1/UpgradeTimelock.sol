// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/governance/TimelockController.sol";

interface IOwnable2Step {
    function acceptOwnership() external;
}

/// @title UpgradeTimelock
/// @notice A TimelockController with a fixed 7 day delay, intended to control contract upgrades.
contract UpgradeTimelock is TimelockController {
    uint256 public constant MIN_DELAY = 7 days;

    /// @param admin The timelock admin (can grant/revoke roles).
    /// @param proposer The proposer allowed to schedule/cancel operations.
    constructor(
        address admin,
        address proposer
    )
        TimelockController(
            MIN_DELAY,
            _asSingletonArray(proposer),
            _asSingletonArray(address(0)),
            admin
        )
    {}

    /// @notice Accept ownership for an Ownable2Step contract whose pending owner is this timelock.
    /// @dev Useful during deployment when transferring ownership to a timelock contract.
    function acceptOwnership(address ownable2Step) external onlyRole(DEFAULT_ADMIN_ROLE) {
        IOwnable2Step(ownable2Step).acceptOwnership();
    }

    function _asSingletonArray(
        address element
    ) private pure returns (address[] memory array) {
        array = new address[](1);
        array[0] = element;
    }
}

