# Ethrex as a local development node

## Prerequisites

This guide assumes you've read the dev [installation guide](../installing.md)

## Dev mode

In dev mode ethrex acts as a local Ethereum development node it can be run with the following command

```sh
ethrex --dev
```

Then you can use a tool like [rex](https://github.com/lambdaclass/rex) to make sure that the network is advancing

```sh
rex block-number
```

Rich account private keys are listed at the folder `fixtures/keys/private_keys_l1.txt` located at the root of the repo. You can then use these keys to deploy contracts and send transactions in the localnet.
