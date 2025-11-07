# Steps to test a transfer between 2 L2s

## Start an L1

```bash
make init-l1
```

## Deploy the first L2

```bash
ETHREX_SHARED_BRIDGE_DEPLOY_ROUTER=true make deploy-l1
```

## Start the first L2

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
	--l1.bridge-address 0xc12f570116c82f1ff70d4cc1e75b178578870570 \
	--l1.on-chain-proposer-address 0x38fd92f2ad8da983e2bbf7cde41f7c35eba24b44 \
	--eth.rpc-url http://localhost:8545 \
	--osaka-activation-time 1761677592 \
	--block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
	--block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
	--block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
	--block-producer.operator-fee-per-gas 1000000000 \
	--committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
	--proof-coordinator.addr 127.0.0.1 \
    --l1.router-address 0x2bc74c22739625e06609ac16eea025f31fd350e3 \
    --watcher.l2-rpcs http://localhost:1730 \
    --watcher.l2-chain-ids 1730
```

## Deploy the second L2

Copy the `../../fixtures/genesis/l2.json` file to `../../fixtures/genesis/l2_2.json` and modify chain id to 1730

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
    --router.address 0x2bc74c22739625e06609ac16eea025f31fd350e3
```


## Start the second L2

Replace `L1_BRIDGE_ADDRESS` and `L1_ON_CHAIN_PROPOSER_ADDRESS`

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
    --l1.router-address 0x2bc74c22739625e06609ac16eea025f31fd350e3 \
    --watcher.l2-rpcs http://localhost:1729 \
    --watcher.l2-chain-ids 65536999
```


## Start the prover

```bash
../../target/release/ethrex \
	l2 prover \
	--proof-coordinators tcp://127.0.0.1:3900 tcp://127.0.0.1:3901 \
	--backend exec
```


## Check balances

```bash
rex balance 0x4417092b70a3e5f10dc504d0947dd256b965fc62 http://localhost:1729 # Receiver balance on first L2
rex balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776 http://localhost:1730 # Sender balance on second L2
```


## Send the transfer

```bash
cast send --rpc-url http://localhost:1730 --private-key 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 --value 10000000000000001 0x000000000000000000000000000000000000FFFF 'sendToL2(uint256,address,uint256,bytes)' 65536999 0x4417092b70a3e5f10dc504d0947dd256b965fc62 100000 0x --gas-price 3946771033 --legacy
```


## Check balances

```bash
rex balance 0x4417092b70a3e5f10dc504d0947dd256b965fc62 http://localhost:1729 # Receiver balance on first L2
rex balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776 http://localhost:1730 # Sender balance on second L2
```

