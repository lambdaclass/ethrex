// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

contract Fibonacci {
    function Benchmark(uint256 n) public pure returns (uint256 result) {
        if (n <= 1) return n;

        uint256 a = 0;
        uint256 b = 1;

        for (uint256 i = 2; i <= n; i++) {
            // Check for overflow
            if (b > (type(uint256).max - a)) {
                return type(uint256).max;
            }
            (a, b) = (b, a + b);
        }

        result = b;
        return result;
    }
}
