# Running a node

## Supported networks

Ethrex is designed to support Ethereum mainnet and its testnets

|Network|Chain id|Supported sync modes|
|-------|--------|---------|
|mainnet|1|snap|
|sepolia|11155111|snap|
|holesky|17000|full, snap|
|hoodi|560048|full, snap|

For more information about sync modes please read the [sync modes document](./fundamentals/sync_modes.md). Full syncing is the default, to switch to snap sync use the flag `--syncmode snap`

## Syncing to an Ethreum network

This guide will assume that you already [installed ethrex](../getting-started/installation/installation.md) and you know how to set up a [consensus client](../getting-started/consensus_client.md) to communicate with ethrex.

To sync with mainnet

```
ethrex --syncmode snap
```

To sync with sepolia

```
ethrex --network sepolia --syncmode snap
```

To sync with holesky

```
ethrex --network holesky
```

To sync with hoodi

```
ethrex --network hoodi
```

## Running an Ethereum node with Docker

You can simply start a node with a Consensus client and ethrex as Execution client with Docker using the [docker-compose.yaml](https://github.com/lambdaclass/ethrex/blob/main/docker-compose.yaml)

```sh
curl -L -o docker-compose.yaml https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/docker-compose.yaml
docker compose up
```

Or you can set a different network:

```sh
ETHREX_NETWORK=hoodi docker compose up
```

> [!NOTE]
> This method is currently supported only on Linux.
