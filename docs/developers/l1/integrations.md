# Chain integrations

A chain integration adds support for a chain to ethrex from a crate outside this repository. The crate implements one trait, `Integration`, and builds its own binary. The `ethrex` binary is built the same way: it is the integration for Ethereum, which adds nothing.

An integration can add:

| What | How |
|---|---|
| Named networks for `--network` | `Integration::presets` |
| Its own command line flags | `Integration::Args` |
| RPC namespaces | `Components::rpc` |
| Transaction pool rules | `Components::tx_pool_policy` |

Everything an integration leaves out behaves as on Ethereum. An integration cannot change how blocks are validated or executed.

## Writing an integration

### 1. Create the crate

```toml
# Cargo.toml
[package]
name = "ethrex-example"
edition = "2024"

[dependencies]
ethrex = { git = "https://github.com/lambdaclass/ethrex" }
ethrex-blockchain = { git = "https://github.com/lambdaclass/ethrex" }
ethrex-common = { git = "https://github.com/lambdaclass/ethrex" }
ethrex-config = { git = "https://github.com/lambdaclass/ethrex" }
ethrex-rpc = { git = "https://github.com/lambdaclass/ethrex" }
clap = { version = "4.3", features = ["derive"] }
eyre = "0.6"
serde_json = "1"
```

### 2. Implement `Integration`

```rust
use std::sync::Arc;

use ethrex::integration::{ChainInfo, Components, Integration};
use ethrex_blockchain::TxPoolPolicy;
use ethrex_common::{Address, types::Transaction};
use ethrex_config::networks::NetworkPreset;
use ethrex_rpc::{RpcApiContext, RpcErr, RpcExtension, RpcExtensionFuture, utils::RpcRequest};

pub struct Example;

/// Flags of this integration, added to the node's own.
#[derive(clap::Args)]
pub struct ExampleArgs {
    /// Highest gas limit a transaction may have to enter the pool.
    #[arg(long = "example.max-tx-gas", default_value_t = 1_000_000)]
    max_tx_gas: u64,
}

impl Integration for Example {
    type Args = ExampleArgs;
    const NAME: &'static str = "ethrex-example";

    fn presets(&self) -> &'static [NetworkPreset] {
        &[NetworkPreset {
            name: "example-mainnet",
            genesis: include_str!("../networks/example-mainnet/genesis.json"),
            bootnodes: include_str!("../networks/example-mainnet/bootnodes.json"),
        }]
    }

    fn components(&self, args: &ExampleArgs, _chain: &ChainInfo<'_>) -> eyre::Result<Components> {
        let mut components = Components::default();
        components.rpc.push(Arc::new(ExampleRpc));
        components.tx_pool_policy = Some(Arc::new(MaxTxGas(args.max_tx_gas)));
        Ok(components)
    }
}
```

- **`NAME`** is the name of the binary, shown in `--help`.
- **`presets`** are the networks `--network <name>` accepts in addition to the Ethereum ones. The first preset is the default network of the binary. `genesis` is the contents of a genesis file. `bootnodes` is a JSON array of enode URLs, or `"[]"`. Each network keeps its data in a subdirectory of the datadir named after the preset.
- **`Args`** is a `clap::Args` struct. Prefix the flag names with the integration's name so they cannot collide with the node's. Use `ethrex::integration::NoArgs` if there are none.
- **`components`** is called once per start, after the command line is parsed and the genesis is loaded, and before the database is opened. `chain.genesis` is the genesis of the selected network. Returning an error aborts the start. `Components` is built by filling in the fields of `Components::default()`.

### 3. Add an RPC namespace

```rust
struct ExampleRpc;

impl RpcExtension for ExampleRpc {
    fn namespace(&self) -> &'static str {
        "example"
    }

    fn handle<'a>(&'a self, req: &'a RpcRequest, context: RpcApiContext) -> RpcExtensionFuture<'a> {
        Box::pin(async move {
            match req.method.as_str() {
                "example_ping" => Ok(serde_json::json!("pong")),
                // A built-in method, called in-process.
                "example_chainId" => {
                    let request = RpcRequest::new("eth_chainId", None);
                    ethrex_rpc::map_http_requests(&request, context).await
                }
                _ => Err(RpcErr::MethodNotFound(req.method.clone())),
            }
        })
    }
}
```

