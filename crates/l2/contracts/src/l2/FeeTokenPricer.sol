// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;
import "./interfaces/IFeeTokenPricer.sol";

contract FeeTokenPricer is IFeeTokenPricer {
    address public constant BRIDGE = 0x000000000000000000000000000000000000FFff;

    mapping(address => uint256) private ratios;

    modifier onlyBridge() {
        require(msg.sender == BRIDGE, "FeeTokenPricer: not bridge");
        _;
    }

    /// @inheritdoc IFeeTokenPricer
    function getFeeTokenRatio(
        address feeToken
    ) external view override returns (uint256) {
        require(
            ratios[feeToken] != 0,
            "FeeTokenPricer: token has not set a correct ratio"
        );
        return ratios[feeToken];
    }

    /// @inheritdoc IFeeTokenPricer
    function setFeeTokenRatio(
        address feeToken,
        uint256 ratio
    ) external override onlyBridge {
        require(
            feeToken != address(0),
            "FeeTokenPricer: address cannot be zero"
        );
        require(ratio != 0, "FeeTokenPricer: ratio cannot be zero");
        ratios[feeToken] = ratio;
        emit FeeTokenRatioSet(feeToken, ratio);
    }

    /// @inheritdoc IFeeTokenPricer
    function unsetFeeTokenRatio(address feeToken) external override onlyBridge {
        require(ratios[feeToken] != 0, "FeeTokenPricer: token already unset");
        ratios[feeToken] = uint256(0);
        emit FeeTokenRatioUnset(feeToken);
    }
}
