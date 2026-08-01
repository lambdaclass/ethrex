// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract FactorialRecursive {
    function Benchmark(uint256 n) public view returns (uint256) {
        // Base cases
        if (n == 0 || n == 1) {
            return 1;
        }
        
        // Recursive call via external function
        uint256 rec = this.Benchmark(n - 1);

        // Check for overflow
        if (rec > (type(uint256).max / n)) {
            return type(uint256).max;
        }
        
        return n * rec;
    }
}
