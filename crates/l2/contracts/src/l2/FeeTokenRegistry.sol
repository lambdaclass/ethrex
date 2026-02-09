
// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;
import "./interfaces/IFeeTokenRegistry.sol";

contract FeeTokenRegistry is IFeeTokenRegistry {
    address public constant BRIDGE = 0x000000000000000000000000000000000000FFff;

    mapping(address => bool) private feeTokens;

    modifier onlyBridge() {
        require(msg.sender == BRIDGE, "FeeTokenRegistry: not bridge");
        _;
    }

    /// @inheritdoc IFeeTokenRegistry
    function isFeeToken(address token) external view override returns (bool) {
        return feeTokens[token];
    }

    /// @inheritdoc IFeeTokenRegistry
    function registerFeeToken(address token) external override onlyBridge {
        require(token != address(0), "FeeTokenRegistry: zero address");
        require(
            !feeTokens[token],
            "FeeTokenRegistry: token already registered"
        );
        feeTokens[token] = true;
        emit FeeTokenRegistered(token);
    }

    /// @inheritdoc IFeeTokenRegistry
    function unregisterFeeToken(address token) external override onlyBridge {
        require(feeTokens[token], "FeeTokenRegistry: token not registered");
        feeTokens[token] = false;
        emit FeeTokenUnregistered(token);
    }
}
