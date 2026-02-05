# Assertoor tests

We run some assertoor checks on our CI, to execute them locally you can run the following:

```bash
make localnet-assertoor-tx
# or
make localnet-assertoor-blob
```

Those are two different sets of assertoor checks the details are as follows:

_assertoor-tx_

- [eoa-transaction-test](https://raw.githubusercontent.com/ethpandaops/assertoor/refs/heads/master/playbooks/stable/eoa-transactions-test.yaml)

_assertoor-blob_

- [blob-transaction-test](https://raw.githubusercontent.com/ethpandaops/assertoor/refs/heads/master/playbooks/stable/blob-transactions-test.yaml)
- _Custom_ [el-stability-check](https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/.github/config/assertoor/el-stability-check.yaml)

For reference on each individual check see the [assertoor-wiki](https://github.com/ethpandaops/assertoor/wiki#supported-tasks-in-assertoor)

## Run

Example run:

```bash
cargo run --bin ethrex -- --network fixtures/genesis/kurtosis.json
```

The `network` argument is mandatory, as it defines the parameters of the chain.
For more information about the different cli arguments check out the next section.
