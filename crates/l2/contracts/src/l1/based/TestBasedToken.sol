// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract TestBasedToken is ERC20 {
    constructor() ERC20("TestToken", "TT") {}
}
