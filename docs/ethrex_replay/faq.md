## FAQ

### What's the difference between `eth_getProof` and `debug_executionWitness`?

`eth_getProof` gets the proof for a particular account and the chosen storage slots.
`debug_executionWitness` gets the whole execution witness necessary to execute a block in a stateless manner.

The former endpoint is implemented by all execution clients and you can even find it in RPC Providers like Alchemy, the latter is only implemented by some execution clients and you can't find it in RPC Providers.

When wanting to execute a historical block we tend to use the `eth_getProof` method with an RPC Provider because it will be the most reliable, other way is using it against a Hash-Based Archive Node but this would be too heavy to host ourselves (20TB at least). This method is slow because it performs many requests but it's very flexible.

If instead we want to execute a recent block we use it against synced ethrex or reth nodes that expose the `debug_executionWitness` endpoint, this way retrieval of data will be instant and it will be way faster than the other method, because it won't be doing thousands of RPC requests, just one.

### Why stateless execution of some blocks doesn't work with `eth_getProof`

With this method of execution we get the proof of all the accounts and storage slots accessed during execution, but the problem arises when we want to delete a node from the Merkle Patricia Trie (MPT) when applying the account updates of the block. This is for a particular case in which a tree restructuring happens and we have a missing node that wasn't accessed but we need to know in order to restructure the trie.

The problem can be explained with a simple example: a Branch node has 2 child nodes and only one was accessed and removed, this branch node should stop existing because they shouldn't have only **one** child. It will be either replaced by a leaf node or by an extension node, this depends on its child.

This problem is wonderfully explained in [zkpig docs](https://github.com/kkrt-labs/zk-pig/blob/main/docs/modified-mpt.md), they also have a very good intro to the MPT.
Here they mention two different solutions that we have to implement in order to fix this. The first one works when the missing node is a Leaf or Extension and the second one works then the missing node is a Branch.

In our code we only applied the first solution by injecting all possible nodes to the execution witness that we build when using `eth_getProof`, that's why the witness when using this method will be larger than the witness obtained with `debug_executionWitness`. 

We didn't apply the second change because it needs a change to the MPT that we don't want in our code. However we were able to solve it for execution without using a zkVM by injecting some "fake nodes" to the trie just before execution that have the expected hash but their RLP content doesn't match to it. This way we can "trick" the Trie into thinking that it has the branch nodes when in fact, it doesn't. 



