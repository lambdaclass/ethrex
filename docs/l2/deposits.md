# Deposits

This document contains a detailed explanation of how asset deposits work.

## Native ETH deposits

This section explains step by step how native ETH deposits work.

On L1:

1. The user sends ETH to the `CommonBridge` contract.
2. The bridge adds the hash of the deposit to the `pendingDepositLogs`.
3. The bridge emits a `DepositInitiated` event:

    ```solidity
    emit DepositInitiated(
        msg.value,   // amount
        msg.sender,  // to
        depositId,
        msg.sender,  // recipient of the deposit
        msg.sender,  // sender in L2
        21000 * 5,   // gas limit
        "",          // calldata
        l2MintTxHash
    );
    ```

Off-chain:

1. The L1 watcher on each node processes `DepositInitiated` events, each adding a `PrivilegedL2Transaction` to the L2 mempool.
2. The privileged transaction is treated similarly to an EIP-1559 transaction, but with the following changes:
   1. They don't have sender signatures. Those are validated in the L1, since the sender of the L1 deposit transaction is the sender in the L2.
      As a consequence of this, privileged transactions can also be sent from L1 contracts.
   2. At the start of the execution, the `recipient` account balance is increased by the transaction's value. The transaction's value is set to zero during the rest of the execution.
   3. The sender account's nonce isn't increased as a result of the execution.
   4. The sender isn't charged for the gas costs of the execution.

On L2:

1. A sequencer commits a batch on L1 including the privileged transaction.
2. The `OnChainProposer` notifies the bridge of the consumed privileged transactions.
3. The bridge removes them from `pendingDepositLogs`, asserting the included privileged transactions exist and are included in order.
<!-- TODO: do we require privileged transactions to be included in order inside each batch? -->

<!-- TODO: add diagram -->
