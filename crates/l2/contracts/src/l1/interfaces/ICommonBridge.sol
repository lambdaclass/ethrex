// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

/// @title Interface for the CommonBridge contract.
/// @author LambdaClass
/// @notice A CommonBridge contract is a contract that allows L1<->L2 communication
/// from L1. It both sends messages from L1 to L2 and receives messages from L2.
interface ICommonBridge {
    /// @notice A deposit to L2 has initiated.
    /// @dev Event emitted when a deposit is initiated.
    /// @param amount the amount of tokens being deposited.
    /// @param to the address that will be called in the L2.
    /// @param depositId Id used to differentiate deposits with same amount and recipient.
    /// @param recipient the address that initiated the deposit and will receive the tokens.
    /// @param from the address that initiated the deposit.
    /// @param gasLimit the gas limit for the deposit transaction.
    /// @param data The calldata of the deposit transaction.
    /// @param l2MintTxHash the hash of the transaction that will finalize the
    /// deposit in L2. Could be used to track the status of the deposit finalization
    /// on L2. You can use this hash to retrive the tx data.
    /// It is the result of keccak(abi.encode(transaction)).
    event DepositInitiated(
        uint256 indexed amount,
        address indexed to,
        uint256 indexed depositId,
        address recipient,
        address from,
        uint256 gasLimit,
        bytes data,
        bytes32 l2MintTxHash
    );

    /// @notice L2 withdrawals have been published on L1.
    /// @dev Event emitted when the L2 withdrawals are published on L1.
    /// @param withdrawalLogsBatchNumber the batch number where the withdrawal logs were emitted.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs.
    event WithdrawalsPublished(
        uint256 indexed withdrawalLogsBatchNumber,
        bytes32 indexed withdrawalsLogsMerkleRoot
    );

    /// @notice A withdrawal has been claimed.
    /// @dev Event emitted when a withdrawal is claimed.
    /// @param l2WithdrawalTxHash the hash of the L2 withdrawal transaction.
    /// @param claimee the address that claimed the withdrawal.
    /// @param claimedAmount the amount that was claimed.
    event WithdrawalClaimed(
        bytes32 indexed l2WithdrawalTxHash,
        address indexed claimee,
        uint256 indexed claimedAmount
    );

    struct DepositValues {
        address to;
        address recipient;
        uint256 gasLimit;
        bytes data;
    }

    /// @notice Method to retrieve all the pending deposit logs hashes.
    /// @dev This method is used by the L2 L1_Watcher to get the pending deposit
    /// logs to be processed.
    function getPendingDepositLogs() external view returns (bytes32[] memory);

    /// @notice Method that starts an L2 ETH deposit process.
    /// @dev The deposit process starts here by emitting a DepositInitiated
    /// event. This event will later be intercepted by the L2 operator to
    /// finalize the deposit.
    /// @param depositValues the values needed to create the deposit.
    function deposit(DepositValues calldata depositValues) external payable;

    /// @notice Method to retrieve the versioned hash of the first `number`
    /// pending deposit logs.
    /// @param number of pending deposit logs to retrieve the versioned hash.
    function getPendingDepositLogsVersionedHash(
        uint16 number
    ) external view returns (bytes32);

    /// @notice Remove pending deposit from the pendingDepositLogs queue.
    /// @dev This method is used by the L2 OnChainOperator to remove the pending
    /// deposit logs from the queue after the deposit is verified.
    /// @param number of pending deposit logs to remove.
    /// As deposits are processed in order, we don't need to specify
    /// the pending deposit logs to remove, only the number of them.
    function removePendingDepositLogs(uint16 number) external;

    /// @notice Method to retrieve the merkle root of the withdrawal logs of a
    /// given block.
    /// @dev This method is used by the L2 OnChainOperator at the verify stage.
    /// @param blockNumber the block number in L2 where the withdrawal logs were
    /// emitted.
    /// @return the merkle root of the withdrawal logs of the given block.
    function getWithdrawalLogsMerkleRoot(
        uint256 blockNumber
    ) external view returns (bytes32);

    /// @notice Publishes the L2 withdrawals on L1.
    /// @dev This method is used by the L2 OnChainOperator to publish the L2
    /// withdrawals when an L2 batch is committed.
    /// @param withdrawalLogsBatchNumber the batch number in L2 where the withdrawal logs were emitted.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs.
    function publishWithdrawals(
        uint256 withdrawalLogsBatchNumber,
        bytes32 withdrawalsLogsMerkleRoot
    ) external;

    /// @notice Method that claims an L2 withdrawal.
    /// @dev For a user to claim a withdrawal, this method verifies:
    /// - The l2WithdrawalBatchNumber was committed. If the given batch was not
    /// committed, this means that the withdrawal was not published on L1.
    /// - The l2WithdrawalBatchNumber was verified. If the given batch was not
    /// verified, this means that the withdrawal claim was not enabled.
    /// - The withdrawal was not claimed yet. This is to avoid double claims.
    /// - The withdrawal proof is valid. This is, there exists a merkle path
    /// from the withdrawal log to the withdrawal root, hence the claimed
    /// withdrawal exists.
    /// @dev We do not need to check that the claimee is the same as the
    /// beneficiary of the withdrawal, because the withdrawal proof already
    /// contains the beneficiary.
    /// @param l2WithdrawalTxHash the hash of the L2 withdrawal transaction.
    /// @param claimedAmount the amount that will be claimed.
    /// @param withdrawalProof the merkle path to the withdrawal log.
    /// @param withdrawalLogIndex the index of the withdrawal log in the block.
    /// This is the index of the withdraw transaction relative to the block's
    /// withdrawal transctions.
    /// A pseudocode would be [tx if tx is withdrawx for tx in block.txs()].index(leaf_tx).
    /// @param l2WithdrawalBatchNumber the batch number where the withdrawal log
    /// was emitted.
    function claimWithdrawal(
        bytes32 l2WithdrawalTxHash,
        uint256 claimedAmount,
        uint256 l2WithdrawalBatchNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) external;
}
