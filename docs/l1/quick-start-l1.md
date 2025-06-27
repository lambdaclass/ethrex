# Quick Start (L1 localnet)

This page will show you how to quickly spin up a local development network with ethrex.

## Prerequisites

- [Kurtosis](https://docs.kurtosis.com/install/#ii-install-the-cli)
- [Rust](https://www.rust-lang.org/tools/install)
- [Docker](https://docs.docker.com/engine/install/)

## Starting a local devnet

```shell
make localnet
```

This make target will:

1. Build our node inside a docker image.
2. Fetch our fork [ethereum package](https://github.com/ethpandaops/ethereum-package), a private testnet on which multiple ethereum clients can interact.
3. Start the localnet with kurtosis.

If everything went well, you should be faced with our client's logs (ctrl-c to leave).

## Stopping a local devnet

To stop everything, simply run:

```shell
make stop-localnet
```
