pragma solidity ^0.8.0;

contract Fibonacci {
    function fibonacci(uint256 n) public pure returns (uint256 result) {
        if (n <= 1)
            return n;

        uint256 a = 0;
        uint256 b = 1;

        for (uint256 i = 2; i <= n; i++) {
            (a, b) = (b, a + b);
        }

        result = b;
    }

    fallback() external payable {
    }
}
