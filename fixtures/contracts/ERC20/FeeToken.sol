import "./deps.sol";

pragma solidity ^0.8.0;

contract FeeToken is ERC20 {
    uint256 constant defaultMint = 1000000 * (10 ** 18);
    address constant FEE_COLLECTOR = address(0xffff);

    modifier onlyFeeCollector() {
        require(msg.sender == FEE_COLLECTOR, "Only fee collector");
        _;
    }

    constructor() ERC20("FeeToken", "FEE") {
        _mint(msg.sender, defaultMint);
    }

    // Mint a free amount for whoever
    // calls the function
    function freeMint() public {
        _mint(msg.sender, defaultMint);
    }

    function lockFee(address payer, uint256 amount) public onlyFeeCollector {
        _transfer(payer, FEE_COLLECTOR, amount);
    }

    function payFee(address receiver, uint256 amount) public onlyFeeCollector {
        if (receiver == address(0)) {
            _burn(FEE_COLLECTOR, amount);
        } else {
            _transfer(FEE_COLLECTOR, receiver, amount);
        }
    }
}
