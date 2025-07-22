// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "./interfaces/IOnChainProposer.sol";

contract RewardVault {
    address public onChainProposer;
    address public rewardToken;

    // TODO: we should replace this value with a mechanism that changes over time.
    uint public tokensUnlockedPerDay;

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
        uint256 gasProvenByClaimer = 0;
        for (uint256 i = 0; i < numberOfBatches; i++) {
            uint256 batchNumber = _batchNumbers[i];
            (address proverAddress, uint256 gasProven) = prover.verifiedBatches(batchNumber);
            require(proverAddress == sender, "Sender is not the prover");
            gasProvenByClaimer += gasProven;
        }

        // calculate the rewards for the prover and transfer them
        uint256 totalGasProven = prover.getTotalGasProven();
        uint256 dailyRewardPool = tokensUnlockedPerDay * 10 ** IERC20(rewardToken).decimals();
        uint256 totalRewards = dailyRewardPool * gasProvenByClaimer / totalGasProven;

        IERC20(rewardToken).transfer(sender, totalRewards);
    }
}
