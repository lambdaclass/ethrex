# Ethrex Changelog

## Perf

### 2025-05-27

- Add block snapshots with diff layers. [2664](https://github.com/lambdaclass/ethrex/pull/2664)
- Make disk layer in snapshots use the database. [2848](https://github.com/lambdaclass/ethrex/pull/2848)

### 2025-05-22

- Add immutable cache to LEVM that stores in memory data read from the Database so that getting account doesn't need to consult the Database again. [2829](https://github.com/lambdaclass/ethrex/pull/2829)

### 2025-05-20

- Reduce account clone overhead when account data is retrieved [2684](https://github.com/lambdaclass/ethrex/pull/2684)

### 2025-04-30

- Reduce transaction clone and Vec grow overhead in mempool [2637](https://github.com/lambdaclass/ethrex/pull/2637)

### 2025-04-28

- Make TrieDb trait use NodeHash as key [2517](https://github.com/lambdaclass/ethrex/pull/2517)

### 2025-04-22

- Avoid calculating state transitions after every block in bulk mode [2519](https://github.com/lambdaclass/ethrex/pull/2519)

- Transform the inlined variant of NodeHash to a constant sized array [2516](https://github.com/lambdaclass/ethrex/pull/2516)

### 2025-04-11

- Removed some unnecessary clones and made some functions const: [2438](https://github.com/lambdaclass/ethrex/pull/2438)

- Asyncify some DB read APIs, as well as its users [#2430](https://github.com/lambdaclass/ethrex/pull/2430)

### 2025-04-09

- Fix an issue where the table was locked for up to 20 sec when performing a ping: [2368](https://github.com/lambdaclass/ethrex/pull/2368)

#### 2025-04-03

- Fix a bug where RLP encoding was being done twice: [#2353](https://github.com/lambdaclass/ethrex/pull/2353), check
  the report under perf_report for more information.

#### 2025-04-01

- Asyncify DB write APIs, as well as its users [#2336](https://github.com/lambdaclass/ethrex/pull/2336)

#### 2025-03-30

- Faster block import, use a slice instead of copy
  [#2097](https://github.com/lambdaclass/ethrex/pull/2097)

#### 2025-02-28

- Don't recompute transaction senders when building blocks [#2097](https://github.com/lambdaclass/ethrex/pull/2097)

#### 2025-03-21

- Process blocks in batches when syncing and importing [#2174](https://github.com/lambdaclass/ethrex/pull/2174)

### 2025-03-27

- Compute tx senders in parallel [#2268](https://github.com/lambdaclass/ethrex/pull/2268)
