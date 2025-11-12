# Ethrex L2 contracts

There are two L1 contracts: OnChainProposer and CommonBridge. Both contracts are deployed using UUPS proxies, so they are upgradeables.

## L1 Contracts

### `CommonBridge`
The `CommonBridge` is an upgradeable smart contract that facilitates cross-chain transfers between L1 and L2.

#### **State Variables**

- **`pendingTxHashes`**: Array storing hashed pending privileged transactions
- **`batchWithdrawalLogsMerkleRoots`**: Mapping of L2 batch numbers to merkle roots of withdrawal logs
- **`deposits`**: Tracks how much of each L1 token was deposited for each L2 token (L1 → L2 → amount)
- **`claimedWithdrawalIDs`**: Tracks which withdrawals have been claimed by message ID
- **`ON_CHAIN_PROPOSER`**: Address of the contract that can commit and verify batches
- **`L2_BRIDGE_ADDRESS`**: Constant address (0xffff) representing the L2 bridge

#### **Core Functionality**

1. **Deposits (L1 → L2)**
    - **`deposit()`**: Allows users to deposit ETH to L2
    - **`depositERC20()`**: Allows users to deposit ERC20 tokens to L2
    - **`receive()`**: Fallback function for ETH deposits, forwarding to the sender's address on the L2
    - **`sendToL2()`**: Sends arbitrary data to L2 via privileged transaction

    Internally the deposit functions will use the `SendValues` struct defined as:

    ```solidity
    struct SendValues {
        address to; // Target address on L2
        uint256 gasLimit; // Maximum gas for L2 execution
        uint256 value; // The value of the transaction
        bytes data; // Calldata to execute on the target L2 contract
    }
    ```
    This expresivity allows for arbitrary cross-chain actions, e.g., depositing ETH then interacting with an L2 contract.
2. **Withdrawals (L2 → L1)**
    - **`claimWithdrawal()`**: Withdraw ETH from `CommonBridge` via Merkle proof
    - **`claimWithdrawalERC20()`**: Withdraw ERC20 tokens from `CommonBridge` via Merkle proof
    - **`publishWithdrawals()`**: Priviledged function to add merkle root of L2 withdrawal logs to `batchWithdrawalLogsMerkleRoots` mapping to make them claimable
3. **Transaction Management**
    - **`getPendingTransactionHashes()`**: Returns pending privileged transaction hashes
    - **`removePendingTransactionHashes()`**: Removes processed privileged transactions (only callable by OnChainProposer)
    - **`getPendingTransactionsVersionedHash()`**: Returns a versioned hash of the first `number` of pending privileged transactions

### `OnChainOperator`
The `OnChainProposer` is an upgradeable smart contract that ensures the advancement of the L2. It's used by sequencers to commit batches of L2 blocks and verify their proofs.

#### **State Variables**

- **`batchCommitments`**: Mapping of batch numbers to submitted `BatchCommitmentInfo` structs
- **`lastVerifiedBatch`**: The latest verified batch number (all batches ≤ this are considered verified) 
- **`lastCommittedBatch`**: The latest committed batch number (all batches ≤ this are considered committed)
- **`authorizedSequencerAddresses`**: Mapping of authorized sequencer addresses that can commit and verify batches

#### **Core Functionality**

1. **Batch Commitment**
    - **`commitBatch()`**: Commits a batch of L2 blocks by storing its commitment data and publishing withdrawals
    - **`revertBatch()`**: Removes unverified batches (only callable when paused)

2. **Proof Verification**
    - **`verifyBatch()`**: Verifies a single batch using RISC0, SP1, or TDX proofs
    - **`verifyBatchesAligned()`**: Verifies multiple batches in sequence using aligned proofs with Merkle verification

3. **State Validation**
    - **`_verifyPublicData()`**: Internal function used during `verifyBatch()` or `verifyBatchesAligned()` that validates public proof inputs match previous data from `commitBatch()`


## L2 Contracts

### `CommonBridgeL2`
The `CommonBridgeL2` is an L2 smart contract that facilitates cross-chain transfers between L1 and L2.

