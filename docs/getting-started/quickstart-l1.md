# Quickstart: Run an Ethereum L1 Node

Follow these steps to quickly launch an Ethereum L1 (mainnet) node using Docker. For advanced details, see the links at the end.

## Supported Networks

- **mainnet**
- **sepolia**
- **holesky**
- **hoodi**

By default, the command below run a node on mainnet. To use a different network, change the `ETHREX_NETWORK` environment variable with one of the networks above.

```sh
curl -LO https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/docker-compose.yaml
ETHREX_NETWORK=mainnet docker compose up
```

This will start an ethrex node along with a Lighthouse consensus client that syncs with the Ethereum network.

You can also start a Prometheus/Grafana stack for monitoring with:

```sh
ETHREX_NETWORK=mainnet docker compose --profile metrics up
```

---

For more details on installation, flags, and supported networks:

- [Installation](./installation)
- [Advanced options and networks](../l1/running)
