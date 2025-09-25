# Node startup

## Supported networks

Ethrex is designed to support Ethereum mainnet and its testnets

| Network | Chain id | Supported sync modes |
| ------- | -------- | -------------------- |
| mainnet | 1        | snap                 |
| sepolia | 11155111 | snap                 |
| holesky | 17000    | full, snap           |
| hoodi   | 560048   | full, snap           |

For more information about sync modes please read the [sync modes document](../fundamentals/sync_modes.md). Full syncing is the default, to switch to snap sync use the flag `--syncmode snap`

## Run an Ethereum node

This guide will assume that you already [installed ethrex](../../getting-started/installation/) and you know how to set up a [consensus client](./consensus_client.md) to communicate with ethrex.

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

Once started, you should be able to check the sync status with:

```sh
curl http://localhost:8545 \
    -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","method":"eth_syncing","params":[],"id":1}'
```

The answer should be:

```
{"id":1,"jsonrpc":"2.0","result":{"startingBlock":"0x0","currentBlock":"0x0","highestBlock":"0x0"}}
```

## Run an Ethereum node with Docker

You can simply start a node with a Consensus client and ethrex as Execution client with Docker using the [docker-compose.yaml](https://github.com/lambdaclass/ethrex/blob/main/docker-compose.yaml)

```sh
curl -L -o docker-compose.yaml https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/docker-compose.yaml
docker compose up
```

Or you can set a different network:

```sh
ETHREX_NETWORK=hoodi docker compose up
```
