// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "./BoxUpgradeable.sol";

contract BoxUpgradeableV2 is BoxUpgradeable {
    uint256 public extraValue;

    function setExtraValue(uint256 newValue) external {
        extraValue = newValue;
    }
}
