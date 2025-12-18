// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "./interfaces/IMessenger.sol";

/// @title Messenger contract.
/// @author LambdaClass
contract Messenger is IMessenger {
    /// @notice Id of the last emitted message.
    /// @dev Message Id that should be incremented before a message is sent
    uint256 public lastMessageId;
    address public constant BRIDGE =  0x000000000000000000000000000000000000FFff;

    modifier onlyBridge() {
        require(msg.sender == BRIDGE, "Only bridge can call this function");
        _;
    }

    /// @inheritdoc IMessenger
    function sendMessageToL1(bytes32 data) external {
        // This event gets pushed to L1, the sequencer monitors
        // them on every block.
        lastMessageId += 1;
        emit L1Message(msg.sender, data, lastMessageId);
    }

    /// @inheritdoc IMessenger
    function sendMessageToL2(
        uint256 chainId,
        address from,
        address to,
        uint256 gasLimit,
        uint256 txId,
        uint256 value,
        bytes calldata data
    ) external onlyBridge {
        emit L2Message(chainId, from, to, value, gasLimit, txId, data);
    }
}
