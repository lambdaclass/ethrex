# Shared Bridge

## Motivation

If a user wants to send funds from one L2 to another, the basic flow would be to withdraw its ETH from the first L2 to L1 and then deposit them in the second L2. The goal is to be able to send funds and messages from one L2 to another L2 in just one transaction, without going through L1.


## Overview

The flow is as follows:
- A user on L2-A (Alice) wants to send ETH to a user on L2-B (Bob).
- Alice sends a transaction on L2-A to the L2's CommonBridge, specifying Bob's address on L2-B and the amount of ETH to send. The ETH is burned on L2-A.
- The sequencer eventually seals a batch including Alice's transaction and submits a commitment to L1. This commitment includes, among other things, a merkle root of all transactions for other L2s in the batch and a list of balance to transfer to the other L2s.
- The prover will now generate a zk-proof that the commitment is valid. Once the proof is generated, it is submitted to the L1, which verifies the proof and mark the commitment as valid. If the proof is valid, the OnChainProposer contract will ask L2-A's CommonBridge to transfer the ETH to L2-B's CommonBridge. This is done through the Router contract (more on this later).
- In parallel, the sequencer of L2-B will be periodically checking a list of known L2 servers (including L2-A) for new messages for L2-B. L2-A will respond with the Alice transfer, including transaction data (sender, destination, amount, etc.) and a merkle path for the committed merkle root. L2-B sequencer will then check (through the Router contract) if the information provided is valid, in which case it will mint ETH to Bob's address.

Note this is a simply ETH transfer example but should be easily extensible to arbitrary messages (contract calls).

## Router contract

The Router is the responsible for routing messages between L2s. It is deployed on L1 and chain operators of each L2 need to register their chain on it. For now, the Router is permissioned, meaning only administrators can register new chains. The Router exposes two main functions:

- `sendMessage`: Sends the balance needed to cover outgoing transactions to the destination L2's CommonBridge.
- `verifyMessage`: Verifies that a message coming from another L2 is valid. It checks that the commitment including the message has been verified on L1.
