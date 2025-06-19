# Blockchain Tests
The blockchain tests test block validation and the consensus rules of the Ethereum blockchain. Tests are usually run for multiple forks.
Some [docs](https://ethereum.github.io/execution-spec-tests/main/consuming_tests/blockchain_test/).

## Running the tests

```bash
make test
```

## Running the tests for either levm or revm

```bash
make test-levm
```
or
```bash
make test-revm
```
