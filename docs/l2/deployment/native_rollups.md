# Deploying a native rollups ethrex L2

This guide covers how to deploy a native rollup L2 using ethrex. Native rollups (based on [EIP-8079](https://github.com/ethereum/EIPs/pull/9608)) replace ZK proofs and fraud proofs with direct re-execution on L1: when the L2 submits a block, L1 re-executes it via the `EXECUTE` precompile and verifies the state transition is correct.

> [!NOTE]
> This is a Phase 1 proof-of-concept. The native rollup L2 runs against a local ethrex L1 with the EXECUTE precompile enabled. It is not yet intended for public testnets or production.

## Components

The native rollup L2 integration wires together the EXECUTE precompile (EVM-level block re-execution) and the L2 GenServer actors (block producer, L1 watcher, L1 committer) into a working end-to-end system. The key components are:

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
- `NativeRollupOptions` struct with flags: `--native-rollups`, `--native-rollups.contract-address`, `--native-rollups.relayer-pk`, `--native-rollups.l1-pk`, `--native-rollups.block-time`, `--native-rollups.commit-interval`.

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
│  NativeL1Committer               │
│    ├─ generates execution witness│
│    └─ calls advance() on L1      │
│                                  │
│  Predeploys:                     │
│    L2Bridge  (0x...fffd)         │
│    L1Anchor  (0x...fffe)         │
└──────────────────────────────────┘
```

## Prerequisites

- Rust toolchain (stable)
- `solc` (Solidity compiler) — needed to compile the contracts during build
- `rex` (cast-compatible CLI) — for querying contracts in the demo steps. Install: `cargo install rex-cli` (or use `cast` from Foundry as a drop-in replacement)

Verify solc is installed:

```shell
solc --version
```

## Demo

> **What this demo shows:** a native rollup L2 that settles blocks to L1 via direct re-execution (the EXECUTE precompile), with a live deposit (L1→L2) and withdrawal (L2→L1) roundtrip.

The native rollup L2 runs with three terminals: one for L1, one for contract deployment, and one for L2.

All commands are run from the repository root.

### Setup

#### Terminal 1 — Start L1

Start a local ethrex L1 with the EXECUTE precompile enabled:

```shell
NATIVE_ROLLUPS=1 make -C crates/l2 rm-db-l1 init-l1
```

Wait until you see L1 producing blocks (the `--dev` flag auto-mines).

#### Terminal 2 — Deploy contracts

Deploy `NativeRollup.sol` to L1, generate the L2 genesis, and fund the contract:

```shell
NATIVE_ROLLUPS=1 make -C crates/l2 deploy-l1
```

You should see output like:

```
NativeRollup.sol deployed at: 0x...
Funded NativeRollup contract with 100 ETH
Contract address written to cmd/.env
```

Save the contract address for later:

```shell
source cmd/.env
echo $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS
```

#### Terminal 3 — Start L2

Start the native rollup L2 node:

```shell
NATIVE_ROLLUPS=1 make -C crates/l2 rm-db-l2 init-l2
```

### Step 1: Verify the L2 is advancing

Once the L2 is running, you should see log lines like:

```
NativeBlockProducer: produced block N
NativeL1Committer: advance() tx sent for block N
```

Query the L1 contract to verify:

```shell
# L2 block number committed to L1
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "blockNumber()(uint256)" --rpc-url http://localhost:8545

# L2 block number from the L2 RPC directly
rex block-number --rpc-url http://localhost:1729
```

The L1 value should trail the L2 value by a few blocks (the committer runs on a configurable interval).

### Step 2: Query contract state

The `NativeRollup.sol` contract exposes public getters for all its state:

```shell
# Current L2 state root
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "stateRoot()(bytes32)" --rpc-url http://localhost:8545

# Block gas limit
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "blockGasLimit()(uint256)" --rpc-url http://localhost:8545

# Last base fee per gas
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "lastBaseFeePerGas()(uint256)" --rpc-url http://localhost:8545

# Last gas used
rex call $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS "lastGasUsed()(uint256)" --rpc-url http://localhost:8545
```

### Step 3: Deposit (L1 → L2)

Send ETH from L1 to a fresh account on L2.

```shell
# Pick a fresh address (not pre-funded on L2)
# Private key 0x42 → address 0xC083e9947Cf02b463B4376E084e1a1720F808DfB
DEPOSIT_TO=0xC083e9947Cf02b463B4376E084e1a1720F808DfB

# Check L2 balance is 0
rex balance $DEPOSIT_TO --rpc-url http://localhost:1729

# Deposit 1 ETH via sendL1Message(to, gasLimit, data)
# Uses the L1 deployer key (pre-funded with 1M ETH)
rex send $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS \
  "sendL1Message(address,uint256,bytes)" \
  $DEPOSIT_TO 105000 0x \
  --value 1ether \
  --rpc-url http://localhost:8545 \
  --private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

# Watch the L2 logs in Terminal 3 — the watcher picks up the message and
# the block producer includes a relayer tx for it.

# After a few seconds, check the L2 balance again:
rex balance $DEPOSIT_TO --rpc-url http://localhost:1729
# Should show 1 ETH (1000000000000000000 wei)
```

### Step 4: Withdraw (L2 → L1)

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
  --value 0.5ether \
  --rpc-url http://localhost:1729 \
  --private-key 0x0000000000000000000000000000000000000000000000000000000000000042
```

Wait for the L2 block containing the withdrawal to be committed to L1 (watch the committer logs in Terminal 3).

Then fetch the withdrawal proof and claim on L1:

```shell
# Get the withdrawal proof (replace TX_HASH with the L2 withdrawal tx hash)
curl -s -X POST http://localhost:1729 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"ethrex_getNativeWithdrawalProof","params":["TX_HASH"],"id":1}' | jq '.result'

# The response contains: from, receiver, amount, messageId, blockNumber, accountProof, storageProof
# Use these fields to call claimWithdrawal on L1:
rex send $ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS \
  "claimWithdrawal(address,address,uint256,uint256,uint256,bytes[],bytes[])" \
  FROM RECEIVER AMOUNT MESSAGE_ID BLOCK_NUMBER '[ACCOUNT_PROOF]' '[STORAGE_PROOF]' \
  --rpc-url http://localhost:8545 \
  --private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924

# Verify the L1 balance increased
rex balance $L1_RECEIVER --rpc-url http://localhost:8545
```

> [!TIP]
> The integration test at `test/tests/l2/native_rollup_bridge.rs` automates the full deposit/withdraw roundtrip including proof fetching and claim submission. Run it with:
> ```shell
> cargo test -p ethrex-test --features native-rollups -- l2::native_rollup_bridge --nocapture
> ```

### Cleaning up

Remove the databases to start fresh:

```shell
make -C crates/l2 rm-db-l1 rm-db-l2
```

## Further reading

- [EXECUTE precompile architecture](../../vm/levm/native_rollups.md) — detailed specification of the precompile, contracts, and L1 message mechanism
- [Native rollups gap analysis](../../vm/levm/native_rollups_gap_analysis.md) — comparison with the L2Beat native rollups spec
