# Reconstructing state or Data Availability

Rollups, unlike validiums, need to have their state on L1. If the only thing that's published to it is the state root, then the sequencer could withhold data on the state of the chain. Because it is the one proposing and executing blocks, if it refuses to deliver certain data (like a merkle path to prove a withdrawal on L1), people may not have any place to get it from and get locked out of the chain or some of their funds.

This is called the **Data Availability** problem. Sending the entire state of the chain on every new L2 batch is impossible; state is too big. As a first next step, what we could do is:

- For every new L2 batch, send as part of the `commit` transaction the list of transactions in the batch. Anyone who needs to access the state of the L2 at any point in time can track all `commit` transactions, start executing them from the beginning and reconstruct the state.

This is now feasible; if we take 200 bytes as a rough estimate for the size of a single transfer between two users (see [this post](https://ethereum.stackexchange.com/questions/30175/what-is-the-size-bytes-of-a-simple-ethereum-transaction-versus-a-bitcoin-trans) for the calculation on legacy transactions) and 128 KB as [a reasonable transaction size limit](https://github.com/ethereum/go-ethereum/blob/830f3c764c21f0d314ae0f7e60d6dd581dc540ce/core/txpool/legacypool/legacypool.go#L49-L53) we get around ~650 transactions at maximum per `commit` transaction (we are assuming we use calldata here, blobs can increase this limit as each one is 128 KB and we could use multiple per transaction).

Going a bit further, instead of posting the entire transaction, we could just post which accounts have been modified and their new values (this includes deployed contracts and their bytecode of course). This can reduce the size a lot for most cases; in the case of a regular transfer as above, we only need to record balance updates of two accounts, which requires sending just two `(address, balance)` pairs, so (20 + 32) * 2 = 104 bytes, or around half as before. Some other clever techniques and compression algorithms can push down the publishing cost of this and other transactions much further.

This is called `state diffs`. Instead of publishing entire transactions for data availability, we only publish whatever state they modified. This is enough for anyone to reconstruct the entire state of the chain.

Detailed documentation on [the state diffs spec](./state_diffs.md).

### How do we prevent the sequencer from publishing the wrong state diffs?

Once again, state diffs have to be part of the public input. With them, the prover can show that they are equal to the ones returned by the VM after executing all blocks in the batch. As always, the actual state diffs are not part of the public input, but **their hash** is, so the size is a fixed 32 bytes. This hash is then part of the batch commitment. The prover then assures us that the given state diff hash is correct (i.e. it exactly corresponds to the changes in state of the executed blocks).

There's still a problem however: the L1 contract needs to have the actual state diff for data availability, not just the hash. This is sent as part of calldata of the `commit` transaction (actually later as a blob, we'll get to that), so the sequencer could in theory send the wrong state diff. To make sure this can't happen, the L1 contract hashes it to make sure that it matches the actual state diff hash that is included as part of the public input.

With that, we can be sure that state diffs are published and that they are correct. The sequencer cannot mess with them at all; either it publishes the correct state diffs or the L1 contract will reject its batch.

### Compression

Because state diffs are compressed to save space on L1, this compression needs to be proven as well. Otherwise, once again, the sequencer could send the wrong (compressed) state diffs. This is easy though, we just make the prover run the compression and we're done.

## EIP 4844 (a.k.a. Blobs)

While we could send state diffs through calldata, there is a (hopefully) cheaper way to do it: blobs. The Ethereum Cancun upgrade introduced a new type of transaction where users can submit a list of opaque blobs of data, each one of size at most 128 KB. The main purpose of this new type of transaction is precisely to be used by rollups for data availability; they are priced separately through a `blob_gas` market instead of the regular `gas` one and for all intents and purposes should be much cheaper than calldata.

Using EIP 4844, our state diffs would now be sent through blobs. While this is cheaper, there's a new problem to address with it. The whole point of blobs is that they're cheaper because they are only kept around for approximately two weeks and ONLY in the beacon chain, i.e. the consensus side. The execution side (and thus the EVM when running contracts) does not have access to the contents of a blob. Instead, the only thing it has access to is a **KZG commitment** of it.

This is important. If you recall, the way the L1 ensured that the state diff published by the sequencer was correct was by hashing its contents and ensuring that the hash matched the given state diff hash. With the contents of the state diff now no longer accessible by the contract, we can't do that anymore, so we need another way to ensure the correct contents of the state diff (i.e. the blob).

The solution is to make the prover take the KZG commitment as a public input and the KZG proof as a private input, compute the state diffs after correctly executing a batch of blocks, and verify the proof to check that the commitment binds to the correct state diffs.

Because the KZG commitment is a public input, we can use the `BLOBHASH` EVM opcode to retrieve the blob rolling hash (which is just the hash of the KZG commitment and some other constant data) and compare it to the public input KZG commitment (which needs to be hashed too).
