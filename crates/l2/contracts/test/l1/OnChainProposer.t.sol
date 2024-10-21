// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test, console} from "forge-std/Test.sol";
import {OnChainProposer} from "../../src/l1/OnChainProposer.sol";

contract OnChainProposerTest is Test {
    OnChainProposer public proposer;

    function setUp() public {
        proposer = new OnChainProposer();
    }
}
