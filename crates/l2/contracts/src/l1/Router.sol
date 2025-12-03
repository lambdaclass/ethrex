// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {IRouter} from "./interfaces/IRouter.sol";
import {ICommonBridge} from "./interfaces/ICommonBridge.sol";

/// @title Router contract.
/// @author LambdaClass
contract Router is
    IRouter,
    Initializable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable,
    PausableUpgradeable
{
    mapping(uint256 chainId => address bridge) public bridges;

    uint256[] public registeredChainIds;

    mapping(address => bool) public registeredAddresses;

    function initialize(address owner) public initializer {
        OwnableUpgradeable.__Ownable_init(owner);
    }

    /// @inheritdoc IRouter
    function register(
        uint256 chainId,
        address _commonBridge
    ) public onlyOwner whenNotPaused {
        if (_commonBridge == address(0)) {
            revert InvalidAddress(address(0));
        }

        if (bridges[chainId] != address(0)) {
            revert ChainAlreadyRegistered(chainId);
        }

        bridges[chainId] = _commonBridge;
        registeredChainIds.push(chainId);
        registeredAddresses[_commonBridge] = true;

        emit ChainRegistered(chainId, _commonBridge);
    }

    /// @inheritdoc IRouter
    function deregister(uint256 chainId) public onlyOwner whenNotPaused {
        if (bridges[chainId] == address(0)) {
            revert ChainNotRegistered(chainId);
        }

        address bridge = bridges[chainId];
        delete bridges[chainId];
        removeChainID(chainId);
        registeredAddresses[bridge] = false;

        emit ChainDeregistered(chainId);
    }

    /// @inheritdoc IRouter
    function sendMessages(
        uint256 chainId,
        bytes32[] memory message_hashes
    ) public payable override {
        if (!registeredAddresses[msg.sender]) {
            revert CallerNotBridge(msg.sender);
        }
        if (bridges[chainId] == address(0)) {
            revert TransferToChainNotRegistered(chainId);
        }

        ICommonBridge(bridges[chainId]).receiveFromSharedBridge{
            value: msg.value
        }(chainId, message_hashes);
    }

    function removeChainID(uint256 chainId) internal {
        for (uint i = 0; i < registeredChainIds.length; i++) {
            if (registeredChainIds[i] == chainId) {
                registeredChainIds[i] = registeredChainIds[
                    registeredChainIds.length - 1
                ];
                registeredChainIds.pop();
                return;
            }
        }
    }

    /// @inheritdoc IRouter
    function getRegisteredChainIds()
        external
        view
        override
        returns (uint256[] memory)
    {
        return registeredChainIds;
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}

    function pause() external onlyOwner {
        _pause();
    }

    function unpause() external onlyOwner {
        _unpause();
    }
}
