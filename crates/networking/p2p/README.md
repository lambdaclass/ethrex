# ethrex-p2p

Peer-to-peer networking layer for the ethrex Ethereum client.

For detailed API documentation, see the rustdocs:
```bash
cargo doc --package ethrex-p2p --open
```

## Protocols

- **DiscV4**: Node discovery (legacy)
- **DiscV5**: Node discovery
- **RLPx**: Encrypted transport
- **eth/68**: Block and transaction propagation
- **snap/1**: Snap sync for state synchronization

## Features

- `sync-test`: Testing utilities for sync
