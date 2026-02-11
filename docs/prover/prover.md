# ethrex-prover

The prover leverages ethrex's stateless execution to generate zero-knowledge proofs of a block (or batch of blocks) execution. Stateless execution works by asking a synced node for an execution witness (the minimal state data needed to execute that block or batch) and using the L1 client code to re-execute it. See [Stateless execution](#stateless-execution-and-execution-witness) for more details.

The main interface to try the prover is [ethrex-replay](../ethrex_replay/ethrex_replay.md), we also use it as a component of ethrex's L2 stack, to deploy zk-rollups or zk-validiums (a rollup publishes state information to L1 to reconstruct the L2 state if sequencing were to fail, a validium does not). Because of this the prover also supports some [L2 specific checks](#l2-specific-checks).

## How do you prove block execution?

Now that general purpose zero-knowledge virtual machines (zkVMs) exist, most people have little trouble with the idea that you can prove execution. Just take the usual EVM code you wrote in Rust, compile to some zkVM target instead and you're mostly done. You can now prove it.

What's usually less clear is how you prove state. Let's say we want to prove a new L2 batch of blocks that were just built. Running the `ethrex` `execute_block` function on a Rust zkVM for all the blocks in the batch does the trick, but that only proves that you ran the VM correctly on **some** previous state/batch. How do you know it was the actual previous state of the L2 and not some other, modified one?

In other words, how do you ensure that:

- Every time the EVM **reads** from some account state or storage slot (think an account balance, some contract's bytecode), the value returned matches the actual value present on the previous state of the chain.
- When all **writes** are done to account states or storage slots after execution, the final state matches what the (last executed) block header specified is the state at that block (the header contains the final state MPT root).

### Stateless execution and execution witness

Ethrex implements a way to execute a block (or a batch of blocks) without having access to the entire blockchain state, but only the necessary subset for that particular execution. This subset is called the *execution witness*, and running a block this way is called *stateless execution* (stateless in the sense that you don't need a database with hundreds of gigabytes of the entire state data to execute).

The execution witness is composed of all MPT nodes which are relevant to the execution, so that for each read and write we have all the nodes that form a path from the root to the relevant leaf. This path is a proof that this particular value we read/wrote is part (or not) of the initial or final state MPT.

So, before initiating block execution, we can verify each proof for each state value read from. After execution, we can verify each proof for each state value written to. After these steps we authenticated all state data to two MPT root hashes (initial and final state roots), which later can be compared against reference values to check that the execution started from and arrived to the correct state. If you were to change a single bit, this comparison would fail.

### In a zkVM environment

After stateless execution was done, the initial and final state roots can be committed as public values of the zk proof. By verifying the proof we know that blocks were executed from an initial state and arrived into a final state, and we know the root hashes of the MPT of each one. If the initial root is what we expected (equal to the root of the latest validated state), then we trustlessly verified that the chain advanced its state correctly, and we can authenticate the new, valid state using the final state root.

By proving the execution of L2 blocks and verifying the zk proof (alongside with the initial state root) in an Ethereum smart contract validators attest the new state and the L2 inherits the security of Ethereum (assuming no bugs in the whole pipeline). This is the objective of an Ethereum L2.

Validators themselves could verify L1 block execution proofs to attest Ethereum instead of re-executing.

## L2 specific checks

Apart from stateless execution, the prover does some extra checks needed for L2 specific features.

### Data availability

Rollups publish state diffs as blob data to the L1 so that users can reconstruct the L2 state and rescue their funds if the sequencing were to fail or censors data. This published data needs to be part of the zk proof the prover generated. For this it calculates the valid state diffs and verifies a KZG proof, whose commitment can later be compared to the one published to the L1 using the `BLOBHASH` EVM opcode. See [data availability](../l2/fundamentals/data_availability.md) for more details.

### L1<->L2 messaging

This is a fundamental feature of an L2, used mainly for bridging assets between the L1 and the L2 or between L2s using the ethrex stack. Messages need to be part of the proof to make sure the sequencer included them correctly.

Messages are compressed into a hash or a Merkle tree root which then are stored in an L1 contract together with the rest of the L2 state data. The prover retrieves the transactions or events that the messages produced in the L2, reconstructs the message data and recomputes the hashes or Merkle tree roots, which are then committed as a public input of the zk proof. At verification we can compare these hashes with the ones stored in the L1. This is the same concept used for state data.

For more details checkout [deposits](../l2/fundamentals/deposits.md) and [withdrawals](../l2/fundamentals/withdrawals.md)

## See also

[Guest program](./guest_program.md) for the detailed steps of the program that the prover generates a proof of.
