// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

contract RewardVault {
    address public onChainProposer;
    address public rewardToken;

    constructor(address _onChainProposer, address _rewardToken) {
        onChainProposer = _onChainProposer;
        rewardToken = _rewardToken;
    }


    /// @notice Claims rewards for a list of batches.
    /// @param _batchNumbers The list of batch numbers to claim rewards for.
    function claimRewards(uint256[] calldata _batchNumbers) public {
        // TODO: should we prevent a prover from claiming many times per day?
        address sender = msg.sender;

        uint256 numberOfBatches = _batchNumbers.length;
        IOnChainProposer prover = IOnChainProposer(onChainProposer);
        uint256 totalGasProven = 0;
        for (uint256 i = 0; i < numberOfBatches; i++) {
            uint256 batchNumber = _batchNumbers[i];
            (address proverAddress, uint256 gasProven) = prover.verifiedBatches(batchNumber);
            require(proverAddress == sender, "Sender is not the prover");
            totalGasProven += gasProven;
        }

        // TODO: calculate the rewards for the prover
        // TODO: transfer the rewards to the prover
    }
}