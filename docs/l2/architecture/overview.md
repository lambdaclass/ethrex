# General overview of the ethrex L2 stack

This document aims to explain how the Lambda ethrex L2 and all its moving parts work.

## Intro

At a high level, the way an L2 works is as follows:

- There is a contract in L1 that tracks the current state of the L2. Anyone who wants to know the current state of the chain need only consult this contract.
- Every once in a while, someone (usually the sequencer, but could be a decentralized network, or even anyone at all in the case of a based contestable rollup) builds a batch of new L2 blocks and publishes it to L1. We will call this the `commit` L1 transaction.
- For L2 batches to be considered finalized, a zero-knowledge proof attesting to the validity of the batch needs to be sent to L1, and its verification needs to pass. If it does, everyone is assured that all blocks in the batch were valid and thus the new state is. We call this the `verification` L1 transaction.

We ommited a lot of details in this high level explanation. Some questions that arise are:

- What does it mean for the L1 contract to track the state of L2? Is the entire L2 state kept on it? Isn't it really expensive to store a bunch of state on an Ethereum smart contract?
- What does the ZK proof prove exactly?
- How do we make sure that the sequencer can't do anything malicious if it's the one proposing blocks and running every transaction?
- How does someone go in and out of the L2, i.e., how do you deposit money from L1 into L2 and then withdraw it? How do you ensure this can't be tampered with? Bridges are by far the most vulnerable part of blockchains today and going in and out of the L2 totally sounds like a bridge.

You can find answers to these questions and more in the [prover docs](../../prover/prover.md).

## Recap

### Batch Commitment

An L2 batch commitment contains:

- The new L2 state root.
- The latest block's hash
- The KZG versioned hash of the blobs published by the L2
- The rolling hash of the processed privileged transactions
- The Merkle root of the withdrawal logs

These are committed as public inputs of the zk proof that validates a new L2 state.

## L1 contract checks

### Commit transaction

For the `commit` transaction, the L1 verifier contract receives the batch commitment, as defined previously, for the new batch.

The contract will then:

- Check that the batch number is the immediate successor of the last committed batch.
- Check that the batch has not been committed already.
- Check that the `lastBlockHash` is not zero.
- If privileged transactions were processed, it checks the submitted hash against the one in the `CommonBridge` contract.
- If withdrawals were processed, it publishes them to the `CommonBridge` contract.
- It checks that a blob was published if the L2 is running as a rollup, or that no blob was published if it's running as a validium.
- Calculate the new batch commitment and store it.

### Verify transaction

On a `verification` transaction, the L1 contract receives the following:

- The batch number.
- The RISC-V Zero-Knowledge proof of the batch execution (if enabled).
- The SP1 Zero-Knowledge proof of the batch execution (if enabled).
- The TDX Zero-Knowledge proof of the batch execution (if enabled).

The contract will then:

- Check that the batch number is the immediate successor of the last verified batch.
- Check that the batch has been committed.
- It removes the pending transaction hashes from the `CommonBridge` contract.
- It verifies the public data of the proof, checking that the data committed in the `commitBatch` call matches the data in the public inputs of the proof.
- Pass the proof and public inputs to the verifier and assert the proof passes.
- If the proof passes, finalize the L2 state, setting the latest batch as the given one and allowing any withdrawals for that batch to occur.

## What the sequencer cannot do

- **Forge Transactions**: Invalid transactions (e.g. sending money from someone who did not authorize it) are not possible, since part of transaction execution requires signature verification. Every transaction has to come along with a signature from the sender. That signature needs to be verified; the L1 verifier will reject any block containing a transaction whose signature is not valid.
- **Withhold State**: Every L1 `commit` transaction needs to send the corresponding state diffs for it and the contract, along with the proof, make sure that they indeed correspond to the given batch. TODO: Expand with docs on how this works.
- **Mint money for itself or others**: The only valid protocol transaction that can mint money for a user is an L1 deposit. Every one of these mint transactions is linked to exactly one deposit transaction on L1. TODO: Expand with some docs on the exact details of how this works.

## What the sequencer can do

The main thing the sequencer can do is CENSOR transactions. Any transaction sent to the sequencer could be arbitrarily dropped and not included in blocks. This is not completely enforceable by the protocol, but there is a big mitigation in the form of an **escape hatch**.

TODO: Explain this in detail.
