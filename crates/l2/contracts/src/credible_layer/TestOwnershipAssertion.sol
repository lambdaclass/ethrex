// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title TestOwnershipAssertion
/// @notice A trivial assertion for Credible Layer end-to-end testing.
/// Protects OwnableTarget by asserting that ownership cannot change.
///
/// This contract uses the credible-std library interfaces.
/// To compile and deploy, use the pcl CLI:
///   pcl apply --assertion TestOwnershipAssertion --adopter <OwnableTarget_address>
///
/// For local development without credible-std, this file serves as a reference
/// for the assertion logic. The actual deployment uses pcl which handles
/// compilation with the credible-std dependency.

interface IPhEvm {
    function forkPreTx() external;
    function forkPostTx() external;
    function getAssertionAdopter() external view returns (address);
}

interface IOwnableTarget {
    function owner() external view returns (address);
    function transferOwnership(address newOwner) external;
}

/// @dev In production, this would inherit from credible-std's Assertion base class.
/// The actual assertion contract deployed via pcl would look like:
///
///   import {Assertion} from "credible-std/Assertion.sol";
///
///   contract TestOwnershipAssertion is Assertion {
///       function triggers() external view override {
///           registerCallTrigger(
///               this.assertOwnerUnchanged.selector,
///               IOwnableTarget.transferOwnership.selector
///           );
///       }
///
///       function assertOwnerUnchanged() external {
///           IOwnableTarget target = IOwnableTarget(ph.getAssertionAdopter());
///           ph.forkPreTx();
///           address ownerBefore = target.owner();
///           ph.forkPostTx();
///           address ownerAfter = target.owner();
///           require(ownerBefore == ownerAfter, "ownership changed");
///       }
///   }
contract TestOwnershipAssertion {
    // This is a reference implementation. See the comment above for the
    // actual credible-std version used with pcl.
}
