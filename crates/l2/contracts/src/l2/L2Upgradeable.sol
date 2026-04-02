// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";

/// @title Proxy for an L2 system contract
/// @author LambdaClass
/// @notice This contract is meant to be included in the genesis at the system contract address
/// @dev The genesis generator sets the initial implementation using the well-known (ERC-1967) proxy slot
contract UpgradeableSystemContract is TransparentUpgradeableProxy {
    /// @notice The CommonBridge uses this address to send upgrades
    address constant ADMIN =  0x000000000000000000000000000000000000f000;

    constructor() TransparentUpgradeableProxy(address(0), address(0), "") {
        // This contract is compiled into runtime code when assembling the genesis
        // The setup is done by directly setting the ERC-1967 storage slots
        revert("This contract is not meant to be directly deployed.");
    }

    /// @notice We override this so that it can be easily included in the genesis
    /// @dev The normal constructor has hard to simulate (deploying a ProxyAdmin, setting immutable variables)
    /// @dev behavior, so this is the simpler way.
    function _proxyAdmin() internal pure override returns (address) {
        return ADMIN;
    }
}
