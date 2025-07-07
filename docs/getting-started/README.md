# Getting started

Ethrex is a minimalist, stable, modular and fast implementation of the Ethereum protocol in [Rust](https://www.rust-lang.org/).
The client supports running in two different modes:

- As a regular Ethereum execution client
- As a multi-prover ZK-Rollup (supporting SP1, RISC Zero and TEEs), where block execution is proven and the proof sent to an L1 network for verification, thus inheriting the L1's security. Support for based sequencing is currently in the works.

We call the first one "ethrex L1" and the second one "ethrex L2".
You can find more information on our [L1](../l1) or [L2](../l2) docs, respectively.

## Where to start

To get started with ethrex, you can follow the [installation guide](./installing.md) to set up the client on your machine.
