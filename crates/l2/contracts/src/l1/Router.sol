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

        emit ChainRegistered(chainId, _commonBridge);
    }

    /// @inheritdoc IRouter
    function deregister(uint256 chainId) public onlyOwner whenNotPaused {
        if (bridges[chainId] == address(0)) {
            revert ChainNotRegistered(chainId);
        }

        delete bridges[chainId];
        removeChainID(chainId);

        emit ChainDeregistered(chainId);
    }

    /// @inheritdoc IRouter
    function sendMessage(uint256 chainId) public payable override {
        if (bridges[chainId] == address(0)) {
            emit TransferToChainNotRegistered(chainId);
        } else {
            ICommonBridge(bridges[chainId]).receiveFromSharedBridge{value: msg.value}();
        }
    }

    /// @inheritdoc IRouter
    function verifyMessage(
        uint256 chainId,
        uint256 l2MessageBatchNumber,
        bytes32 l2MessageLeaf,
        bytes32[] calldata l2MessageProof
    ) external view returns (bool) {
        address bridge = bridges[chainId];
        if (bridge == address(0)) {
            revert ChainNotRegistered(chainId);
        }
        return
            ICommonBridge(bridge).verifyMessage(
                l2MessageLeaf,
                l2MessageBatchNumber,
                l2MessageProof
            );
    }

    function removeChainID(uint256 chainId) internal {
        for (uint i = 0; i < registeredChainIds.length; i++) {
            if (registeredChainIds[i] == chainId) {
                registeredChainIds[i] = registeredChainIds[registeredChainIds.length - 1];
                registeredChainIds.pop();
                return;
            }
        }
    }

    function getRegisteredChainIds() external view returns (uint256[] memory) {
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
