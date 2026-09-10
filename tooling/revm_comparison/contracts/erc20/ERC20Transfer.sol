// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

import "./lib/ERC20.sol";

contract ERC20Transfer is ERC20 {
    constructor() ERC20("ERC20_Transfer", "E20_TRANSFER") {}

    function Benchmark(uint256 n) external returns (uint256 result) {
        address testAddress = 0x1234567890123456789012345678901234567890;
        _mint(_msgSender(), 10000 * 10**decimals());
        for (uint256 i = 0; i < n; i++) {
            transfer(testAddress, i);
        }
        return balanceOf(testAddress);
    }
}
