# Ethrex L2 sequencer

## Components

The L2 Proposer is composed of the following components:

### Block Producer

Creates Blocks with a connection to the `auth.rpc` port.

### L1 Watcher

This component monitors the L1 for new deposits made by users. For that, it queries the CommonBridge contract on L1 at regular intervals (defined by the config file) for new DepositInitiated() events. Once a new deposit event is detected, it creates the corresponding deposit transaction on the L2.

### L1 Transaction Sender (a.k.a. L1 Committer)

As the name suggests, this component sends transactions to the L1. But not any transaction, only commit and verify transactions.

Commit transactions are sent when the Proposer wants to commit to a new batch of blocks. These transactions contain the batch data to be committed in the L1.

Verify transactions are sent by the Proposer after the prover has successfully generated a proof of block execution to verify it. These transactions contains the new state root of the L2, the hash of the state diffs produced in the block, the root of the withdrawals logs merkle tree and the hash of the processed deposits.

### Proof Coordinator

The Proof Coordinator is a simple TCP server that manages communication with a component called the Prover. The Prover acts as a simple TCP client that makes requests to prove a block to the Coordinator. It responds with the proof input data required to generate the proof. Then, the Prover executes a zkVM, generates the Groth16 proof, and sends it back to the Coordinator.

The Proof Coordinator centralizes the responsibility of determining which block needs to be proven next and how to retrieve the necessary data for proving. This design simplifies the system by reducing the complexity of the Prover, it only makes requests and proves blocks.

For more information about the Proof Coordinator, the Prover, and the proving process itself, see the [Prover Docs](./prover.md).

### L1 Proof Sender

The L1 Proof Sender is responsible for interacting with Ethereum L1 to manage proof verification. Its key functionalities include:

- Connecting to Ethereum L1 to send proofs for verification.
- Dynamically determine required proof types based on active verifier contracts (`PICOVERIFIER`, `R0VERIFIER`, `SP1VERIFIER`).
- Ensure blocks are verified in the correct order by invoking the `verify(..)` function in the `OnChainProposer` contract. Upon successful verification, an event is emitted to confirm the block's verification status.
- Operating on a configured interval defined by `proof_send_interval_ms`.

## Configuration

Configuration is done either by CLI flags or through environment variables. Run `cargo run --release --bin ethrex -- l2 init --help` in the repository's root directory to see the available CLI flags and envs.
