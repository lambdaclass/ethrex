// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @title Interface for the L2 side of the CommonBridge contract.
/// @author LambdaClass
/// @notice A CommonBridge contract is a contract that allows L1<->L2 communication
/// It handles user withdrawals and message sending to L1.
interface ICommonBridgeL2 {
    /// @notice An ETH deposit was successfully processed
    /// @dev Event emitted when an ETH deposit is processed.
    /// @param receiver the address that received the ETH
    /// @param amount the amount of ether being deposited
    event DepositProcessed(
        address indexed receiver,
        uint256 amount
    );
    /// @notice A withdrawal to L1 has initiated.
    /// @dev Event emitted when a withdrawal is initiated.
    /// @param senderOnL2 the sender of the transaction on L2.
    /// @param receiverOnL1 the address on L1 that will receive the funds back.
    /// @param amount the amount of ether being withdrawn.
    event WithdrawalInitiated(
        address indexed senderOnL2,
        address indexed receiverOnL1,
        uint256 indexed amount
    );


    /// @notice An ERC20 token deposit was successfully processed
    /// @dev Event emitted when an ERC20 deposit is processed.
    /// @param tokenL1 Address of the token on L1
    /// @param tokenL2 Address of the token on L2
    /// @param receiver the address that received the tokens
    /// @param amount the amount of tokens being deposited
    event ERC20DepositProcessed(
        address indexed tokenL1,
        address indexed tokenL2,
        address indexed receiver,
        uint256 amount
    );
    /// @notice An ERC20 token withdrawal has initiated
    /// @dev Event emitted when an ERC20 withdrawal is initiated.
    /// @param tokenL1 Address of the token on L1
    /// @param tokenL2 Address of the token on L2
    /// @param receiverOnL1 the address on L1 that will receive the funds back.
    /// @param amount the amount of tokens being withdrawn.
    event ERC20WithdrawalInitiated(
        address indexed tokenL1,
        address indexed tokenL2,
        address indexed receiverOnL1,
        uint256 amount
    );

    /// @notice Initiates the withdrawal of funds to the L1.
    /// @dev This is the first step in the two step process of a user withdrawal.
    /// @dev It burns funds on L2 and sends a message to the L1 so users
    /// @dev can claim those funds on L1.
    /// @param _receiverOnL1 the address that can claim the funds on L1.
    function withdraw(address _receiverOnL1) external payable;

    /// @notice Transfers ETH to the given address.
    /// @dev This is called by a privileged transaction from the L1 bridge
    /// @dev The transaction itself is what mints the ETH, this is just a helper
    /// @dev If the transfer fails, a withdrawal is initiated.
    /// @param to the address to transfer the funds to
    function mintETH(address to) external payable;

    /// @notice Tries to deposit an ERC20 token
    /// @dev The msg.sender must be the bridge itself, using a privileged transaction
    /// @param tokenL1 Address of the token on L1
    /// @param tokenL2 Address of the token on L2
    /// @param destination Address that should receive the tokens
    /// @param amount Amount of tokens to give
    function mintERC20(address tokenL1, address tokenL2, address destination, uint256 amount) external;

    /// @notice Initiates the withdrawal of ERC20 tokens to the L1.
    /// @dev This is the first step in the two step process of a user withdrawal.
    /// @dev It burns tokens on L2 and sends a message to the L1 so users
    /// @dev can claim those tokens on L1.
    /// @param tokenL1 Address of the token on L1
    /// @param tokenL2 Address of the token on L2
    /// @param destination Address on L1 that should receive the tokens
    /// @param amount Amount of tokens to withdraw
    function withdrawERC20(address tokenL1, address tokenL2, address destination, uint256 amount) external;

    /// @notice Sends an arbitrary message to the another chain.
    /// @dev This can be used to perform simple transfers or to call contracts on the destination chain.
    /// @param chainId The chain ID of the destination chain.
    /// @param to The address of the contract on the destination chain.
    /// @param destGasLimit The gas limit for the destination chain execution.
    /// @param data The calldata to send to the destination contract.
    function sendToL2(uint256 chainId, address to, uint256 destGasLimit, bytes calldata data) external payable;
}
