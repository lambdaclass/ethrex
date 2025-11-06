// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;
import "./interfaces/IFeeTokenRegistry.sol";

contract FeeTokenRegistry is IFeeTokenRegistry {
    address internal constant BRIDGE =
        address(0xffff);

		mapping(address => bool) private feeTokens;

		/// @inheritdoc IFeeTokenRegistry
    function isFeeToken(address token) external view override returns (bool) {
        return feeTokens[token];
    }

		/// @inheritdoc IFeeTokenRegistry
    function registerFeeToken(address token) external override onlyBridge {
        require(token != address(0), "FeeTokenRegistry: zero address");
        require(!feeTokens[token], "Token already registered");
        feeTokens[token] = true;
        emit FeeTokenRegistered(token);
    }

		/// @inheritdoc IFeeTokenRegistry
    function unregisterFeeToken(address token) external override onlyBridge {
        require(feeTokens[token], "Token not registered");
        feeTokens[token] = false;
        emit FeeTokenUnregistered(token);
		}

		modifier onlyBridge() {
        require(msg.sender == BRIDGE, "FeeTokenRegistry: not bridge");
        _;
    }

}
