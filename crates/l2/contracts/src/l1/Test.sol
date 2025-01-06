// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

contract Test {
    uint256 public NUMBER = 20;

    function get() public view returns (uint256) {
        return NUMBER;
    }
}
