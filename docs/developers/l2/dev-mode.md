# Ethrex as a local L2 development node

## Prerequisites

- This guide assumes you've read the dev [installation guide](../installing.md)
- An Ethereum utility tool like [rex](https://github.com/lambdaclass/rex)

## Dev mode

In dev mode ethrex acts as a local Ethereum development node and a local layer 2 rollup

```sh
ethrex l2 --dev
```

after running the command the ethrex monitor will open with information about the status of the local L2.

The default port of the L1 JSON-RPC is 8545 you can test it by running

```sh
rex block-number http://localhost:8545
```

The default port of the L2 JSON-RPC is 1729 you can test it by running

```sh
rex block-number http://localhost:1729
```

## Guides

For more information on how to perform certain operations, go to [Guides](../../l2/guides).
