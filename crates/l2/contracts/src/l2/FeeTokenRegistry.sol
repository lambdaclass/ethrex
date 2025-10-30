// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;
import "./interfaces/IFeeTokenRegistry.sol";

contract FeeTokenRegistry is IFeeTokenRegistry {
    address internal constant ADMIN =
        0x000000000000000000000000000000000000f000;

    mapping(address => bool) private feeTokens;

    constructor(address[] memory initialTokens) {
        uint256 length = initialTokens.length;
        for (uint256 i = 0; i < length; ++i) {
            address token = initialTokens[i];
            require(token != address(0), "FeeTokenRegistry: zero address");
            require(!feeTokens[token], "FeeTokenRegistry: duplicate token");
            feeTokens[token] = true;
            emit FeeTokenRegistered(token);
        }
    }

    function isFeeToken(address token) external view override returns (bool) {
        return feeTokens[token];
    }

    function registerFeeToken(address token) external override onlyAdmin {
        require(token != address(0), "FeeTokenRegistry: zero address");
        require(!feeTokens[token], "Token already registered");
        feeTokens[token] = true;
        emit FeeTokenRegistered(token);
    }

    function unregisterFeeToken(address token) external override onlyAdmin {
        require(feeTokens[token], "Token not registered");
        feeTokens[token] = false;
        emit FeeTokenUnregistered(token);
    }

    modifier onlyAdmin() {
        require(msg.sender == ADMIN, "FeeTokenRegistry: not admin");
        _;
    }
}
