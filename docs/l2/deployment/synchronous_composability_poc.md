# Synchronous Composability (PoC)

## Status

**Development branch:** `sync_comp_poc`

| Sync | Column 1                  | Status |
| -------- | ------------------------- | ------ |
| L1 -> L2 | Deposits                  | ✅     |
| L1 -> L2 | L2 contract calls from L1 | ✅     |
| L2 -> L1 | Withdrawals               | ✅     |
| L2 -> L1 | L1 contract calls from L2 | ❌     |
| L2 -> L2 |                           | 🔜       |

## Commands

### Prerequisites

- A fresh-cloned ethrex repository.
- `rex` installed and available in your PATH. If you haven't installed it yet, follow one of the methods in the [rex repository](https://github.com/lambdaclass/rex?tab=readme-ov-file#rex-cli).

### Run a supernode

The following command will:

1. Remove both L1 and L2 dev databases (to start from scratch).
2. Start an ethrex supernode, i.e. an L1 execution client embedded with an L2 sequencer node.

```shell
rm -rf dev_ethrex_l*; RUSTFLAGS="-Awarnings" COMPILE_CONTRACTS=true RUST_LOG=off cargo run -r -F l2,l2-sql -- l2 --supernode --block-producer.coinbase-address $(rex a -z) --committer.l1-private-key 0x850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c --proof-coordinator.l1-private-key 0xf296c7802555da2a5a662be70e078cbd38b44f96f8615ae529da41122ce8db05 --eth.rpc-url http://localhost:8545 --validium --no-monitor --datadir dev_ethrex_l2 --network ./fixtures/genesis/l2.json --http.port 1729 --committer.commit-time 86400000

# Same but enabling logs

rm -rf dev_ethrex_l*; RUSTFLAGS="-Awarnings" COMPILE_CONTRACTS=true RUST_LOG=info,ethrex_p2p=error,ethrex_l2::sequencer::l1_committer=debug cargo run -r -F l2,l2-sql -- l2 --supernode --block-producer.coinbase-address $(rex a -z) --committer.l1-private-key 0x850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c --proof-coordinator.l1-private-key 0xf296c7802555da2a5a662be70e078cbd38b44f96f8615ae529da41122ce8db05 --eth.rpc-url http://localhost:8545 --validium --no-monitor --datadir dev_ethrex_l2 --network ./fixtures/genesis/l2.json --http.port 1729 --committer.commit-time 86400000
```

### Testing L1 -> L2 synchronous composability

#### Synchronous Deposits

```shell
rex transfer 999999999999999999 0x67cad0d689b799f385d2ebcf3a626254a9074e12 0x41443995d9eb6c6d6df51e55db2b188b12fe0f80d32817e57e11c64acff1feb8
```

#### L1 contract calling into an L2 contract

```shell
# Deploy a Counter.sol contract in the L1

rex deploy --contract-path crates/l2/contracts/src/example/Counter.sol 0 0x41443995d9eb6c6d6df51e55db2b188b12fe0f80d32817e57e11c64acff1feb8 --remappings ""

# Update that contract state by statically calling a contract in the L2

rex send 0x3fe21258005ca065695d205aac21168259e58155 "update(address)" 0x67cad0d689b799f385d2ebcf3a626254a9074e12 --private-key 0x41443995d9eb6c6d6df51e55db2b188b12fe0f80d32817e57e11c64acff1feb8
```

### Testing L2 -> L1 synchronous composability

#### Synchronous Withdrawals

```shell
# Deposit
rex transfer 999999999999999999 0x67cad0d689b799f385d2ebcf3a626254a9074e12 0x41443995d9eb6c6d6df51e55db2b188b12fe0f80d32817e57e11c64acff1feb8

# Withdrawal
rex l2 withdraw 111111111111111111 0x41443995d9eb6c6d6df51e55db2b188b12fe0f80d32817e57e11c64acff1feb8
```

## Introduction

### L1 -> L2 Synchronous Composability

#### Synchronous Deposits

Deposits are the process by which L1 users can enter L2 in some form. This process begins and ends on L1 through a series of steps:

1. **Initiate the deposit on L1**:
   - A user sends a transaction to L1, either via an ETH transfer to the `CommonBridge` contract or by calling the `deposit` function on the same contract. Both actions execute the same logic, which, upon successful execution, emits a log containing the necessary information for the sequencer of the corresponding L2 to process it.
   - This transaction must be included in a block, and that block must be finalized for the sequencer on the corresponding L2 to detect the log on L1.
2. **Process the deposit on L2**:
   - When the sequencer processes this log, it includes a transaction in its mempool that mints the corresponding ETH to the recipient's address, thereby ensuring the recipient has funds on L2.
3. **Commit the deposit process from L2 to L1**:
   - Eventually, the L2 batch that includes this mint transaction is sealed and committed to L1. This commit transaction must be included in an L1 block and finalized.
   - The same batch is sent to a prover to generate a ZK proof validating the previously committed batch.
4. **Verify the deposit process from L2 to L1 (deposit finalization)**:
   - Eventually, the batch execution proof is generated and returned to the sequencer, which submits it for verification on L1 via a `verify` transaction.
   - The `verify` transaction, assuming it is valid, must be included in an L1 block and finalized.

This 4-step process requires, by definition, that it occur across different L1 slots. The number of slots needed can vary based on L1's configuration, but even assuming a sufficiently fast commit time, real-time proving to generate the proof quickly, and a sufficiently fast proof submission time, this process would still require at least 2 slots: the first is always mandatory to emit the log that the sequencer listens for, and with significant luck, finalization could occur in the next slot.

Synchronous Composability enables this entire process to happen within the same L1 slot. In other words, the transaction that initiates the deposit, the deposit processing on L2, the commit transaction for the batch that included the mint, the generation of the execution proof for that batch, and the verify transaction for the same batch all occur in the same L1 slot.

#### L1 Contract Calling into an L2 Contract

Another capability enabled by synchronous composability is the ability to call L2 contracts from L1.

A simple example of this would be updating the state of a counter on L1 with the current state of another counter that resides on L2.

Unlike deposits, which do not require synchronous composability to function normally, calling an L2 contract from L1 and using the result as part of the L1 execution is not possible without this feature.

### L2 -> L1 Synchronous Composability

TBD

### Rollup Requirements for SC and How We Addressed Them in the PoC

To achieve synchronous composability, our rollup needed to fulfill the following requirements:

1. **Reorg with L1**: The rollup consumes unconfirmed L1 data and therefore must reorganize (reorg) with L1.
2. **Instant Settlement**: The rollup must be able to settle within one L1 slot, requiring real-time proving.
3. **Coordinated Sequencing**: The L2 proposer is the L1 proposer or works closely together (e.g., issues L1 inclusion preconfs).

We addressed these requirements in the following manner:

1. For this PoC, we removed reorgs from the equation.
2. Our L2 block builder would force a commit batch transaction after building a block that includes a scoped call. Assuming real-time proving by skipping verification, the commit transaction now serves as a settlement transaction.
3. We extended the ethrex functionality with a supernode mode that operates essentially as an L1 and L2 node sharing both states. This allows the L1 to insert transactions into the L2 mempool and simulate the L2 state in real time, while the L2 can insert transactions into the L1 mempool and simulate the L1 state in real time.

## Future work

TBD
