# ethrex L2 Contracts

## ToC

- [ethrex L2 Contracts](#ethrex-l2-contracts)
  - [ToC](#toc)
  - [L1 side](#l1-side)
    - [`CommonBridge`](#commonbridge)
    - [`OnChainOperator`](#onchainoperator)
    - [`Verifier`](#verifier)
  - [L2 side](#l2-side)
    - [`L1MessageSender`](#l1messagesender)

## L1 side

### `CommonBridge`

Allows L1<->L2 communication from L1. It both sends messages from L1 to L2 and receives messages from L2.

### `OnChainOperator`

Ensures the advancement of the L2. It is used by the operator to commit blocks and verify block proofs

### `Verifier`

Implements the verification logic for L2 block proofs. It validates the correctness of state transitions in the L2 chain by checking cryptographic proofs submitted by the operator. This ensures that only valid L2 blocks are accepted by the L1 contracts.

## L2 side

### `L1MessageSender`

Provides functionality for L2 contracts to send messages to L1. It allows L2 smart contracts to initiate cross-chain communication by queuing messages that will be delivered to the L1 CommonBridge contract when blocks are committed and proven.
