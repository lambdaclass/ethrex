// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import { IRouter } from "./interfaces/IRouter.sol";

/// @title Router contract.
/// @author LambdaClass
contract Router is
    IRouter,
    Initializable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable,
    PausableUpgradeable
{
    mapping(uint256 chainId => ChainInfo) public chains;

    function initialize(address owner) public initializer {
        OwnableUpgradeable.__Ownable_init(owner);
    }

    function bridge(uint256 chainId) public view returns (address) {
        address commonBridge = chains[chainId].commonBridge;

        if (commonBridge == address(0)) {
            revert ChainNotRegistered(chainId);
        }

        return commonBridge;
    }

    function onChainProposer(uint256 chainId) public view returns (address) {
        address _onChainProposer = chains[chainId].onChainProposer;

        if (_onChainProposer == address(0)) {
            revert ChainNotRegistered(chainId);
        }

        return _onChainProposer;
    }

    function register(uint256 chainId, address _onChainProposer, address _commonBridge) onlyOwner whenNotPaused public {
        if (_onChainProposer == address(0) || _commonBridge == address(0)) {
            revert InvalidAddress(address(0));
        }

        if (chains[chainId].onChainProposer != address(0)) {
            revert ChainAlreadyRegistered(chainId);
        }

        chains[chainId] = ChainInfo({
            onChainProposer: _onChainProposer,
            commonBridge: _commonBridge
        });

        emit ChainRegistered(chainId, _onChainProposer, _commonBridge);
    }

    function deregister(uint256 chainId) onlyOwner whenNotPaused public {
        if (chains[chainId].onChainProposer == address(0)) {
            revert ChainNotRegistered(chainId);
        }

        delete chains[chainId];

        emit ChainDeregistered(chainId);
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
