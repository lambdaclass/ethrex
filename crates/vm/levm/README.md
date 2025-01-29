# LEVM (Lambda EVM)

Implementation of a simple Ethereum Virtual Machine in Rust.

## Supported Forks

| Fork           | Status |
| -------------- | ------ |
| Prague         | ✅️    |
| Cancun         | ✅     |
| Shanghai       | ✅     |
| Paris (Merge)  | ✅     |
| London         | ✅     |
| Berlin         | ✅     |
| Istanbul       | ✅     |
| Constantinople | ✅     |
| Byzantium      | ✅     |
| Homestead      | ✅     |
| Frontier       | ✅     |

## Roadmap

| Nº  | Milestone                       | Status |
| --- | ------------------------------- | ------ |
| 1   | Support Merge->Cancun forks     | ✅     |
| 2   | Integrate `ethrex L1` <> `levm` | 🏗️     |
| 3   | Support pre Merge forks         | ✅     |
| 4   | Support Pectra upgrade          | 🏗️     |
| 5   | Integrate `ethrex L2` <> `levm` | ❌     |
| 6   | Performance                     | 🏗️     |

### Milestone 1: Support Merge->Cancun forks

This is having the minimum implementation so all the Ethereum Foundation tests from the fork Merge (Paris) to Cancun pass.

| Task Description            | Status |
| --------------------------- | ------ |
| Make Cancun EF tests pass   | ✅     |
| Make Shanghai EF tests pass | ✅     |
| Make Merge EF tests pass    | ✅     |

### Milestone 2: Integrate `ethrex L1` <> `levm`

Once we support all the forks from Merge to Cancun, we will integrate `ethrex L1` with `levm`.

Nowadays `ethrex L1` uses `revm` as the backend VM. We will replace `revm` with `levm` and make sure all the tests pass.

| Task Description                                                                                                                                                     | Status |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| All the Hive tests that pass with `revm` also pass with `levm`                                                                                                       | 🏗️     |
| We have an insurance that if `levm` some time diverges from `revm`'s behavior, we will know it. And in such case, switching to `revm` while we fix the issue is easy | ✅     |
| The feature flag `levm` is used as the default backend VM for `ethrex L1`                                                                                            | 🏗️     |
| We switch which EVM `ethrex` uses using a `--vm` CLI flag                                                                                                            | 🏗️     |
| We have a `EVM` trait or similar to standardize the VM interface. This trait should be implemented by `levm` and `revm`                                              | ❌     |

### Milestone 3: Support pre Merge forks

This is extending our current implementation so we support all the forks from Frontier to Merge.

| Task Description                  | Status |
| --------------------------------- | ------ |
| Make London EF tests pass         | ✅     |
| Make Berlin EF tests pass         | ✅     |
| Make Istanbul EF tests pass       | ✅     |
| Make Constantinople EF tests pass | ✅     |
| Make Byzantium EF tests pass      | ✅     |
| Make Homestead EF tests pass      | ✅     |
| Make Frontier EF tests pass       | ✅     |

### Milestone 4: Support Pectra upgrade

> [!NOTE]
> This milestone can be started after we finish the milestone 1, and can be done in parallel with milestones 2, 3, and 5 (speaking in terms of the current priorities).

