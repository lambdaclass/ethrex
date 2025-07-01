// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";

/// @title Interface for an L2-capable token.
/// @author LambdaClass
/// @dev Uses the interface described in the ERC-7802 draft
contract UpgradeableSystemContract is ERC1967Proxy {
    constructor() ERC1967Proxy(address(0), "") {
        // This contract is compiled into runtime code when assembling the genesis
        // The setup is done by directly setting the ERC-1967 storage slots
        revert("This contract is not meant to be directly deployed.");
    }

    function upgradeToAndCall(address newImplementation, bytes memory data) public {
        require(msg.sender == ERC1967Utils.getAdmin(), "Proxy: can only be upgraded by administrator");
        ERC1967Utils.upgradeToAndCall(newImplementation, data);
    }

    receive() external payable {
        _fallback();
    }
}
