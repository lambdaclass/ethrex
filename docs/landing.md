# ethrex

ethrex is a minimalist, stable, modular, fast, and ZK native Ethereum client built from the ground up with zero-knowledge proving in mind. Whether you're running an L1 node or building an L2, ethrex provides the foundation for verifiable Ethereum execution.

## Why ethrex?

| Feature | Description |
|---------|-------------|
| **Minimalist** | ~100k lines of Rust vs 500k+ in mature clients. Less code means fewer bugs and faster iteration. |
| **Multi-prover** | Support for SP1, RISC Zero, ZisK, OpenVM, and TEEs. Choose the proving backend that fits your needs. |
| **Unified L1/L2** | Same execution client for mainnet nodes and L2 rollups. Consistent behavior across layers. |
| **ZK-Optimized** | Data structures and algorithms designed to minimize proving overhead from day one. |

## zkVM Integrations

ethrex integrates with multiple zero-knowledge virtual machines, giving you flexibility in how you prove Ethereum execution.

| zkVM | Organization | L1 Support | L2 Support | Status |
|------|--------------|------------|------------|--------|
| **SP1** | Succinct | ✓ | ✓ | Production |
| **RISC Zero** | RISC Zero | ✓ | ✓ | Production |
| **ZisK** | Polygon | ✓ | Planned | Experimental |
| **OpenVM** | Axiom | ✓ | Planned | Experimental |
| **TEE (TDX)** | Intel | — | ✓ | Production |

> [!TIP]
> For L2 deployments, you can run multiple provers simultaneously for redundancy. See [multi-prover deployment](./l2/deployment/prover/multi-prover.md).

## Quick Start

**Run an L1 node:**
```bash
# Install ethrex
cargo install ethrex

# Start syncing mainnet
ethrex --network mainnet
```

**Deploy an L2:**
```bash
# See the full deployment guide
# https://docs.ethrex.xyz/l2/deployment/overview.html
```

## Architecture Highlights

ethrex's architecture is optimized for both traditional execution and ZK proving:

- **Stateless execution** - Block execution can run with only the necessary witness data, enabling efficient proving
- **Modular VM (LEVM)** - Our EVM implementation is designed for clarity and easy auditing
- **Optimized tries** - Merkle Patricia Trie operations are tuned to reduce zkVM cycle counts
- **Precompile patches** - Cryptographic operations use zkVM-accelerated implementations when available

## Learn More

- [zkVM Integrations](./zkvm-integrations.md) - Detailed guide to supported proving backends
- [Benchmark Comparisons](./l2/bench/zkvm_comparison.md) - Performance data vs other implementations
- [Case Studies](./case-studies.md) - How teams are using ethrex
- [Architecture Overview](./l1/architecture/overview.md) - Deep dive into ethrex internals

## Get Involved

- [GitHub](https://github.com/lambdaclass/ethrex) - Star us, open issues, contribute
- [Telegram](https://t.me/ethrex_client) - Join the community chat
- [Blog](https://blog.lambdaclass.com/) - Technical deep dives and updates
