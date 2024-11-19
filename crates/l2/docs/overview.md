# High Level Docs

This document aims to explain how the Lambda ethrex L2 and all its moving parts work.

## Intro

At a high level, the way an L2 works is as follows:

- There is a contract in L1 that tracks the current state of the L2. Anyone who wants to know the current state of the network need only consult this contract.
- Every once in a while, someone (usually the sequencer, but could be a decentralized network, or even anyone at all in the case of a based contestable rollup) builds a new L2 block and publishes it to L1. We will call this the `commit` L1 transaction.
- For L2 blocks to be considered finalized, a zero-knowledge proof attesting to the validity of said block needs to be sent to L1, and its verification needs to pass. If it does, everyone is assured that the block was valid and thus the new state is. We call this the `verification` L1 transaction.

We ommited a lot of details in this high level explanation. Some questions that arise are:

- What does it mean for the L1 contract to track the state of L2? Is the entire L2 state kept on it? Isn't it really expensive to store a bunch of state on an Ethereum smart contract?
- What does the ZK proof prove exactly?
- How do we make sure that the sequencer can't do anything malicious if it's the one proposing blocks and running every transaction?
- How does someone go in and out of the L2, i.e., how do you deposit money from L1 into L2 and then withdraw it? How do you ensure this can't be tampered with? Bridges are by far the most vulnerable part of blockchains today and going in and out of the L2 totally sounds like a bridge.

Below some answers to these questions, along with an overview of all the moving parts of the system.

## How do you prove state?

Now that general purpose `zkVM`s exist, most people have little trouble with the idea that you can prove execution. Just take the usual EVM code you wrote in Rust, compile to some `zkVM` target instead and you're mostly done. You can now prove it.

What's usually less clear is how you prove state. Let's say we want to prove a new L2 block that was just built. Running the `ethereum_rust` `execute_block` function on a Rust `zkVM` does the trick, but that only proves that you ran the VM correctly on **some** previous state/block. How do you know it was the actual previous state of the L2 and not some other, modified one?

In other words, how do you ensure that:

