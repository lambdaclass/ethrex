# Maintain the sequencer

## L2 Gas Limit

The L2 block gas limit is stored on-chain in the `CommonBridge` contract. The sequencer fetches this value on startup and uses it to configure block production.

### Viewing the current gas limit

```shell
rex call <BRIDGE_ADDRESS> "l2GasLimit()" --rpc-url <L1_RPC_URL>
```

### Updating the gas limit

Only the bridge owner can update the gas limit:

```shell
rex send <BRIDGE_ADDRESS> "setL2GasLimit(uint256)" <NEW_GAS_LIMIT> \
  --private-key <OWNER_PRIVATE_KEY> \
  --rpc-url <L1_RPC_URL>
```

After updating the on-chain value, restart the sequencer for it to take effect. Until the restart, the sequencer continues using the previous gas limit, which may cause a temporary mismatch with the on-chain value.
