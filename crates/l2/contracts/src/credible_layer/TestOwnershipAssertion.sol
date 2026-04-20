// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import {Assertion} from "credible-std/Assertion.sol";

interface IOwnableTarget {
    function owner() external view returns (address);
    function transferOwnership(address newOwner) external;
}

/// @title TestOwnershipAssertion
/// @notice Protects OwnableTarget by asserting that ownership cannot change.
/// @dev Compile with `pcl build` (requires credible-std as a dependency).
///      See docs/l2/credible_layer.md for the full deployment workflow.
contract TestOwnershipAssertion is Assertion {
    function triggers() external view override {
        registerCallTrigger(
            this.assertOwnerUnchanged.selector,
            IOwnableTarget.transferOwnership.selector
        );
    }

    function assertOwnerUnchanged() external {
        IOwnableTarget target = IOwnableTarget(ph.getAssertionAdopter());
        ph.forkPreTx();
        address ownerBefore = target.owner();
        ph.forkPostTx();
        address ownerAfter = target.owner();
        require(ownerBefore == ownerAfter, "ownership changed");
    }
}
