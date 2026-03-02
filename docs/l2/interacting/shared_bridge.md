# Interacting with the shared bridge

This document details different scenarios for interacting with shared bridge enabled L2s.

## Prerequisites

This guide assumes you already have two L2s running with the shared bridge enabled.
Refer to [Deploy a shared bridge enabled L2](../deployment/shared_bridge.md)

## ETH Transfer

### Check balances

Check the balances before sending the transfer

```bash
rex balance 0xe25583099ba105d9ec0a67f5ae86d90e50036425 http://localhost:1729 # Receiver balance on first L2
rex balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776 http://localhost:1730 # Sender balance on second L2
```


### Send the transfer

```bash
rex send --rpc-url http://localhost:1730 --private-key 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 --value 10000000000000001 0x000000000000000000000000000000000000FFFF 'sendToL2(uint256,address,uint256,bytes)' 65536999 0xe25583099ba105d9ec0a67f5ae86d90e50036425 100000 "" --gas-price 3946771033
```


### Check balances

After some time the balances should change (about 1-2 minutes)

```bash
rex balance 0xe25583099ba105d9ec0a67f5ae86d90e50036425 http://localhost:1729 # Receiver balance on first L2
rex balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776 http://localhost:1730 # Sender balance on second L2
```

## Contract Call

### Add the contract

Create a `Counter.sol` file with the following content

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract Counter {
    uint256 public count;

    function increment() external {
        count += 1;
    }

    function get() external view returns (uint256) {
        return count;
    }
}
```

### Deploy the contract

```bash
rex deploy --rpc-url http://localhost:1729 --remappings 0 --contract-path ./Counter.sol 0 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31
```

Save the contract address for the next steps:

```bash
export COUNTER_ADDRESS=<COUNTER_ADDRESS> 
```

### Check counter value

```bash
rex call $COUNTER_ADDRESS "get()" --rpc-url http://localhost:1729
```

### Increase the counter from the other L2

```bash
rex send --rpc-url http://localhost:1730 --private-key 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 0x000000000000000000000000000000000000FFFF 'sendToL2(uint256,address,uint256,bytes)' 65536999 $COUNTER_ADDRESS 100000 d09de08a --gas-price 3946771033
```

### Check counter value

```bash
rex call $COUNTER_ADDRESS "get()" --rpc-url http://localhost:1729
```

## Contract Call and ETH Transfer

### Add the contract

Create a `Counter.sol` file with the following content (The increment function is now payable)

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract Counter {
    uint256 public count;

    function increment() external payable {
        count += 1;
    }

    function get() external view returns (uint256) {
        return count;
    }
}
```

### Deploy the contract

```bash
rex deploy --rpc-url http://localhost:1729 --remappings 0 --contract-path ./Counter.sol 0 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31
```

Save the contract address for the next steps:

```bash
export COUNTER_ADDRESS=<COUNTER_ADDRESS> 
```

### Check counter value

```bash
rex call $COUNTER_ADDRESS "get()" --rpc-url http://localhost:1729
```

### Check counter balance

```bash
rex balance $COUNTER_ADDRESS http://localhost:1729
```

### Increase the counter from the other L2

```bash
rex send --rpc-url http://localhost:1730 --private-key 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 --value 1000 0x000000000000000000000000000000000000FFFF 'sendToL2(uint256,address,uint256,bytes)' 65536999 $COUNTER_ADDRESS 100000 d09de08a --gas-price 3946771033
```

### Check counter value

```bash
rex call $COUNTER_ADDRESS "get()" --rpc-url http://localhost:1729
```

### Check counter balance

```bash
rex balance $COUNTER_ADDRESS http://localhost:1729
```

## Troubleshooting

If you can't deploy the counter contract, either because of `Transaction intrinsic gas overflow` or because the transaction is never included in a block.
Retry the deploy command adding `--priority-gas-price` and `--gas-price` with the same value, increment it by 10 until it deploys correctly.
