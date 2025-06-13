// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "../interfaces/ISequencerRegistry.sol";
import "./interfaces/IOnChainProposer.sol";

contract SequencerRegistry is
    ISequencerRegistry,
    Initializable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable
{
    uint256 public constant MIN_COLLATERAL = 1 ether;

    uint256 public constant BATCHES_PER_SEQUENCER = 32;

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

        require(
            potentialOwner != address(0),
            "SequencerRegistry: Invalid owner"
        );

        OwnableUpgradeable.__Ownable_init(owner);
    }

    function register(address sequencer) public payable {
        require(
            collateral[sequencer] == 0,
            "SequencerRegistry: Already registered"
        );
        require(
            amount >= MIN_COLLATERAL,
            "SequencerRegistry: Insufficient collateral"
        );

        collateral[sequencer] = msg.value;
        sequencers.push(sequencer);

        emit SequencerRegistered(sequencer, msg.value);
    }

    function unregister(address sequencer) public {
        require(collateral[sequencer] > 0, "SequencerRegistry: Not registered");

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

        uint256 _id = _targetBatch / BATCHES_PER_SEQUENCER;

        address _leader = sequencers[_id % _sequencers];

        return _leader;
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}
}
