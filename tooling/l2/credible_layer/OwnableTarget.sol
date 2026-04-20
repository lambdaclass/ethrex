// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title OwnableTarget
/// @notice A simple Ownable contract used as a target for Credible Layer testing.
/// The TestOwnershipAssertion protects this contract by preventing ownership transfers.
contract OwnableTarget {
    address public owner;

    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);

    constructor() {
        owner = msg.sender;
    }

    modifier onlyOwner() {
        require(msg.sender == owner, "OwnableTarget: caller is not the owner");
        _;
    }

    /// @notice Transfer ownership to a new address.
    /// When protected by the TestOwnershipAssertion, this call will be dropped by the sidecar.
    function transferOwnership(address newOwner) external onlyOwner {
        require(newOwner != address(0), "OwnableTarget: new owner is the zero address");
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner;
    }

    /// @notice A harmless function that does not trigger any assertion.
    function doSomething() external pure returns (uint256) {
        return 42;
    }
}
