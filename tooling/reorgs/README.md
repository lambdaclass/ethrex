# Reorg integration tests

This directory contains tests for chain reorganization.

## How to run

First, compile the `ethrex` binary if you haven't already:

```bash
cargo build --workspace --bin ethrex
```

Then, run the reorg tests using:

```bash
cargo run
```

You can run a custom binary by specifying the path:

```bash
cargo run -- /path/to/your/binary
```
