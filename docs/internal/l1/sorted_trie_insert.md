# Sorted Trie Insertion

This documents the algorithm found at [crates/common/trie/trie_sorted.rs](/crates/common/trie/trie_sorted.rs)
which is used to speed up the insertion time in snap sync.
During that step we are inserting all of the account state and storage
slots downloaded into the Ethereum world state Merkle Patricia Trie.
To know how that trie works, it's recommended to [read this primer first.](https://epf.wiki/#/wiki/EL/data-structures?id=world-state-trie)

## Concept

Naive algorithm for computing: we just insert unordered. 
This version requires O(n\*log(n)) reads-write to disk. 
This is because each insertion creates a new leaf, which modifies 
the hash of its parent branch recursively. We can avoid reads
to disk by having the trie in memory, but this can be unviable
for large amounts of input data.

Example of the Naive implementation:
![Image showing the insertion of 3 elements 0x0EBB, 0x12E6, 0x172E. Each one requiring multiple new reads and writes](sorted_trie_insert/Naive%20Insertion%20Example%201.png)

If the input data is sorted, the computing can be optimized to be O(n).
In the example, just by reading 0x0EBB and 0x172E, we know that there is 
a branch node as root (because they start with different nibbles), and
that the leaf will have a partial path of 0xEBB (because no node exists
between 0x0EBB and 0x172E if it's sorted). The root branch node we know exists and will be modified, so we don't write until we have read all
input.

## Implementation

The implementation is based on keeping three pointers to data. The current
element we're processing, the next input value and the parent of the current
element. All parents that can still be modified are stored in a "parent stack". 
Based on those we can have enough knowledge to know what is the 
next write operation.

### Scenarios

Depending on the state of the three current pointers, one of 3 scenarios
can happen:

Scenario 1: Current and next value are brothers with the current
parent being the parent of both values. This happens when
the parent and both values share the same amount of nibbles at the beginning of 
their path. In our example, all paths to the nodes starts with 0x1 and
then diverges.

In this scenario, we know the leaf we need to compute from the current value
so we write that, modify the parent to include a pointer to that leaf 
and continue with the algorithm.

![Image showing the insertion of 1 elements with a current parent branch 0x1, the current element 0x12E6 and next element 0x172E. 0x12E6 is inserted with a single write](sorted_trie_insert/Sorted%20Insertion%20Scenario%201.png)

Scenario 2: Current and next values are brothers of a new current parent.
This happens when the parent shares less nibbles from their paths than what the brothers share.
In our example, the current and next value share 0x17, while the parent only shares 0x1.

In this scenario, we know the leaf we need to compute from the current value
so we write that. Furthermore, we know that we need a new branch at 0x17,
so we create it and insert the leaf we just computed and insert into the branch.
The current parent is stored in the "parent stack", and the new branch becomes the 
current parent.

![Image showing the insertion of 1 elements with a current parent branch 0x1, the current element 0x172E and next element 0x175B. 0x172E is inserted with a single write, while the current parent branch is put onto the stack, while a new current parent branch 0x12 is created](sorted_trie_insert/Sorted%20Insertion%20Scenario%202.png)

Scenario 3: The current parent is not the parent of the
next value. This happens when the parent doesn't have
all of the nibbles of it's path.

In this scenario, we know the leaf we need to compute from the current value, 
so we write that. We change the current value to be the current parent, and 
the new current parent is popped from the "parent stack".

![Image showing the insertion of 1 elements with a current parent branch 0x17, the current element 0x175B and next element 0x1825. 0x175B is inserted with a single write, while the current parent branch becomes the current value, while the current parent branch is popped from the stack](sorted_trie_insert/Sorted%20Insertion%20Scenario%203.png)

These three scenarios keep repeating themselves until the trie is complete,
at which point the algorithm returns a hash to the root node branch.

### Inserting with extensions

In general, each write to disk is prepared to properly handle extensions
as the write function knows what it's writing and what was it's parent
and full path. As such, it can check if the insertion is a branch and
if there's an extension need.

A specific edge is the root node, which is assumed to always be a branch
node, but the code has special case code to check if the root node has 
a single child, in which case it changes to an extension or leaf as needed.
While modifying the other nodes in the trie.

### Concurrency

The slowest step in this excercise is the proper flushing to disk.
To avoid stalling while the write is done, we use an internal buffer
which holds a fixed amount of nodes. Once the buffer is filled, it
creates a new task that writes it to disk in the background.

We want to limit the amount of buffers we can have, so we allocate a 
fixed number of buffers at the beginning and we use a channel for the
algorithm to recieve empty buffers and the writing task clears the
buffer and sends it back through the channels.

For execution of these tasks, we use a custom threadpool defined in
[/crates/concurrency/concurrency.rs](/crates/concurrency/concurrency.rs)
