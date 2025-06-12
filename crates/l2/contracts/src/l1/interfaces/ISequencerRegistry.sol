// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

interface ISequencerRegistry {
    event SequencerRegistered(
        address indexed sequencer,
        uint256 collateralAmount
    );

    event SequencerUnregistered(address indexed sequencer);

    function register(address sequencer) external payable;

    function unregister(address sequencer) external;

    function isRegistered(address sequencer) external view returns (bool);

    function leaderSequencer() external view returns (address);

    function leadSequencerForBatch(
        uint256 batchNumber
    ) external view returns (address);

    function pushSequencer(uint256 batchNumber, address sequencer) external;
}
