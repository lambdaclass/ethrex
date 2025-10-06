## Sorted Trie Computing

Problem: we have to compute all of the nodes in the merkle 
patricia trie of ethereum based on the key value pair of all the elements.

### Concept

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

### Implementation
