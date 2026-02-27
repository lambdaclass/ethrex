# Deploying a native rollups ethrex L2

This guide covers how to deploy a native rollup L2 using ethrex. Native rollups (based on [EIP-8079](https://github.com/ethereum/EIPs/pull/9608)) replace ZK proofs and fraud proofs with direct re-execution on L1: when the L2 submits a block, L1 re-executes it via the `EXECUTE` precompile and verifies the state transition is correct.

> [!NOTE]
> This is a Phase 1 proof-of-concept. The native rollup L2 runs against a local ethrex L1 with the EXECUTE precompile enabled. It is not yet intended for public testnets or production.

## Components

The native rollup L2 integration wires together the EXECUTE precompile (EVM-level block re-execution) and the L2 GenServer actors (block producer, L1 watcher, L1 advancer) into a working end-to-end system. The key components are:

**Build system** (`cmd/ethrex/build_l2.rs`):
- Compiles `NativeRollup.sol` (creation bytecode), `L2Bridge.sol` and `L1Anchor.sol` (runtime bytecodes) via solc during the build.

**Deployer** (`cmd/ethrex/l2/deployer.rs`):
- New `--native-rollups` deploy path that:
  1. Generates the L2 genesis file dynamically with pre-deployed L2Bridge (`0x...fffd`) and L1Anchor (`0x...fffe`), a funded relayer account, and test accounts.
  2. Computes the L2 genesis state root from the generated genesis.
  3. Deploys `NativeRollup.sol` to L1 with the genesis state root.
  4. Funds the contract with 100 ETH (matching the relayer's L2 prefund; the contract needs ETH to pay withdrawal claims).
  5. Writes the contract address to `cmd/.env`.

The L2Bridge is preminted with an effectively infinite ETH balance (`U256::MAX / 2`) so it can cover any number of L1-to-L2 deposits without running out.

**CLI options** (`cmd/ethrex/l2/options.rs`):
- `NativeRollupOptions` struct with flags: `--native-rollups`, `--native-rollups.contract-address`, `--native-rollups.relayer-pk`, `--native-rollups.l1-pk`, `--native-rollups.block-time`, `--native-rollups.advance-interval`.

**L2 initializer** (`cmd/ethrex/l2/initializers.rs`):
- `init_native_rollup_l2()` boots the L2 node with `BlockchainType::L1`. This is intentional: native rollups run L2 blocks through an unmodified L1 execution environment (the EXECUTE precompile re-executes them on L1). The L2 must produce blocks compatible with L1's precompile set and execution rules, so it uses the same `BlockchainType` as L1.

**Command routing** (`cmd/ethrex/l2/command.rs`):
- Routes to the native rollup deploy/init paths when `--native-rollups` is set.

**Makefile** (`crates/l2/Makefile`):
- Conditional `deploy-l1` and `init-l2` targets activated by `NATIVE_ROLLUPS=1`.

## Architecture overview

```
┌──────────────────────────────────┐
│            L1 (ethrex)           │
│                                  │
│  NativeRollup.sol                │
│    ├─ advance()                  │
│    │    └─ calls EXECUTE(0x0101) │
│    │         └─ re-executes L2   │
│    │            block in LEVM    │
│    ├─ sendL1Message()            │
│    └─ claimWithdrawal()          │
└──────────┬───────────────────────┘
           │ L1 RPC
           │
┌──────────┴───────────────────────┐
│         L2 (ethrex native)       │
│                                  │
│  NativeL1Watcher                 │
│    └─ polls L1 for new messages  │
│                                  │
│  NativeBlockProducer             │
│    ├─ builds relayer txs for     │
│    │  L1 messages                │
│    ├─ anchors Merkle root in     │
│    │  L1Anchor predeploy         │
│    └─ produces L2 blocks         │
│                                  │
│  NativeL1Advancer               │
│    ├─ generates execution witness│
│    └─ calls advance() on L1      │
│                                  │
│  Predeploys:                     │
│    L2Bridge  (0x...fffd)         │
│    L1Anchor  (0x...fffe)         │
└──────────────────────────────────┘
```

The three L2 GenServer actors run as concurrent tasks:

- **NativeL1Watcher** — Polls the L1 NativeRollup.sol contract at regular intervals for `L1MessageRecorded` events and pushes them into a shared `PendingL1Messages` queue. It scans L1 logs in configurable block ranges and parses the event data (sender, recipient, value, gas limit, calldata, nonce).

- **NativeBlockProducer** — Produces L2 blocks every `block_time_ms` milliseconds. It first consumes pending L1 messages from the queue, builds signed relayer transactions to execute those messages via the L2Bridge contract, then fills remaining block gas with regular mempool transactions. It anchors the L1 messages Merkle root in the L1Anchor predeploy's storage before execution.

- **NativeL1Advancer** — Reads produced L2 blocks from the Store and submits them to the NativeRollup.sol contract via the `advance()` function, passing the block parameters, transaction RLP, execution witness JSON, and the count of L1 messages in the block. It tracks the on-chain block number and advances one block per interval.

## Prerequisites

- Rust toolchain (stable)
- `solc` (Solidity compiler) — needed to compile the contracts during build
- `rex` (cast-compatible CLI) — for querying contracts in the demo steps. Install: `cargo install rex-cli` (or use `cast` from Foundry as a drop-in replacement)

> [!NOTE]
> **Rex CLI syntax quirks:**
> - `--value` is always in **wei** (e.g., `1000000000000000000` for 1 ETH).
> - `bytes` arguments should be passed as hex **without** the `0x` prefix (e.g., `d09de08a`), or as `""` for empty bytes.

Verify solc is installed:

```shell
solc --version
```

## Demo

> **What this demo shows:** a native rollup L2 that settles blocks to L1 via direct re-execution (the EXECUTE precompile), with a live deposit (L1→L2) and withdrawal (L2→L1) roundtrip.

The native rollup L2 runs with three terminals: one for L1, one for contract deployment, and one for L2.

All commands are run from the repository root.

### Setup

Build the binary first (this compiles the Solidity contracts and embeds them):

```shell
COMPILE_CONTRACTS=true cargo build --release --features l2,l2-sql,native-rollups
```

#### Terminal 1 — Start L1

Start a local ethrex L1 with the EXECUTE precompile enabled:

```shell
./target/release/ethrex \
  --network fixtures/genesis/l1.json \
  --http.port 8545 --http.addr 0.0.0.0 --authrpc.port 8551 \
  --dev --datadir /tmp/ethrex_l1
```

Wait until you see L1 producing blocks (the `--dev` flag auto-mines).

#### Terminal 2 — Deploy contracts and start L2

Deploy `NativeRollup.sol` to L1 and generate the L2 genesis:

```shell
./target/release/ethrex l2 deploy \
  --eth-rpc-url http://localhost:8545 \
  --private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --native-rollups \
  --native-rollups.relayer-pk 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d \
  --genesis-l2-path fixtures/genesis/native_l2.json
```

You should see output like:

```
NativeRollup.sol deployed at: 0x...
Contract address written to cmd/.env
```

Save the contract address and start the L2 node:

```shell
source cmd/.env

./target/release/ethrex l2 \
  --native-rollups \
  --native-rollups.contract-address $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS \
  --native-rollups.relayer-pk 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d \
  --native-rollups.l1-pk 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --network fixtures/genesis/native_l2.json \
  --http.port 1729 --http.addr 0.0.0.0 \
  --datadir /tmp/ethrex_l2 \
  --eth.rpc-url http://localhost:8545 \
  --no-monitor
```

### Step 1: Verify the L2 is advancing

Once the L2 is running, you should see log lines like:

```
NativeBlockProducer: produced block N
NativeL1Advancer: advanced block N on L1 (state_root=..., l1_msgs=0, tx=...)
```

Query the L1 contract to verify:

```shell
# L2 block number committed to L1
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "blockNumber()" --rpc-url http://localhost:8545

# L2 block number from the L2 RPC directly
rex block-number --rpc-url http://localhost:1729
```

The L1 value should trail the L2 value by a few blocks (the advancer runs on a configurable interval).

### Step 2: Query contract state

The `NativeRollup.sol` contract exposes public getters for all its state:

```shell
# Current L2 state root
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "stateRoot()" --rpc-url http://localhost:8545

# Block gas limit
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "blockGasLimit()" --rpc-url http://localhost:8545

# Last base fee per gas
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "lastBaseFeePerGas()" --rpc-url http://localhost:8545

# Last gas used
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "lastGasUsed()" --rpc-url http://localhost:8545
```

### Step 3: Deposit ETH (L1 → L2)

Send ETH from L1 to a fresh account on L2.

```shell
# Pick a fresh address (not pre-funded on L2)
# Private key 0x42 → address 0x6f4c950442e1af093bcff730381e63ae9171b87a
DEPOSIT_TO=0x6f4c950442e1af093bcff730381e63ae9171b87a

# Check L2 balance is 0
rex balance $DEPOSIT_TO --rpc-url http://localhost:1729

# Deposit 1 ETH via sendL1Message(to, gasLimit, data)
# Uses the L1 deployer key (pre-funded with 1M ETH)
# Note: --value is in wei (1 ETH = 1000000000000000000)
rex send $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS \
  "sendL1Message(address,uint256,bytes)" \
  $DEPOSIT_TO 105000 "" \
  --value 1000000000000000000 \
  --rpc-url http://localhost:8545 \
  -k 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

# Watch the L2 logs in Terminal 3 — the watcher picks up the message and
# the block producer includes a relayer tx for it.

# After ~10 seconds, check the L2 balance:
rex balance $DEPOSIT_TO --rpc-url http://localhost:1729
# Should show 1000000000000000000 (1 ETH in wei)
```

### Step 4: Deploy a contract on L2 and call it from L1

This step demonstrates that L1→L2 messages can carry arbitrary calldata, not just ETH transfers. We deploy a Counter contract on L2, then increment it by sending a message from L1.

```shell
# Deploy Counter.sol on L2 (increment + get functions)
# Uses the deposit recipient account (funded with 1 ETH in Step 3)
rex deploy --contract-path crates/l2/contracts/src/example/Counter.sol \
  --remappings "" \
  --rpc-url http://localhost:1729 \
  --private-key 0x0000000000000000000000000000000000000000000000000000000000000042
# Note the deployed contract address from the output
COUNTER=<deployed_address>

# Verify counter starts at 0
rex call $COUNTER "count()" --rpc-url http://localhost:1729

# Send an L1 message that calls increment() on the counter
# increment() selector = 0xd09de08a (pass without 0x prefix as bytes arg)
rex send $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS \
  "sendL1Message(address,uint256,bytes)" \
  $COUNTER 105000 d09de08a \
  --value 0 \
  --rpc-url http://localhost:8545 \
  -k 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

# Wait ~10 seconds for the L2 to process the L1 message, then check:
rex call $COUNTER "count()" --rpc-url http://localhost:1729
# Should return 1
```

### Step 5: Withdraw ETH (L2 → L1)

Send ETH from L2 back to an L1 address.

```shell
# L1 receiver address (the deployer account)
L1_RECEIVER=0xE25583099BA105D9ec0A67f5Ae86D90e50036425

# Record L1 balance before
rex balance $L1_RECEIVER --rpc-url http://localhost:8545

# Withdraw 0.5 ETH from L2 via L2Bridge.withdraw(receiver)
# Uses the test account's private key (0x42)
rex send 0x000000000000000000000000000000000000fffd \
  "withdraw(address)" \
  $L1_RECEIVER \
  --value 500000000000000000 \
  --rpc-url http://localhost:1729 \
  -k 0x0000000000000000000000000000000000000000000000000000000000000042
```

Wait for the L2 block containing the withdrawal to be advanced on L1 (watch the advancer logs in Terminal 2).

Then claim the withdrawal on L1 (replace `TX_HASH` with the L2 withdrawal tx hash from the output above):

```shell
# Fetches the proof from L2 and claims the withdrawal on L1 in one step
rex l2 claim-native-withdraw TX_HASH \
  0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS \
  http://localhost:8545 \
  http://localhost:1729

# Verify the L1 balance increased
rex balance $L1_RECEIVER --rpc-url http://localhost:8545
```

> [!TIP]
> The integration test at `test/tests/l2/native_rollup.rs` automates the full deposit/withdraw/counter roundtrip including proof fetching and claim submission. Run it with:
> ```shell
> cargo test -p ethrex-test --features native-rollups -- l2::native_rollup --nocapture
> ```

### Cleaning up

Remove the databases to start fresh:

```shell
rm -rf /tmp/ethrex_l1 /tmp/ethrex_l2
```

## Further reading

- [EXECUTE precompile architecture](../../vm/levm/native_rollups.md) — detailed specification of the precompile, contracts, gap analysis vs the L2Beat native rollups spec, and L1 message mechanism
