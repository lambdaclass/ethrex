import "./deps.sol";
import "./Print.sol";

pragma solidity ^0.8.0;

contract FeeToken is ERC20 {
    uint256 constant defaultMint = 1000000 * (10 ** 18);

    modifier onlyFeeCollector() {
        require(msg.sender == address(0xffff), "Only fee collector");
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
        print(string("locking fee"));
        _transfer(payer, address(this), amount);
    }

    function payFee(address receiver, uint256 amount) public onlyFeeCollector {
        // this does not check for the receiver to be the address zero as it is burning the fees
        print(address(receiver));
        if (receiver == address(0)) {
            print(string("adentro if"));
            _burn(address(this), amount);
        } else {
            print(string("afuera if"), address(receiver));
            _transfer(address(this), receiver, amount);
        }
    }
}