#### **State Variables**

- **`L1_MESSENGER`**: Constant address (`0x000000000000000000000000000000000000FFFE`) representing the L2-to-L1 messenger contract
- **`BURN_ADDRESS`**: Constant address (`0x0000000000000000000000000000000000000000`) used to burn ETH during withdrawals
- **`ETH_TOKEN`**: Constant address (`0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE`) representing ETH as a token

#### **Core Functionality**

1. **ETH Operations**
    - **`withdraw()`**: Initiates ETH withdrawal to L1 by burning ETH on L2 and sending a message to L1
    - **`mintETH()`**: Transfers ETH to a recipient (called by privileged L1 bridge transactions). If it fails a withdrawal is queued.

2. **ERC20 Token Operations**
    - **`mintERC20()`**: Attempts to mint ERC20 tokens on L2 (only callable by the bridge itself via privileged transactions). If it fails a withdrawal is queued.
    - **`tryMintERC20()`**: Internal function that validates token L1 address and performs a cross-chain mint
    - **`withdrawERC20()`**: Initiates ERC20 token withdrawal to L1 by burning tokens on L2 and sending a message to L1

3. **Cross-Chain Messaging**
    - **`_withdraw()`**: Private function that sends withdrawal messages to L1 via the L2-to-L1 messenger
    - Uses keccak256 hashing to encode withdrawal data for L1 processing

4. **Access Control**
    - **`onlySelf`**: Modifier ensuring only the bridge contract itself can call privileged functions
    - Validates that privileged operations (like minting) are only performed by the bridge

### `L2ToL1Messenger`
The `L2ToL1Messenger` is a simple L2 smart contract that enables communication from L2 to L1 by emitting the data as `L1Message` events for sequencers to pick up.

#### **State Variables**

- **`lastMessageId`**: Counter that tracks the ID of the last emitted message (incremented before each message is sent)

#### **Core Functionality**

1. **Message Sending**
    - **`sendMessageToL1()`**: Sends a message to L1 by emitting an `L1Message` event with the sender, data, and `lastMessageId`

## Upgrade the contracts

To upgrade a contract, you have to create the new contract and, as the original one, inherit from OpenZeppelin's `UUPSUpgradeable`. Make sure to implement the `_authorizeUpgrade` function and follow the [proxy pattern restrictions](https://docs.openzeppelin.com/upgrades-plugins/writing-upgradeable).

Once you have the new contract, you need to do the following three steps:

1. Deploy the new contract

    ```sh
    rex deploy <NEW_IMPLEMENTATION_BYTECODE> 0 <DEPLOYER_PRIVATE_KEY>
    ```

2. Upgrade the proxy by calling the method `upgradeToAndCall(address newImplementation, bytes memory data)`. The `data` parameter is the calldata to call on the new implementation as an initialization, you can pass an empty stream.

    ```sh
    rex send <PROXY_ADDRESS> 'upgradeToAndCall(address,bytes)' <NEW_IMPLEMENTATION_ADDRESS> <INITIALIZATION_CALLDATA> --private-key <PRIVATE_KEY>
    ```

3. Check the proxy updated the pointed address to the new implementation. It should return the address of the new implementation:

    ```sh
    curl http://localhost:8545 -d '{"jsonrpc": "2.0", "id": "1", "method": "eth_getStorageAt", "params": [<PROXY_ADDRESS>, "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc", "latest"]}'
    ```

## Transfer ownership

The contracts are `Ownable2Step`, that means that whenever you want to transfer the ownership, the new owner have to accept it to effectively apply the change. This is an extra step of security, to avoid accidentally transfer ownership to a wrong account. You can make the transfer in these steps:

1. Start the transfer:

    ```sh
    rex send <PROXY_ADDRESS> 'transferOwnership(address)' <NEW_OWNER_ADDRESS> --private-key <CURRENT_OWNER_PRIVATE_KEY>
    ```

2. Accept the ownership:

    ```sh
    rex send <PROXY_ADDRESS> 'acceptOwnership()' --private-key <NEW_OWNER_PRIVATE_KEY>
    ```
