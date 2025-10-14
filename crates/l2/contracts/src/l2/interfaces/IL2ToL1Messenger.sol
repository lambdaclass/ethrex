// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

/// @title Interface for the L2 side of the CommonBridge contract.
/// @author LambdaClass
/// @notice The L1Messenger contract is a contract that allows L2->L1 communication
/// It handles message sending to L1, which is used to handle withdrawals.
interface IL2ToL1Messenger {
    /// @notice A withdrawal to L1 has initiated.
    /// @dev Event emitted when a withdrawal is initiated.
    /// @param senderOnL2 the caller on L2
    /// @param data the data being sent, usually a hash
    event L1Message(
        address indexed senderOnL2,
        bytes32 indexed data,
        uint256 indexed messageId
    );

    event L2ToL2Message(
        uint256 chainId,
        address from,
        address to,
        uint256 value,
        bytes data,
        uint256 messageId
    );

    /// @notice Sends the given data to the L1
    /// @param data data to be sent to L1
    function sendMessageToL1(bytes32 data) external;

    function sendMessageToL2(
        uint256 chainId,
        address from,
        address to,
        bytes calldata data
    ) external payable;
}
