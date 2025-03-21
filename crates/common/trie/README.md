## Ethrex-Trie
This is the implementation of the State Trie (a Merkle Patricia Trie) used
by Ethrex.

### Benchmarking
To measure the performance of our implementation, we have a simple benchmark
that compares against [citahub's cita_trie implementation](https://github.com/citahub/cita_trie/tree/master).

To run it, you'll need rust installed of course, and you 
can run a comparison with:
```bash
make bench-all
```
Or, if you want to specifically bench a particular implementation:
```bash
make ethrex-bench
```
Well benchmark our implementation, or to benchmaark
the cita_trie instead:
```bash
make cita-bench
```

Benches are in the `benches` folder.

### Useful Links
- [Ethereum.org -- Merkle Patricia Trie](https://ethereum.org/es/developers/docs/data-structures-and-encoding/patricia-merkle-trie/) 
- [Stack Exchange Discussion](https://ethereum.stackexchange.com/questions/130017/merkle-patricia-trie-in-ethereum)
