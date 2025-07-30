# LEVM (Lambda EVM)

Implementation of a simple, yet fast, Ethereum Virtual Machine in Rust.

## Supported Forks

| Fork           | Status |
| -------------- | ------ |
| Prague         | ✅     |
| Cancun         | ✅     |
| Shanghai       | ✅     |
| Paris (Merge)  | ✅     |                                                                                                        | 🏗️     |

## Trying out LEVM

For running a custom transaction we have a custom runner, start [here](./runner/README.md).

## Ethereum Foundation Tests (EF Tests)

```
make download-evm-ef-tests run-state-tests
```

For more information on running EF state tests go [here](../../../cmd/ef_tests/state/README.md).

For running EF blockchain tests go [here](../../../cmd/ef_tests/blockchain/README.md).

## Running benchmarks locally

> [!IMPORTANT]
> You need to have `hyperfine` installed to run the benchmarks.

```
make revm-comparison
```

## Useful Links

[Ethereum Yellowpaper](https://ethereum.github.io/yellowpaper/paper.pdf) - Formal definition of Ethereum protocol.
[The EVM Handbook](https://noxx3xxon.notion.site/The-EVM-Handbook-bb38e175cc404111a391907c4975426d) - General EVM Resources
[EVM Codes](https://www.evm.codes/) - Reference for opcode implementation
[EVM Playground](https://www.evm.codes/playground) - Useful for seeing opcodes in action
[EVM Deep Dives](https://noxx.substack.com/p/evm-deep-dives-the-path-to-shadowy) - Deep Dive into different aspects of the EVM
