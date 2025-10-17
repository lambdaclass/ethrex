// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import { ICommonBridge } from "./ICommonBridge.sol";

interface IRouter {
    /// @notice Struct containing information about a registered chain.
    /// @param onChainProposer The address of the OnChainProposer for the chain.
    /// @param commonBridge The address of the CommonBridge for the chain.
    struct ChainInfo {
        address onChainProposer;
        address commonBridge;
    }

    /// @notice Returns the address of the CommonBridge for a given chain ID.
    /// @param chainId The ID of the chain.
    function bridge(uint256 chainId) external view returns (address);

    /// @notice Returns the address of the OnChainProposer for a given chain ID.
    /// @param chainId The ID of the chain.
    function onChainProposer(uint256 chainId) external view returns (address);

    /// @notice Registers a new chain with its OnChainProposer and CommonBridge addresses.
    /// @param chainId The ID of the chain to register.
    /// @param onChainProposer The address of the OnChainProposer for the chain
    /// @param commonBridge The address of the CommonBridge for the chain.
    function register(uint256 chainId, address onChainProposer, address commonBridge) external;

    /// @notice Deregisters a chain
    /// @param chainId The ID of the chain to deregister.
    function deregister(uint256 chainId) external;

    /// @notice Sends a message to a specified chain via its CommonBridge.
    /// @param chainId The ID of the destination chain.
    /// @param message The message details to send.
    function sendMessage(uint256 chainId, ICommonBridge.SendValues calldata message) external payable;

    /// @notice Emitted when a new chain is registered.
    /// @param chainId The ID of the registered chain.
    /// @param onChainProposer The address of the OnChainProposer for the registered chain.
    /// @param commonBridge The address of the CommonBridge for the registered chain.
    event ChainRegistered(uint256 indexed chainId, address onChainProposer, address commonBridge);

    /// @notice Emitted when a chain is deregistered.
    /// @param chainId The ID of the deregistered chain.
    event ChainDeregistered(uint256 indexed chainId);

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