This is extending our current implementation so we support the [Pectra upgrade](https://eips.ethereum.org/EIPS/eip-7600).

There are a lot of EIPs schedule to include in this upgrade but for `levm` we'll only focus on:

- EIP-2537: Precompile for BLS12-381 curve operations
- EIP-7623: Increase calldata cost
- EIP-7691: Blob throughput increase
- EIP-7702: Set EOA account code

| Task Description          | Status |
| ------------------------- | ------ |
| Implement EIP-2537        | ✅     |
| Implement EIP-7623        | ✅     |
| Implement EIP-7691        | ✅️    |
| Implement EIP-7702        | ✅️    |
| Make Prague EF tests pass | ✅     |

### Milestone 5: Integrate `ethrex L2` <> `levm`

> [!NOTE]
> This milestone can be started after we finish the milestone 2, and can be done in parallel with the milestones 3, and 4. It is placed at this point in the roadmap because of the current priorities.

Once we support all the forks from Merge to Cancun and we have fully integrated `ethrex L1` with `levm`, we can start integrating with `ethrex L2`.

For this milestone we'll have to refactor the code to support custom builds of the VM. A user should be able to build an instance of the VM modifying or adding new behavior to the VM. We'll call this "hooks" and `ethrex L2` will plug custom hooks to the VM so it can support the current `ethrex L2`'s Privilege transactions and other features.

We'll also have to refactor (and probably re-write) some precompiles implementation to enable the RISC-V zkVM backend to successfully prove `levm`'s execution. We need to ensure that the crates used in LEVM are compatible with these. For instance, most of the precompiles will need to be re-written using patched libraries instead of the ones they currently use.

| Task Description                                                          | Status |
| ------------------------------------------------------------------------- | ------ |
| This does not add breaking changes to the current implementation          | ❌     |
| The feature flag `levm` is used as the default backend VM for `ethrex L2` | ❌     |
| The L2 integration test pass                                              | ❌     |
| The prover tests pass                                                     | ❌     |

### Milestone 6: Performance

> [!NOTE]
> This milestone can be started after we finish the milestone 1, and can be done in parallel with the milestones 2, 3, 4, and 5. It is placed at this point in the roadmap because of the current priorities.

This is improving the performance of the VM.

We'll run flamegraph or Samply over the VM to identify bottlenecks and improve the performance of the VM. We'll also extend the current benchmarks suite to include more complex contracts and compare the performance not only of `levm` with `revm` but also with other known EVM implementations (like `evmone`).

| Task Description                                                                                                                                      | Status |
| ----------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| We have a GitHub workflow that posts the benchmarks results comparison between the PR and the main branch in every PR that includes changes in `levm` | ✅     |
| We have a GitHub workflow that generates a flamegraph over `levm` and `revm` and post the results in GitHub Pages                                     | ✅     |
| Add more benchmarks to the current suite                                                                                                              | ✅     |
| Benchmark a mainnet's block execution                                                                                                                 | 🏗️     |
| We add a table in the README comparing the benchmark results between multiple EVM implementations similar to the one in GitHub Pages                  | 🏗️     |
| All the identified bottlenecks are fixed                                                                                                              | ❌     |

## Ethereum Foundation Tests (EF Tests)

### Status

> [!NOTE]
> This is updated as of this README's last update. For the most up-to-date status, please run the tests locally.

**Total**: 6265/6475 (96.76%)

**Prague:** 2163/2373 (91.15%)
**Cancun:** 3579/3579 (100.00%)
**Shanghai:** 221/221 (100.00%)
**Homestead:** 17/17 (100.00%)
**Istanbul:** 34/34 (100.00%)
**London:** 39/39 (100.00%)
**Byzantium:** 33/33 (100.00%)
**Berlin:** 35/35 (100.00%)
**Constantinople:** 66/66 (100.00%)
**Merge:** 62/62 (100.00%)
**Frontier:** 16/16 (100.00%)

### How to run EF tests locally

```
make download-evm-ef-tests run-evm-ef-tests QUIET=true
```

## Benchmarks

### Status

> [!NOTE]
> This is updated as of this README's last update. For the most up-to-date status, please run the benchmarks locally.

| Benchmark | `levm`             | `revm`            | Difference                                     |
| --------- | ------------------ | ----------------- | ---------------------------------------------- |
| Factorial | 26.534 s ± 0.219 s | 7.116 s ± 0.109 s | `revm` is 3.73 ± 0.07 times faster than `levm` |
| Fibonacci | 24.272 s ± 0.872 s | 7.107 s ± 0.033 s | `revm` is 3.42 ± 0.12 times faster than `levm` |

### How to run benchmarks locally

> [!IMPORTANT]
> You need to have `hyperfine` installed to run the benchmarks.

```
make revm-comparison
```

## LEVM as the backend VM for `ethrex`

This section covers how to run `ethrex`'s Hive tests using LEVM as the backend VM

### Status

> [!NOTE]
> This is updated as of this README's last update. For the most up-to-date status, please run the tests locally.

**Engine**

- **Cancun**: 192/227 (84.58%)
- **Paris**: 103/129 (79.84%)
- **Auth**: 8/8 (100.00%)
- **Exchange Capabilities**: 5/5 (100.00%)
- **Shanghai**: 13/36 (36.11%)

**P2P**

- **Discovery V4**: 15/15 (100.00%)
- **Eth capability**: 15/15 (100.00%)
- **Snap capability**: 6/6 (100.00%)

**RPC**

- **RPC API Compatibility**: 89/90 (98.89%)

**Sync**

- **Node Syncing**: 2/2 (100.00%)

**Total**: 448/533 (84.05%)

### How to run

> [!IMPORTANT]
> You need to have `go` installed to run the Hive tests.

```
make run-hive-debug-levm
```

## Useful Links

[Ethereum Yellowpaper](https://ethereum.github.io/yellowpaper/paper.pdf) - Formal definition of Ethereum protocol.  
[The EVM Handbook](https://noxx3xxon.notion.site/The-EVM-Handbook-bb38e175cc404111a391907c4975426d) - General EVM Resources  
[EVM Codes](https://www.evm.codes/) - Reference for opcode implementation  
[EVM Playground](https://www.evm.codes/playground) - Useful for seeing opcodes in action  
[EVM Deep Dives](https://noxx.substack.com/p/evm-deep-dives-the-path-to-shadowy) - Deep Dive into different aspects of the EVM

## Performance metrics

### To run Flamegraph on the Ethereum Foundation tests

First install Flamegraph

```Shell
cargo install flamegraph
```

Run the tests

```Shell
make flamegraph-run-ef-tests
```

This will create a folder inside `cmd/ef_tests/levm/` named `levm_ef_test_perfgraphs` you can find the flamegraphs inside the folder `levm_ef_test_perfgraphs/flamegraph` open them with your preferred browser.

### To run Samply on the Ethereum Foundation tests

First install Samply

```Shell
cargo install --locked samply
```

Run the tests

```Shell
make samply-run-ef-tests
```

This will create a folder inside `cmd/ef_tests/levm/` named `levm_ef_test_perfgraphs` you can find the flamegraphs inside the folder `levm_ef_test_perfgraphs/samply` run

```Shell
samply load <path-to-perf-file.json>
```

samply will open Firefox with the desired profile file.
