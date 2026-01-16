// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "./BoxUpgradeable.sol";

contract BoxUpgradeableV2 is BoxUpgradeable {
    // New storage is appended after BoxUpgradeable.value.
    uint256 public extraValue;
    uint256 public extraValue2;

    function setExtraValue(uint256 newValue) external {
        extraValue = newValue;
    }

    function setExtraValue2(uint256 newValue) external {
        extraValue2 = newValue;
    }
}
