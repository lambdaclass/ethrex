// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

contract Factorial {
    function Benchmark(uint256 n) public pure returns (uint256 result) {
        if (n == 0 || n == 1) {
            return 1;
        }

        result = 1;
        for (uint256 i = 2; i <= n; i++) {
            // Check for overflow
            if (result > (type(uint256).max / i)) {
                return type(uint256).max;
            } else {
                result *= i;
            }
        }

        return result;
    }
}
