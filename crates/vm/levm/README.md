# LEVM (Lambda EVM)

Implementation of a simple Ethereum Virtual Machine in Rust.

## Status
Meaning:
- ✅: Implemented
- 🏗️: Work in Progress
- ❌: Work not Started yet

Features:
- Opcodes ✅
- Precompiles 🏗️
- Transaction validation 🏗️
- Pass all EF tests 🏗️


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

### Testing

#### Unit Testing
To run the project's tests, do `make test`.

#### EF Tests
To run the EF Tests first download them from [here](https://github.com/ethereum/tests/tree/develop/GeneralStateTests). Then, inside the `tests` folder, create another folder named `ef_testcases` and include all the downloaded folders inside it.

Run `make help` to see available commands
