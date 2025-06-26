// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

contract DeepStack {
    function Benchmark(uint256 n) public pure returns (uint256 result) {
        result = 0;
        for (uint256 i = 2; i <= n; i++) {
            uint256 a = 1;
            uint256 b = 2;
            uint256 c = 3;
            uint256 d = 4;
            uint256 e = 5;
            uint256 f = 6;
            uint256 g = 7;
            uint256 h = 8;
            uint256 ii = 9;
            uint256 sum = big_function(a, b, c, d, e, f, g, h, ii);

            // Check for overflow
            if (result > (type(uint256).max / i)) {
                return type(uint256).max;
            } else {
                result += sum;
            }
        }

        return result;
    }

    function big_function(
        uint256 a,
        uint256 b,
        uint256 c,
        uint256 d,
        uint256 e,
        uint256 f,
        uint256 g,
        uint256 h,
        uint256 i
    ) public pure returns (uint256) {
        return i + h + g + f + e + c + d + a + b;
    }
}
