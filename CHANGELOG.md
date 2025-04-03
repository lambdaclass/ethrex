# Ethrex Changelog

## Perf

#### 2025-04-03
- Fix a bug where RLP encoding was being done twice: [#2353](https://github.com/lambdaclass/ethrex/pull/2353), check
  the report under perf_report for more information.
  

#### 2025-03-30

- Faster block import, use a slice instead of copy
  [#2097](https://github.com/lambdaclass/ethrex/pull/2097)

#### 2025-02-28

* Don't recompute transaction senders when building blocks [#2097](https://github.com/lambdaclass/ethrex/pull/2097)

#### 2025-03-21

- Process blocks in batches when syncing and importing [#2174](https://github.com/lambdaclass/ethrex/pull/2174)

### 2025-03-27

- Compute tx senders in parallel [#2268](https://github.com/lambdaclass/ethrex/pull/2268)
