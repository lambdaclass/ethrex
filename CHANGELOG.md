# Ethrex Changelog

## Perf

### 2025-08-28

- Improve precompiles further: modexp, ecrecover [#4168](https://github.com/lambdaclass/ethrex/pull/4168)

### 2025-08-27

- Improve memory resize performance [#4117](https://github.com/lambdaclass/ethrex/pull/4177)

### 2025-08-25

- Improve calldatacopy opcode further [#4150](https://github.com/lambdaclass/ethrex/pull/4150)

### 2025-08-22

- Improve Memory::load_range by returning a Bytes directly, avoding a vec allocation [#4098](https://github.com/lambdaclass/ethrex/pull/4098)

- Improve ecpairing (bn128) precompile [#4130](https://github.com/lambdaclass/ethrex/pull/4130)

### 2025-08-20

- Improve BLS12 precompile [#4073](https://github.com/lambdaclass/ethrex/pull/4073)

- Improve blobbasefee opcode [#4092](https://github.com/lambdaclass/ethrex/pull/4092)

- Make precompiles use a constant table [#4097](https://github.com/lambdaclass/ethrex/pull/4097)

### 2025-08-19

- Improve addmod and mulmod opcode performance [#4072](https://github.com/lambdaclass/ethrex/pull/4072)

- Improve signextend opcode performance [#4071](https://github.com/lambdaclass/ethrex/pull/4071)

- Improve performance of calldataload, calldatacopy, extcodecopy, codecopy, returndatacopy [#4070](https://github.com/lambdaclass/ethrex/pull/4070)

### 2025-08-14

- Use malachite crate to handle big integers in modexp, improving perfomance [#4045](https://github.com/lambdaclass/ethrex/pull/4045)

### 2025-07-31

- Cache chain config and latest canonical block header [#3878](https://github.com/lambdaclass/ethrex/pull/3878)

- Batching of transaction hashes sent in a single NewPooledTransactionHashes message [#3912](https://github.com/lambdaclass/ethrex/pull/3912)

- Make `JUMPDEST` blacklist lazily generated on-demand [#3812](https://github.com/lambdaclass/ethrex/pull/3812)
- Rewrite Blake2 AVX2 implementation (avoid gather instructions and better loop handling).


### 2025-07-30

- Add a secondary index keyed by sender+nonce to the mempool to avoid linear lookups [#3865](https://github.com/lambdaclass/ethrex/pull/3865)

### 2025-07-24

- Refactor current callframe to avoid handling avoidable errors, improving performance [#3816](https://github.com/lambdaclass/ethrex/pull/3816)

- Add shortcut to avoid callframe creation on precompile invocations [#3802](https://github.com/lambdaclass/ethrex/pull/3802)

### 2025-07-21

- Use `rayon` to recover the sender address from transactions [#3709](https://github.com/lambdaclass/ethrex/pull/3709)

### 2025-07-18

- Migrate EcAdd and EcMul to Arkworks [#3719](https://github.com/lambdaclass/ethrex/pull/3719)

- Add specialized push1 and pop1 to stack [#3705](https://github.com/lambdaclass/ethrex/pull/3705)

- Improve precompiles by avoiding 0 value transfers [#3715](https://github.com/lambdaclass/ethrex/pull/3715)

- Improve BlobHash [#3704](https://github.com/lambdaclass/ethrex/pull/3704)

  Added push1 and pop1 to avoid using arrays for single variable operations.

  Avoid checking for blob hashes length twice.

### 2025-07-17

- Use a lookup table for opcode execution [#3669](https://github.com/lambdaclass/ethrex/pull/3669)

- Improve CodeCopy perfomance [#3675](https://github.com/lambdaclass/ethrex/pull/3675)

- Improve sstore perfomance further [#3657](https://github.com/lambdaclass/ethrex/pull/3657)

### 2025-07-16

- Improve levm memory model [#3564](https://github.com/lambdaclass/ethrex/pull/3564)

### 2025-07-15

- Add sstore bench [#3552](https://github.com/lambdaclass/ethrex/pull/3552)

### 2025-07-10

- Add AVX256 implementation of BLAKE2 [#3590](https://github.com/lambdaclass/ethrex/pull/3590)

### 2025-07-08

- Improve sstore opcodes [#3555](https://github.com/lambdaclass/ethrex/pull/3555)

### 2025-07-07

- Improve blake2f [#3503](https://github.com/lambdaclass/ethrex/pull/3503)

### 2025-06-30

- Use a stack pool [#3386](https://github.com/lambdaclass/ethrex/pull/3386)

### 2025-06-27

- Reduce handle_debug runtime cost [#3356](https://github.com/lambdaclass/ethrex/pull/3356)
- Improve U256 decoding and PUSHX [#3332](https://github.com/lambdaclass/ethrex/pull/3332)

### 2025-06-26

- Refactor jump opcodes to use a blacklist on invalid targets.

### 2025-06-20

- Use a lookup table for opcode parsing [#3253](https://github.com/lambdaclass/ethrex/pull/3253)
- Use specialized PUSH1 and PUSH2 implementations [#3262](https://github.com/lambdaclass/ethrex/pull/3262)

### 2025-05-27

- Improved the performance of shift instructions. [2933](https://github.com/lambdaclass/ethrex/pull/2933)

- Refactor Patricia Merkle Trie to avoid rehashing the entire path on every insert [2687](https://github.com/lambdaclass/ethrex/pull/2687)

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
  the report under `docs/perf_reports` for more information.

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
