// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

contract FibonacciRecursive {
    function Benchmark(uint256 n) public view returns (uint256 result) {
        if (n <= 1) return n;

        uint256 rec = this.Benchmark(n - 1) + this.Benchmark(n - 2);

        // Check for overflow
        if (rec > (type(uint256).max / n)) {
            return type(uint256).max;
        }

        result = rec;
        return result;
    }
}
