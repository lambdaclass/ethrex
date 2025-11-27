# Transfer ETH or Call a contract between L2s

This document explains the steps needed to perform an ETH transfer or a contract call from one L2 to another.

## Start both L2s

### Change directory

Every command should be run under `crates/l2`

```bash
cd crates/l2
```

### Start an L1

```bash
make init-l1
```

### Deploy the first L2

On another terminal

```bash
ETHREX_SHARED_BRIDGE_DEPLOY_ROUTER=true make deploy-l1
```

### Start the first L2

Replace `L1_BRIDGE_ADDRESS`, `L1_ON_CHAIN_PROPOSER_ADDRESS` and `ROUTER_ADDRESS` with the outputs of the previous command, you can also check it under `cmd/.env`.

```bash
../../target/release/ethrex \
	l2 \
	--watcher.block-delay 0 \
	--network ../../fixtures/genesis/l2.json \
	--http.port 1729 \
	--http.addr 0.0.0.0 \
	--metrics \
	--metrics.port 3702 \
	--datadir dev_ethrex_l2 \
	--l1.bridge-address <L1_BRIDGE_ADDRESS> \
	--l1.on-chain-proposer-address <L1_ON_CHAIN_PROPOSER_ADDRESS> \
	--eth.rpc-url http://localhost:8545 \
	--osaka-activation-time 1761677592 \
	--block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
	--block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
	--block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
	--block-producer.operator-fee-per-gas 1000000000 \
	--committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
	--proof-coordinator.addr 127.0.0.1 \
    --l1.router-address <ROUTER_ADDRESS> \
    --watcher.l2-rpcs http://localhost:1730 \
    --watcher.l2-chain-ids 1730
```

### Deploy the second L2

On another terminal

Copy the `../../fixtures/genesis/l2.json` file to `../../fixtures/genesis/l2_2.json` and modify chain id to 1730

Replace `ROUTER_ADDRESS` with the outputs of the first deploy

```bash
../../target/release/ethrex l2 deploy \
	--eth-rpc-url http://localhost:8545 \
	--private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--on-chain-proposer-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
	--bridge-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
	--deposit-rich \
	--private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
	--genesis-l1-path ../../fixtures/genesis/l1-dev.json \
	--genesis-l2-path ../../fixtures/genesis/l2_2.json \
    --randomize-contract-deployment \
    --router.address <ROUTER_ADDRESS>
```


### Start the second L2

Replace `L1_BRIDGE_ADDRESS` and `L1_ON_CHAIN_PROPOSER_ADDRESS` with the outputs of the previous command, you can also check it under `cmd/.env`.
And `ROUTER_ADDRESS` with the outputs of the first deploy


```bash
../../target/release/ethrex \
	l2 \
	--watcher.block-delay 0 \
	--network ../../fixtures/genesis/l2_2.json \
	--http.port 1730 \
	--http.addr 0.0.0.0 \
	--metrics \
	--metrics.port 3703 \
	--datadir dev_ethrex_l2_2 \
	--l1.bridge-address <L1_BRIDGE_ADDRESS> \
	--l1.on-chain-proposer-address <L1_ON_CHAIN_PROPOSER_ADDRESS> \
	--eth.rpc-url http://localhost:8545 \
	--osaka-activation-time 1761677592 \
	--block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
	--block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
	--block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
	--block-producer.operator-fee-per-gas 1000000000 \
	--committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
	--proof-coordinator.addr 127.0.0.1 \
    --proof-coordinator.port 3901 \
    --l1.router-address <ROUTER_ADDRESS> \
    --watcher.l2-rpcs http://localhost:1729 \
    --watcher.l2-chain-ids 65536999
```


### Start the prover

On another terminal 

```bash
../../target/release/ethrex \
	l2 prover \
	--proof-coordinators tcp://127.0.0.1:3900 tcp://127.0.0.1:3901 \
	--backend exec
```

## ETH Transfer

### Check balances

Check the balances before sending the transfer

```bash
rex balance 0xe25583099ba105d9ec0a67f5ae86d90e50036425 http://localhost:1729 # Receiver balance on first L2
rex balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776 http://localhost:1730 # Sender balance on second L2
```


### Send the transfer

