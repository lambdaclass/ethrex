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

A cryptographic proof verification contract that ensures the validity of L2 state transitions. It implements multiple verification schemes (Risc0, SP1, Pico) to validate zero-knowledge proofs submitted by the operator, guaranteeing that the L2 execution was performed correctly.

## L2 side

### `L1MessageSender`

Facilitates sending messages from L2 to L1. It allows users to initiate withdrawals by burning funds on L2 and sending a corresponding message to L1 that can be used to claim those funds. The contract works in conjunction with CommonBridgeL2 to enable secure cross-chain communication.
