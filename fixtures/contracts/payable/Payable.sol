// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

contract Payable {
    function functionThatReverts() public payable {
        revert("told you!");
    }

    // we check the logs for the event to double-check the tx executed successfully
    event Number(uint256 indexed number);
    function functionThatEmitsEvent(uint256 number) public payable {
        emit Number(number);
    }
}
