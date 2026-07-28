// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title MockERC20
/// @notice Minimal ERC-20 for the attack scenarios. Deliberately not a full
///         implementation — it has exactly the surface the scenarios exercise.
///
/// @dev STORAGE LAYOUT IS PART OF THE INTERFACE. Assertion guards derive storage slots
///      from these base slots to read balances and allowances via TXDIFF, so the layout
///      must not be reordered without updating the guards' callers.
///        slot 0: totalSupply
///        slot 1: balanceOf   mapping(address => uint256)
///        slot 2: allowance   mapping(address => mapping(address => uint256))
///
///      balance slot    = keccak256(abi.encode(owner, uint256(1)))
///      allowance slot  = keccak256(abi.encode(spender, keccak256(abi.encode(owner, uint256(2)))))
contract MockERC20 {
    uint256 public totalSupply;                                          // slot 0
    mapping(address => uint256) public balanceOf;                        // slot 1
    mapping(address => mapping(address => uint256)) public allowance;    // slot 2

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    error InsufficientBalance();
    error InsufficientAllowance();

    uint256 public constant BALANCE_BASE_SLOT = 1;
    uint256 public constant ALLOWANCE_BASE_SLOT = 2;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
        totalSupply += amount;
        emit Transfer(address(0), to, amount);
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        _move(msg.sender, to, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        if (allowed < amount) revert InsufficientAllowance();
        if (allowed != type(uint256).max) allowance[from][msg.sender] = allowed - amount;
        _move(from, to, amount);
        return true;
    }

    function _move(address from, address to, uint256 amount) private {
        uint256 bal = balanceOf[from];
        if (bal < amount) revert InsufficientBalance();
        balanceOf[from] = bal - amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
    }
}
