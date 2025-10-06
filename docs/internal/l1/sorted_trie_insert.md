## Sorted Trie Computing

Problem: we have to compute all of the nodes in the merkle patricia trie of ethereum based on the key value pair of all the elements.

### Computing

Naive algorithm for computing: we just insert unordered. This version requires O(n\*log(n)) reads-write to disk. This is because each insertion needs to modify every node in the path to the inserted leaf. 

Example of the Naive implementation:
![Image showing the insertion of 3 elements, which each one requiring multiple new reads and writes](sorted_trie_insert/Naive%20Insertion%20Example%201.png)
