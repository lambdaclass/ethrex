// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "../l2/interfaces/IFeeToken.sol";

contract FeeToken is ERC20, IFeeToken {
    /// @dev Amount minted on construction and via `freeMint`.
    uint256 public constant DEFAULT_MINT = 1_000_000 * (10 ** 18);
    /// @dev Canonical L1 counterpart so the bridge can validate cross-chain state.
    address public immutable L1_TOKEN;
    /// @dev Hardcoded bridge contract allowed to mint/burn/lock fees on L2.
    address public constant BRIDGE = 0x000000000000000000000000000000000000FFff;

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

    function crosschainMint(
        address destination,
        uint256 amount
    ) external override(IERC20L2) onlyBridge {
        _mint(destination, amount);
    }

    function crosschainBurn(
        address from,
        uint256 value
    ) external override(IERC20L2) onlyBridge {
        _burn(from, value);
    }

    function lockFee(
        address payer,
        uint256 amount
    ) external override(IFeeToken) onlyBridge {
        _transfer(payer, BRIDGE, amount);
    }

    function payFee(
        address receiver,
        uint256 amount
    ) external override(IFeeToken) onlyBridge {
        if (receiver == address(0)) {
            _burn(BRIDGE, amount);
        } else {
            _transfer(BRIDGE, receiver, amount);
        }
    }
}
