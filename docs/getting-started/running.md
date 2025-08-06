# Running a node

This guide will help you start a local L1+L2 stack using ethrex.
It is assumed that you have already [installed ethrex](./installing.md).

Running `ethrex --help` will display the help message.

## Running an L1 node

You can run a mainnet node just by running:

```sh
ethrex
```

In case you want to sync with a testnet, you can use:

```sh
ethrex --network [sepolia|holesky|hoodi]
```

## Running an L1 dev environment

After [installing ethrex](./installing.md), you can start a local L1 by running:

```sh
ethrex --dev
```

> [!TIP]
> In case you want to start a new L1, you can remove the data of the old one by executing:
>
> ```sh
> ethrex removedb
> ```
