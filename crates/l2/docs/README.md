# Ethereum Rust L2 Docs

For a high level overview of the L2:

- [General Overview](./overview.md)

For more detailed documentation on each part of the system:

- [Proposer](./proposer.md)
- [Prover](./prover.md)
- [Contracts](./contracts.md)

## Configuration

Configuration is done through env vars. A detailed list is available in each part documentation.

## Testing

Load tests are available via L2 CLI. The test take a list of private keys and send a bunch of transactions from each of them to some address. To run them, use the following command:

```bash
cargo run --bin ethereum_rust_l2 -- test load --path <path-to-pks>
```

The path should point to a plain text file containing a list of private keys, one per line. Those account must be funded on the L2 network. Use `--help` to see more available options.

In the `test_data/` directory, you can find a list of private keys that are funded by the genesis.
