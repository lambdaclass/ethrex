# Prover's block execution program

The zkVM block execution program will:
1. Take as input:
    - the block to verify and its parent's header
    - the L2 initial state, stored in a `ExecutionDB` struct, including the nodes for state and storage [pruned tries](#pruned-tries)
1. Build the initial state tries. This includes:
    - verifying that the initial state values stored in the `ExecutionDB` are included in the tries.
    - checking that the state trie root hash is the same as the one in the parent's header
    - building the trie structures
1. Execute the block
1. Perform validations before and after execution
1. Apply account updates to the tries and compute the new state root
1. Check that the final state root is the same as the one stored in the block's header
1. Commit the program's output

## Public and private inputs
The program interface defines a `ProgramInput` and `ProgramOutput` structures. 

`ProgramInput` contains:
- the block to verify and its parent's header
- an `ExecutionDB` which only holds the relevant initial state data for executing the block. This is built from pre-executing the block outside the zkVM to get the resulting account updates and retrieving the accounts and storage values touched by the execution.
- the `ExecutionDB` will also include all the (encoded) nodes necessary to build [pruned tries](#pruned-tries) for the stored accounts and storage values.

`ProgramOutput` contains:
- the initial state hash
- the final state hash
these outputs will be committed as part of the proof. Both hashes are verified by the program, with the initial hash being checked at the time of building the initial tries (equivalent to verifying inclusion proofs) and the final hash by applying the account updates (that resulted from the block's execution) in the tries and recomputing the state root.

## Pruned Tries
The EVM state is stored in Merkle Patricia Tries, which work differently than standard Merkle binary trees. In particular we have a *state trie* for each block, which contains all account states, and then for each account we have a *storage trie* that contains every storage value if the account in question corresponds to a deployed smart contract.

We need a way to check the integrity of the account and storage values we pass as input to the block execution program. The "Merkle" in Merkle Patricia Tries means that we can cryptographically check inclusion of any value in a trie, and then use the trie's root to check the integrity of the whole data at once.

Particularly, the root node points to its child nodes by storing their hashes, and these also contain the hashes of *their* child nodes, and so and so, until arriving at nodes that contain the values themselves. This means that the root contains the information of the whole trie (which can be compressed in a single word (32 byte value) by hashing the root), and by traversing down the trie we are checking nodes with more specific information until arriving to some value.

So if we store only the necessary nodes that make up a path from the root into a particular value of interest (including the latter and the former), then:
- we know the root hash of this trie
- we know that this trie includes the value we're interested in
thereby **we're storing a proof of inclusion of the value in a trie with some root hash we can check*, which is equivalent to having a "pruned trie" that only contains the path of interest, but contains information of all other non included nodes and paths (subtries) thanks to nodes storing their childs hashes as mentioned earlier. This way we can verify the inclusion of values in some state, and thus the validity of the initial state values in the `ExecutionDB`, because we know the correct root hash.

We can mutate this pruned trie by modifying/removing some value or inserting a new one, and then recalculate all the hashes from the node we inserted/modified up to the root, finally computing the new root hash. Because we know the correct final state root hash, this way we can make sure that the execution lead to the correct final state values.

