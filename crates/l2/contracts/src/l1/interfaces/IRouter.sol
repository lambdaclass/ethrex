// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import {ICommonBridge} from "./ICommonBridge.sol";

interface IRouter {
    /// @notice Registers a new chain with its OnChainProposer and CommonBridge addresses.
    /// @param chainId The ID of the chain to register.
    /// @param commonBridge The address of the CommonBridge for the chain.
    function register(uint256 chainId, address commonBridge) external;

    /// @notice Deregisters a chain
    /// @param chainId The ID of the chain to deregister.
    function deregister(uint256 chainId) external;

    /// @notice Sends messages to a specified chain via its CommonBridge.
    /// @param chainId The ID of the destination chain.
    /// @param message_hashes The hashes of the messages to be sent.
    function sendMessages(
        uint256 chainId,
        bytes32[] calldata message_hashes
    ) external payable;

    /// @notice Retrieves the list of registered chain IDs.
    function getRegisteredChainIds() external view returns (uint256[] memory);

    /// @notice Emitted when a new chain is registered.
    /// @param chainId The ID of the registered chain.
    /// @param commonBridge The address of the CommonBridge for the registered chain.
    event ChainRegistered(uint256 indexed chainId, address commonBridge);

    /// @notice Emitted when a chain is deregistered.
    /// @param chainId The ID of the deregistered chain.
    event ChainDeregistered(uint256 indexed chainId);

    /// @notice Emitted when a message is sent to a chain that is not registered.
    /// @param chainId The ID of the chain that is not registered.
    error TransferToChainNotRegistered(uint256 chainId);

    /// @notice Error indicating an invalid address was provided.
    /// @param addr The invalid address.
    error InvalidAddress(address addr);

    /// @notice Error indicating a chain is already registered.
    /// @param chainId The ID of the already registered chain.
    error ChainAlreadyRegistered(uint256 chainId);

    /// @notice Error indicating the caller is not a registered bridge.
    /// @param caller The address of the caller.
    error CallerNotBridge(address caller);

    /// @notice Error indicating a chain is not registered.
    /// @param chainId The ID of the chain that is not registered.
    error ChainNotRegistered(uint256 chainId);
}
