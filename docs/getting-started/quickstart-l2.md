# Quickstart: Run an L2 Node

**Ethrex** is a framework that lets you launch your own L2 rollup or blockchain. With ethrex, you can deploy, run, and experiment with custom L2 networks, taking advantage of Ethereum's security while enabling high throughput and low fees.

## Start a development L2

Follow these steps to quickly launch an L2 node using Docker. For advanced details, see the links at the end.

```sh
docker run -p 1729:1729 ghcr.io/lambdaclass/ethrex:main l2 --dev
```

This will start a local L1 and L2 network. A JSON-RPC server compatible with Ethereum clients will be available at `http://localhost:1729`.

---

For more details and configuration options, see:

- [Installation](./installation)
- [Advanced options and more details](../l2/introduction.md)
