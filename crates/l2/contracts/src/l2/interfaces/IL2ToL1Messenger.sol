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
        uint256 gasLimit,
        bytes data,
    );

    /// @notice Sends the given data to the L1
    /// @param data data to be sent to L1
    function sendMessageToL1(bytes32 data) external;

    /// @notice Sends a message to another L2 chain
    /// @param chainId the destination chain id
    /// @param from the sender address on the source chain
    /// @param to the recipient address on the destination chain
    /// @param gasLimit the gas limit for the message execution on the destination chain
    /// @param data the calldata to be sent to the recipient on the destination chain
    function sendMessageToL2(
        uint256 chainId,
        address from,
        address to,
        uint256 gasLimit,
        bytes calldata data
    ) external payable;
}
