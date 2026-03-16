# Ethrex REPL

Interactive Read-Eval-Print Loop for Ethereum JSON-RPC, integrated as the `ethrex repl` subcommand.

## Quick Start

```bash
# Via the ethrex binary
ethrex repl

# Or directly
cargo run -p ethrex-repl

# Connect to a specific endpoint
ethrex repl -e https://eth.llamarpc.com

# Execute a single command and exit
ethrex repl -x "eth.blockNumber"
```

## Usage

```
ethrex-repl [OPTIONS]

Options:
  -e, --endpoint <URL>       JSON-RPC endpoint [default: http://localhost:8545]
      --history-file <PATH>  Path to command history file [default: ~/.ethrex/history]
  -x, --execute <COMMAND>    Execute a single command and exit
```

## Features

### RPC Commands

Type `namespace.method` with arguments separated by spaces or in parentheses:

```
> eth.blockNumber
68943

> eth.getBalance 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045
1000000000000000000

> eth.getBalance("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", "latest")
1000000000000000000

> eth.getBlockByNumber 100 true
┌─────────────────────────────────────────┐
│            number   100                 │
│         timestamp   1438270128          │
│  ...                                    │
└─────────────────────────────────────────┘
```

### Supported Namespaces

| Namespace | Methods | Description |
|-----------|--------:|-------------|
| `eth`     | 30      | Accounts, blocks, transactions, filters, gas, proofs |
| `debug`   | 8       | Raw headers/blocks/transactions/receipts, tracing |
| `admin`   | 4       | Node info, peers, log level, add peer |
| `net`     | 2       | Network ID, peer count |
| `web3`    | 1       | Client version |
| `txpool`  | 2       | Transaction pool content and status |

Type `.help` to list all namespaces, `.help eth` to list methods in a namespace, or `.help eth.getBalance` for detailed method documentation.

### ENS Name Resolution

Any command that accepts an address also accepts ENS names:

```
> eth.getBalance vitalik.eth
Resolved vitalik.eth -> 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045
1000000000000000000
```

Resolution is done on-chain by querying the ENS registry at `0x00000000000C2E074eC69A0dFb2997BA6C7d2e1e`.

### Utility Functions

| Function | Example | Description |
|----------|---------|-------------|
| `toWei` | `toWei 1.5 ether` → `1500000000000000000` | Convert to wei |
| `fromWei` | `fromWei 1000000000 gwei` → `1` | Convert from wei |
| `toHex` | `toHex 255` → `0xff` | Decimal to hex |
| `fromHex` | `fromHex 0xff` → `255` | Hex to decimal |
| `keccak256` | `keccak256 0x68656c6c6f` → `0x1c8a...` | Keccak-256 hash |
| `toChecksumAddress` | `toChecksumAddress 0xd8da...` → `0xd8dA...` | EIP-55 checksum |
| `isAddress` | `isAddress 0xd8dA...` → `true` | Validate address format |

Units for `toWei`/`fromWei`: `wei`, `gwei`, `ether` (or `eth`).

### Built-in Commands

| Command | Description |
|---------|-------------|
| `.help [namespace\|command]` | Show help |
| `.exit` / `.quit` | Exit the REPL |
| `.clear` | Clear the screen |
| `.connect <url>` | Show or change endpoint |
| `.history` | Show history file path |

### Other Features

- **Tab completion** for namespaces, methods, block tags, and utilities
- **Parameter hints** shown after typing a full method name
- **Multi-line input** — unbalanced `{}` or `[]` automatically continues to the next line
- **Persistent history** saved to `~/.ethrex/history`
- **Formatted output** — addresses, hashes, hex quantities, and nested objects are colored and auto-formatted

## Architecture

```
tooling/repl/src/
├── lib.rs          Entry point, run() function
├── main.rs         Standalone binary CLI
├── repl.rs         REPL loop, command dispatch, utility functions
├── parser.rs       Tokenizer and command parser
├── client.rs       JSON-RPC HTTP client
├── commands/       Command definitions per namespace
│   ├── mod.rs      CommandDef, ParamDef, validation, registry
│   ├── eth.rs      30 eth_* methods
│   ├── debug.rs    8 debug_* methods
│   ├── admin.rs    4 admin_* methods
│   ├── net.rs      2 net_* methods
│   ├── web3.rs     1 web3_* method
│   └── txpool.rs   2 txpool_* methods
├── completer.rs    Tab completion and parameter hints
├── formatter.rs    Output formatting and colorization
└── ens.rs          ENS name resolution (namehash + on-chain lookup)
```

## Running Tests

```bash
cargo test -p ethrex-repl
```

The test suite includes 189 unit tests covering all modules and 22 end-to-end tests using a mock JSON-RPC server.
