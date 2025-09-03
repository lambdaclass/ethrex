# Ethereum foundation tests

These are the official execution spec tests there two kinds `state tests` and `blockchain tests`, you can execute them with:

### State tests

The state tests are individual transactions not related one to each other that test particular behavior of the EVM. Tests are usually run for multiple forks and the result of execution may vary between forks.
See [docs](https://eest.ethereum.org/v4.1.0/consuming_tests/state_test/).

To run the test first:

```sh
cd tooling/ef_tests/state
```

then download the test vectors:

```sh
make download-evm-ef-tests
```

then run the tests:

```sh
make run-evm-ef-tests
```

### Blockchain tests


The blockchain tests test block validation and the consensus rules of the Ethereum blockchain. Tests are usually run for multiple forks.
See [docs](https://eest.ethereum.org/v4.1.0/consuming_tests/blockchain_test).

To run the tests first:

```sh
cd tooling/ef_tests/blockchain
```

then run the tests:

```sh
make test-levm
```
