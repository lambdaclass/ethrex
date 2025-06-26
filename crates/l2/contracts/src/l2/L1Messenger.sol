// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "./interfaces/IL1Messenger.sol";

/// @title L1Messenger contract.
/// @author LambdaClass
contract L1Messenger is IL1Messenger {
    function sendMessageToL1(bytes32 data) external {
        // This event gets pushed to L1, the sequencer monitors
        // them on every block.
        emit L1Message(msg.sender, data);
    }
}
