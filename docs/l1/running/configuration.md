# Configuration

This page covers the basic configuration options for running an L1 node with ethrex. Full list of options can be found at the [CLI reference](../../CLI.md)

## Sync Modes

Ethrex supports different sync modes for node operation:

- **full**: Downloads and verifies the entire chain.
- **snap**: Fast sync using state snapshots (recommended for most users).

Set the sync mode with:

```sh
ethrex --sync <mode>
```

## File Locations

By default, ethrex stores its data in `~/.ethrex/<network>`, where `<network>` is the network name (e.g., `mainnet`, `hoodi`, `sepolia`, `holesky`).

Examples:
- Mainnet: `~/.ethrex/mainnet`
- Hoodi testnet: `~/.ethrex/hoodi`
- Sepolia testnet: `~/.ethrex/sepolia`
- Local devnet: `~/.ethrex/local-devnet`
- Custom genesis: `~/.ethrex/custom`

You can change the base data directory with:

```sh
ethrex --datadir <path>
```

The network subdirectory will still be appended to the specified path.

## Ports

Default ports used by ethrex:

- `8545`: JSON-RPC (HTTP)
- `8551`: Auth JSON-RPC
- `30303`: P2P networking (TCP/UDP)
- `9090`: Metrics

You can change ports with the corresponding flags: `--http.port`, `--authrpc.port`, `--p2p.port`, `--discovery.port`, `--metrics.port`.

All services listen on `0.0.0.0` by default, except for the auth RPC, which listens on `127.0.0.1`. This can also be changed with flags (e.g., `--http.addr`).

## Log Levels

Control log verbosity with:

```sh
ethrex --log.level <level>
```

Levels: `error`, `warn`, `info` (default), `debug`, `trace`

## Dev Mode (Localnet)

For local development and testing, you can use dev mode:

```sh
ethrex --dev
```

This runs a local network with block production and no external peers. This network has a list of [predefined accounts](https://github.com/lambdaclass/ethrex/blob/main/fixtures/keys/private_keys_l1.txt) with funds for testing purposes.
