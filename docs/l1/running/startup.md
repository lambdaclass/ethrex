# Node startup

## Supported networks

Ethrex is designed to support Ethereum mainnet and its testnets

| Network | Chain id | Supported sync modes |
| ------- | -------- | -------------------- |
| mainnet | 1        | snap                 |
| sepolia | 11155111 | snap                 |
| hoodi   | 560048   | full, snap           |

These are the only names `--network` accepts. Any other value is treated as a path to a genesis file, so a network without a preset must be started with `--network <genesis.json> --bootnodes <enode,...>`.

For more information about sync modes please read the [sync modes document](../fundamentals/sync_modes.md). Snap syncing is the default; to switch to full sync use the flag `--syncmode full`. Full sync is not possible on a fresh mainnet or sepolia database, since ethrex only executes post-merge blocks.

## Run an Ethereum node

This guide will assume that you already [installed ethrex](../../getting-started/installation/) and you know how to set up a [consensus client](./consensus_client.md) to communicate with ethrex.

To sync with mainnet

```
ethrex
```

To sync with sepolia

```
ethrex --network sepolia
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

---

For more details and configuration options:
- [Configuration](./configuration.md)
- [CLI reference](../../CLI.md)
