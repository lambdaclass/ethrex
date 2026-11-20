// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

contract ManyHashes {
    function Benchmark(uint256 n) external pure returns (bytes32 result) {
        result = bytes32(0);
        for (uint256 i = 0; i < n; i++) {
            result = keccak256(abi.encodePacked(i));
        }
    }
}
