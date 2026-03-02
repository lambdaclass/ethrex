// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @title Interface for the SequencerRegistry contract.
/// @author LambdaClass
/// @notice A SequencerRegistry contract is a contract that manages the sequencers
/// of the L2. It allows sequencers to register and unregister themselves. Also,
/// ask for the leader sequencer for a batch.
interface ISequencerRegistry {
    /// @notice A sequencer has been registered.
    /// @dev Event emitted when a sequencer is registered.
    /// @param sequencer The address of the sequencer that was registered.
    /// @param collateralAmount The amount of eth that the sequencer has put as registration.
    event SequencerRegistered(
        address indexed sequencer,
        uint256 collateralAmount
    );

    /// @notice A sequencer has been unregistered.
    /// @dev Event emitted when a sequencer is unregistered.
    /// @dev The sequencer must have been previously registered.
    event SequencerUnregistered(address indexed sequencer);

    /// @notice Unregister a sequencer.
    /// @dev Unregister a sequencer providing the sequencer address.
    /// @dev The address must have been previously registered.
    function unregister(address sequencer) external;

    /// @notice Ask if a sequencer is registered.
    /// @dev Returns true if the sequencer is registered, false otherwise.
    function isRegistered(address sequencer) external view returns (bool);

    /// @notice Get the leader sequencer for the current batch.
    /// @dev Returns the address of the leader sequencer for the current batch.
    function leaderSequencer() external view returns (address);

    /// @notice A specific sequencer for a given batch.
    /// @dev Returns the address of the sequencer that is leading the batch.
    /// The batch number can be from the past or the future.
    function leadSequencerForBatch(
        uint256 batchNumber
    ) external view returns (address);

    /// @notice A sequencer has committed a batch.
    /// @dev Function that the onChainProposer calls once a sequencer has committed a batch.
    /// @dev This is to keep track of the sequencers in the past.
    function pushSequencer(uint256 batchNumber, address sequencer) external;
}
