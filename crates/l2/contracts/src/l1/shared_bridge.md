# Shared Bridge

The goal is to be able to send funds and messages from one L2 to another L2 in just one transaction, without going through L1.

## Overview

The following diagram illustrates the architecture of the shared bridge:

The flow is as follows:
- A user on L2-A (Alice) wants to send ETH to a user on L2-B (Bob).
- Alice sends a transaction on L2-A to the L2's CommonBridge, specifying Bob's address on L2-B and the amount of ETH to send. The ETH is burned on L2-A.
- The sequencer eventually seals a batch including Alice's transaction and submits a commitment to L1. This commitment includes, among other things, a merkle root of all transactions for other L2s in the batch and a list of balance to transfer to the other L2s.
- The prover will now generate a zk-proof that the commitment is valid. Once the proof is generated, it is submitted to the L1, which verifies the proof and mark the commitment as valid. If the proof is valid, the OnChainProposer contract will ask L2-A's CommonBridge to transfer the ETH to L2-B's CommonBridge. This is done through the Router contract (more on this later).
- In parallel, the sequencer of L2-B will be periodically checking a list of known L2 servers (including L2-A) for new messages for L2-B. L2-A will respond with the Alice transfer, including transaction data (sender, destination, amount, etc.) and a merkle path for the committed merkle root. L2-B sequencer will then check (through the Router contract) if the information provided is valid, in which case it will mint ETH to Bob's address.

Note this is a simply ETH transfer example but should be easily extensible to arbitrary messages (contract calls).

## Router contract

The Router is the responsible for routing messages between L2s. It is deployed on L1 and chain operators of each L2 need to register their chain on it. For now, the Router is permissioned, meaning only administrators can register new chains. The Router exposes two main functions:

- `transfer`: Sends the balance needed to cover outgoing transactions to the destination L2's CommonBridge.
- `verifyMessage`: Verifies that a message coming from another L2 is valid. It checks that the commitment including the message has been verified on L1.

## Pending design decisions

- When should L2-A respond with Alice transaction?
  As the transaction needs to be validated, L2-A could offer it as soon as it is included in a block, or wait until its batch is verified.
- Incoming transactions enforcement on L2-B:
  We need a mechanism to enforce destination L2s to process transactions from other L2s, as we have for the ones coming from L1. A similar approach can be used.
- Gas fees:
  When sending a transaction from L2-A to L2-B, who pays for the gas fees on L2-B? Possible approaches:
  - The user on L2-A sets a gas fee for L2-B when sending the transaction. The gas fee is burned in L2-A. This would require estimating the gas fees on L2-B at the time of sending the transaction on L2-A. In this case, essentialy, L2-A operator is getting the fee for a transaction processed in L2-B.
  - The user on L2-B pays for the gas fees when the transaction is processed. This would require Bob to have some ETH on L2-B to pay for the gas fees.
  - The sender user on L2-A pays for the gas fees on L2-B as part of the transaction. This would require Alice to have some extra ETH on L2-B to cover for the gas fees.
- Error handling:
  What happens if the transaction fails at any point? Where does the ETH go? What happens to the gas fees?
## Data availability

If a node fails to respond with the transaction data when queried by another L2's sequencer, the sequencer won't be able to include the transaction on L2-B. This creates a data availability problem, as it could potentially block L2-B if it requires including that transactions. To solve this, L2s will need to send a list of all the batch transactions as Blobs in their commitments. This way, if a node fails to respond, the sequencer can still get the data from L1.
This implies the L2s should trust each other to not include invalid data in their blobs, as L2-B's sequencer won't be able to verify the data itself, hence the permissioned nature of the Router contract.