// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import "../interfaces/ISequencerRegistry.sol";
import "./interfaces/IOnChainProposer.sol";

contract SequencerRegistry is
    ISequencerRegistry,
    Initializable,
    UUPSUpgradeable,
    OwnableUpgradeable
{
    uint256 public constant MIN_COLLATERAL = 1 ether;

    uint256 public constant BATCHES_PER_SEQUENCER = 32;

    address public ON_CHAIN_PROPOSER;

    mapping(address => uint256) public collateral;
    address[] public sequencers;
    mapping(uint256 => address) public sequencerForBatch;

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

    function pushSequencer(uint256 batchNumber, address sequencer) external {
        require(
            msg.sender == ON_CHAIN_PROPOSER,
            "SequencerRegistry: Only onChainProposer can push sequencer"
        );
        sequencerForBatch[batchNumber] = sequencer;
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

    function leaderSequencer() public view returns (address) {
        uint256 _currentBatch = IOnChainProposer(ON_CHAIN_PROPOSER)
            .lastCommittedBatch() + 1;
        return leadSequencerForBatch(_currentBatch);
    }

    function leadSequencerForBatch(
        uint256 batchNumber
    ) public view returns (address) {
        uint256 _currentBatch = IOnChainProposer(ON_CHAIN_PROPOSER)
            .lastCommittedBatch() + 1;
        if (batchNumber < _currentBatch) {
            return sequencerForBatch[batchNumber];
        }
        uint256 _sequencers = sequencers.length;

        if (_sequencers == 0) {
            return address(0);
        }

        uint256 _id = batchNumber / BATCHES_PER_SEQUENCER;

        address _leader = sequencers[_id % _sequencers];

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
    }

    function _validateUnregisterRequest(address sequencer) internal view {
        require(collateral[sequencer] > 0, "SequencerRegistry: Not registered");
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}
}
