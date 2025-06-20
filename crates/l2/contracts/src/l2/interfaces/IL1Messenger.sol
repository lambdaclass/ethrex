// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

/// @title Interface for the L2 side of the CommonBridge contract.
/// @author LambdaClass
/// @notice The L1Messenger contract is a contract that allows L2->L1 communication
/// It handles message sending to L1, which is used to handle withdrawals.
interface IL1Messenger {
    /// @notice A withdrawal to L1 has initiated.
    /// @dev Event emitted when a withdrawal is initiated.
    /// @param senderOnL2 the caller on L2
    /// @param data the data being sent, usually a hash
    event L1Message(
        address indexed senderOnL2,
        bytes32 indexed data
    );

    /// @notice Sends the given data to the L1
    /// @param data data to be sent to L1
    function sendMessageToL1(bytes32 data) external;
}
