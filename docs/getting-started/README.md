# Getting started

Ethrex is a minimalist, stable, modular and fast implementation of the Ethereum protocol in [Rust](https://www.rust-lang.org/).
The client supports running in two different modes:

- As a regular Ethereum execution client
- As a multi-prover ZK-Rollup (supporting SP1, RISC Zero and TEEs), where block execution is proven and the proof sent to an L1 network for verification, thus inheriting the L1's security. Support for based sequencing is currently in the works.

We call the first one "ethrex L1" and the second one "ethrex L2".

## Quickstart L1

Follow these steps to sync an ethrex node on the Hoodi testnet.

### MacOS

Install ethrex and lighthouse:

```sh
# install lightouse and ethrex
brew install lambdaclass/tap/ethrex
brew install lighthouse

# create secrets directory and jwt secret
mkdir -p ethereum/secrets/
cd ethereum/
openssl rand -hex 32 | tr -d "\n" | tee ./secrets/jwt.hex
```

On one terminal:

```sh
ethrex --authrpc.jwtsecret ./secrets/jwt.hex --network hoodi
```

and on another one:

```sh
lighthouse bn --network hoodi --execution-endpoint http://localhost:8551 --execution-jwt ./secrets/jwt.hex --checkpoint-sync-url https://hoodi.checkpoint.sigp.io --http
```

### Linux x86

Install ethrex and lighthouse:

```sh
# create secrets directory and jwt secret
mkdir -p ethereum/secrets/
cd ethereum/
openssl rand -hex 32 | tr -d "\n" | tee ./secrets/jwt.hex

# install lightouse and ethrex
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_x86_64 -o ethrex
chmod +x ethrex
curl -LO https://github.com/sigp/lighthouse/releases/download/v7.1.0/lighthouse-v7.1.0-x86_64-unknown-linux-gnu.tar.gz
tar -xvf lighthouse-v7.1.0-x86_64-unknown-linux-gnu.tar.gz
```

On one terminal:

```sh
./ethrex --authrpc.jwtsecret ./secrets/jwt.hex --network hoodi
```

and on another one:

```sh
./lighthouse bn --network hoodi --execution-endpoint http://localhost:8551 --execution-jwt ./secrets/jwt.hex --checkpoint-sync-url https://hoodi.checkpoint.sigp.io --http
```

For other CPU architectures, see the [releases page](https://github.com/lambdaclass/ethrex/releases/).

## Quickstart L2

Follow these steps to quickly launch a local L2 node. For advanced options and real deployments, see the links at the end.

### MacOS

```sh
# install ethrex
brew install lambdaclass/tap/ethrex
ethrex l2 --dev
```

### Linux x86

```sh
# install ethrex
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-linux_x86_64 -o ethrex
chmod +x ethrex
./ethrex l2 --dev
```

For other CPU architectures, see the [releases page](https://github.com/lambdaclass/ethrex/releases/).

## Where to Start

- **Want to run ethrex in production as an execution client?**

  See [Node operation](../l1/running) for setup, configuration, monitoring, and best practices.

- **Interested in deploying your own L2?**

  See [L2 rollup deployment](../l2/deploy.md) for launching your own rollup, deploying contracts, and interacting with your L2.

- **Looking to contribute or develop?**

  Visit the [Developer resources](../developers) for local dev mode, testing, debugging, advanced CLI usage, and the [CLI reference](../CLI.md).

- **Want to understand how ethrex works?**

  Explore [L1 fundamentals](../l1/fundamentals) and [L2 Architecture](../l2/architecture) for deep dives into ethrex's design, sync modes, networking, and more.
