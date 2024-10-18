// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test, console} from "forge-std/Test.sol";
import {OnChainOperator} from "../../src/l1/OnChainOperator.sol";

contract OnChainOperatorTest is Test {
    OnChainOperator public blockExecutor;

    function setUp() public {
        blockExecutor = new OnChainOperator();
    }
}
