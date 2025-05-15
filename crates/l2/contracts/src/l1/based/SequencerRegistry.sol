// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import "../interfaces/ISequencerRegistry.sol";
import "../interfaces/IOnChainProposer.sol";

contract SequencerRegistry is
    ISequencerRegistry,
    Initializable,
    UUPSUpgradeable,
    OwnableUpgradeable
{
    uint256 public constant MIN_COLLATERAL = 1 ether;
    uint256 public constant MAX_COLLATERAL = 100 ether;

    address public ON_CHAIN_PROPOSER;

    mapping(address => uint256) public collateral;
    address[] public sequencers;

    function initialize(
        address owner,
        address onChainProposer
    ) public initializer {
        require(
            onChainProposer != address(0),
            "SequencerRegistry: Invalid onChainProposer"
        );
        ON_CHAIN_PROPOSER = onChainProposer;

        _validateOwner(owner);
        OwnableUpgradeable.__Ownable_init(owner);
    }

    function register(address sequencer) public payable {
        _validateRegisterRequest(sequencer, msg.value);

        collateral[sequencer] = msg.value;
        sequencers.push(sequencer);

        emit SequencerRegistered(sequencer, msg.value);
    }

    function unregister(address sequencer) public {
        _validateUnregisterRequest(sequencer);

        uint256 amount = collateral[sequencer];
        collateral[sequencer] = 0;
        for (uint256 i = 0; i < sequencers.length; i++) {
            if (sequencers[i] == sequencer) {
                sequencers[i] = sequencers[sequencers.length - 1];
                sequencers.pop();
                break;
            }
        }

        payable(sequencer).transfer(amount);

        emit SequencerUnregistered(sequencer);
    }

    function isRegistered(address sequencer) public view returns (bool) {
        return collateral[sequencer] >= MIN_COLLATERAL;
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

    function leaderSequencer() public view returns (address) {
        return futureLeaderSequencer(0);
    }

    function futureLeaderSequencer(
        uint256 nBatchesInTheFuture
    ) public view returns (address) {
        uint256 _sequencers = sequencers.length;

        if (_sequencers == 0) {
            return address(0);
        }

        uint256 _currentBatch = IOnChainProposer(ON_CHAIN_PROPOSER)
            .lastCommittedBatch() + 1;

        uint256 _targetBatch = _currentBatch + nBatchesInTheFuture;

        address _leader = sequencers[_targetBatch % _sequencers];

        return _leader;
    }

    function _validateOwner(address potentialOwner) internal pure {
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
