// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import "../../lib/openzeppelin-contracts/contracts/access/Ownable.sol";
import "../../lib/openzeppelin-contracts/contracts/utils/ReentrancyGuard.sol";
import "./interfaces/ICommonBridge.sol";
import "./interfaces/IOnChainProposer.sol";

/// @title CommonBridge contract.
/// @author LambdaClass
contract CommonBridge is ICommonBridge, Ownable, ReentrancyGuard {
    /// @notice Mapping of unclaimed withdrawals. A withdrawal is claimed if
    /// there is a non-zero value in the mapping (a merkle root) for the hash
    /// of the L2 transaction that requested the withdrawal.
    /// @dev The key is the hash of the L2 transaction that requested the
    /// withdrawal.
    /// @dev The value is a boolean indicating if the withdrawal was claimed or not.
    mapping(bytes32 => bool) public claimedWithdrawals;

    /// @notice Mapping of merkle roots to the L2 withdrawal transaction logs.
    /// @dev The key is the L2 block number where the logs were emitted.
    /// @dev The value is the merkle root of the logs.
    /// @dev If there exist a merkle root for a given block number it means
    /// that the logs were published on L1, and that that block was committed.
    mapping(uint256 => bytes32) public blockWithdrawalsLogs;

    bytes32[] public depositLogs;

    address public ON_CHAIN_PROPOSER;

    /// @notice Block in which the CommonBridge was initialized.
    /// @dev Used by the L1Watcher to fetch logs starting from this block.
    uint256 public lastFetchedL1Block;

    /// @notice Global deposit identifier, it is incremented each time a new deposit is made.
    /// @dev It is used as the nonce of the mint transaction created by the L1Watcher.
    uint256 public depositId;

    modifier onlyOnChainProposer() {
        require(
            msg.sender == ON_CHAIN_PROPOSER,
            "CommonBridge: caller is not the OnChainProposer"
        );
        _;
    }

    constructor(address owner) Ownable(owner) {}

    /// @inheritdoc ICommonBridge
    function initialize(address onChainProposer) public nonReentrant {
        require(
            ON_CHAIN_PROPOSER == address(0),
            "CommonBridge: contract already initialized"
        );
        require(
            onChainProposer != address(0),
            "CommonBridge: onChainProposer is the zero address"
        );
        require(
            onChainProposer != address(this),
            "CommonBridge: onChainProposer is the contract address"
        );
        ON_CHAIN_PROPOSER = onChainProposer;

        lastFetchedL1Block = block.number;
        depositId = 0;
    }

    /// @inheritdoc ICommonBridge
    function getDepositLogs() public view returns (bytes32[] memory) {
        return depositLogs;
    }

    /// @inheritdoc ICommonBridge
    function deposit(address to) public payable {
        require(msg.value > 0, "CommonBridge: amount to deposit is zero");

        // TODO: Build the tx.
        bytes32 l2MintTxHash = keccak256(abi.encodePacked("dummyl2MintTxHash"));
        depositLogs.push(
            keccak256(
                bytes.concat(
                    bytes20(to),
                    bytes32(msg.value),
                    bytes32(depositId)
                )
            )
        );
        emit DepositInitiated(msg.value, to, depositId, l2MintTxHash);
        depositId += 1;
    }

    receive() external payable {
        deposit(msg.sender);
    }

    /// @inheritdoc ICommonBridge
    function getDepositLogsVersionedHash(
        uint16 number
    ) public view returns (bytes32) {
        require(number > 0, "CommonBridge: number is zero (get)");
        require(
            uint256(number) <= depositLogs.length,
            "CommonBridge: number is greater than the length of depositLogs (get)"
        );

        bytes memory logs;
        for (uint i = 0; i < number; i++) {
            logs = bytes.concat(logs, depositLogs[i]);
        }

        return
            bytes32(bytes2(number)) |
            bytes32(uint256(uint240(uint256(keccak256(logs)))));
    }

    /// @inheritdoc ICommonBridge
    function removeDepositLogs(uint16 number) public onlyOnChainProposer {
        require(
            number <= depositLogs.length,
            "CommonBridge: number is greater than the length of depositLogs (remove)"
        );

        for (uint i = 0; i < depositLogs.length - number; i++) {
            depositLogs[i] = depositLogs[i + number];
        }

        for (uint _i = 0; _i < number; _i++) {
            depositLogs.pop();
        }
    }

    /// @inheritdoc ICommonBridge
    function publishWithdrawals(
        uint256 withdrawalLogsBlockNumber,
        bytes32 withdrawalsLogsMerkleRoot
    ) public onlyOnChainProposer {
        require(
            blockWithdrawalsLogs[withdrawalLogsBlockNumber] == bytes32(0),
            "CommonBridge: withdrawal logs already published"
        );
        blockWithdrawalsLogs[
            withdrawalLogsBlockNumber
        ] = withdrawalsLogsMerkleRoot;
        emit WithdrawalsPublished(
            withdrawalLogsBlockNumber,
            withdrawalsLogsMerkleRoot
        );
    }

    /// @inheritdoc ICommonBridge
    function claimWithdrawal(
        bytes32 l2WithdrawalTxHash,
        uint256 claimedAmount,
        uint256 withdrawalBlockNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) public nonReentrant {
        require(
            blockWithdrawalsLogs[withdrawalBlockNumber] != bytes32(0),
            "CommonBridge: the block that emitted the withdrawal logs was not committed"
        );
        require(
            withdrawalBlockNumber <=
                IOnChainProposer(ON_CHAIN_PROPOSER).lastVerifiedBlock(),
            "CommonBridge: the block that emitted the withdrawal logs was not verified"
        );
        require(
            claimedWithdrawals[l2WithdrawalTxHash] == false,
            "CommonBridge: the withdrawal was already claimed"
        );
        require(
            _verifyWithdrawProof(
                l2WithdrawalTxHash,
                claimedAmount,
                withdrawalBlockNumber,
                withdrawalLogIndex,
                withdrawalProof
            ),
            "CommonBridge: invalid withdrawal proof"
        );

        (bool success, ) = payable(msg.sender).call{value: claimedAmount}("");

        require(success, "CommonBridge: failed to send the claimed amount");

        claimedWithdrawals[l2WithdrawalTxHash] = true;

        emit WithdrawalClaimed(l2WithdrawalTxHash, msg.sender, claimedAmount);
    }

    function _verifyWithdrawProof(
        bytes32 l2WithdrawalTxHash,
        uint256 claimedAmount,
        uint256 withdrawalBlockNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) internal view returns (bool) {
        bytes32 withdrawalLeaf = keccak256(
            abi.encodePacked(msg.sender, claimedAmount, l2WithdrawalTxHash)
        );
        for (uint256 i = 0; i < withdrawalProof.length; i++) {
            if (withdrawalLogIndex % 2 == 0) {
                withdrawalLeaf = keccak256(
                    abi.encodePacked(withdrawalLeaf, withdrawalProof[i])
                );
            } else {
                withdrawalLeaf = keccak256(
                    abi.encodePacked(withdrawalProof[i], withdrawalLeaf)
                );
            }
            withdrawalLogIndex /= 2;
        }
        return withdrawalLeaf == blockWithdrawalsLogs[withdrawalBlockNumber];
    }
}
