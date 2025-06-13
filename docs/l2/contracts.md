# Ethrex L2 contracts

## ToC

- [Ethrex L2 contracts](#ethrex-l2-contracts)
  - [ToC](#toc)
  - [L1 side](#l1-side)
    - [`CommonBridge`](#commonbridge)
      - [Deposit Functions](#deposit-functions)
        - [Simple Deposits](#simple-deposits)
        - [Deposits with Contract Interaction](#deposits-with-contract-interaction)
    - [`OnChainOperator`](#onchainoperator)
    - [`Verifier`](#verifier)
  - [L2 side](#l2-side)
    - [`L1MessageSender`](#l1messagesender)

## L1 side

### `CommonBridge`

Allows L1<->L2 communication from L1. It both sends messages from L1 to L2 and receives messages from L2.

#### Deposit Functions

##### Simple Deposits

- Send ETH directly to the contract address using a standard transfer
- The contract's `receive()` function automatically forwards funds to your identical address on L2
- No additional parameters needed

##### Deposits with Contract Interaction

```solidity
function deposit(DepositValues calldata depositValues) public payable
```

Parameters:

- `to`: Target address on L2
- `recipient`: Address that will receive the ETH on L2 (can differ from sender)
- `gasLimit`: Maximum gas for L2 execution
- `data`: Calldata to execute on the target L2 contract

This method enables atomic operations like:

- Depositing ETH while simultaneously interacting with L2 contracts
- Funding another user's L2 account

### `OnChainOperator`

Ensures the advancement of the L2. It is used by the operator to commit batches of blocks and verify batch proofs.

### `Verifier`

TODO

## L2 side

### `L1MessageSender`

TODO
