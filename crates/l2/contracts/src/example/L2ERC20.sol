// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "../l2/interfaces/IERC20L2.sol";

/// @title OnChainProposer contract.
/// @author LambdaClass
contract TestTokenL2 is ERC20, IERC20L2 {
    address public L1_TOKEN = address(0);
    address public constant BRIDGE =  0x000000000000000000000000000000000000FFff;

    constructor(address l1Addr) ERC20("TestTokenL2", "TEST") {
        L1_TOKEN = l1Addr;
    }

    function l1Address() external view returns (address) {
        return L1_TOKEN;
    }
    function mint(address destination, uint256 amount) external {
        require(msg.sender == BRIDGE, "TestToken: not authorized to mint");
        _mint(destination, amount);
    }
}
