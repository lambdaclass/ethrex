# Transaction Fees

This page describes the different types of transaction fees that the Ethrex L2 rollup can charge and how they can be configured.

> [!NOTE]  
> Privileged transactions are exempt from all fees.

## Execution Fees

Execution fees consist of two components: **base fee** and **priority fee**.

### Base Fee

The base fee follows the same rules as the Ethereum L1 base fee. It adjusts dynamically depending on network congestion to ensure stable transaction pricing.  
By default, base fees are burned. However a sequencer can configure a `base fee vault` address to receive the collected base fees instead of burning them.

```sh
ethrex l2 --block-producer.fee-vault-address <l2-fee-vault-address>
```

> [!CAUTION]  
> If the base fee vault and coinbase addresses are the same, its balance will change in a way that differs from the standard L1 behavior, which may break assumptions about EVM compatibility.

### Priority Fee

The priority fee works exactly the same way as on Ethereum L1.  
It is an additional tip paid by the transaction sender to incentivize the sequencer to prioritize the inclusion of their transaction. The priority fee is always forwarded directly to the sequencer’s coinbase address.

## Operator Fees

Operator fees cover the operational costs of maintaining the L2 infrastructure.

Unlike execution fees, this amount is fixed and does not depend on gas usage, state changes, or network congestion.  
All collected operator fees are deposited into a dedicated `operator fee vault` address.

The operator fee is specified during the contract deployment step. It is initialized in the `OnChainProposer` contract and used as a public input in batch verifications.

To set the operator fee amount:

```
ethrex l2 deploy --operator-fee <amount-in-wei>
```

To set the operator fee vault address:

```
ethrex l2 --block-producer.operator-fee-vault-address <operator-fee-vault-address>
```

> [!CAUTION]  
> If the operator fee vault and coinbase addresses are the same, its balance will change in a way that differs from the standard L1 behavior, which may break assumptions about EVM compatibility.


## L1 Fees
