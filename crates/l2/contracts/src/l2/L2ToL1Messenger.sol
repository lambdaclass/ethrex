// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "./interfaces/IL2ToL1Messenger.sol";

/// @title L2ToL1Messenger contract.
/// @author LambdaClass
contract L2ToL1Messenger is IL2ToL1Messenger {
    /// @notice Id of the last emitted message.
    /// @dev Message Id that should be incremented before a message is sent
    uint256 public lastMessageId;
    uint256 public lastMessageIdL2;

    function sendMessageToL1(bytes32 data) external {
        // This event gets pushed to L1, the sequencer monitors
        // them on every block.
        lastMessageId += 1;
        emit L1Message(msg.sender, data, lastMessageId);
    }

    function sendMessageToL2(uint256 chainId, address from, address to, uint256 gasLimit, bytes calldata data) external payable {
        lastMessageIdL2 += 1;
        emit L2ToL2Message(chainId, from, to, msg.value, gasLimit, data, lastMessageIdL2);
    }
}
