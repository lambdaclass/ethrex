// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

import "./lib/ERC20.sol";

contract ERC20Approval is ERC20 {
    constructor() ERC20("ERC20Approval", "E20_APP") {}

    function Benchmark(uint256 n) external returns (uint256 result) {
        _mint(msg.sender, 1000000000 * 10**decimals());
        for (uint256 i = 1; i < n; i++) {
            require(
                allowance(msg.sender, msg.sender) == 0,
                "Allowance at start is nonzero"
            );
            approve(msg.sender, i);
            require(
                allowance(msg.sender, msg.sender) == i,
                "Sender has no allowance"
            );
            transferFrom(msg.sender, msg.sender, i);
            require(
                allowance(msg.sender, msg.sender) == 0,
                "Allowance at end is nonzero"
            );
        }
        return balanceOf(msg.sender);
    }
}
