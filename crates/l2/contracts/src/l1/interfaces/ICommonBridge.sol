// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title Interface for the CommonBridge contract.
/// @author LambdaClass
/// @notice A CommonBridge contract is a contract that allows L1<->L2 communication
/// from L1. It both sends messages from L1 to L2 and receives messages from L2.
interface ICommonBridge {
    /// @notice A deposit to L2 has initiated.
    /// @dev Event emitted when a deposit is initiated.
    /// @param amount the amount of tokens being deposited.
    /// @param to the address in L2 to which the tokens will be minted to.
    /// @param l2MintTxHash the hash of the transaction that will finalize the
    /// deposit in L2. Could be used to track the status of the deposit finalization
    /// on L2. You can use this hash to retrive the tx data.
    /// It is the result of keccak(abi.encode(transaction)).
    event DepositInitiated(
        uint256 indexed amount,
        address indexed to,
        bytes32 indexed l2MintTxHash
    );

    /// @notice L2 withdrawals have been published on L1.
    /// @dev Event emitted when the L2 withdrawals are published on L1.
    /// @param withdrawalLogsBlockNumber the block number in L2 where the
    /// withdrawal logs were emitted.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs.
    event WithdrawalsPublished(
        uint256 indexed withdrawalLogsBlockNumber,
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

    /// @notice Initializes the contract.
    /// @dev This method is called only once after the contract is deployed.
    /// @dev It sets the OnChainProposer address.
    /// @param onChainProposer the address of the OnChainProposer contract.
    function initialize(address onChainProposer) external;

    /// @notice Method that starts an L2 ETH deposit process.
    /// @dev The deposit process starts here by emitting a DepositInitiated
    /// event. This event will later be intercepted by the L2 operator to
    /// finalize the deposit.
    /// @param to, the address in L2 to which the tokens will be minted to.
    function deposit(address to) external payable;

    /// @notice Remove deposit from depositLogs queue.
    /// @dev This method is used by the L2 OnChainOperator to remove the deposit
    /// logs from the queue after the deposit is verified.
    /// @param number of deposit logs to remove.
    /// As deposits are processed in order, we don't need to specify
    /// the deposit logs to remove, only the number of them.
    function removeDepositLogs(uint number) external;

    /// @notice Publishes the L2 withdrawals on L1.
    /// @dev This method is used by the L2 OnChainOperator to publish the L2
    /// withdrawals when an L2 block is committed.
    /// @param withdrawalLogsBlockNumber the block number in L2 where the
    /// withdrawal logs were emitted.
    /// @param withdrawalsLogsMerkleRoot the merkle root of the withdrawal logs.
    function publishWithdrawals(
        uint256 withdrawalLogsBlockNumber,
        bytes32 withdrawalsLogsMerkleRoot
    ) external;

    /// @notice Method that claims an L2 withdrawal.
    /// @dev For a user to claim a withdrawal, this method verifies:
    /// - The l2WithdrawalBlockNumber was committed. If the given block was not
    /// committed, this means that the withdrawal was not published on L1.
    /// - The l2WithdrawalBlockNumber was verified. If the given block was not
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
    /// @param l2WithdrawalBlockNumber the block number where the withdrawal log
    /// was emitted.
    function claimWithdrawal(
        bytes32 l2WithdrawalTxHash,
        uint256 claimedAmount,
        uint256 l2WithdrawalBlockNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) external;
}
