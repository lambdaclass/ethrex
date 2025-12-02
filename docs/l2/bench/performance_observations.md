# Notes Taken While Exploring zkVM Optimization Topics. Dec 1, 2025

- **Transaction Sender Recovery:**  
  I noticed that in zkVMs we're using `k256`. It might be worth checking whether there's a performance difference when using `secp256k1` directly. This impacts blocks mostly composed of transfers, because every 21k gas equals one transaction.  
  However, this isn’t very relevant for typical mainnet blocks, where recovering the sender message accounts for roughly 2%, so transfer performance isn’t very meaningful. But if you fill 60M gas with transfers, performance would become extremely important.
  - Even with SP1—where `secp256k1` is somewhat broken for recovering the sender address—we could still use that crate. The issues SP1 has with `ecrecover` come from their decision not to recover high-s signatures.

- **ZisK Patches:**  
  We should check whether we have all the patches we need in ZisK; maybe more can be added. Some of them are listed [here](https://github.com/0xPolygonHermez?q=zisk-patch&type=all&language=&sort=).

- **MULMOD Optimization Experiment:**  
  I have a very POC-level PR ([link](https://github.com/lambdaclass/ethrex/pull/5466))—not meant for merging, it’s very messy—where I wanted to measure how much performance could be improved for `MULMOD` using a patched `crypto-bigint` in SP1.  
  I achieved a **57× speedup**.  
  The block I used for testing was **Block 23890684**. Before the patch, about 7% of total execution time was spent on MULMOD; after patching, the cost became almost negligible.  
  Some blocks spend nearly 10% of their execution time on MULMOD; others barely use it.
  - ZisK’s list of precompiles includes things related to modular multiplication. More information is in [this docs page](https://0xpolygonhermez.github.io/zisk/getting_started/precompiles.html).  
    We might also benefit from using other precompiles beyond those if we find advantages for other operations.

- **zkVM Recommended Settings PR:**  
  I have a PR ([link](https://github.com/lambdaclass/ethrex/pull/5458)) that compiles the zkVMs using recommended settings. I measured a good performance improvement on SP1, but didn’t get to measure before/after on ZisK.

- **VM Performance Tweaks:**  
  I also made a PR ([link](https://github.com/lambdaclass/ethrex/pull/5467)) with several VM changes that slightly improve performance.
  - Possibly related to the Substate change: we could experiment with replacing the `HashSet` with something else. I tested `BTreeSet`, and although there was some improvement on a custom block, it wasn’t as impactful as expected—maybe 0.5% at most. The actual numbers were almost identical, though sampling with SP1 showed slightly reduced time.  
    There's also a PR switching those fields to `FxHashSet`: [#5362](https://github.com/lambdaclass/ethrex/pull/5362).

- **GPU Proving Stability:**  
  I ran proving with GPU on both ZisK and SP1 many times using the same block to see whether timings are consistent enough to serve as a comparison metric (instead of looking only at cycles/steps).  
  - With **ZisK**, results were more consistent: out of 10 runs, 7 had identical proving time, and the remaining 3 differed by less than 2%.  
  - With **SP1**, 10 runs varied by up to 6%, so proving time is less stable.

- **SP1 Gas Metric vs Execution Cycles:**  
  SP1 exposes a gas metric that is useful when measuring performance improvements. However, in my experience, I’ve made changes that clearly impacted sampling time, but execution gas remained unchanged.  
  For very small or subtle optimizations, gas may not be a good indicator.  
  I think in SP1 it's useful to monitor both **gas** and **cycles**.  
  ZisK doesn’t appear to have a gas-like metric, so what we can observe are **cycles**, and also proving time across identical repeated executions (which helps a lot).

- **SP1 Sampling Results (Function Cost Breakdown):**  
  Running multiple samplings with SP1 across different block types, I noted which functions are most worth optimizing, measuring their percentage of total runtime:

  ### Empty Block
  - `execute`: **35%**
  - `new`: **18.6%**
  - `rkyv deser`: **12%**
  - `try_from Witness`: **11%**

  ### Block With 100 Transfers
  - `get_transactions_with_sender`: **27%** → Sender recovery is very significant if the block is full of transfers.
  - `new`: **25%** → `new` takes much longer than `execute` because transfers don’t execute much.
  - `execute`: **16%**
  - `try_from witness`: **2%** → With many transfers, accessed state is smaller; fewer trie nodes needed.
  - `rkyv deser`: **3.2%** → Same reasoning as above.
  - `receipts root`: **5.3%** → More relevant than usual.

  ### Typical Mainnet Block (30–45M gas)
  - `execute`: **55%** → The heaviest part of a real block; opcode execution in the EVM. Storage/account access tends to dominate.  
    In zkVMs, opcodes like `MULMOD` become highly relevant (~10% of total) unless patched—if patched, they become negligible.
  - `new`: **1.6%**
  - `try_from witness`: **16%**
  - `rkyv deser`: **16%**
  - `get_transactions_by_sender`: **2%** → Irrelevant in real blocks.
  - `receipts root`: **1.4%**
