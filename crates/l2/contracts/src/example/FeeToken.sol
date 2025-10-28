// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "../l2/interfaces/IFeeToken.sol";

contract FeeToken is ERC20, IFeeToken {
    uint256 public constant DEFAULT_MINT = 1_000_000 * (10 ** 18);
    address public immutable L1_TOKEN;
    address public constant BRIDGE = 0x000000000000000000000000000000000000FFff;
    address public constant FEE_COLLECTOR = address(0xffff);

    modifier onlyFeeCollector() {
        require(msg.sender == FEE_COLLECTOR, "Only fee collector");
        _;
    }

    modifier onlyBridge() {
        require(msg.sender == BRIDGE, "FeeToken: not authorized");
        _;
    }

    constructor(address l1Token) ERC20("FeeToken", "FEE") {
        L1_TOKEN = l1Token;
        _mint(msg.sender, DEFAULT_MINT);
    }

    // Mint a free amount for whoever
    // calls the function
    function freeMint() public {
        _mint(msg.sender, DEFAULT_MINT);
    }

    function l1Address() external view override(IERC20L2) returns (address) {
        return L1_TOKEN;
    }

    function crosschainMint(address destination, uint256 amount)
        external
        override(IERC20L2)
        onlyBridge
    {
        _mint(destination, amount);
    }

    function crosschainBurn(address from, uint256 value)
        external
        override(IERC20L2)
        onlyBridge
    {
        _burn(from, value);
    }

    function lockFee(address payer, uint256 amount)
        external
        override(IFeeToken)
        onlyFeeCollector
    {
        _transfer(payer, FEE_COLLECTOR, amount);
    }

    function payFee(address receiver, uint256 amount)
        external
        override(IFeeToken)
        onlyFeeCollector
    {
        if (receiver == address(0)) {
            _burn(FEE_COLLECTOR, amount);
        } else {
            _transfer(FEE_COLLECTOR, receiver, amount);
        }
    }
}
