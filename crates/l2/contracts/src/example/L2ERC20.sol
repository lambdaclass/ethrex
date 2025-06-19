// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "../l2/interfaces/IERC20L2.sol";

/// @title OnChainProposer contract.
/// @author LambdaClass
contract TestTokenL2 is ERC20, IERC20L2 {
    address public constant L1_TOKEN = 0xD8DAF03ba8F2d664E6CE21735505a787c78D2179;
    address public constant BRIDGE =  0x000000000000000000000000000000000000FFff;

    constructor() ERC20("TestTokenL2", "TEST") {}

    function l1Address() external pure returns (address) {
        return L1_TOKEN;
    }
    function mint(address destination, uint256 amount) external {
        require(msg.sender == BRIDGE, "TestToken: not authorized to mint");
        _mint(destination, amount);
    }
}
