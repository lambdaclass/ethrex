// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;
import "./interfaces/IFeeTokenRegistry.sol";

contract FeeTokenRegistry is IFeeTokenRegistry {
    address internal constant FEE_TOKEN =
        0xb7E811662Fa10ac068aeE115AC2e682821630535;

    function isFeeToken(address token) external pure override returns (bool) {
        return token == FEE_TOKEN;
    }

}
