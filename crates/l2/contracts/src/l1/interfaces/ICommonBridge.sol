// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @title Interface for the CommonBridge contract.
/// @author LambdaClass
/// @notice A CommonBridge contract is a contract that allows L1<->L2 communication
/// from L1. It both sends messages from L1 to L2 and receives messages from L2.
interface ICommonBridge {
    /// @notice A privileged transaction to L2 has initiated.
    /// @dev Event emitted when a privileged transaction is initiated.
    /// @param l1From the address that initiated the transaction.
    /// @param from the address sending the transaction on L2.
    /// @param to the recipient on L2.
    /// @param transactionId Id used to make transactions unique.
    /// @param value the value of the transaction.
    /// @param gasLimit the gas limit for the deposit transaction.
    /// @param data The calldata of the deposit transaction.
    event PrivilegedTxSent(
        address indexed l1From,
        address from,
        address to,
        uint256 transactionId,
        uint256 value,
        uint256 gasLimit,
        bytes data
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
    /// @param withdrawalId the message Id of the claimed withdrawal
    event WithdrawalClaimed(uint256 indexed withdrawalId);

    struct SendValues {
        address to;
        uint256 gasLimit;
        uint256 value;
        bytes data;
    }

    /// @notice Structure representing balance to send to each chain.
    struct BalanceDiff {
        uint256 chainId;
        uint256 value;
        bytes32[] message_hashes;
    }

    struct L2MessageRollingHash {
        uint256 chainId;
        bytes32 rollingHash;
    }

    /// @notice Method to retrieve all the pending transaction hashes.
    /// @dev This method is used by the L2 L1_Watcher to get the pending
    /// privileged transactions to be processed.
    function getPendingTransactionHashes()
        external
        view
        returns (bytes32[] memory);

    /// @notice Method to retrieve all the pending L2 message hashes for a given chain.
    /// @dev This method is used by the L1 watcher to get the pending L2 messages
    /// to be processed for a given chain.
    function getPendingL2MessagesHashes(
        uint256 chainId
    ) external view returns (bytes32[] memory);

    /// @notice Method that sends a transaction to L2.
    /// @dev The deposit process starts here by emitting a L1ToL2Message
    /// event. This event will later be intercepted by the L2 operator to
    /// be inserted as a transaction.
    /// @param sendValues the parameters of the transaction being sent.
    function sendToL2(SendValues calldata sendValues) external;

    /// @notice Method that starts an L2 ETH deposit process.
    /// @dev The deposit process starts here by emitting a L1ToL2Message
    /// event. This event will later be intercepted by the L2 operator to
    /// finalize the deposit.
    /// @param l2Recipient the address on L2 that will receive the deposit.
    function deposit(address l2Recipient) external payable;

    /// @notice Method to retrieve the versioned hash of the first `number`
    /// pending privileged transactions.
    /// @param number of pending privileged transaction to retrieve the versioned hash.
    function getPendingTransactionsVersionedHash(
        uint16 number
    ) external view returns (bytes32);

    /// @notice Method to retrieve the versioned hash of the first `number`
    /// pending L2 messages.
    /// @param chainId the chain id of the L2 messages to retrieve.
    /// @param number of pending L2 messages to retrieve the versioned hash.
    function getPendingL2MessagesVersionedHash(
        uint256 chainId,
        uint16 number
    ) external view returns (bytes32);

    /// @notice Remove pending transaction hashes from the queue.
    /// @dev This method is used by the L2 OnChainOperator to remove the pending
    /// privileged transactions from the queue after the transaction is included.
    /// @param number of pending transaction hashes to remove.
    /// As transactions are processed in order, we don't need to specify
    /// the transaction hashes to remove, only the number of them.
    function removePendingTransactionHashes(uint16 number) external;

    /// @notice Remove pending L2 messages from the queue.
    /// @dev This method is used by the L2 OnChainProposer to remove the pending
    /// L2 messages from the queue after the messages are included.
    /// @param chainId the chain id of the L2 messages to remove.
    /// @param number of pending transaction hashes to remove.
    /// As transactions are processed in order, we don't need to specify
    /// the transaction hashes to remove, only the number of them.
    function removePendingL2Messages(uint256 chainId, uint16 number) external;

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

    /// @notice Publishes the L2 messages in the router contract.
    /// @dev This method is used by the L2 OnChainProposer to publish the L2
    /// messages when an L2 batch is committed.
    /// @param balanceDiffs Array of balance differences and associated message hashes to be sent to different chains.
    function publishL2Messages(BalanceDiff[] calldata balanceDiffs) external;

    /// @notice Receives messages from another chain via shared bridge router.
    /// @dev This method should only be called by the shared bridge router, as this
    /// method will not burn the L2 gas.
    /// @param chainID The ID of the source chain.
    /// @param message_hashes The hashes of the messages being received.
    function receiveFromSharedBridge(
        uint256 chainID,
        bytes32[] calldata message_hashes
    ) external payable;

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
    /// @param claimedAmount the amount that will be claimed.
    /// @param withdrawalProof the merkle path to the withdrawal log.
    /// @param withdrawalLogIndex the index of the message log in the block.
    /// This is the index of the withdraw transaction relative to the block's messages.
    /// @param l2WithdrawalBatchNumber the batch number where the withdrawal log
    /// was emitted.
    function claimWithdrawal(
        uint256 claimedAmount,
        uint256 l2WithdrawalBatchNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) external;

    /// @notice Claims an ERC20 withdrawal
    /// @param tokenL1 Address of the token on the L1
    /// @param tokenL2 Address of the token on the L2
    /// @param claimedAmount the amount that will be claimed.
    /// @param withdrawalProof the merkle path to the withdrawal log.
    /// @param withdrawalLogIndex the index of the message log in the batch.
    /// @param l2WithdrawalBatchNumber the batch number where the withdrawal log
    /// was emitted.
    function claimWithdrawalERC20(
        address tokenL1,
        address tokenL2,
        uint256 claimedAmount,
        uint256 l2WithdrawalBatchNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) external;

    /// @notice Checks if the sequencer has exceeded it's processing deadlines
    function hasExpiredPrivilegedTransactions() external view returns (bool);

    /// @notice Allows the owner to pause the contract
    function pause() external;

    /// @notice Allows the owner to unpause the contract
    function unpause() external;

    /// @notice Register a new fee token on the L2.
    /// @param newFeeToken Address of the token to authorize for fees.
    function registerNewFeeToken(address newFeeToken) external;

    /// @notice Unregister a fee token on the L2.
    /// @param existingFeeToken Address of the token to be removed.
    function unregisterFeeToken(address existingFeeToken) external;
}
