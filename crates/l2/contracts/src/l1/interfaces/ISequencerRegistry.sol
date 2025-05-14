// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

interface ISequencerRegistry {
    event SequencerRegistered(
        address indexed sequencer,
        uint256 collateralAmount
    );

    event SequencerUnregistered(address indexed sequencer);

    event CollateralIncreased(address indexed sequencer, uint256 amount);

    event CollateralDecreased(address indexed sequencer, uint256 amount);

    function register(address sequencer) external payable;

    function unregister(address sequencer) external;

    function isRegistered(address sequencer) external view returns (bool);

    function increaseCollateral(address sequencer) external payable;

    function decreaseCollateral(address sequencer, uint256 amount) external;
}
