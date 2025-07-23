// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "./interfaces/IOnChainProposer.sol";

contract RewardVault {
    IOnChainProposer public onChainProposer;
    IERC20 public rewardToken;

    // TODO: we should replace this value with a mechanism that changes over time.
    uint256 public tokensUnlockedPerDay;

    constructor(address _onChainProposer, address _rewardToken, uint256 _tokensUnlockedPerDay) {
        onChainProposer = IOnChainProposer(_onChainProposer);
        rewardToken = IERC20(_rewardToken);
        // TODO: remove this once the mechanism is in place.
        tokensUnlockedPerDay = _tokensUnlockedPerDay;
    }


    /// @notice Claims rewards for a list of batches.
    /// @param _batchNumbers The list of batch numbers to claim rewards for.
    function claimRewards(uint256[] calldata _batchNumbers) public {
        // TODO: should we prevent a prover from claiming many times per day?
        address sender = msg.sender;

        uint256 numberOfBatches = _batchNumbers.length;
        uint256 gasProvenByClaimer = 0;
        for (uint256 i = 0; i < numberOfBatches; i++) {
            uint256 batchNumber = _batchNumbers[i];
            (address proverAddress, uint256 gasProven) = onChainProposer.verifiedBatches(batchNumber);
            require(proverAddress == sender, "Sender is not the prover");
            gasProvenByClaimer += gasProven;
        }

        // calculate the rewards for the prover and transfer them
        uint256 totalGasProven = onChainProposer.getTotalGasProven();
        uint256 dailyRewardPool = tokensUnlockedPerDay * 10 ** rewardToken.decimals();
        uint256 totalRewards = dailyRewardPool * gasProvenByClaimer / totalGasProven;

        rewardToken.transfer(sender, totalRewards);
    }
}
