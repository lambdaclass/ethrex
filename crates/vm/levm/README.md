# LEVM (Lambda EVM)

Implementation of a simple Ethereum Virtual Machine in Rust.

## Supported Forks

| Fork           | Status |
| -------------- | ------ |
| Osaka          | ✅     |
| Prague         | ✅     |
| Cancun         | ✅     |
| Shanghai       | ✅     |
| Paris (Merge)  | ✅     |

## Docs

There is a large amount of docs in comments inside the code. For more information check out the [FAQ](../../../docs/vm/levm/faq.rs) and related documents.

## Testing

We run `EELS`, `ethereum/tests` and `legacyTests` both in their [state](../../../tooling/ef_tests/state/README.md) and [blockchain](../../../tooling/ef_tests/blockchain/README.md) form. More info on each README.

For running state tests from the current directory use:
```
make download-evm-ef-tests run-evm-ef-tests QUIET=true
```

## Useful Links

[Ethereum Yellowpaper](https://ethereum.github.io/yellowpaper/paper.pdf) - Formal definition of Ethereum protocol.
[The EVM Handbook](https://noxx3xxon.notion.site/The-EVM-Handbook-bb38e175cc404111a391907c4975426d) - General EVM Resources
[EVM Codes](https://www.evm.codes/) - Reference for opcode implementation
[EVM Playground](https://www.evm.codes/playground) - Useful for seeing opcodes in action
[EVM Deep Dives](https://noxx.substack.com/p/evm-deep-dives-the-path-to-shadowy) - Deep Dive into different aspects of the EVM
