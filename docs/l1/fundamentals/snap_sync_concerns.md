# Snap Sync Concerns

## Code Improvement opportunities

### Storage downloads

When downloading storages, there are scenarios when the storages are never finished downloading. The most likely explanation is that big accounts intervals aren't properly redownloaded. 

### Handling the pivot and reorgs

We are currently asking the pivot from our peers. We should have a system for handling the pivot from our consensus client. We should also be able to understand if the new pivot received is a reorg. In that case, we can't fullsync, but we can fast-sync between those pivots relatively easily.

### Potential Bytecode Nonresponse

We are currently asking for all the bytecodes that we have seen, never checking if those bytecodes are currently in the tree. This isn't a problem for most codes that are immutable, but EOA may change their code to be a delegated, and the old code may be deleted from other peers in that scenario. We should consider pruning the bytecode requests if one is downloaded during healing.

## Performance

For performance, having an efficient cache of accounts that need to have their storage downloaded in memory is key, and we should avoid going to the db as much as possible. In particular in storage healing we start by trying to get all of the storage healing roots, and this can be sped up considerably if we avoid going to the db.

### Improving debug

The functions `validate_state_root` and `validate_storage_roots` are very slow, as they rebuild the entire state trie in memory. This is 

## Code Quality

In general, snap sync lacks explanation comments that detail the functioning on the algorithm. Variables and structs should be renamed to make it properly readable.

### Storage downloads

Request storages is a very hard function to read, as the data structures were constantly modified to introduce speed optimizations. As such, this function is critical to restructure and manage it taking into account memory concerns.

This function also has a lot of numeric constants inserted in the code directly, and should be handled better by having defined consts with explanations.

### Healing

There are two healing challenges. We should have a single main algorithm for healing, not have the code duplicated across two files. On top of that, the Membatch structure should be replaced with "PendingNodes" as it's a far more descriptive name.

## Memory Concerns

### Storage accounts

Currently, we use a struct `accounts_by_root_hash` that we don't check the memory size. When rewriting this algorithm we should check if we're not going over the memory limit.
