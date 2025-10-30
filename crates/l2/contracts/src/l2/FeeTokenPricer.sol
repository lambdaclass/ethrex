// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;
import "./interfaces/IFeeTokenPricer.sol";

contract FeeTokenPricer is IFeeTokenPricer {
    address internal constant ADMIN =
        0x000000000000000000000000000000000000f000;
    mapping(address => uint256) private feeTokenPerEthInWei;

    modifier onlyAdmin() {
        require(msg.sender == ADMIN, "FeeTokenPricer: not admin");
        _;
    }
    /// @inheritdoc IFeeTokenPricer
    function getFeeTokenRatio(
        address feeToken
    ) external view override returns (uint256) {
        return feeTokenPerEthInWei[feeToken];
    }

    /// @inheritdoc IFeeTokenPricer
    function setFeeTokenRatio(
        address feeToken,
        uint256 ratio
    ) external override onlyAdmin {
        feeTokenPerEthInWei[feeToken] = ratio;
    }
}
