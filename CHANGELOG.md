# Ethrex Changelog

## Perf

### 2025-04-11

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
