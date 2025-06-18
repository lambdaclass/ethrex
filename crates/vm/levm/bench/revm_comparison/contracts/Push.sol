// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Push {
    function Benchmark(uint256 n) public pure returns (uint256) {
        uint256 sum = 0;
        for (uint256 i = 1; i <= n; i++) {
            sum += i;
            if (i % 10 == 0) {
                sum = sum / 10;
            }
        }
        return sum;
    }
}
