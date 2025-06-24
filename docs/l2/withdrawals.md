# Withdrawals

This document contains a detailed explanation of the changes needed to handle withdrawals and the withdrawal flow.

First, we need to understand the generic mechanism behind it:

## L1Message

To allow generic L2->L1 messages, a system contract is added which allows sending arbitary data.

```
struct L1Message {
    tx_hash: H256, // L2 transaction where it was included
    address: Address, // Who called L1Message.sol
    data: bytes32 // payload
}
```

This data is collected, put in a merkle tree whose root is published as part of the batch commitment.

This way, L1 contracts can access the data.

## Bridging

On the L2 side, a contract burns the eth (or other assets, in the future) and emits a message to the L1 containing the details of this operation:
- Destination: L1 address that can claim the deposit
- Amount: how much was burnt

When the batch is commited, the OnChainProposer notifies the bridge which saves the message tree root.

Once the batch containing this transaction is verified, the user can claim their funds on the L1.

To do this, they compute a merkle proof for the included batch and call the L1 bridge contract.

This contract then:
- Verifies that the batch is validated
- Ensures the withdrawal wasn't already claimed
- Computes the expected leaf
- Validates that the proof leads from the leaf to the root of the message tree
- Gives the funds to the user
- Marks the withdrawl as claimed
