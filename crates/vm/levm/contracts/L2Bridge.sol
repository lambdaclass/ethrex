// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title L2Bridge — Unified L1 message processing and withdrawal bridge for Native Rollups PoC.
///
/// Deployed at 0x000000000000000000000000000000000000fffd (L2 predeploy).
/// Preminted with a large ETH balance in L2 genesis to cover all future L1
/// messages (similar to Taiko/Linea). The NativeRollup contract on L1
/// accumulates ETH over time as users call sendL1Message().
///
/// L1 Messages: the sequencer/relayer calls processL1Message() for each pending
/// L1 message, executing the subcall and emitting L1MessageProcessed.
/// The EXECUTE precompile scans L1MessageProcessed events to rebuild the L1
/// messages rolling hash and verifies it matches the value committed in
/// NativeRollup.advance().
///
/// Withdrawals: users call withdraw() to lock ETH and emit WithdrawalInitiated.
/// The EXECUTE precompile scans these events and builds a Merkle root for
/// withdrawal claiming on L1.
///
/// Storage layout:
///   Slot 0: relayer (address)
///   Slot 1: l1MessageNonce (uint256)
///   Slot 2: withdrawalNonce (uint256)
contract L2Bridge {
    address public relayer;
    uint256 public l1MessageNonce;
    uint256 public withdrawalNonce;

    event L1MessageProcessed(
        address indexed from,
        address indexed to,
        uint256 value,
        uint256 gasLimit,
        bytes32 dataHash,
        uint256 indexed nonce
    );

    event WithdrawalInitiated(
        address indexed from,
        address indexed receiver,
        uint256 amount,
        uint256 indexed messageId
    );

    /// @notice Process a single L1 message: execute the subcall and emit event.
    /// @dev If the subcall fails, the nonce is still incremented and the event
    ///      is still emitted so that L1/L2 nonces stay in sync. Assets stay in the
    ///      bridge. User recovery mechanism is TBD.
    /// @param from     Original L1 sender (msg.sender on L1).
    /// @param to       Target address on L2.
    /// @param value    Amount of ETH to send.
    /// @param gasLimit Maximum gas for the L2 subcall.
    /// @param data     Calldata to execute on L2 (can be empty for simple ETH transfers).
    /// @param nonce    Nonce from the L1 message (must match current l1MessageNonce).
    function processL1Message(
        address from,
        address to,
        uint256 value,
        uint256 gasLimit,
        bytes calldata data,
        uint256 nonce
    ) external {
        require(msg.sender == relayer, "L2Bridge: not relayer");
        require(nonce == l1MessageNonce, "L2Bridge: nonce mismatch");

        uint256 currentNonce = l1MessageNonce;
        l1MessageNonce = currentNonce + 1;

        // Execute the L2 subcall. Don't revert on failure — nonce stays in sync, assets stay in bridge.
        to.call{value: value, gas: gasLimit}(data);

        emit L1MessageProcessed(from, to, value, gasLimit, keccak256(data), currentNonce);
    }

    /// @notice Initiate a withdrawal by sending ETH with the L1 receiver address.
    /// @dev The ETH stays locked in the bridge contract (not burned). On L1,
    ///      claimWithdrawal releases the corresponding ETH from NativeRollup.
    /// @param _receiver Address on L1 that will receive the withdrawn ETH.
    function withdraw(address _receiver) external payable {
        require(msg.value > 0, "Withdrawal amount must be positive");
        require(_receiver != address(0), "Invalid receiver");

        uint256 msgId = withdrawalNonce;
        withdrawalNonce = msgId + 1;

        emit WithdrawalInitiated(msg.sender, _receiver, msg.value, msgId);
    }
}
