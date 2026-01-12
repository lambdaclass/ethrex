# ethrex-rpc

JSON-RPC API implementation for the ethrex Ethereum client.

For detailed API documentation, see the rustdocs:
```bash
cargo doc --package ethrex-rpc --open
```

## Supported Namespaces

- `eth_*`: Standard Ethereum methods
- `engine_*`: Consensus client communication
- `debug_*`: Debugging and tracing
- `net_*`: Network information
- `admin_*`: Node administration
- `web3_*`: Web3 utilities
- `txpool_*`: Transaction pool inspection
