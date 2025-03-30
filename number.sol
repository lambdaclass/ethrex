// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

contract Test {
    event NumberSet(uint256 indexed number);

    function emitNumber(uint256 _number) public {
        emit NumberSet(_number);
    }
}
