# Transaction Fees

This page describes the different types of transaction fees that the Ethrex L2 rollup can charge and how they can be configured.

> [!NOTE]  
> Privileged transactions are exempt from all fees.

## Priority Fee

The priority fee works exactly the same way as on Ethereum L1.  
It is an additional tip paid by the transaction sender to incentivize the sequencer to prioritize the inclusion of their transaction.  
The priority fee is always forwarded directly to the sequencer’s coinbase address.

## Base Fee

The base fee follows the same rules as the Ethereum L1 base fee. It adjusts dynamically depending on network congestion to ensure stable transaction pricing.  
By default, base fees are burned. However, a sequencer can configure a `base fee vault` address to receive the collected base fees instead of burning them.

```sh
ethrex l2 --block-producer.base-fee-vault-address <l2-base-fee-vault-address>
```

> [!CAUTION]  
> If the base fee vault and coinbase addresses are the same, its balance will change in a way that differs from the standard L1 behavior, which may break assumptions about EVM compatibility.


## Operator Fee

The operator fee represents an additional per-gas cost charged by the sequencer to cover the operational costs of maintaining the L2 infrastructure.

This fee works similarly to the base fee — it is **multiplied by the gas used** for each transaction.  
All collected operator fees are deposited into a dedicated `operator fee vault` address.

To set the operator fee amount:

```sh
ethrex l2 --block-producer.operator-fee-per-gas <amount-in-wei>
```

To set the operator fee vault address:

```sh
ethrex l2 --block-producer.operator-fee-vault-address <operator-fee-vault-address>
```

> [!CAUTION]  
> If the operator fee vault and coinbase addresses are the same, its balance will change in a way that differs from the standard L1 behavior, which may break assumptions about EVM compatibility.


---

## Fee Calculation

When executing a transaction, all gas-related fees are subject to the **`max_fee_per_gas`** value defined in the transaction.  
This value acts as an absolute cap over the **sum of all fee components**.

This means that the **effective priority fee** is capped to ensure the total does not exceed `max_fee_per_gas`.  
Specifically:

```
effective_priority_fee_per_gas = min(
    max_priority_fee_per_gas,
    max_fee_per_gas - base_fee_per_gas - operator_fee_per_gas
)
```

Then, the total fees are calculated as:

```sh
total_fees = (base_fee_per_gas + operator_fee_per_gas + priority_fee_per_gas) * gas_used
```

This behavior ensures that transaction senders **never pay more than `max_fee_per_gas * gas_used`**, even when the operator fee is enabled.

> [!IMPORTANT]  
> The current `effective_gas_price` field in the transaction receipt **does not include** the operator fee component.  
> Therefore, `effective_gas_price * gas_used` will only reflect the **base + priority** portions of the total cost.  

> [!IMPORTANT]  
> The `eth_gasPrice` RPC endpoint has been **modified** to include the `operator_fee_per_gas` value when the operator fee mechanism is active.  
> This means that the value returned by `eth_gasPrice` corresponds to `base_fee_per_gas + operator_fee_per_gas + estimated_gas_tip`.

## L1 Fees

L1 fees represent the cost of posting data from the L2 to the L1.  
Each transaction is charged based on the amount of **L1 Blob space** it occupies (the size of the transaction when RLP-encoded).

Each time a transaction is executed, the sequencer calculates its RLP-encoded size.
Then, the L1 fee for that transaction is computed as:

```
l1_fee = blob_base_fee_per_byte * tx_encoded_size
```

An additional amount of gas (`l1_gas`) is added to the transaction execution so that:

```
l1_gas * gas_price = l1_fee
```

This guarantees that the total amount charged to the user never exceeds `gas_limit * gas_price`, while transparently accounting for the L1 posting cost.  
Importantly, this process happens automatically — users do **not** need to perform any additional steps.  
Calls to `eth_estimateGas` already inherit this behavior and will include the extra gas required for the L1 fee.

The computed L1 fee is deducted from the sender’s balance and transferred to the `L1 Fee Vault` address.

The **blob base fee per byte** is derived from the L1 `BlobBaseFee`.  
The `L1Watcher` periodically fetches the `BlobBaseFee` from L1 (at a configured interval) and uses it to compute:

```
blob_base_fee_per_byte = (l1_fee_per_blob_gas * GAS_PER_BLOB) / SAFE_BYTES_PER_BLOB
```

See the `Data availability` section [here](../architecture/overview.md) for more information about how data availability works.


L1 fee is deactivated by default. To activate it, configure the **L1 fee vault address**:

```sh
ethrex l2 --block-producer.l1-fee-vault-address <l1-fee-vault-address>
```

To configure the **interval** at which the `BlobBaseFee` is fetched from L1:

```sh
ethrex l2 --block-producer.blob-base-fee-update-interval <milliseconds>
```

> [!CAUTION]  
> If the L1 fee vault and coinbase addresses are the same, its balance will change in a way that differs from the standard L1 behavior, which may break assumptions about EVM compatibility.


## Useful RPC Methods

The following custom RPC methods are available to query fee-related parameters directly from the L2 node.  
Each method accepts a single argument: the **`block_number`** to query historical or current values.

| Method Name | Description | Example |
|--------------|-------------|----------|
| `ethrex_getBaseFeeVaultAddress` | Returns the address configured to receive the **base fees** collected in the specified block. | ```ethrex_getBaseFeeVaultAddress {"block_number": 12345}``` |
| `ethrex_getOperatorFeeVaultAddress` | Returns the address configured as the **operator fee vault** in the specified block. | ```ethrex_getOperatorFeeVaultAddress {"block_number": 12345}``` |
| `ethrex_getOperatorFee` | Returns the **operator fee per gas** value active at the specified block. | ```ethrex_getOperatorFee {"block_number": 12345}``` |
| `ethrex_getL1BlobBaseFee` | Returns the **L1 blob base fee per gas** fetched from L1 and used for L1 fee computation at the specified block. | ```ethrex_getL1BlobBaseFee {"block_number": 12345}``` |
