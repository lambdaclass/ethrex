// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {ICommonBridge} from "./ICommonBridge.sol";

interface IRouter {
    /// @notice Registers a new chain with its OnChainProposer and CommonBridge addresses.
    /// @param chainId The ID of the chain to register.
    /// @param commonBridge The address of the CommonBridge for the chain.
    function register(uint256 chainId, address commonBridge) external;

    /// @notice Deregisters a chain
    /// @param chainId The ID of the chain to deregister.
    function deregister(uint256 chainId) external;

    /// @notice Sends a message to a specified chain via its CommonBridge.
    /// @param chainId The ID of the destination chain.
    function sendMessage(uint256 chainId) external payable;

    /// @notice Verifies a message from a specified chain via its CommonBridge.
    /// @param chainId The ID of the source chain.
    /// @param l2MessageBatchNumber The batch number where the L2 message was emitted.
    /// @param l2MessageLeaf The leaf of the L2 message to verify.
    /// @param l2MessageProof The Merkle proof for the L2 message.
    function verifyMessage(
        uint256 chainId,
        uint256 l2MessageBatchNumber,
        bytes32 l2MessageLeaf,
        bytes32[] calldata l2MessageProof
    ) external view returns (bool);

    /// @notice Emitted when a new chain is registered.
    /// @param chainId The ID of the registered chain.
    /// @param commonBridge The address of the CommonBridge for the registered chain.
    event ChainRegistered(uint256 indexed chainId, address commonBridge);

    /// @notice Emitted when a chain is deregistered.
    /// @param chainId The ID of the deregistered chain.
    event ChainDeregistered(uint256 indexed chainId);


    /// @notice Emitted when a message is sent to a chain that is not registered.
    /// @param chainId The ID of the chain that is not registered.
    event TransferToChainNotRegistered(uint256 indexed chainId);

    /// @notice Error indicating an invalid address was provided.
    /// @param addr The invalid address.
    error InvalidAddress(address addr);

    /// @notice Error indicating a chain is already registered.
    /// @param chainId The ID of the already registered chain.
    error ChainAlreadyRegistered(uint256 chainId);

    /// @notice Error indicating a chain is not registered.
    /// @param chainId The ID of the not registered chain.
    error ChainNotRegistered(uint256 chainId);
}
