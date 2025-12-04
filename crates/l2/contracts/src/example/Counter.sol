// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract Counter {
    uint256 public count;

    function increment() external payable {
        count += 1;
    }

    function get() external view returns (uint256) {
        return count;
    }
}