- Every method named `<namespace>_...` is routed to `handle`, on the HTTP and WebSocket endpoints. Extensions are not served on the authenticated port.
- The namespace must be lowercase letters and digits, and cannot be a built-in one (`eth`, `engine`, `debug`, `admin`, `net`, `web3`, `txpool`, `testing`). The node refuses to start otherwise.
- `context` is the same `RpcApiContext` the built-in handlers receive: the store, the blockchain, the node's configuration.
- An extension is served whenever it is in `Components::rpc`. `--http.api` selects built-in namespaces only. To let operators turn a namespace off, give the integration a flag and leave the extension out of `Components`.

### 4. Add a transaction pool rule

```rust
#[derive(Debug)]
struct MaxTxGas(u64);

impl TxPoolPolicy for MaxTxGas {
    fn admit(&self, tx: &Transaction, _sender: Address) -> Result<(), String> {
        if tx.gas_limit() > self.0 {
            return Err(format!("gas limit {} is above the cap of {}", tx.gas_limit(), self.0));
        }
        Ok(())
    }
}
```

- `admit` runs for every transaction offered to the pool, from RPC or from a peer, before the standard checks.
- It can only reject. A transaction it admits still has to pass the standard checks.
- The `Err` text is returned to whoever submitted the transaction.
- It has no effect on block validation: a block that contains a transaction the policy would reject is still valid.

### 5. Write `main`

```rust
// src/main.rs
ethrex::global_allocator!();

fn main() -> eyre::Result<()> {
    ethrex::integration::run(ethrex_example::Example)
}
```

- `run` parses the command line, starts the node, and returns when the node shuts down.
- `global_allocator!` declares the memory allocator and tuning the `ethrex` binary uses. Without it the binary runs on the system allocator, which is slower.

Build settings do not carry over from the ethrex repository, so copy them:

```toml
# Cargo.toml: the release profile of the ethrex workspace
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
```

Use the Rust version in ethrex's `rust-toolchain.toml`.

### 6. Run it

```sh
cargo build --release
./target/release/ethrex-example --help
# `--network` lists example-mainnet as a known network and as the default.
# `--example.max-tx-gas` is listed with the node's flags.

./target/release/ethrex-example --authrpc.jwtsecret ./jwt.hex --example.max-tx-gas 500000
```

```sh
curl -s localhost:8545 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"example_ping","params":[]}'
# {"id":1,"jsonrpc":"2.0","result":"pong"}
```

A transaction with a gas limit above 500000, sent with `eth_sendRawTransaction`, is answered with:

```
Transaction rejected by the pool policy: gas limit 600000 is above the cap of 500000
```

## How it works

```
your crate                              ethrex
──────────                              ──────
main() ── run(Example) ───────────────▶ parse the command line
                                          node flags + Example::Args
                                          --network resolved against presets()
                                        load the genesis
components(args, chain) ◀────────────── build components        (once)
        │
        └─ Components { rpc, tx_pool_policy }
                                        open the database
                 tx_pool_policy ──────▶ blockchain: checked on every pool admission
                 rpc ─────────────────▶ RPC server: unknown namespaces go to extensions
                                        start p2p and sync, serve until shutdown
```

- The integration is compiled into the binary. There is no runtime loading: one binary per chain.
- `run(Ethereum)` and `run(Example)` go through the same code. A node whose integration returns `Components::default()` behaves exactly like `ethrex`.
- Components are built once, before the database, the pool or the RPC server exist. Nothing can be added to a running node.

## Design decisions

- **Compiled in, not loaded.** Rust has no stable ABI, so an integration is a crate, and each chain ships its own binary.
- **Defaults mean Ethereum.** Every method of `Integration` and every field of `Components` has a default, so an integration states only what its chain does differently.
- **Fail at startup.** A preset that does not parse, an invalid RPC namespace, or an error from `components` aborts the start with a message naming the cause. Nothing an integration adds is optional at run time.
- **Extensions receive the node's own types** (`RpcApiContext`, `Transaction`). These are not a stable API: an integration is built against a specific ethrex commit and may need changes when it moves to a newer one.
- **`Components` can grow.** It is `#[non_exhaustive]`, so later versions can add fields without breaking integrations that fill in the ones they use.

## Testing

`test/tests/cmd/integration_tests.rs` defines an invented chain against the public API, as an outside crate would, and checks each extension point alone and on a running node. Extend it when adding an extension point.

```sh
cargo test --release -p ethrex-test --test ethrex_tests cmd::integration_tests
```

## References

- `cmd/ethrex/integration.rs`: `Integration`, `Components`, `run`.
- `crates/networking/rpc/rpc.rs`: `RpcExtension`, `validate_rpc_extensions`.
- `crates/blockchain/blockchain.rs`: `TxPoolPolicy`.
- `crates/common/config/networks.rs`: `NetworkPreset`.
