// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "./interfaces/ICommonBridgeL2.sol";
import "./interfaces/IMessenger.sol";
import "./interfaces/IERC20L2.sol";

/// @title CommonBridge L2 contract.
/// @author LambdaClass
contract CommonBridgeL2 is ICommonBridgeL2 {
    address public constant L1_MESSENGER =
        0x000000000000000000000000000000000000FFFE;
    address public constant BURN_ADDRESS =
        0x0000000000000000000000000000000000000000;
    /// @notice Token address used to represent ETH
    address public constant ETH_TOKEN =
        0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    mapping(uint256 chainId => uint256 txId) public transactionIds;

    // Some calls come as a privileged transaction, whose sender is the bridge itself.
    modifier onlySelf() {
        require(
            msg.sender == address(this),
            "CommonBridgeL2: caller is not the bridge"
        );
        _;
    }

    function withdraw(address _receiverOnL1) external payable {
        require(msg.value > 0, "Withdrawal amount must be positive");

        (bool success, ) = BURN_ADDRESS.call{value: msg.value}("");
        require(success, "Failed to burn Ether");

        emit WithdrawalInitiated(msg.sender, _receiverOnL1, msg.value);

        IMessenger(L1_MESSENGER).sendMessageToL1(
            keccak256(
                abi.encodePacked(ETH_TOKEN, ETH_TOKEN, _receiverOnL1, msg.value)
            )
        );
    }

    function mintETH(address to) external payable {
        (bool success, ) = to.call{value: msg.value}("");
        if (!success) {
            this.withdraw{value: msg.value}(to);
        }
        emit DepositProcessed(to, msg.value);
    }

    function mintERC20(
        address tokenL1,
        address tokenL2,
        address destination,
        uint256 amount
    ) external onlySelf {
        (bool success, ) = address(this).call(
            abi.encodeCall(
                this.tryMintERC20,
                (tokenL1, tokenL2, destination, amount)
            )
        );
        if (!success) {
            _withdraw(tokenL1, tokenL2, destination, amount);
        }
        emit ERC20DepositProcessed(tokenL1, tokenL2, destination, amount);
    }

    function tryMintERC20(
        address tokenL1,
        address tokenL2,
        address destination,
        uint256 amount
    ) external onlySelf {
        IERC20L2 token = IERC20L2(tokenL2);
        require(token.l1Address() == tokenL1);
        token.crosschainMint(destination, amount);
    }

    function withdrawERC20(
        address tokenL1,
        address tokenL2,
        address destination,
        uint256 amount
    ) external {
        require(amount > 0, "Withdrawal amount must be positive");
        IERC20L2(tokenL2).crosschainBurn(msg.sender, amount);
        emit ERC20WithdrawalInitiated(tokenL1, tokenL2, destination, amount);
        _withdraw(tokenL1, tokenL2, destination, amount);
    }

    function _withdraw(
        address tokenL1,
        address tokenL2,
        address destination,
        uint256 amount
    ) private {
        IMessenger(L1_MESSENGER).sendMessageToL1(
            keccak256(abi.encodePacked(tokenL1, tokenL2, destination, amount))
        );
    }
    /// @inheritdoc ICommonBridgeL2
    function sendToL2(
        uint256 chainId,
        address to,
        uint256 destGasLimit,
        bytes calldata data
    ) external payable override {
        _burnGas(destGasLimit);
        if (msg.value > 0) {
            IMessenger(L1_MESSENGER).sendMessageToL2(
                chainId,
                address(this),
                address(this),
                destGasLimit,
                transactionIds[chainId],
                msg.value,
                abi.encodeCall(ICommonBridgeL2.mintETH,(msg.sender))
            );
            transactionIds[chainId] += 1;
        }
        IMessenger(L1_MESSENGER).sendMessageToL2(
            chainId,
            msg.sender,
            to,
            destGasLimit,
            transactionIds[chainId],
            msg.value,
            data
        );
        transactionIds[chainId] += 1;
        (bool success, ) = BURN_ADDRESS.call{value: msg.value}("");
        require(success, "Failed to burn Ether");
    }

    /// Burns at least {amount} gas
    function _burnGas(uint256 amount) private view {
        uint256 startingGas = gasleft();
        while (startingGas - gasleft() < amount) {}
    }
}
