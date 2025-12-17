// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @author LambdaClass
/// @notice The Messenger contract is a contract that allows L2->L1 communication and L2->L2 communication.
/// It handles message sending to L1, which is used to handle withdrawals.
/// It also handles message sending to other L2 chains.
interface IMessenger {
    /// @notice A withdrawal to L1 has initiated.
    /// @dev Event emitted when a withdrawal is initiated.
    /// @param senderOnL2 the caller on L2
    /// @param data the data being sent, usually a hash
    event L1Message(
        address indexed senderOnL2,
        bytes32 indexed data,
        uint256 indexed messageId
    );

    /// @notice A message to another L2 chain has been sent.
    /// @dev Event emitted when a message to another L2 chain is sent.
    /// @param chainId the destination chain id
    /// @param from the sender address on the destination chain
    /// @param to the recipient address on the destination chain
    /// @param value the amount of ETH to send to the recipient on the destination chain
    /// @param gasLimit the gas limit for the message execution on the destination chain
    /// @param txId the unique transaction id for the message
    /// @param data the calldata to be sent to the recipient on the destination chain
    event L2Message(
        uint256 indexed chainId,
        address from,
        address to,
        uint256 value,
        uint256 gasLimit,
        uint256 txId,
        bytes data
    );

    /// @notice Sends the given data to the L1
    /// @param data data to be sent to L1
    function sendMessageToL1(bytes32 data) external;

    /// @notice Sends a message to another L2 chain
    /// @param chainId the destination chain id
    /// @param from the sender address on the source chain
    /// @param to the recipient address on the destination chain
    /// @param gasLimit the gas limit for the message execution on the destination chain
    /// @param txId the unique transaction id for the message
    /// @param value the amount of ETH to send to the recipient on the destination chain
    /// @param data the calldata to be sent to the recipient on the destination chain
    function sendMessageToL2(
        uint256 chainId,
        address from,
        address to,
        uint256 gasLimit,
        uint256 txId,
        uint256 value,
        bytes calldata data
    ) external;
}
