# ethrex-prover performance

## Optimizations

| Name | PR | tl;dr |
| --- | --- | --- |
| jumpdest opt. | https://github.com/lambdaclass/ethrex/pull/4608 | Cuts 15% total zkvm cycles |
| trie opt. 1 | https://github.com/lambdaclass/ethrex/pull/4648 | Halves trie hashing calls |
| trie opt. 2 | https://github.com/lambdaclass/ethrex/pull/4723 | Trie hashing zkVM cycles reduced 1/4, faster than risc0-trie |
| trie opt. 3 | https://github.com/lambdaclass/ethrex/pull/4763 | Trie `get` and `insert` ops. reduced cycles by 1/14. |
| trie opt. 4 | 🚧 WIP https://github.com/lambdaclass/ethrex/pull/4808 | Should allow us to reduce 10%-20% total proving cycles |
| ecpairing precompile  | 🚧 WIP https://github.com/lambdaclass/ethrex/pull/4809 | Reduced ecpairing cycles from 138k to 6k (10% total proving cycles gone) |
| ecmul precompile | unimplemented!(); | Should allow us to reduce 80k cycles to a few thousands. |

Trie operations are one of the most expensive in our prover right now. We are using [risc0-trie](https://github.com/risc0/risc0-ethereum/tree/main/crates/trie) as a fast zkVM trie reference to optimize our own. [ethrex-trie optimization for zkvm](#ethrex-trie-optimization-for-zkvm) see for a detailed exploration of our trie vs risc0’s.

## Proving times (SP1)

Benchmark server hardware:

- AMD EPYC 7713 64-Core Processor
- 128 GB RAM
- RTX 4090 24 GB

### Absolute times

| Block (Mainnet) | Gas | ethrex main (`70fc63`) | ethrex (jumpdest opt.) | ethrex (ecpairing precompile) | ethrex (trie opt1.) | ethrex (trie opt2.) | ethrex (trie opt3.) | ethrex (trie opt1. + trie opt2. + trie opt3.) | ethrex risc0_trie | rsp main (`9a7048`) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 23426993 | 35.7M | 20m 40s | 20m 04s | 20m 10s | 20m 12s | 19m 31s | 17m 20s | 16m 24s | 14m 32s | 08m 39s |
| 23426994 | 20.7M | 13m 24s | 12m 55s | 12m 32s | 13m 08s | 12m 48s | 11m 31s | 10m 53s | 10m 04s | 05m 39s |
| 23426995 | 16.6M | 10m 14s | 09m 54s | 09m 56s | 09m 56s | 09m 52s | 08m 39s | 08m 06s | 07m 19s | 04m 33s |
| 23426996 | 22.5M | 16m 58s | 16m 37s | 15m 48s | 16m 50s | 15m 42s | 14m 44s | 14m 08s | 12m 59s | 06m 39s |

### Relative to main

formula: `base time / optimization time`

| Block (Mainnet) | Gas | ethrex main (70fc63) | ethrex (jumpdest opt.) | ethrex (ecpairing precompile) | ethrex (trie opt1.) | ethrex (trie opt2.) | ethrex (trie opt3.) | ethrex (trie opt1. + trie opt2. + trie opt3.) | ethrex risc0_trie | rsp main (9a7048) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 23426993 | 35.7M | base | 0.971 | 0.976 | 0.977 | 0.944 | 0.839 | 0.794 | 0.703 | 0.419 |
| 23426994 | 20.7M | base | 0.964 | 0.935 | 0.980 | 0.955 | 0.859 | 0.812 | 0.751 | 0.422 |
| 23426995 | 16.6M | base | 0.967 | 0.971 | 0.971 | 0.964 | 0.845 | 0.792 | 0.715 | 0.445 |
| 23426996 | 22.5M | base | 0.979 | 0.931 | 0.992 | 0.925 | 0.868 | 0.833 | 0.765 | 0.392 |

# ethrex-trie optimization for zkvm

# Overview

Using the risc0-trie we get the next perf. gains in cycle count over our ethrex-trie:

- `rebuild_storage_trie()` 218.999 -> 64.435 (x3.4)
- `rebuild_state_trie()` 119.510 -> 37.426 (x3.2)
- `apply_account_updates()` 139.519 -> 16.702 (x8.4)
- `state_trie_root()` 63.482 -> 17.276 (x3.7)
- `validate_receipts_root()` 13.068 -> 2.013 (x6.5)

our goal is to have our trie perform as close as possible to risc0’s.

**we can’t directly integrate the risc0-trie because it brings `alloy` dependencies with it, though we could fork the trie and replace `alloy` with our own libs.**

# Flamegraph overview
ethrex-trie (filtering for `ethrex_trie` functions)
![alt text](img/ethrex_trie_flamegraph.png)


risc0-trie (filtering for `risc0_ethereum_trie` functions)
![alt text](img/risc0_trie_flamegraph.png)

# Optimizations to our trie

## 1. Caching the initial node hashes

https://github.com/lambdaclass/ethrex/pull/4648

We were calculating all the hashes of the initial state trie (and storage) nodes, but we weren’t caching them when building the tries, so there was double hashing. https://github.com/lambdaclass/ethrex/pull/4648 fixes it by introducing a function to manually set the cached hash of a node, and caching whenever building a trie using `Trie::from_nodes` .

Calls to `compute_hash` were reduced by half after those changes:

before
![alt text](img/before_initial_hash_caching.png)

after
![alt text](img/after_initial_hash_caching.png)

although the total proving time didn’t change that much:

![alt text](img/initial_hash_caching_prove_time.png)

(proved with SP1 in `l2-gpu-3`, RTX 4090)

## 2. `memcpy` calls on trie hashing

ethrex-trie:
![alt text](img/ethrex_trie_hashing.png)

risc0-trie:
![alt text](img/risc0_trie_hashing.png)

our `Node::compute_hash` is almost on par with risc0’s `Node::memoize` (hashes and caches the hash) but we have a big `memcpy` call that’s almost twice as long in cycles as the actual hashing.

initially I thought this was an overhead of `OnceLock::initialize` because risc0 uses an `Option` to cache the hash, but apparently that’s not the case as I ran a small experiment in which I compared initialization cycles for both and got the next results:

```bash
Report cycle tracker: {"oncelock": 3830000, "option": 3670000}
```

<details>
<summary>Experiment code</summary>

    lib

    ```rust
    use std::sync::OnceLock;

    pub struct OnceLockCache {
        pub inner: OnceLock<u128>,
    }

    impl OnceLockCache {
        pub fn init(&self, inner: u128) {
            println!("cycle-tracker-report-start: oncelock");
            self.inner.get_or_init(|| compute_inner(inner));
            println!("cycle-tracker-report-end: oncelock");
        }
    }

    pub struct OptionCache {
        pub inner: Option<u128>,
    }

    impl OptionCache {
        pub fn init(&mut self, inner: u128) {
            println!("cycle-tracker-report-start: option");
            self.inner = Some(compute_inner(inner));
            println!("cycle-tracker-report-end: option");
        }
    }

    pub fn compute_inner(inner: u128) -> u128 {
        inner.pow(2)
    }
    ```

    guest program

    ```rust
    #![no_main]
    sp1_zkvm::entrypoint!(main);

    use std::sync::OnceLock;

    use fibonacci_lib::{OnceLockCache, OptionCache};

    pub fn main() {
        let mut oncelocks = Vec::with_capacity(10000);
        let mut options = Vec::with_capacity(10000);

        for i in (1 << 64)..((1 << 64) + 10000) {
            let oncelock_cache = OnceLockCache {
                inner: OnceLock::new(),
            };
            oncelock_cache.init(i);
            oncelocks.push(oncelock_cache);
        }

        for i in (1 << 64)..((1 << 64) + 10000) {
            let mut option_cache = OptionCache { inner: None };
            option_cache.init(i);
            options.push(option_cache);
        }
    }
    ```

</details>

in the flamegraph it’s not clear where that `memcpy` is originating (any ideas?). The calls that are on top of it are `encode_bytes()` (from our RLP lib) and some `sha3` calls.

> Node hashing is composed of two operations: first the node is encoded into RLP, then we keccak hash the result.
>

### RLP Encoding

Our RLP lib uses a temp buffer to write encoded payloads to, and in the end it `finalize`s the encoding by writing the payload prefix (which contains a length) to an output buffer, plus the actual payload. This means we are allocating two buffers, initializing them, and copying bytes to both of them for each node that’s being encoded and hashed.

First I tried preallocating the buffers (which are vecs) by estimating a node’s final RLP length.

This was done in the `l2/opt_rlp_buffer`, this had an influence the `memcpy` length by reducing it almost 15%:

then I tried to bypass our RLP encoder altogether by implementing a small and fast encoding for the `Branch` node only (which is the majoritarian node type), this reduced `memcpy`  further, 30% over `main`:

I didn’t test implementing fast RLP for the other node types as I was expecting a bigger reduction in `memcpy` from `Branch` alone.

### Keccak

A difference between our trie and risc0’s is that they use the `tiny-keccak` crate instead of `sha3` as we do. I tried replacing it in our trie but the `memcpy` got bigger.

Regarding memory, I also noticed that when hashing we are initializing a 32 byte array and writing the hash to it after. I wanted to test if this mattered so I experimented by creating an uninitialized buffer with `MaybeUninit` (unsafe rust) and writing the hash to it. This had no impact in `memcpy`'s length.

Both changes are in the `l2/opt_keccak` branch.

### Final

At the end the problem was a combination of slow RLP encoding and that we did a preorder traversal while hashing a trie recursively. This was fixed in https://github.com/lambdaclass/ethrex/pull/4723.

## 3. Node cloning in `NodeRef::get_node`

Whenever we `get`, `insert` or `remove` from a trie we are calling `NodeRef::get_node` which clones the `Node` from memory and returns it, which means that for each operation over a trie we are cloning all the relevant nodes for that op.

This is fixed in https://github.com/lambdaclass/ethrex/pull/4763see that PR desc. for details
