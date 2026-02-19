// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title L2Bridge — Unified deposit and withdrawal bridge for Native Rollups PoC.
///
/// Deployed at 0x000000000000000000000000000000000000fffd (L2 predeploy).
/// Preminted with ETH in L2 genesis. The NativeRollup contract on L1 holds the
/// corresponding backing ETH.
///
/// Deposits: the sequencer/relayer calls processDeposit() for each pending L1
/// deposit, distributing ETH to the recipient and emitting DepositProcessed.
/// The EXECUTE precompile scans DepositProcessed events to rebuild the deposits
/// rolling hash and verifies it matches the value committed in NativeRollup.advance().
///
/// Withdrawals: users call withdraw() to burn ETH and emit WithdrawalInitiated.
/// The EXECUTE precompile scans these events and builds a Merkle root for
/// withdrawal claiming on L1.
///
/// Storage layout:
///   Slot 0: relayer (address)
///   Slot 1: depositNonce (uint256)
///   Slot 2: withdrawalNonce (uint256)
contract L2Bridge {
    address public relayer;
    uint256 public depositNonce;
    uint256 public withdrawalNonce;

    event DepositProcessed(
        address indexed recipient,
        uint256 amount,
        uint256 indexed depositNonce
    );

    event WithdrawalInitiated(
        address indexed from,
        address indexed receiver,
        uint256 amount,
        uint256 indexed messageId
    );

    /// @notice Process a single L1 deposit: send ETH to recipient and emit event.
    /// @dev If the ETH transfer fails, the nonce is still incremented and the event
    ///      is still emitted so that L1/L2 nonces stay in sync. The ETH stays in the
    ///      bridge. User recovery mechanism is TBD.
    /// @param recipient  L2 address receiving the ETH.
    /// @param amount     Amount in wei.
    /// @param nonce      Nonce from the L1 deposit (must match current depositNonce).
    function processDeposit(
        address recipient,
        uint256 amount,
        uint256 nonce
    ) external {
        require(msg.sender == relayer, "L2Bridge: not relayer");
        require(nonce == depositNonce, "L2Bridge: nonce mismatch");

        uint256 currentNonce = depositNonce;
        depositNonce = currentNonce + 1;

        // Don't revert on failed transfer — nonce stays in sync, ETH stays in bridge.
        recipient.call{value: amount}("");

        emit DepositProcessed(recipient, amount, currentNonce);
    }

    /// @notice Initiate a withdrawal by sending ETH with the L1 receiver address.
    /// @param _receiver Address on L1 that will receive the withdrawn ETH.
    function withdraw(address _receiver) external payable {
        require(msg.value > 0, "Withdrawal amount must be positive");
        require(_receiver != address(0), "Invalid receiver");

        uint256 msgId = withdrawalNonce;
        withdrawalNonce = msgId + 1;

        (bool ok, ) = address(0).call{value: msg.value}("");
        require(ok, "Failed to burn Ether");

        emit WithdrawalInitiated(msg.sender, _receiver, msg.value, msgId);
    }
}
