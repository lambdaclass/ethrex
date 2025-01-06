# LEVM (Lambda EVM)

Implementation of a simple Ethereum Virtual Machine in Rust.

## Status

> [!NOTE]
>
> - ✅: Implemented
> - 🏗️: Work in Progress
> - ❌: Work not Started yet

Features:

- Opcodes ✅
- Precompiles ✅
- Transaction validation ✅
- Pass all EF tests 🏗️

## Ethereum Foundation Tests (EF Tests)

### Status

> [!NOTE]
> This is updated as of this README's last update. For the most up-to-date status, please run the tests locally.

**Total**: 3933/4095 (96.04%)

**Cancun**: 3572/3572 (100.00%)
**Shanghai**: 221/221 (100.00%)
**Merge**: 62/62 (100.00%)
**London**: 39/39 (100.00%)
**Berlin**: 35/35 (100.00%)
**Istanbul**: 1/34 (2.94%)
**Constantinople**: 2/66 (3.03%)
**Byzantium**: 1/33 (3.03%)
**Homestead**: 0/17 (0.00%)
**Frontier**: 0/16 (0.00%)

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
| Factorial | 29.828 s ± 1.217 s | 7.295 s ± 0.089 s | `revm` is 3.74 ± 0.11 times faster than `levm` |
| Fibonacci | 26.437 s ± 0.730 s | 7.068 s ± 0.039 s | `revm` is 4.09 ± 0.17 times faster than `levm` |

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
- **Shanghai**: 5/36 (13.89%)

**P2P**

- **Discovery V4**: 15/15 (100.00%)
- **Eth capability**: 13/15 (86.67%)
- **Snap capability**: 6/6 (100.00%)

**RPC**

- **RPC API Compatibility**: 89/90 (98.89%)

**Sync**

- **Node Syncing**: 2/2 (100.00%)
- **Total**: 438/533 (82.18%)

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

## Getting Started

### Dependencies

- Rust
- Git

### Documentation

[CallFrame](./docs/callframe.md)
[FAQ](./docs/faq.md)

### Testing

To run the project's tests, do `make download-evm-ef-tests run-evm-ef-tests`.

Run `make help` to see available commands

## Performance metrics

### To run Flamegraph on the Ethereum Foundation tests

First, install Flamegraph
```Shell
cargo install flamegraph
