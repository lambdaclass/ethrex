// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/utils/ReentrancyGuardUpgradeable.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import "./interfaces/ICommonBridge.sol";
import "./interfaces/IOnChainProposer.sol";
import "../l2/interfaces/ICommonBridgeL2.sol";

/// @title CommonBridge contract.
/// @author LambdaClass
contract CommonBridge is
    ICommonBridge,
    Initializable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable,
    ReentrancyGuardUpgradeable
{
    using SafeERC20 for IERC20;

    /// @notice Mapping of unclaimed withdrawals. A withdrawal is claimed if
    /// there is a non-zero value in the mapping (a merkle root) for the hash
    /// of the L2 transaction that requested the withdrawal.
    /// @dev The key is the hash of the L2 transaction that requested the
    /// withdrawal.
    /// @dev The value is a boolean indicating if the withdrawal was claimed or not.
    mapping(bytes32 => bool) public claimedWithdrawals;

    /// @notice Mapping of merkle roots to the L2 withdrawal transaction logs.
    /// @dev The key is the L2 batch number where the logs were emitted.
    /// @dev The value is the merkle root of the logs.
    /// @dev If there exist a merkle root for a given batch number it means
    /// that the logs were published on L1, and that that batch was committed.
    mapping(uint256 => bytes32) public batchWithdrawalLogsMerkleRoots;

    /// @notice Array of hashed pending deposit logs.
    bytes32[] public pendingDepositLogs;

    address public ON_CHAIN_PROPOSER;

    /// @notice Block in which the CommonBridge was initialized.
    /// @dev Used by the L1Watcher to fetch logs starting from this block.
    uint256 public lastFetchedL1Block;

    /// @notice Global deposit identifier, it is incremented each time a new deposit is made.
    /// @dev It is used as the nonce of the mint transaction created by the L1Watcher.
    uint256 public depositId;

    /// @notice Address of the bridge on the L2
    /// @dev It's used to validate withdrawals
    address public constant L2_BRIDGE_ADDRESS = address(0xffff);

    /// @notice How much of each L1 token was deposited to each L2 token.
    /// @dev Stored as L1 -> L2 -> amount
    /// @dev Prevents L2 tokens from faking their L1 address and stealing tokens
    mapping (address => mapping (address => uint256)) public depositsERC20;

    /// @notice Token address used to represent ETH
    address public constant ETH_TOKEN =  0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    modifier onlyOnChainProposer() {
        require(
            msg.sender == ON_CHAIN_PROPOSER,
            "CommonBridge: caller is not the OnChainProposer"
        );
        _;
    }

    /// @notice Initializes the contract.
    /// @dev This method is called only once after the contract is deployed.
    /// @dev It sets the OnChainProposer address.
    /// @param owner the address of the owner who can perform upgrades.
    /// @param onChainProposer the address of the OnChainProposer contract.
    function initialize(
        address owner,
        address onChainProposer
    ) public initializer {
        require(
            onChainProposer != address(0),
            "CommonBridge: onChainProposer is the zero address"
        );
        ON_CHAIN_PROPOSER = onChainProposer;

        lastFetchedL1Block = block.number;
        depositId = 0;

        OwnableUpgradeable.__Ownable_init(owner);
        ReentrancyGuardUpgradeable.__ReentrancyGuard_init();
    }

    /// @inheritdoc ICommonBridge
    function getPendingDepositLogs() public view returns (bytes32[] memory) {
        return pendingDepositLogs;
    }

    function _deposit(address from, DepositValues memory depositValues) private {
        bytes32 l2MintTxHash = keccak256(
            bytes.concat(
                bytes20(depositValues.to),
                bytes32(msg.value),
                bytes32(depositId),
                bytes20(depositValues.recipient),
                bytes20(from),
                bytes32(depositValues.gasLimit),
                bytes32(keccak256(depositValues.data))
            )
        );

        pendingDepositLogs.push(l2MintTxHash);

        emit DepositInitiated(
            msg.value,
            depositValues.to,
            depositId,
            depositValues.recipient,
            from,
            depositValues.gasLimit,
            depositValues.data,
            l2MintTxHash
        );
        depositId += 1;
    }

    /// @inheritdoc ICommonBridge
    function deposit(DepositValues calldata depositValues) public payable {
        _deposit(msg.sender, depositValues);
    }

    receive() external payable {
        depositsERC20[ETH_TOKEN][ETH_TOKEN] += msg.value;
        DepositValues memory depositValues = DepositValues({
            to: msg.sender,
            recipient: msg.sender,
            gasLimit: 21000 * 5,
            data: bytes("")
        });
        _deposit(msg.sender, depositValues);
    }

    function depositERC20(address tokenL1, address tokenL2, address destination, uint256 amount) external {
        require(amount > 0, "CommonBridge: amount to deposit is zero");
        depositsERC20[tokenL1][tokenL2] += amount;
        IERC20(tokenL1).safeTransferFrom(msg.sender, address(this), amount);

        bytes memory callData = abi.encodeCall(ICommonBridgeL2.mintERC20, (tokenL1, tokenL2, destination, amount));
        DepositValues memory depositValues = DepositValues({
            to: L2_BRIDGE_ADDRESS,
            recipient: L2_BRIDGE_ADDRESS,
            gasLimit: 21000 * 5,
            data: callData
        });
        _deposit(L2_BRIDGE_ADDRESS, depositValues);
    }

    /// @inheritdoc ICommonBridge
    function getPendingDepositLogsVersionedHash(
        uint16 number
    ) public view returns (bytes32) {
        require(number > 0, "CommonBridge: number is zero (get)");
        require(
            uint256(number) <= pendingDepositLogs.length,
            "CommonBridge: number is greater than the length of depositLogs (get)"
        );

        bytes memory logs;
        for (uint i = 0; i < number; i++) {
            logs = bytes.concat(logs, pendingDepositLogs[i]);
        }

        return
            bytes32(bytes2(number)) |
            bytes32(uint256(uint240(uint256(keccak256(logs)))));
    }

    /// @inheritdoc ICommonBridge
    function removePendingDepositLogs(
        uint16 number
    ) public onlyOnChainProposer {
        require(
            number <= pendingDepositLogs.length,
            "CommonBridge: number is greater than the length of depositLogs (remove)"
        );

        for (uint i = 0; i < pendingDepositLogs.length - number; i++) {
            pendingDepositLogs[i] = pendingDepositLogs[i + number];
        }

        for (uint _i = 0; _i < number; _i++) {
            pendingDepositLogs.pop();
        }
    }

    /// @inheritdoc ICommonBridge
    function getWithdrawalLogsMerkleRoot(
        uint256 blockNumber
    ) public view returns (bytes32) {
        return batchWithdrawalLogsMerkleRoots[blockNumber];
    }

    /// @inheritdoc ICommonBridge
    function publishWithdrawals(
        uint256 withdrawalLogsBatchNumber,
        bytes32 withdrawalsLogsMerkleRoot
    ) public onlyOnChainProposer {
        require(
            batchWithdrawalLogsMerkleRoots[withdrawalLogsBatchNumber] ==
                bytes32(0),
            "CommonBridge: withdrawal logs already published"
        );
        batchWithdrawalLogsMerkleRoots[
            withdrawalLogsBatchNumber
        ] = withdrawalsLogsMerkleRoot;
        emit WithdrawalsPublished(
            withdrawalLogsBatchNumber,
            withdrawalsLogsMerkleRoot
        );
    }

    /// @inheritdoc ICommonBridge
    function claimWithdrawal(
        bytes32 l2WithdrawalTxHash,
        uint256 claimedAmount,
        uint256 withdrawalBatchNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) public {
        _claimWithdrawal(
            l2WithdrawalTxHash,
            ETH_TOKEN,
            ETH_TOKEN,
            claimedAmount,
            withdrawalBatchNumber,
            withdrawalLogIndex,
            withdrawalProof
        );
        (bool success, ) = payable(msg.sender).call{value: claimedAmount}("");
        require(success, "CommonBridge: failed to send the claimed amount");
    }

    /// @inheritdoc ICommonBridge
    function claimWithdrawalERC20(
        bytes32 l2WithdrawalTxHash,
        address tokenL1,
        address tokenL2,
        uint256 claimedAmount,
        uint256 withdrawalBatchNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) public nonReentrant {
        _claimWithdrawal(
            l2WithdrawalTxHash,
            tokenL1,
            tokenL2,
            claimedAmount,
            withdrawalBatchNumber,
            withdrawalLogIndex,
            withdrawalProof
        );
        require(tokenL1 != ETH_TOKEN, "CommonBridge: attempted to withdraw ETH as if it were ERC20, use claimWithdrawal()");
        IERC20(tokenL1).safeTransfer(msg.sender, claimedAmount);
    }

    function _claimWithdrawal(
        bytes32 l2WithdrawalTxHash,
        address tokenL1,
        address tokenL2,
        uint256 claimedAmount,
        uint256 withdrawalBatchNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) private {
        require(depositsERC20[tokenL1][tokenL2] >= claimedAmount, "CommonBridge: trying to withdraw more tokens/ETH than were deposited");
        depositsERC20[tokenL1][tokenL2] -= claimedAmount;
        bytes32 msgHash = keccak256(abi.encodePacked(tokenL1, tokenL2, msg.sender, claimedAmount));
        require(
            _claimWithdrawProof(
                l2WithdrawalTxHash,
                msgHash,
                withdrawalBatchNumber,
                withdrawalLogIndex,
                withdrawalProof
            ),
            "CommonBridge: invalid withdrawal proof"
        );
    }

    /// @dev msgHash must be derived using msg.sender to prevent malicious claims
    function _claimWithdrawProof(
        bytes32 l2WithdrawalTxHash,
        bytes32 msgHash,
        uint256 withdrawalBatchNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) internal returns (bool) {
        bytes32 withdrawalId = keccak256(abi.encodePacked(withdrawalBatchNumber, withdrawalLogIndex));
        require(
            batchWithdrawalLogsMerkleRoots[withdrawalBatchNumber] != bytes32(0),
            "CommonBridge: the batch that emitted the withdrawal logs was not committed"
        );
        require(
            withdrawalBatchNumber <=
                IOnChainProposer(ON_CHAIN_PROPOSER).lastVerifiedBatch(),
            "CommonBridge: the batch that emitted the withdrawal logs was not verified"
        );
        require(
            claimedWithdrawals[withdrawalId] == false,
            "CommonBridge: the withdrawal was already claimed"
        );
        claimedWithdrawals[withdrawalId] = true;
        emit WithdrawalClaimed(withdrawalId);
        return _verifyWithdrawProof(
            l2WithdrawalTxHash,
            msgHash,
            withdrawalBatchNumber,
            withdrawalLogIndex,
            withdrawalProof
        );
    }

    function _verifyWithdrawProof(
        bytes32 l2WithdrawalTxHash,
        bytes32 msgHash,
        uint256 withdrawalBatchNumber,
        uint256 withdrawalLogIndex,
        bytes32[] calldata withdrawalProof
    ) internal view returns (bool) {
        bytes32 withdrawalLeaf = keccak256(
            abi.encodePacked(l2WithdrawalTxHash, L2_BRIDGE_ADDRESS, msgHash)
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
        return
            withdrawalLeaf ==
            batchWithdrawalLogsMerkleRoots[withdrawalBatchNumber];
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}
}
