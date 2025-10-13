# Hive tests

End-to-End tests with hive.
Hive is a system which simply sends RPC commands to our node,
and expects a certain response. You can read more about it [here](https://github.com/ethereum/hive/blob/master/docs/overview.md).

#### Prereqs

We need to have go installed for the first time we run hive, an easy way to do this is adding the asdf go plugin:

```shell
asdf plugin add golang https://github.com/asdf-community/asdf-golang.git

# If you need to set GOROOT please follow: https://github.com/asdf-community/asdf-golang?tab=readme-ov-file#goroot
```

And uncommenting the golang line in the asdf `.tool-versions` file:

```text
rust 1.90.0
golang 1.23.2
```

#### Running Simulations

Hive tests are categorized by "simulations', and test instances can be filtered with a regex:

```bash
make run-hive-debug SIMULATION=<simulation> TEST_PATTERN=<test-regex>
```

This is an example of a Hive simulation called `ethereum/rpc-compat`, which will specificaly
run chain id and transaction by hash rpc tests:

```bash
make run-hive SIMULATION=ethereum/rpc-compat TEST_PATTERN="/eth_chainId|eth_getTransactionByHash"
```

If you want debug output from hive, use the run-hive-debug instead:

```bash
make run-hive-debug SIMULATION=ethereum/rpc-compat TEST_PATTERN="*"
```

This example runs **every** test under rpc, with debug output
