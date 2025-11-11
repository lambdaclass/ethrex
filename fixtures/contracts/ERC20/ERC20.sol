import "./deps.sol";

pragma solidity ^0.8.0;

contract TestToken is ERC20 {

    uint256 constant defaultMint = 1000000 * (10**18);

    constructor() ERC20("TestToken", "TEST") {
        _mint(msg.sender, defaultMint);
    }

    // Mint a free amount for whoever
    // calls the function
    function freeMint() public {
        _mint(msg.sender, defaultMint);
    }
}
