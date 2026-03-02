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

    /// @notice L1 ERC-20 address of the custom native gas token.
    /// @dev address(0) means ETH is the native token.
    address public immutable NATIVE_TOKEN_L1;

    /// @notice Scale factor for L1-to-L2 decimal conversion: 10^(18 - l1_decimals).
    uint256 public immutable NATIVE_TOKEN_SCALE_FACTOR;

    mapping(uint256 chainId => uint256 txId) public transactionIds;

    constructor(address nativeTokenL1, uint256 nativeTokenScaleFactor) {
        NATIVE_TOKEN_L1 = nativeTokenL1;
        NATIVE_TOKEN_SCALE_FACTOR = nativeTokenScaleFactor > 0
            ? nativeTokenScaleFactor
            : 1;
    }

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

    function mintETH(address to) external payable onlySelf {
        (bool success, ) = to.call{value: msg.value}("");
        if (!success) {
            this.withdraw{value: msg.value}(to);
        }
        emit DepositProcessed(to, msg.value);
    }

    /// @notice Transfers native token to the given address.
    /// @dev Called via privileged transaction. msg.value is in L2 18-decimal units.
    /// @dev If the transfer fails, a withdrawal is automatically initiated.
    function mintNativeToken(address to) external payable onlySelf {
        (bool success, ) = to.call{value: msg.value}("");
        if (!success) {
            this.withdrawNativeToken{value: msg.value}(to);
        }
        emit DepositProcessed(to, msg.value);
    }

    /// @notice Initiates withdrawal of native token to L1.
    /// @dev Burns the native token on L2 and emits a withdrawal message.
    /// @dev The message hash uses L1 units (scaled down by NATIVE_TOKEN_SCALE_FACTOR).
    function withdrawNativeToken(address _receiverOnL1) external payable {
        require(msg.value > 0, "Withdrawal amount must be positive");
        require(
            NATIVE_TOKEN_L1 != address(0),
            "CommonBridgeL2: native token not configured"
        );
        require(
            msg.value % NATIVE_TOKEN_SCALE_FACTOR == 0,
            "CommonBridgeL2: dust amount not withdrawable"
        );

        (bool success, ) = BURN_ADDRESS.call{value: msg.value}("");
        require(success, "Failed to burn native token");

        // Scale down to L1 units for the withdrawal message hash
        uint256 l1Amount = msg.value / NATIVE_TOKEN_SCALE_FACTOR;

        emit WithdrawalInitiated(msg.sender, _receiverOnL1, msg.value);

        IMessenger(L1_MESSENGER).sendMessageToL1(
            keccak256(
                abi.encodePacked(
                    NATIVE_TOKEN_L1,
                    NATIVE_TOKEN_L1,
                    _receiverOnL1,
                    l1Amount
                )
            )
        );
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
        require(
            token.l1Address() == tokenL1,
            "CommonBridgeL2: L1 address mismatch"
        );
        token.crosschainMint(destination, amount);
    }

    function withdrawERC20(
        address tokenL1,
        address tokenL2,
        address destination,
        uint256 amount
    ) external {
        require(amount > 0, "Withdrawal amount must be positive");
        IERC20L2 token = IERC20L2(tokenL2);
        require(
            token.l1Address() == tokenL1,
            "CommonBridgeL2: L1 address mismatch"
        );
        token.crosschainBurn(msg.sender, amount);
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
    function transferERC20(
        uint256 chainId,
        address to,
        uint256 amount,
        address tokenL2,
        address destTokenL2,
        uint256 destGasLimit
    ) external override {
        IERC20L2 token = IERC20L2(tokenL2);
        token.crosschainBurn(msg.sender, amount);
        address tokenL1 = token.l1Address();
        bytes memory data = abi.encodeCall(
            ICommonBridgeL2.crosschainMintERC20,
            (tokenL1, tokenL2, destTokenL2, to, amount)
        );
        this.sendToL2(chainId, address(this), destGasLimit, data);
    }

    function crosschainMintERC20(
        address tokenL1,
        address tokenL2,
        address destTokenL2,
        address to,
        uint256 amount
    ) external onlySelf {
        this.tryMintERC20(tokenL1, destTokenL2, to, amount);
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
            // Use mintNativeToken if custom native token is configured, otherwise mintETH
            bytes memory mintCallData = NATIVE_TOKEN_L1 != address(0)
                ? abi.encodeCall(ICommonBridgeL2.mintNativeToken, (msg.sender))
                : abi.encodeCall(ICommonBridgeL2.mintETH, (msg.sender));
            IMessenger(L1_MESSENGER).sendMessageToL2(
                chainId,
                address(this),
                address(this),
                destGasLimit,
                transactionIds[chainId],
                msg.value,
                mintCallData
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
