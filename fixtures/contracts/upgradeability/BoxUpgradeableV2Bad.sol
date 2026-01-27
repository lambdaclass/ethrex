// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";

contract BoxUpgradeableV2Bad is Initializable, OwnableUpgradeable, UUPSUpgradeable {
    uint256 public extraValue;
    uint256 public value;

    function initialize(uint256 initialValue) public initializer {
        __Ownable_init(msg.sender);
        value = initialValue;
    }

    function setValue(uint256 newValue) external {
        value = newValue;
    }

    function setExtraValue(uint256 newValue) external {
        extraValue = newValue;
    }

    function _authorizeUpgrade(address newImplementation) internal override onlyOwner {
        (newImplementation);
    }
}
