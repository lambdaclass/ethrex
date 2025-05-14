// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import "../interfaces/ISequencerRegistry.sol";

contract SequencerRegistry is
    ISequencerRegistry,
    Initializable,
    UUPSUpgradeable,
    OwnableUpgradeable
{
    uint256 constant MIN_COLLATERAL = 1 ether;
    uint256 constant MAX_COLLATERAL = 100 ether;

    mapping(address => uint256) public collateral;

    function initialize(address owner) public initializer {
        _validateOwner(owner);
        OwnableUpgradeable.__Ownable_init(owner);
    }

    function register(address sequencer) public payable {
        _validateRegisterRequest(sequencer, msg.value);

        collateral[sequencer] = msg.value;

        emit SequencerRegistered(sequencer, msg.value);
    }

    function unregister(address sequencer) public {
        _validateUnregisterRequest(sequencer);

        uint256 amount = collateral[sequencer];
        collateral[sequencer] = 0;

        payable(sequencer).transfer(amount);

        emit SequencerUnregistered(sequencer);
    }

    function isRegistered(address sequencer) public view returns (bool) {
        return collateral[sequencer] > MIN_COLLATERAL;
    }

    function increaseCollateral(address sequencer) public payable {
        _validateCollateralIncreaseRequest(sequencer, msg.value);

        collateral[sequencer] += msg.value;

        emit CollateralIncreased(sequencer, msg.value);
    }

    function decreaseCollateral(address sequencer, uint256 amount) public {
        _validateCollateralDecreaseRequest(sequencer, amount);

        collateral[sequencer] -= amount;

        payable(sequencer).transfer(amount);

        emit CollateralDecreased(sequencer, amount);
    }

    function _validateOwner(address potentialOwner) internal view {
        require(
            potentialOwner != address(0),
            "SequencerRegistry: Invalid owner"
        );
    }

    function _validateRegisterRequest(
        address sequencer,
        uint256 amount
    ) internal view {
        require(
            collateral[sequencer] == 0,
            "SequencerRegistry: Already registered"
        );
        require(
            amount >= MIN_COLLATERAL,
            "SequencerRegistry: Insufficient collateral"
        );
        require(
            amount <= MAX_COLLATERAL,
            "SequencerRegistry: Excessive collateral"
        );
    }

    function _validateUnregisterRequest(address sequencer) internal view {
        require(collateral[sequencer] > 0, "SequencerRegistry: Not registered");
    }

    function _validateCollateralIncreaseRequest(
        address sequencer,
        uint256 amount
    ) internal view {
        require(collateral[sequencer] > 0, "SequencerRegistry: Not registered");
        require(
            amount > 0,
            "SequencerRegistry: Collateral amount must be greater than zero"
        );
    }

    function _validateCollateralDecreaseRequest(
        address sequencer,
        uint256 amount
    ) internal view {
        require(collateral[sequencer] > 0, "SequencerRegistry: Not registered");
        require(
            amount > 0,
            "SequencerRegistry: Collateral amount must be greater than zero"
        );
        require(
            collateral[sequencer] - amount >= MIN_COLLATERAL,
            "SequencerRegistry: Cannot decrease collateral below minimum"
        );
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}
}
