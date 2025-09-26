# Getting started

Ethrex is a minimalist, stable, modular and fast implementation of the Ethereum protocol in [Rust](https://www.rust-lang.org/).
The client supports running in two different modes:

- As a regular Ethereum execution client
- As a multi-prover ZK-Rollup (supporting SP1, RISC Zero and TEEs), where block execution is proven and the proof sent to an L1 network for verification, thus inheriting the L1's security. Support for based sequencing is currently in the works.

We call the first one "ethrex L1" and the second one "ethrex L2".

## Quickstart

### L1: Run an Ethereum Node

Follow these steps to quickly launch an Ethereum L1 (mainnet) node using Docker. For advanced details, see the links at the end.

#### Supported Networks

- **mainnet**
- **sepolia**
- **holesky**
- **hoodi**

By default, the command below runs a node on mainnet. To use a different network, change the `ETHREX_NETWORK` environment variable with one of the networks above.

```sh
curl -LO https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/docker-compose.yaml
ETHREX_NETWORK=mainnet docker compose up
```

This will start an ethrex node along with a Lighthouse consensus client that syncs with the Ethereum network.

You can also start a Prometheus/Grafana stack for monitoring with:

```sh
ETHREX_NETWORK=mainnet docker compose --profile metrics up
```

### L2: Run an L2 Node

Follow these steps to quickly launch an L2 node using Docker. For advanced details, see the links at the end.

```sh
docker run -p 1729:1729 ghcr.io/lambdaclass/ethrex:main l2 --dev
```

This will start a local L1 and L2 network.

## Where to Start

- **Just want to run an Ethereum node?**

  Start with the [Quickstart](#quickstart) or see [Node operation](../l1/running) for setup, configuration, monitoring, and best practices.

- **Interested in building your own L2?**

  Begin with the [L2 introduction](../l2/introduction.md), [L2 quickstart](../getting-started/quickstart-l2.md), and see [L2 rollup deployment](../l2/deploy.md) for launching your own rollup, deploying contracts, and interacting with your L2.

- **Looking to contribute or develop?**

  Visit the [Developer resources](../developers) for local dev mode, testing, debugging, advanced CLI usage, and the [CLI reference](../CLI.md).

- **Want to understand how ethrex works?**

  Explore [L1 fundamentals](../l1/fundamentals) and [L2 Architecture](../l2/architecture) for deep dives into ethrex's design, sync modes, networking, and more.
