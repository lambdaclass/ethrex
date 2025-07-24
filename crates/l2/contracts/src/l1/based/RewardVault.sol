// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import "./interfaces/IOnChainProposer.sol";

contract RewardVault is Initializable {
    IOnChainProposer public onChainProposer;
    // It must implement IERC20 and IERC20Metadata
    address public rewardToken;

    // TODO: we should replace this value with a mechanism that changes over time.
    uint256 public tokensUnlockedPerDay;

    bool public claimed;

    function initialize(address _onChainProposer /* address _rewardToken */) public initializer {
        onChainProposer = IOnChainProposer(_onChainProposer);
        // rewardToken = IERC20(_rewardToken);
        // TODO: change this value to a mechanism that changes over time.
        tokensUnlockedPerDay = 1_000_000;
        claimed = false;
    }


    /// @notice Claims rewards for a list of batches.
    /// @param _batchNumbers The list of batch numbers to claim rewards for.
    function claimRewards(uint256[] calldata _batchNumbers) external {
        // TODO: should we prevent a prover from claiming many times per day?
        address sender = msg.sender;

        uint256 numberOfBatches = _batchNumbers.length;
        uint256 gasProvenByClaimer = 0;
        for (uint256 i = 0; i < numberOfBatches; i++) {
            uint256 batchNumber = _batchNumbers[i];
            VerifiedBatchInfo memory verifiedBatchInfo = onChainProposer.verifiedBatches(batchNumber);
            require(verifiedBatchInfo.prover == sender, "Sender is not the prover");
            gasProvenByClaimer += verifiedBatchInfo.gasProven;
        }

        // calculate the rewards for the prover and transfer them
        uint256 totalGasProven = onChainProposer.getTotalGasProven();
        uint256 dailyRewardPool = tokensUnlockedPerDay * 10 ** IERC20Metadata(rewardToken).decimals();
        uint256 totalRewards = dailyRewardPool * gasProvenByClaimer / totalGasProven;

        // TODO: mark the claimer as having claimed
        // to prevent double claiming.
        
        // rewardToken.transfer(sender, totalRewards);
        claimed = true;
    }
}
