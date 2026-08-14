// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

import "./lib/ERC20.sol";

contract ERC20Mint is ERC20 {
    constructor() ERC20("ERC20_Mint", "E20_MINT") {}

    function Benchmark(uint256 n) external returns (uint256 result) {
        address testAddress = 0x1234567890123456789012345678901234567890;
        for (uint256 i = 0; i < n; i++) {
            _mint(testAddress, i);
        }
        return balanceOf(testAddress);
    }
}