```bash
cast send --rpc-url http://localhost:1730 --private-key 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 --value 10000000000000001 0x000000000000000000000000000000000000FFFF 'sendToL2(uint256,address,uint256,bytes)' 65536999 0xe25583099ba105d9ec0a67f5ae86d90e50036425 100000 0x --gas-price 3946771033 --legacy
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

Remember the contract address for the next steps

### Check counter value

```bash
rex call <COUNTER_ADDRESS> "get()" --rpc-url http://localhost:1729
```

### Increase the counter from the other L2

```bash
cast send --rpc-url http://localhost:1730 --private-key 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 0x000000000000000000000000000000000000FFFF 'sendToL2(uint256,address,uint256,bytes)' 65536999 <COUNTER_ADDRESS> 100000 0xd09de08a --gas-price 3946771033 --legacy
```

### Check counter value

```bash
rex call <COUNTER_ADDRESS> "get()" --rpc-url http://localhost:1729
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

Remember the contract address for the next steps

### Check counter value

```bash
rex call <COUNTER_ADDRESS> "get()" --rpc-url http://localhost:1729
```

### Check counter balance

```bash
rex balance <COUNTER_ADDRESS> http://localhost:1729
```

### Increase the counter from the other L2

```bash
cast send --rpc-url http://localhost:1730 --private-key 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 --value 1000 0x000000000000000000000000000000000000FFFF 'sendToL2(uint256,address,uint256,bytes)' 65536999 <COUNTER_ADDRESS> 100000 0xd09de08a --gas-price 3946771033 --legacy
```

### Check counter value

```bash
rex call <COUNTER_ADDRESS> "get()" --rpc-url http://localhost:1729
```

### Check counter balance

```bash
rex balance <COUNTER_ADDRESS> http://localhost:1729
```

## ERC20 Transfer

### Add the token contract

Create a `TestToken.sol` file with the following content

```solidity
// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/// @title Example L2-side bridgeable token
/// @author LambdaClass
contract TestToken is ERC20 {
    address public constant BRIDGE =  0x000000000000000000000000000000000000FFff;

    constructor() ERC20("TestTokenL2", "TEST") {
        _mint(msg.sender, 1_000_000 * (10 ** 18));
    }

    modifier onlyBridge() {
        require(msg.sender == BRIDGE, "TestToken: not authorized to mint");
        _;
    }

    function crosschainMint(address destination, uint256 amount) external onlyBridge {
        _mint(destination, amount);
    }

    function crosschainBurn(address from, uint256 value) external onlyBridge {
        _burn(from, value);
    }
}
```

### Deploy the contract on both L2s

Destination L2:

```bash
rex deploy 0 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
    --rpc-url http://localhost:1729 \
    --contract-path TestToken.sol \
    --remappings "@openzeppelin=https://github.com/OpenZeppelin/openzeppelin-contracts.git"
```

Source L2:

```bash
rex deploy 0 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
    --rpc-url http://localhost:1730 \
    --contract-path TestToken.sol \
    --remappings "@openzeppelin=https://github.com/OpenZeppelin/openzeppelin-contracts.git" 
```

Remember both addresses

### Check balances

Source L2:

```bash
rex call <TOKEN_ADDRESS_L2_SOURCE> "balanceOf(address)" 0x4417092b70a3e5f10dc504d0947dd256b965fc62  --rpc-url http://localhost:1730
```

Destination L2:

```bash
rex call <TOKEN_ADDRESS_L2_DESTINATION> "balanceOf(address)" 0x8943545177806ed17b9f23f0a21ee5948ecaa776 --rpc-url http://localhost:1729
```

### Send the transfer

```bash
rex send --rpc-url http://localhost:1730 --private-key 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e 0x000000000000000000000000000000000000FFFF 'sendERC20ToL2(uint256,address,uint256,address,address,uint256)' 65536999 0x8943545177806ed17b9f23f0a21ee5948ecaa776 100000 <TOKEN_ADDRESS_L2_SOURCE> <TOKEN_ADDRESS_L2_DESTINATION> 100 --gas-price 3946771033
```

### Check balances

Source L2:

```bash
rex call <TOKEN_ADDRESS_L2_SOURCE> "balanceOf(address)" 0x4417092b70a3e5f10dc504d0947dd256b965fc62 --rpc-url http://localhost:1730
```

Destination L2:

```bash
rex call <TOKEN_ADDRESS_L2_DESTINATION> "balanceOf(address)" 0x8943545177806ed17b9f23f0a21ee5948ecaa776 --rpc-url http://localhost:1729
```

## Troubleshooting

If you can't deploy the counter contract, either because of `Transaction intrinsic gas overflow` or because the transaction is never included in a block.
Retry the deploy command adding `--priority-gas-price` and `--gas-price` with the same value, increment it by 10 until it deploys correctly.
