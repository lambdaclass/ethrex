// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

// Contract used in the `load-test-io` Makefile target. The test sends transactions calling
// the `incrementNumbers()` function, which writes to 100 storage slots.

contract Counter {
    uint256[100] public number;

    constructor() {
        for(uint i = 0; i < 100; i++) {
            number[i] = i;
        }
    }

    function incrementNumbers() public {
        for(uint i = 0; i < 100; i++) {
            number[i] = number[i] + 1;
        }
    }

    function getFirstNumber() public view returns(uint256) {
        return number[0];
    }
}
