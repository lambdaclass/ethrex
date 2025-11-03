// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;
import "./interfaces/IFeeTokenPricer.sol";

contract FeeTokenPricer is IFeeTokenPricer {  
	address internal constant FEE_TOKEN =
        0xb7E811662Fa10ac068aeE115AC2e682821630535;
	
  /// @inheritdoc IFeeTokenPricer
  function getFeeTokenRatio(
      address fee_token
  ) external view override returns (uint256) {
		require(fee_token == FEE_TOKEN, "The fee token does not match with the one set for the chain");
		return 2;
  }	
}
