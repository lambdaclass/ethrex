import "./deps.sol";

pragma solidity ^0.8.0;

contract TestToken is ERC20 {
    uint256 constant defaultMint = 1000000 * (10 ** 18);

    constructor() ERC20("TestToken", "TEST") {
        _mint(msg.sender, defaultMint);
    }

    // Mint a free amount for whoever
    // calls the function
    function freeMint() public {
        _mint(msg.sender, defaultMint);
    }

    function lockFee(address payer, uint256 amount) internal {
        // onlyFeeCollector
        IERC20(address(this)).transferFrom(payer, address(this), amount);
    }

    function payFee(address receiver, uint256 amount) public {
        IERC20(address(this)).transferFrom(address(this), receiver, amount);
    }
}
