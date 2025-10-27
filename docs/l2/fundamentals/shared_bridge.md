# Shared Bridge

If a user wants to transfer funds from an L2-A to an account on an L2-B, the conventional flow involves moving the assets from L2-A to Ethereum (withdraw), unlocking the funds on Ethereum (claim withdrawal), and then moving those assets from Ethereum to L2-B (deposit). This process requires several steps that ultimately degrade the user experience (UX), and two of them involve transactions on Ethereum, which are often expensive. This happens because there is currently no direct communication path between different L2s, forcing all communication to pass through their common point, Ethereum.

The Shared Bridge feature enables the sending of messages between different L2s, allowing a user to perform the above meta-operation by only interacting with the source L2 (L2-A) and eventually seeing the operation's result reflected on the destination L2 (L2-B).

Although the user only needs to perform a single interaction and wait for the result, a similar flow to the one described above occurs behind the scenes. Below, we will explain how it works.

## High-Level Overview

To understand the behind-the-scenes mechanics, we'll use the previous example. For a recap, here is a detailed breakdown of what happens:

[Image Placeholder]

1. On the source L2: The user calls the sendToL2 function on the L2Bridge contract, specifying the chain ID of the destination L2, the address of the recipient account, and optionally the calldata. The L2Bridge then emits an L2ToL2Message event.
2. On the source L2: The L1Committer collects L2ToL2Message events, builds a Merkle root, attaches it to the commit, and finally calls the commitBatch function on the corresponding OnChainProposer on L1.
3. On L1: The OnChainProposer for the source L2, in response to the commitBatch call, stores the Merkle root of the L2-to-L2 messages.
4. On the source L2: The L1ProofSender calls the verifyBatch function on the OnChainProposer on L1, attaching the L2ToL2Messages.
5. On L1: The OnChainProposer for the source L2, in response to the verifyBatch call, builds a Merkle root from the L2-to-L2 messages and compares it to the one stored earlier in step 3. If the Merkle root built from the messages matches (i.e., it is valid), the incoming messages are forwarded to the Router.
6. On L1: The Router redirects each message to the corresponding L1Bridges via the receiveMessage function.
7. On L1: The L1Bridge for the destination L2 processes the received message in the receiveMessage call and emits an event.
8. On the destination L2: The L1Watcher intercepts and processes the events emitted by the L1Bridge.

## Protocol Details

### How It Works

- The user calls the sendToL2 function on the L2Bridge contract, specifying:
  - The chain ID of the destination L2,
  - The address of the message recipient on the destination L2,
  - The gas limit that the sender is willing to consume for the final transaction on the destination L2. Note that this gas is burned on the source L2. It remains pending to define how the user should proceed to recover the burned gas if the transaction on the destination L2 reverts.
  - (Optional) The calldata for the transaction to execute on the destination L2.
- The sendToL2 function on the L2Bridge contract burns the amount of gas specified by the user and then calls the sendMessageToL2 function on the L2ToL1Messanger contract, passing the same parameters provided by the user, along with the address of the user (msg.sender) who initiated the call.
- The sendMessageToL2 function on the L2ToL1Messanger contract increments the counter for sent L2 messages (L2 message IDs) and emits the L2ToL2Message event, which includes the parameters provided by the user plus the user's address. This event represents the message from the source L2 to the destination L2.
- The L1Committer collects the L2ToL2Message events emitted in the blocks of the batch it is about to commit, builds a Merkle root from them, attaches it to the commit, and finally calls the commitBatch function on the OnChainProposer corresponding to the source L2.
- The OnChainProposer corresponding to the source L2...

## Gas

TODO

## Troubleshooting

TODO

## Proving

TODO
