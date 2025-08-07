# Running a node

## Supported networks

Ethrex is designed to support Ethereum mainnet and its testnets

|network|sync mode|
|-------|---------|
|mainnet|snap|
|sepolia|snap|
|holesky|full,snap|
|hoodi|full,snap|

For more information about sync modes please read the [sync modes document](./fundamentals/sync_modes.md). Full syncing is the default mode, to switch to snap sync use the flag `--syncmode snap`

## Syncing to an Ethreum network

This guide will assume that you already [installed ethrex](../getting-started/installation/README.md) and you know how to set up a [consensus client](../getting-started/consensus_client.md) to communicate with ethrex.

To sync with any ethereum network simply run

```
ethrex --authrpc.jwtsecret path/to/jwt.hex --network <mainnet,sepolia,holesky,hoodi> --syncmode <full,snap>
```

> [!NOTE]
> The flag `--network` can be omitted if you are trying to sync to mainnet

> [!NOTE]
> The flag `--syncmode` can be omitted if you want to use full sync
