# Supporting other chains

ethrex can run on a chain other than Ethereum without knowing about it. Support for the chain lives in a crate outside this repository, which describes the chain with a `Chain` value and builds its own binary. The `ethrex` binary is built the same way: it runs the `ETHEREUM` chain, which adds nothing.

A chain can add:

| What | How |
|---|---|
| Named networks for `--network` | `Chain::networks` |
| Its own command line flags | The type parameter of `Chain` |
| RPC namespaces | `NodeComponents::rpc` |
| Mempool rules | `NodeComponents::mempool_policy` |

Everything a chain leaves out behaves as on Ethereum. A chain cannot change how blocks are validated or executed.

## Adding a chain

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

# The release profile of the ethrex workspace. Build settings do not carry over
# from the ethrex repository.
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
```

Use the Rust version in ethrex's `rust-toolchain.toml`.

### 2. Describe the chain

```rust
use std::sync::Arc;

use ethrex::chain::{Chain, NodeComponents};
use ethrex_blockchain::MempoolPolicy;
use ethrex_common::{
    Address,
    types::{BlockHeader, Genesis, Transaction},
};
use ethrex_config::networks::NetworkPreset;
use ethrex_rpc::{RpcApiContext, RpcErr, RpcExtension, RpcExtensionFuture, utils::RpcRequest};

/// Flags of the example chain, added to the node's own.
#[derive(clap::Args)]
pub struct ExampleArgs {
    /// Highest gas limit a transaction may have to enter the pool.
    #[arg(long = "example.max-tx-gas", default_value_t = 1_000_000)]
    max_tx_gas: u64,
}

pub const EXAMPLE: Chain<ExampleArgs> = Chain {
    name: "ethrex-example",
    networks: &[NetworkPreset {
        name: "example-mainnet",
        genesis: include_str!("../networks/example-mainnet/genesis.json"),
        bootnodes: include_str!("../networks/example-mainnet/bootnodes.json"),
    }],
    components,
};

fn components(args: &ExampleArgs, _genesis: &Genesis) -> eyre::Result<NodeComponents> {
    let mut components = NodeComponents::default();
    components.rpc.push(Arc::new(ExampleRpc));
    components.mempool_policy = Some(Arc::new(MaxTxGas(args.max_tx_gas)));
    Ok(components)
}
```

- **`name`** is the name of the binary, shown in `--help`.
- **`networks`** are the networks `--network <name>` accepts in addition to the Ethereum ones. The first one is the default network of the binary. `genesis` is the contents of a genesis file. `bootnodes` is a JSON array of enode URLs, or `"[]"`. Each network keeps its data in a subdirectory of the datadir named after it, so a name is lowercase letters, digits and inner hyphens, and cannot be one ethrex already uses (`mainnet`, `sepolia`, `hoodi`, `plataberget`, `dev`, or anything starting with `chain-`). The node refuses to start otherwise, and also when the genesis or the bootnodes of any of the chain's networks do not parse. A network's name wins over a file of the same name in the current directory; pass `./<name>` to use the file. The `ETHREX_NETWORK` environment variable takes the same names as `--network`.
- **The type parameter** is the chain's flags, a `clap::Args` struct. Prefix the flag names with the chain's name so they cannot collide with the node's. Use `ethrex::chain::NoArgs` if there are none.
- **`components`** is a function called once per start, after the command line is parsed and the genesis is loaded, and before the database is opened. It receives the chain's flags and the genesis of the selected network. Returning an error aborts the start. It is called on the node's async runtime, so it must not block on async work. `NodeComponents` is built by filling in the fields of `NodeComponents::default()`.

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

- Every method named `<namespace>_<name>` is routed to `handle`, on the HTTP and WebSocket endpoints. Extensions are not served on the authenticated port.
- The namespace must be lowercase letters and digits, and cannot be a built-in one (`eth`, `engine`, `debug`, `admin`, `net`, `web3`, `txpool`, `testing`). The node refuses to start otherwise.
- `context` is the same `RpcApiContext` the built-in handlers receive: the store, the blockchain, the node's configuration. Through it an extension can call built-in methods, but not methods of an extension. Such a call is subject to `--http.api` like one from outside: a built-in namespace the operator turned off is off for extensions too.
- An extension is served whenever it is in `NodeComponents::rpc`. `--http.api` selects built-in namespaces only. To let operators turn a namespace off, give the chain a flag and leave the extension out of the components.
- Calls to an extension are not counted in the node's RPC metrics.

### 4. Add a mempool rule

```rust
#[derive(Debug)]
struct MaxTxGas(u64);

