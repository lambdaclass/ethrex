# Transaction Fees

This page describes the different types of transaction fees that the Ethrex L2 rollup can charge and how they can be configured.

> [!NOTE]  
> Privileged transactions are exempt from all fees.

## Execution Fees

Execution fees are divided into two components: **base fee** and **priority fee**.

### Base Fee

The base fee follows the same rules as the Ethereum L1 base fee. It adjusts dynamically depending on network congestion to ensure stable transaction pricing.  
By default, base fees are burned. However a sequencer can configure a `fee vault` address to receive the collected base fees instead of burning them.

```sh
ethrex l2 --block-producer.fee-vault-address <l2-fee-vault-address>
```

> [!CAUTION]  
> If the fee vault and coinbase addresses are the same, its balance will change in a way that differs from the standard L1 behavior, which may break assumptions about EVM compatibility.

### Priority Fee

The priority fee works exactly the same way as on Ethereum L1.  
It is an additional tip paid by the transaction sender to incentivize the sequencer to prioritize the inclusion of their transaction. The priority fee is always forwarded directly to the sequencer’s coinbase address.

## Operator Fees

## L1 Fees