- Every time the EVM **reads** from some storage slot (think an account balance, some contract's bytecode), the value returned matches the actual value present on the previous state of the network.

For this, the VM needs to take as a public input the previous state of the L2, so the prover can show that every storage slot it reads is consistent with it, and the verifier contract on L1 can check that the given public input is the actual previous state it had stored. However, we can't send the entire previous state as public input because it would be too big; this input needs to be sent on the `verification` transaction, and the entire L2 state does not fit on it.

To solve this, we do what we always do: instead of having the actual previous state be the public input, we build a **Merkle Tree** of the state and **use its root as the input**. Now the state is compressed into a single 32-byte value, an unforgeable representation of it; if you try to change a single bit, the root will change. This means we now have, for every L2 block, a single hash that we use to represent it, which we call the block `commitment` (we call it "commitment" and not simply "state root" because, as we'll see later, this won't just be the state root, but rather the hash of a few different values including the state root).

The flow for the prover is then roughly as follows:

- Take as public input the previous block commitment and the next (output) block commitment.
- Execute the current block to prove its execution is valid. Here "execution" means more than just transaction execution; there's also header validation, transaction validation, etc. (essentially all the logic `ethereum_rust` needs to follow when executing and adding a new block to the chain).
- For every storage slot read, present and verify a merkle path from it to the previous state root (i.e. previous block commitment).
- For every storage slot written, present and verify a merkle path from it to the next state root (i.e. next block commitment).

As a final note, to keep the public input a 32 byte value, instead of passing the previous and next block commitments separately, we hash the two of them and pass that. The L1 contract will then have an extra step of first taking both commitments and hashing them together to form the public input.

These two ideas will be used extensively throughout the rest of the documentation:

- Whenever we need to add some state as input, we build a merkle tree and use its **root** instead. Whenever we use some part of that state in some way, the prover provides merkle paths to the values involved. Sometimes, if we don't care about efficient inclusion proofs of parts of the state, we just hash the data altogether and use that instead.
- To keep the block commitment (i.e. the value attesting to the entire state of the network) a 32 byte value, we hash the different public inputs into one. The L1 contract is given all the public inputs on `commit`, checks their validity and then squashes them into one through hashing.

## Reconstructing state/Data Availability

While using a merkle root as a public input for the proof works well, there is still a need to have the state on L1. If the only thing that's published to it is the state root, then the sequencer could withhold data on the state of the network. Because it is the one proposing and executing blocks, if it refuses to deliver certain data (like a merkle path to prove a withdrawal on L1), people may not have any place to get it from and get locked out of the network or some of their funds.

This is called the **Data Availability** problem. As discussed before, sending the entire state of the network on every new L2 block is impossible; state is too big. As a first next step, what we could do is:

- For every new L2 block, send as part of the `commit` transaction the list of transactions in the block. Anyone who needs to access the state of the L2 at any point in time can track all `commit` transactions, start executing them from the beginning and recontruct the state.

This is now feasible; if we take 200 bytes as a rough estimate for the size of a single transfer between two users (see [this post](https://ethereum.stackexchange.com/questions/30175/what-is-the-size-bytes-of-a-simple-ethereum-transaction-versus-a-bitcoin-trans) for the calculation on legacy transactions) and 128 KB as [a reasonable transaction size limit](https://github.com/ethereum/go-ethereum/blob/830f3c764c21f0d314ae0f7e60d6dd581dc540ce/core/txpool/legacypool/legacypool.go#L49-L53) we get around ~650 transactions at maximum per `commit` transaction (we are assuming we use calldata here, blobs can increase this limit as each one is 128 KB and we could use multiple per transaction).

Going a bit further, instead of posting the entire transaction, we could just post which storage slots have been modified and their new value (this includes deployed contracts and their bytecode of course). This can reduce the size a lot for most cases; in the case of a regular transfer as above, we are modifying storage for two accounts, which is just two `(address, balance)` pairs, so (20 + 32) * 2 = 104 bytes, or around half as before. Some other clever techniques and compression algorithms can push down the publishing cost of this and other transactions much further.

This is called `state diffs`. Instead of publishing entire transactions for data availability, we only publish whatever state they modified. This is enough for anyone to reconstruct the entire state of the network.

Detailed documentation on the state diffs spec [here](./state_diffs.md).

### How do we prevent the sequencer from publishing the wrong state diffs?

Once again, state diffs have to be part of the public input. With them, the prover can show that they are equal to the ones returned by the VM after executing the block. As always, the actual state diffs are not part of the public input, but **their hash** is, so the size is a fixed 32 bytes. This hash is then part of the block commitment. The prover then assures us that the given state diff hash is correct (i.e. it exactly corresponds to the changes in state of the executed block). 

There's still a problem however: the L1 contract needs to have the actual state diff for data availability, not just the hash. This is sent as part of calldata of the `commit` transaction (actually later as a blob, we'll get to that), so the sequencer could in theory send the wrong state diff. To make sure this can't happen, the L1 contract hashes it to make sure that it matches the actual state diff hash that is included as part of the public input.

With that, we can be sure that state diffs are published and that they are correct. The sequencer cannot mess with them at all; either it publishes the correct state diffs or the L1 contract will reject its block.

### Compression

Because state diffs are compressed to save space on L1, this compression needs to be proven as well. Otherwise, once again, the sequencer could send the wrong (compressed) state diffs. This is easy though, we just make the prover run the compression and we're done.

## EIP 4844 (a.k.a. Blobs)

While we could send state diffs through calldata, there is a (hopefully) cheaper way to do it: blobs. The Ethereum Cancun upgrade introduced a new type of transaction where users can submit a list of opaque blobs of data, each one of size at most 128 KB. The main purpose of this new type of transaction is precisely to be used by rollups for data availability; they are priced separately through a `blob_gas` market instead of the regular `gas` one and for all intents and purposes should be much cheaper than calldata.

Using EIP 4844, our state diffs would now be sent through blobs. While this is cheaper, there's a new problem to address with it. The whole point of blobs is that they're cheaper because they are only kept around for approximately two weeks and ONLY in the beacon chain, i.e. the consensus side. The execution side (and thus the EVM when running contracts) does not have access to the contents of a blob. Instead, the only thing it has access to is a **KZG commitment** of it.

This is important. If you recall, the way the L1 ensured that the state diff published by the sequencer was correct was by hashing its contents and ensuring that the hash matched the given state diff hash. With the contents of the state diff now no longer accesible by the contract, we can't do that anymore, so we need another way to ensure the correct contents of the state diff (i.e. the blob).

The solution is through a [proof of equivalence](https://ethresear.ch/t/easy-proof-of-equivalence-between-multiple-polynomial-commitment-schemes-to-the-same-data/8188) between polynomial commitment schemes. The idea is as follows: proofs of equivalence allow you to show that two (polynomial) commitments point to the same underlying data. In our case, we have two commitments:

- The state diff commitment calculated by the sequencer/prover. 
- The KZG commitment of the blob sent on the commit transaction (recall that the blob should just be the state diff).

If we turn the first one into a polynomial commitment, we can take a random evaluation point through Fiat Shamir and prove that it evaluates to the same value as the KZG blob commitment at that point. The `commit` transaction then sends the blob commitment and, through the point evaluation precompile, verifies that the given blob evaluates to that same value. If it does, the underlying blob is indeed the correct state diff.

Our proof of equivalence implementation follows Method 1 [here](https://notes.ethereum.org/@dankrad/kzg_commitments_in_proofs). What we do is the following.

### Prover side

- Take the state diff being commited to as `4096` 32-byte chunks (these will be interpreted as field elements later on, but for now we don't care). Call these chunks $d_i$, with `i` ranging from 0 to 4095.
- Build a merkle tree with the $d_i$ as leaves. Note that we can think of the merkle root as a polynomial commitment, where the `i`-th leaf is the evaluation of the polynomial on the `i`-th power of $\omega$, the `4096`-th root of unity on $F_q$, the field modulus of the `BLS12-381` curve. Call this polynomial $f$. This is the same polynomial that the L1 KZG blob commits to (by definition). Call the L1 blob KZG commitment $C_1$ and the merkle root we just computed $C_2$.
- Choose `x` as keccak($C_1$, $C_2$) and calculate the evaluation $f(x)$; call it `y`. To do this calculation, because we only have the $d_i$, the easiest way to do it is through the [barycentric formula](https://dankradfeist.de/ethereum/2021/06/18/pcs-multiproofs.html#evaluating-a-polynomial-in-evaluation-form-on-a-point-outside-the-domain). IMPORTANT: we are taking the $d_i$, `x`, `y`, and $\omega$ as elements of $F_q$, NOT the native field used by our prover. The evaluation thus is:

    $$y = f(x) = \dfrac{x^{4096} - 1}{4096} \sum_{i = 0}^{4095} d_i \dfrac{\omega^i}{x - \omega^i}$$
- Set `x` and `y` as public inputs. All the above shows the verifier on L1 that we made a polynomial commitment to the state diff, that its evaluation on `x` is `y`, and that `x` was chosen through Fiat-Shamir by hashing the two commitments.

### Verifier side

- When commiting to the data on L1 send, as part of the calldata, a kzg blob commitment along with an opening proving that it evaluates to `y` on `x`. The contract, through the point evaluation precompile, checks that both:
  - The commitment's hash is equal to the versioned hash for that blob.
  - The evaluation is correct.

## How do deposits and withdrawals work?

### Deposits

TODO

### Withdrawals

Detailed specs [here](./withdrawals.md).

TODO: Explain it a high level maybe?

## Recap

### Block Commitment

An L2 block commitment is the hash of the following things:

- The L2 state root.
- The state diff hash or polynomial commitments, depending on whether we are using calldata or blobs.
- The Withdrawal logs merkle root.

The public input to the proof is then the hash of the previous block commitment and the new one.

## L1 contract checks

### Commit transaction

For the `commit` transaction, the L1 verifier contract then receives the following things from the sequencer:

- The L2 block number to be commited.
- The new L2 state root/
- The Withdrawal logs merkle root.
- The state diffs hash or polynomial commitment scheme accordingly.

The contract will then:

- Check that the block number is the immediate successor of the last block processed.
- Check that the state diffs are valid, either through hashing or the point evaluation precompile.
- Calculate the new block commitment and store it.

### Verify transaction

On a `verification` transaction, the L1 contract receives the following:

- The block number.
- The block proof.

The contract will then:

- Compute the proof public input from the new and previous block commitments (both are already stored in the contract).
- Pass the proof and public inputs to the verifier and assert the proof passes.
- If the proof passes, finalize the L2 state, setting the latest block as the given one and allowing any withdrawals for that block to occur.


## What the sequencer cannot do

- **Forge Transactions**: Invalid transactions (e.g. sending money from someone who did not authorize it) are not possible, since part of transaction execution requires signature verification. Every transaction has to come along with a signature from the sender. That signature needs to be verified; the L1 verifier will reject any block containing a transaction whose signature is not valid.
- **Withhold State**: Every L1 `commit` transaction needs to send the corresponding state diffs for it and the contract, along with the proof, make sure that they indeed correspond to the given block. TODO: Expand with docs on how this works.
- **Mint money for itself or others**: The only valid protocol transaction that can mint money for a user is an L1 deposit. Every one of these mint transactions is linked to exactly one deposit transaction on L1. TODO: Expand with some docs on the exact details of how this works.

## What the sequencer can do

The main thing the sequencer can do is CENSOR transactions. Any transaction sent to the sequencer could be arbitrarily dropped and not included in blocks. This is not completely enforceable by the protocol, but there is a big mitigation in the form of an **escape hatch**.

TODO: Explain this in detail.