impl MempoolPolicy for MaxTxGas {
    fn admit(&self, tx: &Transaction, _sender: Address, _head: &BlockHeader) -> Result<(), String> {
        if tx.gas_limit() > self.0 {
            return Err(format!("gas limit {} is above the cap of {}", tx.gas_limit(), self.0));
        }
        Ok(())
    }
}
```

- `admit` runs for every transaction offered to the mempool, from RPC or from a peer, before the standard checks. Keep it cheap, and never block in it.
- `head` is the header of the latest block, for rules that depend on the state of the chain.
- It can only reject. A transaction it admits still has to pass the standard checks.
- The `Err` text is returned to whoever submitted the transaction.
- It has no effect on block validation: a block that contains a transaction the policy would reject is still valid.
- It runs when a transaction is offered and not again. A transaction admitted under one head stays in the mempool when the head changes, and block building does not consult the policy.
- A rejected transaction that came from a peer does not count against that peer.

### 5. Write `main`

```rust
// src/main.rs
ethrex::global_allocator!();

fn main() -> eyre::Result<()> {
    ethrex::chain::run(ethrex_example::EXAMPLE, GLOBAL_ALLOCATOR)
}
```

- `run` parses the command line, starts the node, and returns when the node shuts down. It sets up the whole process (async runtime, thread pool, logging, signals), so call it once and from `main`.
- `global_allocator!` declares the memory allocator and tuning the `ethrex` binary uses, and defines the `GLOBAL_ALLOCATOR` constant that `run` takes. A binary cannot start the node without it, so it cannot end up on the slower system allocator by omission.

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
Transaction rejected by the mempool policy: gas limit 600000 is above the cap of 500000
```

## How it works

```
your crate                              ethrex
──────────                              ──────
main() ── run(EXAMPLE, ..) ───────────▶ parse the command line
                                          node flags + ExampleArgs
                                          --network resolved against networks
                                        load the genesis
components(args, genesis) ◀──────────── build the components    (once)
        │
        └─ NodeComponents { rpc, mempool_policy }
                                        open the database
                 mempool_policy ──────▶ blockchain: checked on every pool admission
                 rpc ─────────────────▶ RPC server: unknown namespaces go to extensions
                                        start p2p and sync, serve until shutdown
```

- The chain is compiled into the binary. There is no runtime loading: one binary per chain.
- The node still identifies itself as ethrex, the client it is built from, to its peers and in `web3_clientVersion`. It does not look for new ethrex releases, which only the `ethrex` binary does.
- `run(ETHEREUM, ..)` and `run(EXAMPLE, ..)` go through the same code. A node whose chain returns `NodeComponents::default()` behaves exactly like `ethrex`.
- The components are built once, before the database, the pool or the RPC server exist. Nothing can be added to a running node.

## Design decisions

- **Compiled in, not loaded.** Rust has no stable ABI, so support for a chain is a crate, and each chain ships its own binary.
- **Defaults mean Ethereum.** Every field of `NodeComponents` has a default, so a chain states only what it does differently.
- **Fail at startup.** A network whose name, genesis or bootnodes are not valid, an invalid RPC namespace, or an error from `components` aborts the start with a message naming the cause. Nothing a chain adds is optional at run time.
- **Extensions receive the node's own types** (`RpcApiContext`, `Transaction`, `BlockHeader`). These are not a stable API: a chain's crate is built against a specific ethrex commit and may need changes when it moves to a newer one.
- **`NodeComponents` can grow.** It is `#[non_exhaustive]`, so later versions can add fields without breaking chains that fill in the ones they use.

## Testing

`test/tests/cmd/chain_tests.rs` defines an invented chain against the public API, as an outside crate would, and checks everything it adds, alone and on a running node. Extend it when adding something a chain can supply.

```sh
cargo test --release -p ethrex-test --test ethrex_tests cmd::chain_tests
```

## References

- `cmd/ethrex/chain.rs`: `Chain`, `NodeComponents`, `run`, `ETHEREUM`.
- `crates/networking/rpc/rpc.rs`: `RpcExtension`, `RpcExtensions`.
- `crates/blockchain/blockchain.rs`: `MempoolPolicy`.
- `crates/common/config/networks.rs`: `NetworkPreset`.
