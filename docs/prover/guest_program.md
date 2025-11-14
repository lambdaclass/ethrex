# ethrex-prover guest program

The guest program is the code that is compiled into a zkVM-compatible binary (e.g., RISC-V), to then generate a zero-knowledge proof of its execution.

## Program inputs

The inputs for the blocks execution program (also called prover inputs) are:

- `blocks`: The blocks to be proven (header and body).
- `execution_witness`: A structure containing the necessary state data (like account and storage values with their Merkle proofs) required for the execution of the blocks. It includes the parent header of the first block.
- `elasticity_multiplier`: A parameter for block validation.
- `fee_configs`: L2-specific fee configurations for each block.
- `blob_commitment` and `blob_proof`: L2-specific data for verifying the state diff blob.

These inputs are required for proof generation. The public values of the proof (also called program outputs), which are needed for proof verification, are:

- `initial_state_hash`: The state root from the parent header of the first block.
- `final_state_hash`: The state root from the header of the last block.
- `l1messages_merkle_root`: The Merkle root of L1 messages (withdrawals) generated during block execution.
- `privileged_transactions_hash`: A hash representing all privileged transactions processed in the blocks.
- `blob_versioned_hash`: The versioned hash of the state diff blob, derived from its KZG commitment.
- `last_block_hash`: The hash of the last block in the batch.
- `chain_id`: The chain ID of the network.
- `non_privileged_count`: The number of non-privileged transactions in the batch.

## Blocks execution program

The program leverages `ethrex-common` primitives and `ethrex-vm` methods. `ethrex-prover` implements a program that uses the existing execution logic and generates a proof of its execution using a zkVM. Some L2-specific logic and input validation are added on top of the basic blocks execution.

The following sections outline the steps taken by the execution program.

### Prelude 1: state trie basics

We recommend learning about Merkle Patricia Tries (MPTs) to better understand this section.

Each executed block transitions the Ethereum state from an initial state to a final state. State values are stored in MPTs:

1. Each account has a Storage Trie containing its storage values.
2. The World State Trie contains all account information, including each account's storage root hash (linking storage tries to the world trie).

Hashing the root node of the world state trie generates a unique identifier for a particular Ethereum state, known as the "state hash".

There are two kinds of MPT proofs:

1. Inclusion proofs: Prove that `key: value` is a valid entry in the MPT with root hash `h`.
2. Exclusion proofs: Prove that `key` does not exist in the MPT with root hash `h`.
   These proofs allow verifying that a value is included (or its key doesn't exist) in a specific state.

### Prelude 2: privileged transactions, L1 messages and state diffs

These three components are specific additions for ethrex's L2 protocol, layered on top of standard Ethereum execution logic. They each require specific validation steps within the program.

For more details, refer to [Overview](../l2/architecture/overview.md), [Withdrawals](../l2/fundamentals/withdrawals.md), and [State diffs](../l2/fundamentals/state_diffs.md).

### Step 1: initial state validation

The program validates the initial state by converting the `ExecutionWitness` into a `GuestProgramState` and verifying that its trie structure correctly represents the expected state. This involves checking that the calculated state trie root hash matches the initial state hash (obtained from the first block's parent block header).

The validation happens in several steps:

1. The `ExecutionWitness` (collected during pre-execution) is converted to `GuestProgramState`.
2. A `GuestProgramStateWrapper` is created to provide database functionality.
3. For each state value in the database (account state and storage slots), the program verifies merkle proofs of the inclusion (or exclusion, in the case of accounts that didn't exist before this batch) of the value in the state trie
4. The state trie root is compared against the first block's parent state root.

This validation ensures that all state values needed for execution are properly linked to the initial state via their MPT proofs. Having the initial state proofs (paths from the root to each relevant leaf) is equivalent to having a relevant subset of the world state trie and storage tries - a set of "pruned tries". This allows operating directly on these pruned tries (adding, removing, modifying values) during execution.

### Step 2: blocks execution

After validating the initial state, the program executes the blocks sequentially. This leverages the existing `ethrex-vm` execution logic. For each block, it performs validation checks and then executes the transactions within it. State changes from each block are applied before executing the next one.

### Step 3: final state validation

During execution, state values are updated (modified, created, or removed). After executing all blocks, the program calculates the final state by applying all state updates to the initial pruned tries.

Applying the updates results in a new world state root node for the pruned tries. Hashing this node yields the calculated final state hash. The program then verifies that this calculated hash matches the expected final state hash (from the last block header), thus validating the final state.

### Step 4: privileged transactions hash calculation

After execution and final state validation, the program calculates a hash encompassing all privileged transactions (like L1 to L2 deposits) processed within the blocks. This hash is committed as a public input, required for verification on the L1 bridge contract.

### Step 5: L1 messages Merkle root calculation

Similarly, the program constructs a binary Merkle tree of all L2->L1 messages (withdrawals) initiated in the blocks and calculates its root hash. This hash is also committed as a public input. Later, L1 accounts can claim their withdrawals by providing a Merkle proof of inclusion that validates against this root hash on the L1 bridge contract.

### Step 6: state diff calculation and commitment

Finally, the program calculates the state diffs (changes between initial and final state) intended for publication to L1 as blob data. It then verifies the provided `blob_commitment` and `blob_proof` against the calculated state diff. The resulting `blob_versioned_hash` (derived from the KZG commitment) is committed as a public input for verification on the L1 contract.
