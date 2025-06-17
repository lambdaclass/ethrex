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

    function futureLeaderSequencer(
        uint256 nBatchesInTheFuture
    ) external view returns (address);
}
