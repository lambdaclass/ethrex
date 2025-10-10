// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

interface IRouter {
    struct ChainInfo {
        address onChainProposer;
        address commonBridge;
    }

    function bridge(uint256 chainId) external view returns (address);

    function onChainProposer(uint256 chainId) external view returns (address);

    function register(uint256 chainId, address onChainProposer, address commonBridge) external;

    function deregister(uint256 chainId) external;

    event ChainRegistered(uint256 indexed chainId, address onChainProposer, address commonBridge);

    event ChainDeregistered(uint256 indexed chainId);

    error InvalidAddress(address addr);

    error ChainAlreadyRegistered(uint256 chainId);

    error ChainNotRegistered(uint256 chainId);
}
