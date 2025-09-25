# Configuration

This page covers the advanced configuration options for running an L2 node with ethrex.

## Base fee Configuration

You can configure the base fee behavior in ethrex.

Set a fee vault address with:

```sh
ethrex l2 --block-producer.fee-vault-address <l2-fee-vault-address>
```

When configured, the sequencer redirects collected base fees to the specified address instead of burning them. The sequencer may designate any address as the fee vault, including the coinbase address.

> [!CAUTION]  
> If the fee vault and coinbase addresses are the same, its balance will change in a way that differs from the standard L1 behavior, which may break assumptions about EVM compatibility.


