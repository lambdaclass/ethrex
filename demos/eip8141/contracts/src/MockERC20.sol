// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title MockERC20
/// @author Lambda Class
/// @notice Minimal ERC20 token for demo purposes. Has no access control on minting.
contract MockERC20 {
    /// @notice Token balances by address.
    mapping(address => uint256) public balanceOf;

    /// @notice Emitted when tokens are transferred.
    /// @param from The sender address
    /// @param to The recipient address
    /// @param amount The amount of tokens transferred
    event Transfer(address indexed from, address indexed to, uint256 amount);

    /// @notice Transfers tokens from the caller to another address.
    /// @param to The recipient address
    /// @param amount The amount of tokens to transfer
    /// @return success Always true if the transfer succeeds
    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "MockERC20: insufficient balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
        return true;
    }

    /// @notice Mints tokens to the specified address. No access control (demo only).
    /// @param to The recipient address
    /// @param amount The amount of tokens to mint
    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }
}
