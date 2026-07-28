// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IERC20Min {
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address) external view returns (uint256);
}

/// @notice Constant-product pool over two tokens, with no slippage protection of its own.
///         Omitting `amountOutMin` is deliberate: it isolates the POST_TX assertion as the
///         only thing standing between the victim and a bad fill, which is what the
///         sandwich scenario is meant to demonstrate.
contract MockAMM {
    IERC20Min public immutable tokenA;
    IERC20Min public immutable tokenB;

    constructor(IERC20Min a, IERC20Min b) {
        tokenA = a;
        tokenB = b;
    }

    function reserves() public view returns (uint256 ra, uint256 rb) {
        ra = tokenA.balanceOf(address(this));
        rb = tokenB.balanceOf(address(this));
    }

    function quote(uint256 amountIn, bool aToB) public view returns (uint256) {
        (uint256 ra, uint256 rb) = reserves();
        (uint256 rIn, uint256 rOut) = aToB ? (ra, rb) : (rb, ra);
        return (amountIn * rOut) / (rIn + amountIn);
    }

    /// @dev No minimum-output parameter, by design (see the contract comment).
    function swap(uint256 amountIn, bool aToB) external returns (uint256 out) {
        (IERC20Min tIn, IERC20Min tOut) = aToB ? (tokenA, tokenB) : (tokenB, tokenA);
        out = quote(amountIn, aToB);
        tIn.transferFrom(msg.sender, address(this), amountIn);
        tOut.transfer(msg.sender, out);
    }
}

/// @notice A price feed an owner can move, standing in for an oracle whose value shifts
///         between the moment a user is shown a quote and the moment their transaction
///         executes.
contract MockOracle {
    uint256 public price;    // slot 0 — assertions read this slot directly
    address public owner;    // slot 1

    constructor(uint256 initial) {
        price = initial;
        owner = msg.sender;
    }

    function set(uint256 p) external {
        require(msg.sender == owner, "not owner");
        price = p;
    }
}

/// @notice Sells a token at whatever the oracle currently says, so a stale or manipulated
///         price is paid by whoever transacts after the move.
contract OracleDesk {
    IERC20Min public immutable token;
    MockOracle public immutable oracle;

    constructor(IERC20Min t, MockOracle o) {
        token = t;
        oracle = o;
    }

    /// @notice Pay `units` of token and receive `units * 1e18 / price` of token back.
    ///         A higher price at execution time means a worse fill for the buyer.
    function buy(uint256 units) external returns (uint256 out) {
        out = (units * 1e18) / oracle.price();
        token.transferFrom(msg.sender, address(this), units);
        token.transfer(msg.sender, out);
    }
}
