// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";

/// @title Interface for an L2-capable token.
/// @author LambdaClass
contract UpgradeableSystemContract is TransparentUpgradeableProxy {
    address constant ADMIN =  0x000000000000000000000000000000000000f000;

    constructor() TransparentUpgradeableProxy(address(0), address(0), "") {
        // This contract is compiled into runtime code when assembling the genesis
        // The setup is done by directly setting the ERC-1967 storage slots
        revert("This contract is not meant to be directly deployed.");
    }

    function _proxyAdmin() internal pure override returns (address) {
        return ADMIN;
    }
}
